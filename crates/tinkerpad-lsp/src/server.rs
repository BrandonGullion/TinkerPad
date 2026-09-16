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
use tokio::sync::{broadcast, oneshot, watch};
use tracing::{debug, info, warn};

use crate::jsonrpc::RawMessage;
use crate::transport::{Sender, Transport, TransportEvent};

/// How long a request waits for a server that is still starting before giving
/// up. Long enough for rust-analyzer on a cold cache to answer the handshake,
/// short enough that the editor doesn't look hung.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `shutdown` and `exit` are given before the process is killed.
pub const STOP_GRACE: Duration = Duration::from_secs(2);

/// How many server-initiated messages are held for a subscriber that is slow.
const BROADCAST: usize = 1024;

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

/// A running (or failed) language server.
#[derive(Debug)]
pub struct ServerHandle {
    language: String,
    /// The process id, or 0 when the server never started.
    pid: u32,
    start_timeout: Duration,
    root: Option<std::path::PathBuf>,
    state: watch::Sender<ServerState>,
    sender: Option<Sender>,
    kill: Option<tinkerpad_process::KillHandle>,
    next_id: AtomicI64,
    pending: Mutex<HashMap<i64, oneshot::Sender<RawMessage>>>,
    open_docs: Mutex<HashMap<String, DocState>>,
    /// Notifications and server-to-client requests, for the proxy (02-03).
    from_server: broadcast::Sender<RawMessage>,
}

impl ServerHandle {
    /// A handle for a server that could not be started. It answers every
    /// request with [`RequestError::NotRunning`], so a missing language server
    /// never stops the editor from working.
    pub fn failed(config: &ServerConfig, reason: String) -> Self {
        let state = ServerState::failed(reason, config.install_hint.clone());
        Self {
            language: config.language.clone(),
            pid: 0,
            start_timeout: config.start_timeout,
            root: config.root.clone(),
            state: watch::Sender::new(state),
            sender: None,
            kill: None,
            next_id: AtomicI64::new(1),
            pending: Mutex::default(),
            open_docs: Mutex::default(),
            from_server: broadcast::channel(BROADCAST).0,
        }
    }

    /// Spawn the server and start its router. The `initialize` handshake runs
    /// in the background: the state goes `Starting` → `Ready`, and requests
    /// made in between wait for it.
    pub fn start(config: &ServerConfig) -> std::sync::Arc<Self> {
        let transport = match Transport::spawn(&config.language, config.spawn.clone()) {
            Ok(transport) => transport,
            Err(e) => {
                warn!(language = %config.language, error = %e, "language server did not start");
                return std::sync::Arc::new(Self::failed(config, e.to_string()));
            }
        };

        let server = std::sync::Arc::new(Self {
            language: config.language.clone(),
            pid: transport.pid(),
            start_timeout: config.start_timeout,
            root: config.root.clone(),
            state: watch::Sender::new(ServerState::Starting),
            sender: Some(transport.sender()),
            kill: Some(transport.kill_handle()),
            next_id: AtomicI64::new(1),
            pending: Mutex::default(),
            open_docs: Mutex::default(),
            from_server: broadcast::channel(BROADCAST).0,
        });

        tokio::spawn(route(server.clone(), transport));
        tokio::spawn(handshake(server.clone(), config.clone()));
        server
    }

    pub fn language(&self) -> &str {
        &self.language
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn state(&self) -> ServerState {
        self.state.borrow().clone()
    }

    /// State changes, for the pool to forward to the UI.
    pub fn watch_state(&self) -> watch::Receiver<ServerState> {
        self.state.subscribe()
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

    pub fn is_open(&self, uri: &str) -> bool {
        self.open_docs
            .lock()
            .expect("open_docs lock")
            .contains_key(uri)
    }

    /// Make a request and wait for its answer. If the server is still
    /// starting, the request waits for the handshake ([`START_TIMEOUT`]).
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, RequestError> {
        self.wait_ready().await?;
        let sender = self.sender.as_ref().ok_or_else(|| self.not_running())?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(id, tx);

        if sender
            .send(RawMessage::request(id, method, params))
            .await
            .is_err()
        {
            self.pending.lock().expect("pending lock").remove(&id);
            return Err(self.disconnected());
        }

        let response = rx.await.map_err(|_| self.disconnected())?;
        if let Some(error) = response.error_object() {
            return Err(RequestError::Failed {
                method: method.to_owned(),
                code: error["code"].as_i64().unwrap_or(0),
                message: error["message"].as_str().unwrap_or("no message").to_owned(),
            });
        }
        Ok(response.result().cloned().unwrap_or(Value::Null))
    }

    /// Send a notification, which has no answer.
    pub async fn notify(&self, method: &str, params: Value) -> Result<(), RequestError> {
        self.wait_ready().await?;
        self.notify_now(method, params).await
    }

    /// Send a notification without waiting for the handshake. Used by the
    /// handshake itself (`initialized`) and by shutdown.
    async fn notify_now(&self, method: &str, params: Value) -> Result<(), RequestError> {
        let sender = self.sender.as_ref().ok_or_else(|| self.not_running())?;
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
        if self.sender.is_none() || matches!(self.state(), ServerState::Stopped { .. }) {
            return;
        }
        let mut exited = self.state.subscribe();
        self.set_state(ServerState::Stopping);

        // A server that is wedged shouldn't hold up the app, so the polite
        // path gets a deadline of its own.
        let polite = async {
            let _ = tokio::time::timeout(STOP_GRACE, self.request("shutdown", Value::Null)).await;
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
        if let Some(kill) = &self.kill {
            kill.kill();
        }
    }

    fn set_state(&self, state: ServerState) {
        self.state.send_if_modified(|current| {
            if *current == state {
                return false;
            }
            debug!(language = %self.language, ?state, "server state");
            *current = state;
            true
        });
    }

    /// Wait for the handshake to finish, if it is still running.
    async fn wait_ready(&self) -> Result<(), RequestError> {
        match self.state() {
            ServerState::Ready { .. } => return Ok(()),
            ServerState::Starting => {}
            // Stopping still accepts the `shutdown` request that started it.
            ServerState::Stopping => return Ok(()),
            ServerState::Stopped { .. } | ServerState::Crashed { .. } => {
                return Err(self.not_running());
            }
        }
        let mut states = self.state.subscribe();
        let settled = states.wait_for(|state| !state.is_settling());
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
        "initializationOptions": config.init_options,
    });

    // Sent directly: the state is `Starting`, which is exactly what everything
    // else is waiting for.
    let started = std::time::Instant::now();
    let id = server.next_id.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    server.pending.lock().expect("pending lock").insert(id, tx);
    let sent = match server.sender.as_ref() {
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

    let info = response
        .result()
        .and_then(|result| result.get("serverInfo").cloned())
        .unwrap_or(Value::Null);
    let _ = server.notify_now("initialized", json!({})).await;
    server.set_state(ServerState::Ready {
        pid: server.pid,
        name: info["name"].as_str().map(str::to_owned),
        version: info["version"].as_str().map(str::to_owned),
    });
    info!(
        language = %server.language,
        ms = started.elapsed().as_millis() as u64,
        "language server ready"
    );
}

/// Deliver responses to whoever asked, and broadcast the rest.
async fn route(server: std::sync::Arc<ServerHandle>, mut transport: Transport) {
    while let Some(event) = transport.recv().await {
        match event {
            TransportEvent::Message(message) => {
                if let Some(id) = message.id().and_then(Value::as_i64)
                    && message.is_response()
                {
                    let waiting = server.pending.lock().expect("pending lock").remove(&id);
                    match waiting {
                        Some(tx) => {
                            let _ = tx.send(message);
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
                server.set_state(if expected {
                    ServerState::stopped()
                } else {
                    ServerState::Crashed {
                        reason: match exit.code {
                            Some(code) => format!("it exited with code {code}"),
                            None => "it was ended by a signal".to_owned(),
                        },
                    }
                });
                break;
            }
        }
    }
    // Nothing else is coming, so nobody should be left waiting.
    server.pending.lock().expect("pending lock").clear();
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
