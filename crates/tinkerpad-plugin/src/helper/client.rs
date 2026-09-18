//! Talking to one helper process.
//!
//! A helper is a program that stays alive between runs so the language's
//! runtime doesn't have to start again each time. This is the core's side of
//! it: spawn, say `hello`, wait for `ready`, then hand it runs and hooks until
//! something stops it.
//!
//! One router task owns the child and fans its messages out: a run's output
//! goes to whoever is holding that [`HelperRun`], a hook's `result` wakes the
//! call that is waiting, and anything else goes to the log. The helper's own
//! stderr is log only; a *run's* stderr arrives as a `stderr` message, because
//! the two are different things and only one of them belongs in the results
//! pane.
//!
//! Keeping helpers warm across runs, restarting them when they die, and
//! falling back to `[run]` when one never says `ready` are the pool's job
//! (work item 03-02). This layer holds exactly one process and reports
//! honestly when it is gone.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use bytes::Bytes;
use tinkerpad_process::{Exit, KillHandle, ProcessEvent, SpawnError, SpawnSpec, Supervised};
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio::sync::{Mutex, mpsc, oneshot, watch};
use tracing::{Instrument, debug, info, info_span, warn};

use super::codec::{Decoder, Garbled, encode_into};
use super::protocol::{
    Diagnostic, FromHelper, HookRequest, HookResult, PROTOCOL, RunRequest, ToHelper,
};

/// How long a helper has to answer `hello`. A runtime that takes longer than
/// this to start is one the user would rather not wait for: the pool falls
/// back to per-run execution and says so (`docs/PLUGINS.md` §5).
pub const READY_TIMEOUT: Duration = Duration::from_secs(10);

/// How long a `cancel` has to take effect before the helper is killed. Warm
/// state is lost; a cancel that doesn't cancel is worse.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// How long a hook has to reply. After this the core carries on as if the hook
/// returned nothing, because a hook is an opportunity, not a gate.
pub const HOOK_TIMEOUT: Duration = Duration::from_secs(5);

/// How long `shutdown` has before the process is killed.
pub const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// How many messages may be queued towards the helper, and how many events may
/// be queued towards a run's reader, before the sender waits.
const QUEUE: usize = 256;

/// What to start, and what to tell it about itself.
///
/// The [`SpawnSpec`] arrives built: turning a manifest's `[helper]` into a
/// command, with its placeholders filled in, belongs to the pool (03-02).
#[derive(Clone, Debug)]
pub struct HelperStart {
    /// What the log calls it, usually the plugin's language id.
    pub name: String,
    pub spawn: SpawnSpec,
    /// The plugin folder, passed through in `hello`.
    pub plugin_dir: PathBuf,
    /// A folder private to this plugin that the helper may write to.
    pub scratch_dir: PathBuf,
    /// The hooks the manifest says this helper implements. Anything not listed
    /// is never sent (`docs/PLUGINS.md` §5).
    pub hooks: Vec<String>,
    /// Overridable so tests don't wait ten seconds.
    pub ready_timeout: Duration,
}

impl HelperStart {
    pub fn new(
        name: impl Into<String>,
        spawn: SpawnSpec,
        plugin_dir: impl Into<PathBuf>,
        scratch_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            name: name.into(),
            spawn,
            plugin_dir: plugin_dir.into(),
            scratch_dir: scratch_dir.into(),
            hooks: Vec::new(),
            ready_timeout: READY_TIMEOUT,
        }
    }

    pub fn hooks<I, S>(mut self, hooks: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.hooks = hooks.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_ready_timeout(mut self, timeout: Duration) -> Self {
        self.ready_timeout = timeout;
        self
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HelperError {
    #[error("the {name} helper could not be started: {source}")]
    Spawn {
        name: String,
        #[source]
        source: SpawnError,
    },
    #[error("the {name} helper did not answer hello within {}s", timeout.as_secs())]
    NoReady { name: String, timeout: Duration },
    #[error("the {name} helper is not running")]
    Gone { name: String },
}

/// Something that happened during a run.
///
/// Deliberately the same shape as the per-run runner's `RunEvent`, minus
/// `Started`: a helper's run has no pid of its own. Turning these into the
/// events the UI sees, dump sentinels and all, is the run path's job (03-03),
/// which is also where the same decoder handles both kinds of run.
#[derive(Clone, Debug, PartialEq)]
pub enum HelperEvent {
    /// Program output, exactly as the program wrote it. Sentinels intact.
    Stdout(Bytes),
    Stderr(Bytes),
    /// The program ended on its own.
    Exited {
        code: Option<i32>,
        duration: Duration,
    },
    /// [`HelperRun::cancel`] ended it, or the helper was killed to stop it.
    Cancelled {
        duration: Duration,
    },
    /// The program never ran, or the helper died before it could finish.
    Failed {
        message: String,
        diagnostics: Vec<Diagnostic>,
    },
}

/// A run in progress on a helper.
///
/// Dropping it does not stop the program: call [`HelperRun::cancel`]. That
/// matches the per-run [`RunHandle`](tinkerpad_runner) and means a dropped UI
/// tab can't silently orphan a program mid-write.
#[derive(Debug)]
pub struct HelperRun {
    id: String,
    events: mpsc::Receiver<HelperEvent>,
    shared: Arc<Shared>,
    to_helper: mpsc::Sender<ToHelper>,
    kill: KillHandle,
    exit: watch::Receiver<Option<Exit>>,
}

impl HelperRun {
    /// The run's protocol id, as the helper knows it.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Next event. `None` once the run has ended, which is always after an
    /// `Exited`, `Cancelled`, or `Failed`.
    pub async fn recv(&mut self) -> Option<HelperEvent> {
        self.events.recv().await
    }

    /// Ask the helper to stop the program, and kill the helper if it doesn't.
    ///
    /// Returns once the run is over either way. A helper that ignores `cancel`
    /// gets [`CANCEL_GRACE`] and then dies: warm state is cheap to rebuild,
    /// and a cancel button that doesn't cancel is not a button.
    pub async fn cancel(&self) {
        cancel_run(
            &self.shared,
            &self.to_helper,
            &self.kill,
            &self.exit,
            &self.id,
        )
        .await;
    }
}

/// Shared between the client and its router task.
#[derive(Debug, Default)]
struct Shared {
    run: Mutex<Option<ActiveRun>>,
    hooks: Mutex<HashMap<String, oneshot::Sender<HookResult>>>,
}

#[derive(Debug)]
struct ActiveRun {
    id: String,
    started: Instant,
    /// Set once a `cancel` has gone out, so the ending is reported as a
    /// cancellation rather than as whatever exit code the helper gives.
    cancelled: bool,
    events: mpsc::Sender<HelperEvent>,
    /// Fires when the run is over, however it ended.
    done: Option<oneshot::Sender<()>>,
}

/// A running helper.
#[derive(Debug)]
pub struct HelperClient {
    name: Arc<str>,
    pid: u32,
    /// What the helper called itself in `ready`.
    helper_name: Option<String>,
    version: Option<String>,
    hooks: HashSet<String>,
    kill: KillHandle,
    to_helper: mpsc::Sender<ToHelper>,
    shared: Arc<Shared>,
    exit: watch::Receiver<Option<Exit>>,
    next_id: AtomicU64,
}

impl HelperClient {
    /// Spawn a helper, say `hello`, and wait for `ready`.
    ///
    /// Anything the helper writes before `ready` that isn't a message is
    /// treated as the banner it almost always is, and logged rather than
    /// failed on.
    pub async fn start(start: HelperStart) -> Result<Self, HelperError> {
        let name: Arc<str> = Arc::from(start.name.as_str());
        let mut spec = start.spawn;
        // A helper with no stdin can't be spoken to, whatever the spec says.
        spec.stdin = true;
        let mut child = tinkerpad_process::spawn(spec).map_err(|source| HelperError::Spawn {
            name: start.name.clone(),
            source,
        })?;
        let pid = child.pid();
        let kill = child.kill_handle();
        let stdin = child.take_stdin();
        let span = info_span!("helper", helper = %name, pid);

        let shared = Arc::new(Shared::default());
        let (to_helper, outgoing) = mpsc::channel(QUEUE);
        let (ready_tx, ready_rx) = oneshot::channel();
        let (exit_tx, exit_rx) = watch::channel(None);

        tokio::spawn(
            route(name.clone(), child, Arc::clone(&shared), ready_tx, exit_tx)
                .instrument(span.clone()),
        );
        tokio::spawn(write(name.clone(), stdin, outgoing).instrument(span));

        let hello = ToHelper::Hello {
            protocol: PROTOCOL,
            plugin_dir: start.plugin_dir,
            scratch_dir: start.scratch_dir,
        };
        if to_helper.send(hello).await.is_err() {
            kill.kill();
            return Err(HelperError::Gone {
                name: start.name.clone(),
            });
        }

        let ready = match tokio::time::timeout(start.ready_timeout, ready_rx).await {
            Ok(Ok(ready)) => ready,
            // Timed out, or the process died before it got there. Both end the
            // same way: this helper is no use, and the pool falls back.
            _ => {
                warn!(
                    timeout_ms = start.ready_timeout.as_millis() as u64,
                    "the helper never said ready; killing it"
                );
                kill.kill();
                return Err(HelperError::NoReady {
                    name: start.name.clone(),
                    timeout: start.ready_timeout,
                });
            }
        };
        info!(
            helper_name = ready.name.as_deref().unwrap_or("unnamed"),
            version = ready.version.as_deref().unwrap_or("unknown"),
            "helper ready"
        );

        Ok(Self {
            name,
            pid,
            helper_name: ready.name,
            version: ready.version,
            hooks: start.hooks.into_iter().collect(),
            kill,
            to_helper,
            shared,
            exit: exit_rx,
            next_id: AtomicU64::new(1),
        })
    }

    /// What the log calls this helper: the plugin's language id.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// What the helper called itself in `ready`, if it said.
    pub fn helper_name(&self) -> Option<&str> {
        self.helper_name.as_deref()
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Whether the process is still there.
    pub fn is_alive(&self) -> bool {
        self.exit.borrow().as_ref().is_none()
    }

    /// Whether the manifest says this helper implements a hook.
    pub fn has_hook(&self, name: &str) -> bool {
        self.hooks.contains(name)
    }

    /// Start a run.
    ///
    /// One at a time: if something is already running, it is cancelled first,
    /// which is what a user pressing run twice means.
    pub async fn run(&self, request: RunRequest) -> Result<HelperRun, HelperError> {
        self.cancel_current().await;
        if !self.is_alive() {
            return Err(self.gone());
        }

        let id = self.new_id("r");
        let (events, rx) = mpsc::channel(QUEUE);
        {
            let mut slot = self.shared.run.lock().await;
            *slot = Some(ActiveRun {
                id: id.clone(),
                started: Instant::now(),
                cancelled: false,
                events,
                // A cancel puts a channel here and waits on it.
                done: None,
            });
        }

        let message = ToHelper::Run {
            id: id.clone(),
            main: request.main,
            dir: request.dir,
            kind: request.kind,
        };
        if self.to_helper.send(message).await.is_err() {
            self.shared.run.lock().await.take();
            return Err(self.gone());
        }
        debug!(run = %id, "run sent");

        Ok(HelperRun {
            id,
            events: rx,
            shared: Arc::clone(&self.shared),
            to_helper: self.to_helper.clone(),
            kill: self.kill.clone(),
            exit: self.exit.clone(),
        })
    }

    /// Call a hook and wait for its `result`.
    ///
    /// A hook the manifest didn't list is never sent, and one that doesn't
    /// answer within [`HOOK_TIMEOUT`] is treated as having returned nothing.
    /// Both give `Ok(HookResult::default())`: a hook is how a language adds
    /// something useful, never how it blocks the app.
    pub async fn hook(&self, request: HookRequest) -> Result<HookResult, HelperError> {
        let hook = request.name();
        if !self.has_hook(hook) {
            return Ok(HookResult::default());
        }
        if !self.is_alive() {
            return Err(self.gone());
        }

        let id = self.new_id("h");
        let (tx, rx) = oneshot::channel();
        self.shared.hooks.lock().await.insert(id.clone(), tx);

        if self
            .to_helper
            .send(request.into_message(id.clone()))
            .await
            .is_err()
        {
            self.shared.hooks.lock().await.remove(&id);
            return Err(self.gone());
        }

        match tokio::time::timeout(HOOK_TIMEOUT, rx).await {
            Ok(Ok(result)) => Ok(result),
            Ok(Err(_)) => {
                // The router dropped the sender: the helper died mid-hook.
                self.shared.hooks.lock().await.remove(&id);
                Err(self.gone())
            }
            Err(_) => {
                self.shared.hooks.lock().await.remove(&id);
                warn!(
                    hook,
                    timeout_ms = HOOK_TIMEOUT.as_millis() as u64,
                    "the helper did not answer in time; carrying on without it"
                );
                Ok(HookResult::default())
            }
        }
    }

    /// Stop the helper: `shutdown`, then [`SHUTDOWN_GRACE`], then the axe.
    ///
    /// Returns once the process is gone.
    pub async fn shutdown(&self) {
        if !self.is_alive() {
            return;
        }
        let _ = self.to_helper.send(ToHelper::Shutdown).await;
        if tokio::time::timeout(SHUTDOWN_GRACE, self.wait_exit())
            .await
            .is_err()
        {
            info!("the helper did not stop when asked; killing it");
            self.kill.kill();
            let _ = self.wait_exit().await;
        }
    }

    /// End the helper now, without asking.
    pub fn kill(&self) {
        self.kill.kill();
    }

    pub fn kill_handle(&self) -> KillHandle {
        self.kill.clone()
    }

    /// Wait until the process is gone, and say how it went.
    pub async fn wait_exit(&self) -> Option<Exit> {
        exited(&self.exit).await
    }

    /// Cancel whatever is running, if anything is.
    pub async fn cancel_current(&self) {
        let id = self
            .shared
            .run
            .lock()
            .await
            .as_ref()
            .map(|run| run.id.clone());
        if let Some(id) = id {
            cancel_run(&self.shared, &self.to_helper, &self.kill, &self.exit, &id).await;
        }
    }

    fn new_id(&self, prefix: &str) -> String {
        format!("{prefix}{}", self.next_id.fetch_add(1, Ordering::Relaxed))
    }

    fn gone(&self) -> HelperError {
        HelperError::Gone {
            name: self.name.to_string(),
        }
    }
}

/// Ask for a run to stop, and make sure it does.
async fn cancel_run(
    shared: &Arc<Shared>,
    to_helper: &mpsc::Sender<ToHelper>,
    kill: &KillHandle,
    exit: &watch::Receiver<Option<Exit>>,
    id: &str,
) {
    // Mark it first, so the `exited` that follows is reported as a cancel.
    let waiting = {
        let mut slot = shared.run.lock().await;
        match slot.as_mut() {
            Some(run) if run.id == id => {
                run.cancelled = true;
                let (tx, rx) = oneshot::channel();
                run.done = Some(tx);
                Some(rx)
            }
            // Already over, or something else is running now.
            _ => None,
        }
    };
    let Some(waiting) = waiting else { return };

    let _ = to_helper.send(ToHelper::Cancel { id: id.to_owned() }).await;
    debug!(run = %id, "cancel sent");

    if tokio::time::timeout(CANCEL_GRACE, waiting).await.is_ok() {
        return;
    }
    warn!(
        run = %id,
        grace_ms = CANCEL_GRACE.as_millis() as u64,
        "the helper did not stop the run; killing it"
    );
    kill.kill();
    // Wait for it to actually be gone, so a caller that cancels and then asks
    // whether the helper is alive gets the truth, and the pool can respawn
    // without racing the process it just killed.
    let _ = tokio::time::timeout(CANCEL_GRACE, exited(exit)).await;
    // The router reports the run cancelled when the process exits; if it was
    // already gone, finish the run here so nobody waits forever.
    finish_run(shared, id, |run| HelperEvent::Cancelled {
        duration: run.started.elapsed(),
    })
    .await;
}

/// Wait until the process has exited, and say how it went.
async fn exited(exit: &watch::Receiver<Option<Exit>>) -> Option<Exit> {
    let mut exit = exit.clone();
    loop {
        let current = exit.borrow().as_ref().cloned();
        if current.is_some() {
            return current;
        }
        if exit.changed().await.is_err() {
            return exit.borrow().as_ref().cloned();
        }
    }
}

/// End the active run with a final event, if it is still the one named.
async fn finish_run(shared: &Arc<Shared>, id: &str, last: impl FnOnce(&ActiveRun) -> HelperEvent) {
    let mut slot = shared.run.lock().await;
    let matching = slot.as_ref().is_some_and(|run| run.id == id);
    if !matching {
        return;
    }
    let Some(mut run) = slot.take() else { return };
    let event = last(&run);
    // The run is out of the slot, so the next one need not wait behind a
    // reader that is slow to take this last event.
    drop(slot);
    let _ = run.events.send(event).await;
    if let Some(done) = run.done.take() {
        let _ = done.send(());
    }
}

/// What `ready` said.
#[derive(Debug, Default)]
struct Ready {
    name: Option<String>,
    version: Option<String>,
}

/// Read the helper's stdout, hand each message to whoever is waiting for it,
/// and log its stderr.
async fn route(
    name: Arc<str>,
    mut child: Supervised,
    shared: Arc<Shared>,
    ready_tx: oneshot::Sender<Ready>,
    exit_tx: watch::Sender<Option<Exit>>,
) {
    let mut decoder = Decoder::new();
    let mut stderr = Lines::default();
    let mut ready_tx = Some(ready_tx);
    let mut messages = 0u64;

    while let Some(event) = child.recv().await {
        match event {
            ProcessEvent::Stdout(bytes) => {
                for message in decoder.push(&bytes) {
                    messages += 1;
                    dispatch(&name, message, &shared, &mut ready_tx).await;
                }
            }
            ProcessEvent::Stderr(bytes) => stderr.push(&bytes),
            ProcessEvent::Exited(exit) => {
                stderr.flush();
                if let Some(message) = decoder.end() {
                    dispatch(&name, message, &shared, &mut ready_tx).await;
                }
                info!(
                    code = ?exit.code,
                    killed = exit.killed,
                    messages,
                    "helper exited"
                );
                ended(&name, &shared, &exit).await;
                let _ = exit_tx.send(Some(exit));
                return;
            }
        }
    }
}

/// One message to whoever is waiting for it.
async fn dispatch(
    name: &str,
    message: Result<FromHelper, Garbled>,
    shared: &Arc<Shared>,
    ready_tx: &mut Option<oneshot::Sender<Ready>>,
) {
    let message = match message {
        Ok(message) => message,
        Err(garbled) => {
            // Before `ready` this is a runtime announcing itself, which is
            // normal and not worth a warning. After it, the helper is saying
            // something it didn't mean to.
            if ready_tx.is_some() {
                debug!(line = %garbled, "ignoring output from the helper before ready");
            } else {
                warn!(line = %garbled, "the helper wrote a line that is not a message");
            }
            return;
        }
    };

    match message {
        FromHelper::Ready {
            name: helper_name,
            version,
        } => {
            if let Some(tx) = ready_tx.take() {
                let _ = tx.send(Ready {
                    name: helper_name,
                    version,
                });
            } else {
                debug!("the helper said ready twice; ignoring the second");
            }
        }
        FromHelper::Stdout { id, data } => {
            if let Some(bytes) = decode(&id, "stdout", &data) {
                send_to_run(shared, &id, HelperEvent::Stdout(bytes)).await;
            }
        }
        FromHelper::Stderr { id, data } => {
            if let Some(bytes) = decode(&id, "stderr", &data) {
                send_to_run(shared, &id, HelperEvent::Stderr(bytes)).await;
            }
        }
        FromHelper::Exited {
            id,
            code,
            duration_ms,
        } => {
            finish_run(shared, &id, |run| {
                // The helper's own timing when it has one: it knows when the
                // program started better than we do.
                let duration = duration_ms
                    .map(Duration::from_millis)
                    .unwrap_or_else(|| run.started.elapsed());
                if run.cancelled {
                    HelperEvent::Cancelled { duration }
                } else {
                    HelperEvent::Exited { code, duration }
                }
            })
            .await;
        }
        FromHelper::Failed {
            id,
            message,
            diagnostics,
        } => {
            finish_run(shared, &id, |_| HelperEvent::Failed {
                message,
                diagnostics,
            })
            .await;
        }
        FromHelper::Result {
            id,
            files,
            lsp_config,
        } => {
            let waiting = shared.hooks.lock().await.remove(&id);
            match waiting {
                Some(tx) => {
                    let _ = tx.send(HookResult { files, lsp_config });
                }
                None => debug!(hook = %id, "a result arrived for a hook nobody is waiting for"),
            }
        }
        FromHelper::Unknown => {
            debug!(
                helper = name,
                "ignoring a message type this version does not know"
            );
        }
    }
}

/// Base64 in, bytes out. A chunk that isn't base64 is dropped with a warning:
/// the rest of the run is still worth having.
fn decode(id: &str, what: &str, data: &str) -> Option<Bytes> {
    match BASE64.decode(data) {
        Ok(bytes) => Some(Bytes::from(bytes)),
        Err(e) => {
            let start: String = data.chars().take(60).collect();
            warn!(run = %id, stream = what, error = %e, %start, "a chunk was not base64");
            None
        }
    }
}

/// Give an event to the run it belongs to.
async fn send_to_run(shared: &Arc<Shared>, id: &str, event: HelperEvent) {
    // Cloned and the lock released before sending: a reader that is slow to
    // take its output must not be able to hold up a cancel.
    let events = {
        let slot = shared.run.lock().await;
        match slot.as_ref() {
            Some(run) if run.id == id => run.events.clone(),
            _ => {
                debug!(run = %id, "output arrived for a run that is over");
                return;
            }
        }
    };
    if events.send(event).await.is_err() {
        // Nobody is reading this run any more. Keep the slot, so a later
        // `exited` still clears it.
        debug!(run = %id, "the run's reader has gone");
    }
}

/// The process died. Anything that was waiting on it is told so.
async fn ended(name: &str, shared: &Arc<Shared>, exit: &Exit) {
    let id = shared.run.lock().await.as_ref().map(|run| run.id.clone());
    if let Some(id) = id {
        finish_run(shared, &id, |run| {
            let duration = run.started.elapsed();
            if run.cancelled || exit.killed {
                HelperEvent::Cancelled { duration }
            } else {
                HelperEvent::Failed {
                    message: format!("the {name} helper stopped while the program was running"),
                    diagnostics: Vec::new(),
                }
            }
        })
        .await;
    }
    // Dropping the senders wakes every waiting hook with an error.
    shared.hooks.lock().await.clear();
}

/// Write queued messages to the helper's stdin, one line each.
async fn write(name: Arc<str>, stdin: Option<ChildStdin>, mut queue: mpsc::Receiver<ToHelper>) {
    let Some(mut stdin) = stdin else {
        warn!(helper = %name, "no stdin pipe, so nothing can be sent to this helper");
        return;
    };
    let mut buf = Vec::with_capacity(4 * 1024);
    while let Some(message) = queue.recv().await {
        buf.clear();
        encode_into(&mut buf, &message);
        if buf.is_empty() {
            continue;
        }
        if let Err(e) = stdin.write_all(&buf).await {
            warn!(error = %e, "could not write to the helper; it has probably gone");
            return;
        }
        if let Err(e) = stdin.flush().await {
            warn!(error = %e, "could not flush to the helper");
            return;
        }
    }
    // The client was dropped. Closing stdin is the other way to tell a helper
    // it is done, and the one that works when it has stopped reading messages.
    debug!(helper = %name, "closing the helper's stdin");
}

/// Buffers the helper's own stderr into whole lines for the log.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.emit();
            } else {
                self.partial.push(byte);
            }
        }
    }

    fn flush(&mut self) {
        if !self.partial.is_empty() {
            self.emit();
        }
    }

    fn emit(&mut self) {
        let line = String::from_utf8_lossy(&std::mem::take(&mut self.partial))
            .trim_end()
            .to_owned();
        if !line.is_empty() {
            info!(target: "tinkerpad_plugin::helper", "{line}");
        }
    }
}
