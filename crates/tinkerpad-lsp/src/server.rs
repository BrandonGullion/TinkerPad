//! One language server: the handshake, the request table, and the documents it
//! knows about.
//!
//! A server outlives the tabs that use it (`docs/ARCHITECTURE.md` §4.1). It is
//! spawned on first use, `initialize`d once, and then shared: opening another
//! playground of the same language is a `textDocument/didOpen` and nothing
//! more.
//!
//! Everything the server says goes through one router task, which matches
//! responses to the requests waiting for them and broadcasts the rest
//! (notifications, and requests the server makes of us) for the proxy to
//! handle in 02-03.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tinkerpad_core::ServerState;
use tinkerpad_process::SpawnSpec;
use tinkerpad_process::supervise::{Crash, Lifecycle, Policy, Supervised};
use tokio::sync::{broadcast, oneshot, watch};
use tracing::{debug, info, warn};

/// Supervision is the same problem for a language server, a plugin helper, and
/// a debug adapter, so it lives in one place and these are its names
/// (work item 03-02). Re-exported because they are part of this crate's
/// vocabulary: a caller tuning a server's backoff shouldn't have to know which
/// crate the number comes from.
pub use tinkerpad_process::supervise::STOP_GRACE;
use tinkerpad_process::supervise::{BACKOFF_START, IDLE_UNUSED, WATCH_TICK};

use crate::jsonrpc::RawMessage;
use crate::transport::{Sender, Transport, TransportEvent};

/// How long a request waits for a server that is still starting before giving
/// up. Long enough for rust-analyzer on a cold cache to answer the handshake,
/// short enough that the editor doesn't look hung.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How many server-initiated messages are held for a subscriber that is slow.
const BROADCAST: usize = 1024;

/// How many request round trips are kept for the percentiles the strip shows.
const TIMINGS: usize = 512;

/// The error code a request gets when its server went away under it. The
/// editor treats it as transient and asks again on the next keystroke.
pub const SERVER_RESTARTED: i64 = -32000;

/// How long to wait for a server to read a configuration it has just been
/// told about, before asking it to reload anyway (work item 02-07).
const CONFIG_READ_WAIT: Duration = Duration::from_secs(2);

/// What the pool needs to start one server. Plain data: the app builds it from
/// the plugin's `[lsp]` section, which keeps this crate free of the plugin
/// format (see the dependency direction in the root `Cargo.toml`).
#[derive(Clone, Debug)]
pub struct ServerConfig {
    /// The language id, used as the server's name in logs and events.
    pub language: String,
    /// The workspace root the server is given, or `None` for a server that
    /// needs none (`[lsp].workspace = "none"`).
    pub root: Option<std::path::PathBuf>,
    /// How to start it, already resolved to a program path.
    pub spawn: SpawnSpec,
    /// `initializationOptions`, verbatim from the plugin.
    pub init_options: Option<Value>,
    /// Shown when the server binary is missing.
    pub install_hint: Option<String>,
    /// How long a request waits for the handshake. [`START_TIMEOUT`] unless a
    /// caller (or a test) says otherwise.
    pub start_timeout: Duration,
    /// How long to wait before the first restart after a crash; it doubles
    /// from there. [`BACKOFF_START`] unless a caller (or a test) says
    /// otherwise.
    pub backoff: Duration,
    /// How long the server may sit with no open documents before it is stopped.
    /// `None` is the plugin's `idle_shutdown = "never"`.
    pub idle_shutdown: Option<Duration>,
    /// The limit for a server that has never had a document at all. See
    /// [`IDLE_UNUSED`].
    pub idle_unused: Duration,
    /// How often the idle clock is checked and memory sampled. Shortened by
    /// tests; there is nothing to be gained from a shorter one in the app.
    pub watch_tick: Duration,
    /// The setting a shared server is told about its playgrounds through, as a
    /// dotted path: `rust-analyzer.linkedProjects`. From the plugin's
    /// `[lsp.register]` (work item 02-07).
    pub register_setting: Option<String>,
    /// A request to send after that setting changes, for a server that reads
    /// its projects once rather than on every change.
    pub register_reload: Option<String>,
    /// How long to give a server to read a changed configuration before the
    /// reload request is sent anyway.
    pub config_read_wait: Duration,
}

impl ServerConfig {
    pub fn new(language: impl Into<String>, spawn: SpawnSpec) -> Self {
        Self {
            language: language.into(),
            root: None,
            spawn,
            init_options: None,
            install_hint: None,
            start_timeout: START_TIMEOUT,
            backoff: BACKOFF_START,
            idle_shutdown: None,
            idle_unused: IDLE_UNUSED,
            watch_tick: WATCH_TICK,
            register_setting: None,
            register_reload: None,
            config_read_wait: CONFIG_READ_WAIT,
        }
    }

    /// How this server is to be supervised, in the terms the shared
    /// supervision code uses (03-02).
    pub fn policy(&self) -> Policy {
        Policy {
            backoff: self.backoff,
            idle_shutdown: self.idle_shutdown,
            idle_unused: self.idle_unused,
            watch_tick: self.watch_tick,
            ..Policy::default()
        }
    }

    pub fn with_root(mut self, root: impl Into<std::path::PathBuf>) -> Self {
        self.root = Some(root.into());
        self
    }

    pub fn with_init_options(mut self, options: Value) -> Self {
        self.init_options = Some(options);
        self
    }

    pub fn with_install_hint(mut self, hint: impl Into<String>) -> Self {
        self.install_hint = Some(hint.into());
        self
    }

    /// Mostly for tests: the default is long enough for a cold rust-analyzer.
    pub fn with_start_timeout(mut self, timeout: Duration) -> Self {
        self.start_timeout = timeout;
        self
    }

    /// Mostly for tests: the default gives a crashing server a second to
    /// settle, which is a long time to sit through five times over.
    pub fn with_backoff(mut self, backoff: Duration) -> Self {
        self.backoff = backoff;
        self
    }

    /// Stop the server when it has had no open documents this long. `None`
    /// keeps it forever, which is what a plugin's `idle_shutdown = "never"`
    /// asks for.
    pub fn with_idle_shutdown(mut self, idle: Option<Duration>) -> Self {
        self.idle_shutdown = idle;
        self
    }

    /// The limit for a server nobody ever opened a document on.
    pub fn with_idle_unused(mut self, idle: Duration) -> Self {
        self.idle_unused = idle;
        self
    }

    /// How often to check the idle clock. Tests shorten it; nothing else needs
    /// to.
    pub fn with_watch_tick(mut self, tick: Duration) -> Self {
        self.watch_tick = tick;
        self
    }

    /// The dotted setting a shared server learns about its playgrounds through.
    pub fn with_register_setting(mut self, setting: impl Into<String>) -> Self {
        self.register_setting = Some(setting.into());
        self
    }

    /// A request to send after that setting changes.
    pub fn with_register_reload(mut self, method: impl Into<String>) -> Self {
        self.register_reload = Some(method.into());
        self
    }
}

/// A request on the wire: who is waiting, and since when.
#[derive(Debug)]
struct Pending {
    answer: oneshot::Sender<RawMessage>,
    sent: std::time::Instant,
}

/// What one server has been doing, for the status strip (work item 02-08).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServerStats {
    /// Messages received from the server since it last started.
    pub messages: u64,
    /// Requests the core has sent it, answered or not.
    pub requests: u64,
    /// Round trips, in microseconds, over the last few hundred requests.
    pub p50_us: u64,
    pub p95_us: u64,
    /// How long the current process has been up.
    pub uptime_ms: u64,
}

/// Request round trips, kept in a fixed window.
#[derive(Debug, Default)]
struct Timings {
    requests: u64,
    recent: std::collections::VecDeque<u64>,
}

impl Timings {
    fn record(&mut self, micros: u64) {
        self.requests += 1;
        if self.recent.len() == TIMINGS {
            self.recent.pop_front();
        }
        self.recent.push_back(micros);
    }

    /// Nearest-rank percentiles over the window, and zero when it is empty.
    fn percentiles(&self) -> (u64, u64) {
        let mut sorted: Vec<u64> = self.recent.iter().copied().collect();
        sorted.sort_unstable();
        let at = |fraction: f64| -> u64 {
            if sorted.is_empty() {
                return 0;
            }
            sorted[((sorted.len() - 1) as f64 * fraction).round() as usize]
        };
        (at(0.5), at(0.95))
    }
}

/// A document the server has been told about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocState {
    pub uri: String,
    pub language_id: String,
    /// The version last sent. The core owns this counter; the editor's own
    /// numbering is never trusted (LSP requires it to increase per document).
    pub version: i32,
    pub text: String,
}

#[derive(Debug, thiserror::Error)]
pub enum RequestError {
    #[error("{language} is not running")]
    NotRunning { language: String },
    #[error("{language} did not finish starting in time")]
    StartTimeout { language: String },
    #[error("{language} stopped before it answered")]
    Disconnected { language: String },
    #[error("{method} failed: {message} (code {code})")]
    Failed {
        method: String,
        code: i64,
        message: String,
    },
}

/// One process, and the two ends of the pipe to it. Replaced wholesale when a
/// crashed server is restarted, which is why the handle around it can be held
/// by tabs that never notice (02-05).
#[derive(Debug)]
struct Connection {
    pid: u32,
    sender: Sender,
    kill: tinkerpad_process::KillHandle,
}

/// A running (or failed) language server.
#[derive(Debug)]
pub struct ServerHandle {
    language: String,
    start_timeout: Duration,
    root: Option<std::path::PathBuf>,
    /// How to start it, kept so a crashed server can be started again.
    config: ServerConfig,
    /// State, crash budget, idle clock, uptime, and the last few things the
    /// server said. Shared with the helper pool (03-02).
    life: Lifecycle,
    connection: Mutex<Option<Connection>>,
    /// What each attached playground contributes to the registered setting,
    /// by document URI (work item 02-07).
    registrations: Mutex<std::collections::BTreeMap<String, Value>>,
    /// What a plugin's hooks have contributed to this server's settings, by
    /// document URI (work item 03-07). Merged on top of the registered
    /// setting, so a plugin can say things `[lsp.register]` has no shape for.
    hooked: Mutex<std::collections::BTreeMap<String, Value>>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, Pending>>,
    open_docs: Mutex<HashMap<String, DocState>>,
    /// What the server said it could do, from the handshake. The proxy hands
    /// this to each editor client instead of a second `initialize` (02-04).
    capabilities: Mutex<Option<Value>>,
    /// Notifications and server-to-client requests, for the proxy (02-03).
    from_server: broadcast::Sender<RawMessage>,
    /// Fired when the server asks for its configuration, so a reload request
    /// can wait until it has actually read the new one.
    config_asked: std::sync::Arc<tokio::sync::Notify>,
    /// Traffic and round trips since the current process started (02-08).
    timings: Mutex<Timings>,
    messages: std::sync::atomic::AtomicU64,
}

impl ServerHandle {
    /// A handle for a server that could not be started. It answers every
    /// request with [`RequestError::NotRunning`], so a missing language server
    /// never stops the editor from working.
    pub fn failed(config: &ServerConfig, reason: String) -> Self {
        let state = ServerState::failed(reason, config.install_hint.clone());
        Self::with_state(config, state)
    }

    fn with_state(config: &ServerConfig, state: ServerState) -> Self {
        Self {
            language: config.language.clone(),
            start_timeout: config.start_timeout,
            root: config.root.clone(),
            life: Lifecycle::in_state(
                config.language.clone(),
                "language server",
                config.policy(),
                state,
            ),
            config: config.clone(),
            connection: Mutex::default(),
            registrations: Mutex::default(),
            hooked: Mutex::default(),
            next_id: AtomicI64::new(1),
            pending: Mutex::default(),
            open_docs: Mutex::default(),
            capabilities: Mutex::default(),
            from_server: broadcast::channel(BROADCAST).0,
            config_asked: std::sync::Arc::new(tokio::sync::Notify::new()),
            timings: Mutex::default(),
            messages: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Spawn the server and start its router. The `initialize` handshake runs
    /// in the background: the state goes `Starting` → `Ready`, and requests
    /// made in between wait for it.
    pub fn start(config: &ServerConfig) -> std::sync::Arc<Self> {
        let server = Self::prepare(config);
        server.begin();
        server
    }

    /// Build the handle without starting anything. The pool uses this to
    /// record which playgrounds a server is for *before* it starts, so the
    /// handshake can carry them: rust-analyzer reads its projects once, at
    /// startup, and a list that arrives later means a reload (02-07).
    pub fn prepare(config: &ServerConfig) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::with_state(config, ServerState::Starting))
    }

    /// Start a prepared server.
    pub fn begin(self: &std::sync::Arc<Self>) {
        self.connect();
        tokio::spawn(tinkerpad_process::supervise::watch(self.clone()));
    }

    /// Tell a shared server about one more playground.
    ///
    /// A server rooted at a workspace of its own (`scratch_workspace`) has no
    /// way to discover the playgrounds it is meant to serve, so the plugin
    /// names a setting — `rust-analyzer.linkedProjects` — and each attached
    /// playground adds a value to it. The whole list is pushed every time it
    /// changes, which is what a server expects from
    /// `workspace/didChangeConfiguration`.
    pub async fn register(&self, uri: &str, value: Value) {
        if self.note_registration(uri, value) {
            self.push_configuration().await;
        }
    }

    /// Record a playground's contribution without pushing it yet. The pool
    /// does this as the tab attaches, before the handshake has finished, so a
    /// server that asks for its configuration is answered correctly the first
    /// time it asks.
    pub fn note_registration(&self, uri: &str, value: Value) -> bool {
        if self.config.register_setting.is_none() {
            return false;
        }
        let mut registrations = self.registrations.lock().expect("registrations lock");
        registrations.insert(uri.to_owned(), value.clone()) != Some(value)
    }

    /// Traffic, round trips, and uptime since this process started.
    pub fn stats(&self) -> ServerStats {
        let (p50_us, p95_us) = {
            let timings = self.timings.lock().expect("timings lock");
            let (p50, p95) = timings.percentiles();
            (p50, p95)
        };
        ServerStats {
            messages: self.messages.load(std::sync::atomic::Ordering::Relaxed),
            requests: self.timings.lock().expect("timings lock").requests,
            p50_us,
            p95_us,
            uptime_ms: self
                .life
                .uptime()
                .map(|up| up.as_millis() as u64)
                .unwrap_or(0),
        }
    }

    /// The server's own last words: stderr lines and `window/logMessage`s,
    /// oldest first, at most [`LOG_LINES`] of them.
    pub fn log(&self) -> Vec<String> {
        self.life.log()
    }

    /// Record one line of what the server said.
    fn note_log(&self, line: String) {
        self.life.note_log(line);
    }

    /// Where this server's program lives, as resolved when it was configured.
    pub fn program(&self) -> String {
        self.config.spawn.program.to_string_lossy().into_owned()
    }

    /// The plugin's install hint, for a server that isn't there.
    pub fn install_hint(&self) -> Option<&str> {
        self.config.install_hint.as_deref()
    }

    /// The playgrounds this server has been told to serve.
    pub fn registrations(&self) -> Vec<Value> {
        self.registrations
            .lock()
            .expect("registrations lock")
            .values()
            .cloned()
            .collect()
    }

    /// Send the registered setting as it stands now. The pool calls this
    /// after noting a registration.
    pub async fn push_registrations(&self) {
        if self.config.register_setting.is_some() {
            self.push_configuration().await;
        }
    }

    /// Forget a playground's contribution to the registered setting.
    pub async fn unregister(&self, uri: &str) {
        if self.forget_registration(uri) {
            self.push_configuration().await;
        }
    }

    /// Take a playground out of the registered setting without telling the
    /// server yet. A tab closing does this as it goes, so the next tab to
    /// attach sees an accurate list rather than one that is still being
    /// cleaned up.
    pub fn forget_registration(&self, uri: &str) -> bool {
        if self.config.register_setting.is_none() {
            return false;
        }
        self.registrations
            .lock()
            .expect("registrations lock")
            .remove(uri)
            .is_some()
    }

    /// The settings object this server should be seeing: the registered
    /// setting's dotted path, holding every attached playground's value.
    ///
    /// Empty when the plugin registers nothing, which is most of them.
    pub fn configuration(&self) -> Value {
        let mut settings = match &self.config.register_setting {
            Some(setting) => {
                let values: Vec<Value> = self
                    .registrations
                    .lock()
                    .expect("registrations lock")
                    .values()
                    .cloned()
                    .collect();
                nest(setting, Value::Array(values))
            }
            None => json!({}),
        };
        // What the plugin's hooks asked for, on top: a hook is more specific
        // than a manifest, and later playgrounds win over earlier ones only
        // where they actually overlap (work item 03-07).
        for value in self.hooked.lock().expect("hooked lock").values() {
            merge(&mut settings, value);
        }
        settings
    }

    /// Add what a plugin's hook said this server should know, for one
    /// playground, and tell the server.
    pub async fn configure(&self, uri: &str, value: Value) {
        let changed = self
            .hooked
            .lock()
            .expect("hooked lock")
            .insert(uri.to_owned(), value.clone())
            .as_ref()
            != Some(&value);
        if changed {
            self.push_configuration().await;
        }
    }

    /// Forget what a hook said for a playground that has closed.
    pub fn forget_configuration(&self, uri: &str) -> bool {
        self.hooked
            .lock()
            .expect("hooked lock")
            .remove(uri)
            .is_some()
    }

    /// Note that the server has asked for its configuration. The proxy calls
    /// this when it answers one, so a reload request can wait for it.
    pub fn note_configuration_request(&self) {
        self.config_asked.notify_waiters();
    }

    /// The value at a dotted section of the configuration, for a server that
    /// asks with `workspace/configuration` instead of waiting to be told.
    pub fn configuration_section(&self, section: &str) -> Value {
        let mut value = self.configuration();
        if section.is_empty() {
            return value;
        }
        for part in section.split('.') {
            value = value.get(part).cloned().unwrap_or(Value::Null);
            if value.is_null() {
                break;
            }
        }
        value
    }

    async fn push_configuration(&self) {
        let settings = self.configuration();
        debug!(language = %self.language, %settings, "telling the server about its playgrounds");
        let _ = self
            .notify(
                "workspace/didChangeConfiguration",
                json!({ "settings": settings }),
            )
            .await;

        // Some servers read their projects once and act on a change only when
        // asked to; the plugin names the request to send (rust-analyzer wants
        // `rust-analyzer/reloadWorkspace`). Reloading before the server has
        // read the new configuration would reload the old one, so wait for it
        // to ask — and go ahead anyway if it doesn't. Failures are the server's
        // business, not the user's.
        if let Some(method) = self.config.register_reload.clone() {
            let asked = self.config_asked.clone();
            let _ = tokio::time::timeout(self.config.config_read_wait, asked.notified()).await;
            match self.request(&method, Value::Null).await {
                Ok(_) => debug!(language = %self.language, %method, "asked the server to reload"),
                Err(e) => {
                    debug!(language = %self.language, %method, error = %e, "reload request failed")
                }
            }
        }
    }

    /// Note that the set of open documents changed: the idle clock runs only
    /// while there are none.
    fn docs_changed(&self) {
        if self.open_docs.lock().expect("open_docs lock").is_empty() {
            self.life.idle();
        } else {
            self.life.busy();
        }
    }

    /// How long the server has had no documents, if it has none.
    pub fn idle_for(&self) -> Option<Duration> {
        self.life.idle_for()
    }

    /// Spawn the process and start talking to it. Used to start the server and
    /// to start it again after a crash.
    fn connect(self: &std::sync::Arc<Self>) {
        let transport = match Transport::spawn(&self.language, self.config.spawn.clone()) {
            Ok(transport) => transport,
            Err(e) => {
                warn!(language = %self.language, error = %e, "language server did not start");
                self.set_state(ServerState::failed(
                    e.to_string(),
                    self.config.install_hint.clone(),
                ));
                return;
            }
        };

        *self.capabilities.lock().expect("capabilities lock") = None;
        *self.timings.lock().expect("timings lock") = Timings::default();
        self.messages.store(0, std::sync::atomic::Ordering::Relaxed);
        self.life.starting_now();
        *self.connection.lock().expect("connection lock") = Some(Connection {
            pid: transport.pid(),
            sender: transport.sender(),
            kill: transport.kill_handle(),
        });
        self.set_state(ServerState::Starting);

        tokio::spawn(route(self.clone(), transport));
        tokio::spawn(handshake(self.clone(), self.config.clone()));
    }

    /// Start the server again now, forgetting how often it has crashed.
    ///
    /// This is the Restart button in the status strip (02-08), and what a new
    /// tab does when it finds a server the pool had given up on: asking for it
    /// again is a person saying "try once more".
    pub fn restart(self: &std::sync::Arc<Self>) {
        self.life.forget_crashes();
        let running = self
            .connection
            .lock()
            .expect("connection lock")
            .as_ref()
            .map(|c| c.kill.clone());
        self.set_state(if running.is_some() {
            ServerState::Stopping
        } else {
            ServerState::Starting
        });
        let server = self.clone();
        tokio::spawn(async move {
            if let Some(kill) = running {
                let mut states = server.watch_state();
                kill.kill();
                let _ = tokio::time::timeout(
                    STOP_GRACE,
                    states.wait_for(|state| !matches!(state, ServerState::Stopping)),
                )
                .await;
            }
            server.connect();
            server.replay().await;
        });
    }

    /// The server died without being asked to. Count it, say so, and start it
    /// again after a wait that doubles with each crash — unless it has been
    /// crashing steadily, in which case something is properly wrong and
    /// restarting it again would only burn the machine.
    fn crashed(self: &std::sync::Arc<Self>, reason: String) {
        let (wait, count) = match self.life.note_crash() {
            Crash::Restart { after, count } => (after, count),
            Crash::GiveUp { count } => {
                warn!(language = %self.language, crashes = count, "giving up on the language server");
                self.set_state(ServerState::failed(
                    format!("it crashed {count} times in a row: {reason}"),
                    Some(
                        "Restart it from the status strip when you have fixed the cause."
                            .to_owned(),
                    ),
                ));
                return;
            }
        };
        warn!(
            language = %self.language,
            crashes = count,
            wait_ms = wait.as_millis() as u64,
            %reason,
            "language server crashed; restarting"
        );
        self.set_state(ServerState::Crashed {
            reason,
            restarts: count,
        });

        let server = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(wait).await;
            // Someone may have stopped or restarted it while we waited.
            if !matches!(server.state(), ServerState::Crashed { .. }) {
                return;
            }
            server.connect();
            server.replay().await;
        });
    }

    /// Open every document the server knew about again, at the text the editor
    /// last sent. Diagnostics come back on their own after that.
    ///
    /// Only the current text is replayed, never the history of changes: the
    /// text is the whole of what a server needs.
    async fn replay(self: &std::sync::Arc<Self>) {
        // The registered setting first: a server that learns about a project
        // after the file in it is open may not connect the two.
        if self.config.register_setting.is_some()
            && !self
                .registrations
                .lock()
                .expect("registrations lock")
                .is_empty()
        {
            self.push_configuration().await;
        }

        let docs = self.open_docs();
        if docs.is_empty() {
            return;
        }
        for doc in &docs {
            let message = RawMessage::notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": doc.uri,
                        "languageId": doc.language_id,
                        "version": doc.version,
                        "text": doc.text,
                    }
                }),
            );
            if let Err(e) = self.send(message).await {
                warn!(language = %self.language, uri = doc.uri, error = %e, "could not reopen a document");
                return;
            }
        }
        info!(language = %self.language, documents = docs.len(), "reopened documents after a restart");
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    /// The process id of the server as it is running now, or 0 if it isn't.
    pub fn pid(&self) -> u32 {
        self.connection
            .lock()
            .expect("connection lock")
            .as_ref()
            .map_or(0, |c| c.pid)
    }

    /// How many times this server has crashed and been restarted in a row.
    pub fn restarts(&self) -> u32 {
        self.life.restarts()
    }

    /// The end of the pipe to the server, if it is running.
    fn sender(&self) -> Option<Sender> {
        self.connection
            .lock()
            .expect("connection lock")
            .as_ref()
            .map(|c| c.sender.clone())
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn state(&self) -> ServerState {
        self.life.state()
    }

    /// State changes, for the pool to forward to the UI.
    pub fn watch_state(&self) -> watch::Receiver<ServerState> {
        self.life.watch_state()
    }

    /// Everything the server says that wasn't a reply: notifications such as
    /// `textDocument/publishDiagnostics`, and requests it makes of us.
    pub fn subscribe(&self) -> broadcast::Receiver<RawMessage> {
        self.from_server.subscribe()
    }

    /// The documents this server has been told about, for replay after a
    /// restart (02-05).
    pub fn open_docs(&self) -> Vec<DocState> {
        let docs = self.open_docs.lock().expect("open_docs lock");
        let mut docs: Vec<_> = docs.values().cloned().collect();
        docs.sort_by(|a, b| a.uri.cmp(&b.uri));
        docs
    }

    /// What the server said it could do, once the handshake is done.
    pub fn capabilities(&self) -> Option<Value> {
        self.capabilities.lock().expect("capabilities lock").clone()
    }

    /// Wait for the handshake, then report what the server can do.
    ///
    /// The proxy answers each editor client's own `initialize` with this. An
    /// editor told "no capabilities" because the handshake hadn't finished
    /// would believe it for as long as the tab is open, and offer no
    /// completions at all (02-04).
    pub async fn ready(&self) -> Result<Value, RequestError> {
        self.wait_ready().await?;
        Ok(self.capabilities().unwrap_or_else(|| json!({})))
    }

    /// Record a document-sync notification on its way from an editor.
    ///
    /// The editor's LSP client numbers its own versions, and the server echoes
    /// those numbers back in `publishDiagnostics`; a client drops diagnostics
    /// whose version isn't the one it sent. So the notification passes through
    /// untouched — incremental changes and all — and the core just keeps up
    /// with what it says, which is what 02-05 replays after a restart.
    ///
    /// Returns the message to send on, or `None` when there is nothing to send
    /// (a close for a document nobody opened).
    pub fn sync_from_editor(&self, message: &RawMessage) -> Option<RawMessage> {
        let method = message.method()?;
        let params = message.params()?;
        let document = &params["textDocument"];
        let uri = document["uri"].as_str()?.to_owned();

        match method {
            "textDocument/didOpen" => {
                let doc = DocState {
                    uri: uri.clone(),
                    language_id: document["languageId"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                    version: document["version"].as_i64().unwrap_or(1) as i32,
                    text: document["text"].as_str().unwrap_or_default().to_owned(),
                };
                let mut docs = self.open_docs.lock().expect("open_docs lock");
                if docs.insert(uri, doc).is_some() {
                    // Two editors on one document would be two `didOpen`s for
                    // one URI, which LSP doesn't allow. Tabs are one per
                    // playground, so this means something is wrong upstream.
                    warn!(language = %self.language, "a document was opened twice");
                }
            }
            "textDocument/didChange" => {
                let mut docs = self.open_docs.lock().expect("open_docs lock");
                let doc = docs.get_mut(&uri)?;
                doc.version = document["version"]
                    .as_i64()
                    .unwrap_or(doc.version as i64 + 1) as i32;
                // A change carrying the whole document keeps the core's copy
                // current; an incremental one leaves it behind, and 02-05
                // re-opens from the editor rather than guessing.
                if let Some(changes) = params["contentChanges"].as_array()
                    && let [only] = changes.as_slice()
                    && only.get("range").is_none()
                    && let Some(text) = only["text"].as_str()
                {
                    doc.text = text.to_owned();
                }
            }
            "textDocument/didClose" => {
                self.open_docs
                    .lock()
                    .expect("open_docs lock")
                    .remove(&uri)?;
            }
            _ => {}
        }
        self.docs_changed();
        Some(message.clone())
    }

    pub fn is_open(&self, uri: &str) -> bool {
        self.open_docs
            .lock()
            .expect("open_docs lock")
            .contains_key(uri)
    }

    /// Make a request and wait for its answer. If the server is still
    /// starting, the request waits for the handshake ([`START_TIMEOUT`]).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RequestError> {
        let (_, answer) = self.begin_request(method, params).await?;
        let response = answer.await.map_err(|_| self.disconnected())?;
        if let Some(error) = response.error_object() {
            return Err(RequestError::Failed {
                method: method.to_owned(),
                code: error["code"].as_i64().unwrap_or(0),
                message: error["message"].as_str().unwrap_or("no message").to_owned(),
            });
        }
        Ok(response.result().cloned().unwrap_or(Value::Null))
    }

    /// A request that doesn't wait for the server to be ready, for the one
    /// caller that is talking to a server on its way down.
    async fn request_now(&self, method: &str, params: Value) -> Result<Value, RequestError> {
        let sender = self.sender().ok_or_else(|| self.not_running())?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(
            id,
            Pending {
                answer: tx,
                sent: std::time::Instant::now(),
            },
        );
        if sender
            .send(RawMessage::request(id, method, params))
            .await
            .is_err()
        {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(self.disconnected());
        }
        let response = rx.await.map_err(|_| self.disconnected())?;
        Ok(response.result().cloned().unwrap_or(Value::Null))
    }

    /// Send a request and hand back its id along with the answer to come.
    ///
    /// The id is the core's, not the editor's: the proxy maps between them, so
    /// a server restart can never leave the editor holding an id that means
    /// something else (`docs/ARCHITECTURE.md` §4.2). It is also what
    /// [`cancel_request`](Self::cancel_request) needs.
    pub async fn begin_request(
        &self,
        method: &str,
        params: Value,
    ) -> Result<(i64, oneshot::Receiver<RawMessage>), RequestError> {
        self.wait_ready().await?;
        let sender = self.sender().ok_or_else(|| self.not_running())?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(
            id,
            Pending {
                answer: tx,
                sent: std::time::Instant::now(),
            },
        );

        if sender
            .send(RawMessage::request(id, method, params))
            .await
            .is_err()
        {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(self.disconnected());
        }
        Ok((id, rx))
    }

    /// Tell the server to stop working on a request, and forget it. Sent when
    /// the tab that asked goes away; a server is free to answer anyway, and
    /// the answer is then dropped.
    pub async fn cancel_request(&self, id: i64) {
        self.pending.lock().expect("pending lock").remove(&id);
        let _ = self
            .notify_now("$/cancelRequest", json!({ "id": id }))
            .await;
    }

    /// Send a message the caller has already built, after the handshake.
    /// The proxy uses this to pass a notification through untouched.
    pub async fn send(&self, message: RawMessage) -> Result<(), RequestError> {
        self.wait_ready().await?;
        let sender = self.sender().ok_or_else(|| self.not_running())?;
        sender.send(message).await.map_err(|_| self.disconnected())
    }

    /// Send a notification, which has no answer.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RequestError> {
        self.wait_ready().await?;
        self.notify_now(method, params).await
    }

    /// Send a notification without waiting for the handshake. Used by the
    /// handshake itself (`initialized`) and by shutdown.
    async fn notify_now(&self, method: &str, params: Value) -> Result<(), RequestError> {
        let sender = self.sender().ok_or_else(|| self.not_running())?;
        sender
            .send(RawMessage::notification(method, params))
            .await
            .map_err(|_| self.disconnected())
    }

    /// Tell the server about a document. Opening the same URI twice is a
    /// change, not a second open, which is what happens when two tabs show one
    /// playground.
    pub async fn did_open(
        &self,
        uri: &str,
        language_id: &str,
        text: &str,
    ) -> Result<(), RequestError> {
        let already = self.is_open(uri);
        if already {
            return self.did_change(uri, text).await;
        }
        let doc = {
            let mut docs = self.open_docs.lock().expect("open_docs lock");
            let doc = DocState {
                uri: uri.to_owned(),
                language_id: language_id.to_owned(),
                version: 1,
                text: text.to_owned(),
            };
            docs.insert(uri.to_owned(), doc.clone());
            doc
        };
        self.docs_changed();
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": {
                    "uri": doc.uri,
                    "languageId": doc.language_id,
                    "version": doc.version,
                    "text": doc.text,
                }
            }),
        )
        .await
    }

    /// Send the whole document again under the next version. Incremental sync
    /// is the editor client's business (02-04); the pool owns the counter.
    pub async fn did_change(&self, uri: &str, text: &str) -> Result<(), RequestError> {
        let doc = {
            let mut docs = self.open_docs.lock().expect("open_docs lock");
            let Some(doc) = docs.get_mut(uri) else {
                return Ok(());
            };
            doc.version += 1;
            doc.text = text.to_owned();
            doc.clone()
        };
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": doc.uri, "version": doc.version },
                "contentChanges": [{ "text": doc.text }],
            }),
        )
        .await
    }

    /// Forget a document. The server stays up.
    pub async fn did_close(&self, uri: &str) -> Result<(), RequestError> {
        let known = self
            .open_docs
            .lock()
            .expect("open_docs lock")
            .remove(uri)
            .is_some();
        self.docs_changed();
        if !known {
            return Ok(());
        }
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await
    }

    /// `shutdown`, then `exit`, then kill if it is still there. Returns once
    /// the process is gone or the grace period is up.
    pub async fn stop(&self) {
        if self.sender().is_none() || matches!(self.state(), ServerState::Stopped { .. }) {
            return;
        }
        let mut exited = self.life.watch_state();
        self.set_state(ServerState::Stopping);

        // A server that is wedged shouldn't hold up the app, so the polite
        // path gets a deadline of its own.
        let polite = async {
            let _ =
                tokio::time::timeout(STOP_GRACE, self.request_now("shutdown", Value::Null)).await;
            let _ = self.notify_now("exit", Value::Null).await;
            let _ = exited
                .wait_for(|state| matches!(state, ServerState::Stopped { .. }))
                .await;
        };
        if tokio::time::timeout(STOP_GRACE, polite).await.is_err() {
            warn!(language = %self.language, "server did not stop politely; killing it");
            self.kill();
        }
    }

    /// End the process now. The router publishes the stop.
    pub fn kill(&self) {
        if let Some(connection) = self.connection.lock().expect("connection lock").as_ref() {
            connection.kill.kill();
        }
    }

    fn set_state(&self, state: ServerState) {
        self.life.set_state(state);
    }

    /// Wait until the server can be spoken to.
    ///
    /// A server that is starting, stopping on its way to a restart, or waiting
    /// out a crash is all the same to a caller: it is coming back, so wait for
    /// it rather than refusing. Only one that has stopped for good says no.
    async fn wait_ready(&self) -> Result<(), RequestError> {
        match self.state() {
            ServerState::Ready { .. } => return Ok(()),
            ServerState::Starting | ServerState::Stopping | ServerState::Crashed { .. } => {}
            ServerState::Stopped { .. } => return Err(self.not_running()),
        }
        let mut states = self.life.watch_state();
        let settled = states.wait_for(|state| {
            matches!(
                state,
                ServerState::Ready { .. } | ServerState::Stopped { .. }
            )
        });
        match tokio::time::timeout(self.start_timeout, settled).await {
            Ok(Ok(state)) if state.is_ready() => Ok(()),
            Ok(Ok(_)) => Err(self.not_running()),
            Ok(Err(_)) => Err(self.disconnected()),
            Err(_) => Err(RequestError::StartTimeout {
                language: self.language.clone(),
            }),
        }
    }

    fn not_running(&self) -> RequestError {
        RequestError::NotRunning {
            language: self.language.clone(),
        }
    }

    fn disconnected(&self) -> RequestError {
        RequestError::Disconnected {
            language: self.language.clone(),
        }
    }
}

/// The `initialize` / `initialized` exchange, once per server.
async fn handshake(server: std::sync::Arc<ServerHandle>, config: ServerConfig) {
    let root_uri = config.root.as_deref().map(path_to_uri);
    let params = json!({
        "processId": std::process::id(),
        "clientInfo": { "name": "TinkerPad", "version": env!("CARGO_PKG_VERSION") },
        "rootUri": root_uri,
        "workspaceFolders": root_uri.as_ref().map(|uri| json!([{
            "uri": uri,
            "name": config.language,
        }])),
        "capabilities": client_capabilities(),
        "initializationOptions": initialization_options(&config, &server),
    });

    debug!(
        language = %server.language,
        options = %params["initializationOptions"],
        "handshake"
    );
    // Sent directly: the state is `Starting`, which is exactly what everything
    // else is waiting for.
    let started = std::time::Instant::now();
    let id = server.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    server.pending.lock().expect("pending lock").insert(
        id,
        Pending {
            answer: tx,
            sent: std::time::Instant::now(),
        },
    );
    let sent = match server.sender().as_ref() {
        Some(sender) => sender
            .send(RawMessage::request(id, "initialize", params))
            .await
            .is_ok(),
        None => false,
    };
    if !sent {
        server.set_state(ServerState::failed("it stopped before the handshake", None));
        return;
    }

    let response = match tokio::time::timeout(config.start_timeout, rx).await {
        Ok(Ok(response)) => response,
        Ok(Err(_)) => {
            // The router is gone, so the process is too; it publishes why.
            return;
        }
        Err(_) => {
            warn!(language = %server.language, "initialize was not answered in time");
            server.set_state(ServerState::failed("it did not answer `initialize`", None));
            server.kill();
            return;
        }
    };

    if let Some(error) = response.error_object() {
        let message = error["message"].as_str().unwrap_or("no message").to_owned();
        server.set_state(ServerState::failed(
            format!("`initialize` failed: {message}"),
            None,
        ));
        server.kill();
        return;
    }

    let result = response.result().cloned().unwrap_or(Value::Null);
    let info = result.get("serverInfo").cloned().unwrap_or(Value::Null);
    *server.capabilities.lock().expect("capabilities lock") =
        Some(result.get("capabilities").cloned().unwrap_or(json!({})));
    let _ = server.notify_now("initialized", json!({})).await;
    server.set_state(ServerState::Ready {
        pid: server.pid(),
        name: info["name"].as_str().map(str::to_owned),
        version: info["version"].as_str().map(str::to_owned),
        rss_bytes: None,
    });
    info!(
        language = %server.language,
        ms = started.elapsed().as_millis() as u64,
        "language server ready"
    );
}

/// A language server is one of the long-lived children the app supervises, and
/// is watched by the same code that watches plugin helpers (03-02).
impl Supervised for ServerHandle {
    fn lifecycle(&self) -> &Lifecycle {
        &self.life
    }

    fn pid(&self) -> u32 {
        ServerHandle::pid(self)
    }

    fn stop(&self) -> impl std::future::Future<Output = ()> + Send {
        ServerHandle::stop(self)
    }

    fn kill(&self) {
        ServerHandle::kill(self);
    }
}

/// Deliver responses to whoever asked, and broadcast the rest.
async fn route(server: std::sync::Arc<ServerHandle>, mut transport: Transport) {
    while let Some(event) = transport.recv().await {
        match event {
            TransportEvent::Stderr(line) => server.note_log(line),
            TransportEvent::Message(message) => {
                server
                    .messages
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if message.method() == Some("window/logMessage")
                    && let Some(text) = message.params().and_then(|p| p["message"].as_str())
                {
                    server.note_log(text.to_owned());
                }
                if let Some(id) = message.id().and_then(Value::as_i64)
                    && message.is_response()
                {
                    let waiting = server.pending.lock().expect("pending lock").remove(&id);
                    match waiting {
                        Some(pending) => {
                            server
                                .timings
                                .lock()
                                .expect("timings lock")
                                .record(pending.sent.elapsed().as_micros() as u64);
                            let _ = pending.answer.send(message);
                        }
                        None => debug!(language = %server.language, id, "reply to nothing"),
                    }
                    continue;
                }
                // Notifications, and requests the server makes of us. A
                // subscriber that has fallen behind loses messages rather than
                // stalling the server; diagnostics are resent on the next edit.
                let _ = server.from_server.send(message);
            }
            TransportEvent::Broken(e) => {
                warn!(language = %server.language, error = %e, "unreadable server output");
            }
            TransportEvent::Exited(exit) => {
                let expected = exit.killed || matches!(server.state(), ServerState::Stopping);
                *server.connection.lock().expect("connection lock") = None;
                // Waking the waiters before the state changes means a request
                // in flight fails as "disconnected" rather than hanging until
                // its own timeout; the proxy turns that into an error response
                // for the tab that asked.
                server.pending.lock().expect("pending lock").clear();
                if expected {
                    server.set_state(ServerState::stopped());
                } else {
                    server.crashed(match exit.code {
                        Some(code) => format!("it exited with code {code}"),
                        None => "it was ended by a signal".to_owned(),
                    });
                }
                break;
            }
        }
    }
    // Nothing else is coming, so nobody should be left waiting.
    server.pending.lock().expect("pending lock").clear();
}

/// The plugin's `initializationOptions` with the registered setting folded in.
///
/// `initializationOptions` is a server's own settings without its namespace —
/// rust-analyzer reads `linkedProjects`, not `rust-analyzer.linkedProjects` —
/// so the leading segment of the registered setting is dropped. Sending it here
/// as well as in `didChangeConfiguration` is what lets a server that reads its
/// projects once get them right the first time.
fn initialization_options(config: &ServerConfig, server: &ServerHandle) -> Value {
    let Some(setting) = &config.register_setting else {
        return config.init_options.clone().unwrap_or(Value::Null);
    };
    let namespace = setting.split('.').next().unwrap_or_default();
    let registered = server.configuration_section(namespace);
    let Some(registered) = registered.as_object().filter(|map| !map.is_empty()) else {
        return config.init_options.clone().unwrap_or(Value::Null);
    };
    let mut merged = config.init_options.clone().unwrap_or_else(|| json!({}));
    if let Some(target) = merged.as_object_mut() {
        for (key, value) in registered {
            target.insert(key.clone(), value.clone());
        }
    }
    merged
}

/// A dotted path as a nested object: `a.b` and `1` become `{"a":{"b":1}}`.
fn nest(path: &str, value: Value) -> Value {
    path.split('.')
        .rev()
        .fold(value, |inner, part| json!({ part: inner }))
}

/// Fold `extra` into `settings`, objects merged key by key and everything else
/// replaced.
///
/// A settings object is a tree of small objects, and two plugins' hooks (or a
/// hook and `[lsp.register]`) usually touch different branches of it; merging
/// rather than replacing is what lets them coexist (work item 03-07).
fn merge(settings: &mut Value, extra: &Value) {
    match (settings, extra) {
        (Value::Object(into), Value::Object(from)) => {
            for (key, value) in from {
                match into.get_mut(key) {
                    Some(existing) => merge(existing, value),
                    None => {
                        into.insert(key.clone(), value.clone());
                    }
                }
            }
        }
        (into, from) => *into = from.clone(),
    }
}

/// What this client can do, which is what the editor client (02-04) supports.
/// Kept small on purpose: a server should not send what nothing will render.
fn client_capabilities() -> Value {
    json!({
        "general": { "positionEncodings": ["utf-16"] },
        "workspace": {
            "workspaceFolders": true,
            "configuration": true,
            "didChangeConfiguration": { "dynamicRegistration": false },
        },
        "textDocument": {
            "synchronization": { "dynamicRegistration": false, "didSave": true },
            "publishDiagnostics": { "relatedInformation": true },
            "completion": {
                "completionItem": {
                    "snippetSupport": false,
                    "documentationFormat": ["markdown", "plaintext"],
                },
                "contextSupport": true,
            },
            "hover": { "contentFormat": ["markdown", "plaintext"] },
            "signatureHelp": {
                "signatureInformation": { "documentationFormat": ["markdown", "plaintext"] },
            },
            "definition": { "linkSupport": false },
        },
        "window": { "workDoneProgress": true },
    })
}

/// `file:///C:/x/y` on Windows, `file:///x/y` elsewhere. Enough for a root;
/// document URIs are the editor client's business (02-04).
pub fn path_to_uri(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text);
    let mut uri = String::from("file://");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for ch in text.chars() {
        match ch {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '.' | '_' | '~' | '/' | ':' => uri.push(ch),
            ' ' => uri.push_str("%20"),
            other => {
                let mut buf = [0u8; 4];
                for byte in other.encode_utf8(&mut buf).as_bytes() {
                    uri.push_str(&format!("%{byte:02X}"));
                }
            }
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dotted_setting_becomes_a_nested_object() {
        assert_eq!(
            nest("rust-analyzer.linkedProjects", json!(["a", "b"])),
            json!({ "rust-analyzer": { "linkedProjects": ["a", "b"] } }),
        );
        assert_eq!(nest("flat", json!(1)), json!({ "flat": 1 }));
    }

    #[test]
    fn a_hook_adds_to_the_manifest_instead_of_replacing_it() {
        let mut settings =
            json!({ "gopls": { "buildFlags": ["-tags=x"], "usePlaceholders": true } });
        merge(
            &mut settings,
            &json!({ "gopls": { "env": { "GOFLAGS": "-mod=mod" } } }),
        );
        assert_eq!(
            settings,
            json!({
                "gopls": {
                    "buildFlags": ["-tags=x"],
                    "usePlaceholders": true,
                    "env": { "GOFLAGS": "-mod=mod" },
                }
            }),
        );
    }

    #[test]
    fn a_hook_wins_where_the_two_overlap() {
        let mut settings = json!({ "a": { "b": 1, "c": [1] }, "d": 1 });
        merge(&mut settings, &json!({ "a": { "b": 2, "c": [2, 3] } }));
        assert_eq!(settings, json!({ "a": { "b": 2, "c": [2, 3] }, "d": 1 }));
    }

    #[test]
    fn anything_that_is_not_two_objects_is_a_replacement() {
        let mut settings = json!({ "a": 1 });
        merge(&mut settings, &json!(["not", "an", "object"]));
        assert_eq!(settings, json!(["not", "an", "object"]));
    }
}
