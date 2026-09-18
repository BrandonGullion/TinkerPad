//! The helper pool: one warm helper per plugin (work item 03-02).
//!
//! A helper is the thing that makes the second run fast. It starts the first
//! time something needs it, stays up between runs, comes back after a crash
//! until it has crashed too often, and stops when nothing has used it for a
//! while — the same life a language server has, and run by the same code
//! (`tinkerpad_process::supervise`).
//!
//! Two things are the helper's own. A plugin may ship its helper as *source*
//! (`[helper.build]`), so the first use builds it, and only the first: a hash
//! of the helper folder is kept beside the app's data, and the build runs
//! again only when that hash changes. And a helper's work is runs rather than
//! documents, so the idle clock is held by [`Use`] guards that the run path
//! takes out (03-03).
//!
//! Running code on a helper is 03-03; hooks are 03-07. This module is only
//! about there being one, warm, when they are wanted.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use tinkerpad_core::{CoreEvent, LanguageId, Level, ServerState};
use tinkerpad_plugin::helper::{HelperClient, HelperStart};
use tinkerpad_process::supervise::{Crash, Lifecycle, Policy, Pool, SHUTDOWN_BUDGET, Supervised};
use tinkerpad_process::{ProcessEvent, SpawnSpec};
use tokio::sync::{Mutex, broadcast, mpsc};
use tracing::{debug, info, warn};

/// How long a helper's build may take before it is given up on. Generous: a
/// first `dotnet build` on a cold machine is minutes, not seconds. A build
/// that hits this is stuck, not slow.
pub const BUILD_TIMEOUT: Duration = Duration::from_secs(600);

/// How many build output lines are held for a subscriber that is slow.
const BUILD_LINES: usize = 1024;

/// What the pool needs to start one helper.
///
/// Plain data, like [`ServerConfig`](tinkerpad_lsp::ServerConfig) next door:
/// the app builds it from the plugin's `[helper]` section with the same
/// placeholder and toolchain resolution a `[run]` command gets (01-06), which
/// keeps this crate free of the manifest format.
#[derive(Clone, Debug)]
pub struct HelperConfig {
    /// The plugin id. One helper per plugin, so this is the pool's key.
    pub plugin: String,
    /// How to start it, already resolved to a program path.
    pub spawn: SpawnSpec,
    /// `[helper.build]`, if the plugin ships its helper as source.
    pub build: Option<SpawnSpec>,
    /// The folder whose contents decide whether a build is needed. Usually the
    /// plugin's `helper/` folder.
    pub source_dir: Option<PathBuf>,
    /// Where the build stamp is kept: `<app-data>/helpers`.
    pub stamp_dir: PathBuf,
    /// The plugin folder, passed through in `hello`.
    pub plugin_dir: PathBuf,
    /// A folder private to this plugin that the helper may write to.
    pub scratch_dir: PathBuf,
    /// The hooks the manifest says this helper implements.
    pub hooks: Vec<String>,
    /// Shown when the helper can't be started at all.
    pub install_hint: Option<String>,
    /// How this helper is supervised: backoff, idle shutdown, watch tick.
    pub policy: Policy,
    /// How long the helper has to answer `hello`.
    pub ready_timeout: Duration,
}

impl HelperConfig {
    pub fn new(
        plugin: impl Into<String>,
        spawn: SpawnSpec,
        plugin_dir: impl Into<PathBuf>,
        scratch_dir: impl Into<PathBuf>,
    ) -> Self {
        let scratch_dir = scratch_dir.into();
        Self {
            plugin: plugin.into(),
            spawn,
            build: None,
            source_dir: None,
            stamp_dir: scratch_dir.clone(),
            plugin_dir: plugin_dir.into(),
            scratch_dir,
            hooks: Vec::new(),
            install_hint: None,
            policy: Policy::default(),
            ready_timeout: tinkerpad_plugin::helper::READY_TIMEOUT,
        }
    }

    /// The build command, and the folder whose changes make it worth running
    /// again.
    pub fn with_build(mut self, build: SpawnSpec, source_dir: impl Into<PathBuf>) -> Self {
        self.build = Some(build);
        self.source_dir = Some(source_dir.into());
        self
    }

    pub fn with_stamp_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.stamp_dir = dir.into();
        self
    }

    pub fn with_hooks<I, S>(mut self, hooks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.hooks = hooks.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_policy(mut self, policy: Policy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_ready_timeout(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }

    pub fn with_install_hint(mut self, hint: impl Into<String>) -> Self {
        self.install_hint = Some(hint.into());
        self
    }

    /// Where this helper's build stamp lives.
    fn stamp_file(&self) -> PathBuf {
        self.stamp_dir.join(format!("{}.build", self.plugin))
    }

    fn start(&self) -> HelperStart {
        HelperStart {
            name: self.plugin.clone(),
            spawn: self.spawn.clone(),
            plugin_dir: self.plugin_dir.clone(),
            scratch_dir: self.scratch_dir.clone(),
            hooks: self.hooks.clone(),
            ready_timeout: self.ready_timeout,
        }
    }
}

/// Why there is no helper to run on. Every one of these means the same thing
/// to the run path: use the plugin's `[run]` command instead (03-03).
#[derive(Clone, Debug, thiserror::Error)]
pub enum Unavailable {
    #[error("the {plugin} helper could not be built: {reason}")]
    Build { plugin: String, reason: String },
    #[error("{reason}")]
    Start { plugin: String, reason: String },
    #[error("the {plugin} helper stopped working: {reason}")]
    GaveUp { plugin: String, reason: String },
}

impl Unavailable {
    pub fn plugin(&self) -> &str {
        match self {
            Unavailable::Build { plugin, .. }
            | Unavailable::Start { plugin, .. }
            | Unavailable::GaveUp { plugin, .. } => plugin,
        }
    }
}

/// One plugin's helper: the supervision around a [`HelperClient`], which is
/// replaced each time the process is.
#[derive(Debug)]
pub struct HelperHandle {
    config: HelperConfig,
    life: Lifecycle,
    /// The running client. The lock is held across a start, which is what
    /// makes two callers racing for a cold helper wait for one start rather
    /// than begin two.
    client: Mutex<Option<Arc<HelperClient>>>,
    /// How many runs (or hooks) are using it. The idle clock runs at zero.
    uses: AtomicUsize,
    /// Set while a stop is deliberate, so the exit that follows isn't counted
    /// as a crash.
    stopping: AtomicBool,
    /// Whether the idle watch is already running for this handle. One task per
    /// helper, not per process, so a restart doesn't lose the clock.
    watching: AtomicBool,
    /// Build output, for whoever is waiting on the build to show it (03-03).
    build_output: broadcast::Sender<String>,
}

/// Something is using a helper. While one of these is alive the idle clock is
/// stopped; dropping the last one starts it.
#[derive(Debug)]
pub struct Use {
    handle: Arc<HelperHandle>,
}

impl Drop for Use {
    fn drop(&mut self) {
        if self.handle.uses.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.handle.life.idle();
        }
    }
}

impl Use {
    pub fn helper(&self) -> &Arc<HelperHandle> {
        &self.handle
    }
}

impl HelperHandle {
    /// A handle for a helper that has not been started.
    pub fn prepare(config: &HelperConfig) -> Arc<Self> {
        Arc::new(Self {
            life: Lifecycle::new(config.plugin.clone(), "helper", config.policy),
            config: config.clone(),
            client: Mutex::default(),
            uses: AtomicUsize::new(0),
            stopping: AtomicBool::new(false),
            watching: AtomicBool::new(false),
            build_output: broadcast::channel(BUILD_LINES).0,
        })
    }

    pub fn plugin(&self) -> &str {
        &self.config.plugin
    }

    pub fn config(&self) -> &HelperConfig {
        &self.config
    }

    /// Lines the build wrote, for a run that is waiting on it. Subscribe
    /// before asking for the client, or the first lines are missed.
    pub fn build_output(&self) -> broadcast::Receiver<String> {
        self.build_output.subscribe()
    }

    /// The last few things this helper said, for the status strip (02-08).
    pub fn log(&self) -> Vec<String> {
        self.life.log()
    }

    /// What it is doing: the same states a language server has.
    pub fn state(&self) -> ServerState {
        self.life.state()
    }

    /// State changes, for the pool to forward to the UI.
    pub fn watch_state(&self) -> tokio::sync::watch::Receiver<ServerState> {
        self.life.watch_state()
    }

    /// How long it has had nothing to do, if it has nothing to do.
    pub fn idle_for(&self) -> Option<Duration> {
        self.life.idle_for()
    }

    /// How many times it has crashed and been started again in a row.
    pub fn restarts(&self) -> u32 {
        self.life.restarts()
    }

    /// How long the current process has been up.
    pub fn uptime(&self) -> Option<Duration> {
        self.life.uptime()
    }

    /// The process id, or 0 when it isn't running.
    pub fn pid(&self) -> u32 {
        Supervised::pid(self)
    }

    /// Stop it politely: `shutdown`, a grace, then the axe.
    pub async fn stop(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        let client = self.client.lock().await.take();
        let Some(client) = client else {
            self.life.set_state(ServerState::stopped());
            return;
        };
        self.life.set_state(ServerState::Stopping);
        client.shutdown().await;
        self.life.set_state(ServerState::stopped());
    }

    /// End it now, without asking.
    pub fn kill(&self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Ok(slot) = self.client.try_lock()
            && let Some(client) = slot.as_ref()
        {
            client.kill();
        }
    }

    /// The running client, starting the helper — and building it — if needed.
    ///
    /// Concurrent callers wait on the one start. A helper that is already up
    /// is handed over without a lock being held for long.
    pub async fn client(self: &Arc<Self>) -> Result<Arc<HelperClient>, Unavailable> {
        let mut slot = self.client.lock().await;
        if let Some(client) = slot.as_ref()
            && client.is_alive()
        {
            return Ok(client.clone());
        }
        // Whatever is in the slot is dead; a start from here is deliberate.
        *slot = None;
        self.stopping.store(false, Ordering::Relaxed);

        if let ServerState::Stopped {
            reason: Some(reason),
            ..
        } = self.life.state()
            && self.life.restarts() > self.config.policy.max_restarts
        {
            // Given up on after too many crashes. It comes back when someone
            // asks for it by name (the Restart button), not on its own.
            return Err(Unavailable::GaveUp {
                plugin: self.config.plugin.clone(),
                reason,
            });
        }

        self.life.set_state(ServerState::Starting);
        if let Err(reason) = self.build_if_needed().await {
            self.life.set_state(ServerState::failed(
                reason.clone(),
                self.config.install_hint.clone(),
            ));
            return Err(Unavailable::Build {
                plugin: self.config.plugin.clone(),
                reason,
            });
        }

        let client = match HelperClient::start(self.config.start()).await {
            Ok(client) => Arc::new(client),
            Err(e) => {
                let reason = e.to_string();
                warn!(plugin = %self.config.plugin, error = %reason, "the helper did not start");
                self.life.set_state(ServerState::failed(
                    reason.clone(),
                    self.config.install_hint.clone(),
                ));
                return Err(Unavailable::Start {
                    plugin: self.config.plugin.clone(),
                    reason,
                });
            }
        };

        self.life.starting_now();
        self.life.set_state(ServerState::Ready {
            pid: client.pid(),
            name: client.helper_name().map(str::to_owned),
            version: client.version().map(str::to_owned),
            rss_bytes: None,
        });
        info!(
            plugin = %self.config.plugin,
            pid = client.pid(),
            "helper ready",
        );
        *slot = Some(client.clone());
        self.watch_for_exit(client.clone());
        if !self.watching.swap(true, Ordering::AcqRel) {
            tokio::spawn(tinkerpad_process::supervise::watch(self.clone()));
        }
        Ok(client)
    }

    /// Take out a use: while one is held the helper is not idle.
    pub fn using(self: &Arc<Self>) -> Use {
        self.uses.fetch_add(1, Ordering::AcqRel);
        self.life.busy();
        Use {
            handle: self.clone(),
        }
    }

    /// How many runs or hooks are using it now.
    pub fn uses(&self) -> usize {
        self.uses.load(Ordering::Relaxed)
    }

    /// Start it again now, forgetting how often it has crashed. The status
    /// strip's Restart button, and what asking for a helper the pool gave up
    /// on means.
    pub fn restart(self: &Arc<Self>) {
        self.life.forget_crashes();
        let handle = self.clone();
        tokio::spawn(async move {
            handle.stop().await;
            if let Err(e) = handle.client().await {
                debug!(error = %e, "the helper did not come back");
            }
        });
    }

    /// Watch the process, and put it back when it dies of its own accord.
    fn watch_for_exit(self: &Arc<Self>, client: Arc<HelperClient>) {
        let handle = self.clone();
        tokio::spawn(async move {
            client.wait_exit().await;
            // A stop we asked for is not a crash.
            if handle.stopping.load(Ordering::Relaxed) {
                handle.life.set_state(ServerState::stopped());
                return;
            }
            let reason = "the helper exited".to_owned();
            match handle.life.note_crash() {
                Crash::GiveUp { count } => {
                    warn!(
                        plugin = %handle.config.plugin,
                        crashes = count,
                        "giving up on the helper",
                    );
                    handle.life.set_state(ServerState::failed(
                        format!("it crashed {count} times in a row"),
                        Some(
                            "Restart it from the status strip when you have fixed the cause."
                                .to_owned(),
                        ),
                    ));
                }
                Crash::Restart { after, count } => {
                    warn!(
                        plugin = %handle.config.plugin,
                        crashes = count,
                        wait_ms = after.as_millis() as u64,
                        "the helper crashed; starting it again",
                    );
                    handle.life.set_state(ServerState::Crashed {
                        reason,
                        restarts: count,
                    });
                    tokio::time::sleep(after).await;
                    // Someone may have stopped or restarted it while we waited.
                    if !matches!(handle.life.state(), ServerState::Crashed { .. }) {
                        return;
                    }
                    // Warm again before anything asks: a helper that is only
                    // started by the next run is not a warm runner.
                    if let Err(e) = handle.client().await {
                        debug!(error = %e, "the helper did not come back");
                    }
                }
            }
        });
    }

    /// Build the helper if its source has changed since the last build.
    ///
    /// The stamp is a hash of the source folder — names, sizes, and modified
    /// times — kept beside the app's data. Cheap to compute, and wrong only in
    /// the direction of building again when it needn't have.
    async fn build_if_needed(&self) -> Result<(), String> {
        let (Some(build), Some(source)) = (&self.config.build, &self.config.source_dir) else {
            return Ok(());
        };
        let stamp = self.config.stamp_file();
        let hash = source_hash(source);
        let last = std::fs::read_to_string(&stamp).ok();
        if last.as_deref() == Some(hash.as_str()) {
            debug!(plugin = %self.config.plugin, "the helper is already built");
            return Ok(());
        }

        let why = match last {
            Some(_) => "its source changed",
            None => "it has not been built here",
        };
        info!(plugin = %self.config.plugin, why, "building the helper");
        self.say(format!(
            "Building the {} helper… ({why})",
            self.config.plugin
        ));

        let started = std::time::Instant::now();
        let code = tokio::time::timeout(BUILD_TIMEOUT, self.run_build(build.clone()))
            .await
            .map_err(|_| {
                format!(
                    "the build took longer than {} minutes",
                    BUILD_TIMEOUT.as_secs() / 60
                )
            })??;
        if code != Some(0) {
            return Err(match code {
                Some(code) => format!("the build command exited {code}"),
                None => "the build command was killed".to_owned(),
            });
        }

        let ms = started.elapsed().as_millis() as u64;
        self.say(format!(
            "Built the {} helper in {ms} ms.",
            self.config.plugin
        ));
        if let Some(parent) = stamp.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::write(&stamp, &hash) {
            // Not fatal: the worst of it is building again next time.
            warn!(error = %e, stamp = %stamp.display(), "could not record the build");
        }
        Ok(())
    }

    /// Run the build command, putting every line it writes where a waiting run
    /// can show it.
    async fn run_build(&self, spec: SpawnSpec) -> Result<Option<i32>, String> {
        let mut child = tinkerpad_process::spawn(spec).map_err(|e| e.to_string())?;
        let mut line = Vec::new();
        while let Some(event) = child.recv().await {
            let bytes = match event {
                ProcessEvent::Stdout(bytes) | ProcessEvent::Stderr(bytes) => bytes,
                ProcessEvent::Exited(exit) => {
                    if !line.is_empty() {
                        self.say(String::from_utf8_lossy(&line).trim_end().to_owned());
                    }
                    return Ok(exit.code);
                }
            };
            for &byte in bytes.iter() {
                if byte == b'\n' {
                    self.say(String::from_utf8_lossy(&line).trim_end().to_owned());
                    line.clear();
                } else {
                    line.push(byte);
                }
            }
        }
        Err("the build command went away".to_owned())
    }

    /// Say something about this helper: into its log for the status strip, and
    /// to anyone watching a build.
    fn say(&self, line: String) {
        if line.is_empty() {
            return;
        }
        debug!(plugin = %self.config.plugin, "{line}");
        self.life.note_log(line.clone());
        let _ = self.build_output.send(line);
    }
}

impl Supervised for HelperHandle {
    fn lifecycle(&self) -> &Lifecycle {
        &self.life
    }

    fn pid(&self) -> u32 {
        // `try_lock` rather than a wait: this is called by the watch task
        // every few seconds, and a helper in the middle of starting has no pid
        // worth reporting yet.
        match self.client.try_lock() {
            Ok(slot) => slot.as_ref().map(|c| c.pid()).unwrap_or(0),
            Err(_) => 0,
        }
    }

    fn stop(&self) -> impl std::future::Future<Output = ()> + Send {
        HelperHandle::stop(self)
    }

    fn kill(&self) {
        HelperHandle::kill(self);
    }
}

/// Every plugin helper the app has going.
#[derive(Debug)]
pub struct HelperPool {
    helpers: Pool<String, HelperHandle>,
    /// State changes for the UI. The app forwards these to its event bus.
    events: Option<mpsc::UnboundedSender<CoreEvent>>,
}

impl Default for HelperPool {
    fn default() -> Self {
        Self::new(None)
    }
}

impl HelperPool {
    pub fn new(events: Option<mpsc::UnboundedSender<CoreEvent>>) -> Self {
        Self {
            helpers: Pool::new(),
            events,
        }
    }

    /// The helper for a plugin, started (and built) if this is the first call.
    pub async fn get(&self, config: &HelperConfig) -> Result<Arc<HelperClient>, Unavailable> {
        self.handle(config).client().await
    }

    /// The handle for a plugin's helper, without starting anything.
    ///
    /// This is what the run path holds: it can take a [`Use`] out before the
    /// helper is up, and watch the build output while it starts (03-03).
    pub fn handle(&self, config: &HelperConfig) -> Arc<HelperHandle> {
        let (handle, fresh) = self.helpers.get_or_make(
            &config.plugin,
            || {
                info!(plugin = %config.plugin, "preparing the helper");
                HelperHandle::prepare(config)
            },
            |_| {},
        );
        if fresh {
            self.publish(&handle);
        }
        handle
    }

    /// The helper for a plugin, if the pool has one.
    pub fn get_handle(&self, plugin: &str) -> Option<Arc<HelperHandle>> {
        self.helpers.get(&plugin.to_owned())
    }

    /// Every helper, for the status strip (02-08).
    pub fn helpers(&self) -> Vec<Arc<HelperHandle>> {
        self.helpers.all()
    }

    pub fn len(&self) -> usize {
        self.helpers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.helpers.is_empty()
    }

    /// Restart one helper now, whatever state it is in.
    pub fn restart(&self, plugin: &str) -> bool {
        match self.get_handle(plugin) {
            Some(handle) => {
                handle.restart();
                true
            }
            None => false,
        }
    }

    /// Stop one helper now.
    pub async fn stop(&self, plugin: &str) -> bool {
        match self.get_handle(plugin) {
            Some(handle) => {
                handle.stop().await;
                true
            }
            None => false,
        }
    }

    /// Stop everything, at app exit, alongside the language servers.
    pub async fn shutdown(&self) {
        self.helpers.shutdown(SHUTDOWN_BUDGET).await;
    }

    /// Forward a helper's state changes to the UI until it stops for good.
    fn publish(&self, handle: &Arc<HelperHandle>) {
        let Some(events) = self.events.clone() else {
            return;
        };
        let handle = handle.clone();
        tokio::spawn(async move {
            let mut states = handle.life.watch_state();
            loop {
                let state = states.borrow_and_update().clone();
                let given_up = matches!(
                    state,
                    ServerState::Stopped {
                        reason: Some(_),
                        ..
                    }
                );
                if let ServerState::Crashed { restarts, .. } = &state {
                    let _ = events.send(CoreEvent::Toast {
                        level: Level::Warning,
                        message: format!(
                            "The {} helper crashed, restarting… (attempt {restarts})",
                            handle.plugin()
                        ),
                    });
                }
                let event = CoreEvent::Helper {
                    plugin: LanguageId::new(handle.plugin()),
                    state,
                };
                if events.send(event).is_err() || given_up {
                    return;
                }
                if states.changed().await.is_err() {
                    return;
                }
            }
        });
    }
}

/// A hash of what is in a folder: every file's path relative to it, its size,
/// and when it was last modified. Deliberately not the contents — a helper's
/// source tree can be large, and this is only deciding whether to run a build
/// that will do its own dependency checking anyway.
///
/// FNV-1a rather than the standard library's hasher, because this number is
/// written to disk and read back by a later build of TinkerPad.
pub fn source_hash(dir: &Path) -> String {
    let mut files = Vec::new();
    collect(dir, dir, &mut files);
    files.sort();

    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for entry in &files {
        for byte in entry.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{hash:016x}")
}

/// Every file under `dir`, as `path|size|modified`.
fn collect(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else { continue };
        if meta.is_dir() {
            // Build output is not source: it changes because of the build, and
            // hashing it would mean every build asks for another.
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if matches!(&*name, "bin" | "obj" | "target" | "node_modules" | ".git") {
                continue;
            }
            collect(root, &path, out);
            continue;
        }
        let modified = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let relative = path.strip_prefix(root).unwrap_or(&path);
        out.push(format!("{}|{}|{modified}", relative.display(), meta.len()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("tinkerpad-helpers-{}", std::process::id()))
            .join(name);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_hash_follows_what_is_in_the_folder() {
        let dir = scratch("hash");
        let file = dir.join("helper.cs");
        std::fs::write(&file, "class Helper {}").unwrap();
        let first = source_hash(&dir);
        assert_eq!(first, source_hash(&dir), "the same folder, the same hash");

        std::fs::write(&file, "class Helper { int x; }").unwrap();
        assert_ne!(first, source_hash(&dir), "a changed file shows");

        let second = source_hash(&dir);
        std::fs::write(dir.join("extra.cs"), "class More {}").unwrap();
        assert_ne!(second, source_hash(&dir), "a new file shows");
    }

    #[test]
    fn build_output_is_not_source() {
        let dir = scratch("ignored");
        std::fs::write(dir.join("helper.cs"), "class Helper {}").unwrap();
        let before = source_hash(&dir);

        let built = dir.join("bin");
        std::fs::create_dir_all(&built).unwrap();
        std::fs::write(built.join("helper.dll"), "not source").unwrap();
        assert_eq!(before, source_hash(&dir), "bin/ is ignored");
    }

    #[test]
    fn a_folder_that_is_not_there_hashes_to_something_anyway() {
        let missing = scratch("gone").join("nowhere");
        assert_eq!(source_hash(&missing).len(), 16);
    }
}
