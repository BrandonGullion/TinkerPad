//! The pool: one server per language and workspace root, shared by every
//! playground that needs it (`docs/ARCHITECTURE.md` §4.1, work item 02-02).
//!
//! Servers start on first use and outlive the tabs that used them. Closing a
//! tab drops its [`Attachment`], which sends `textDocument/didClose`; the
//! process stays up, so the next tab in that language is a `didOpen` and
//! nothing else. Only [`LspPool::shutdown`] (app exit) stops them.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinkerpad_core::{CoreEvent, LanguageId, Level, ServerState};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::server::{RequestError, ServerConfig, ServerHandle, path_to_uri};

/// The whole of app exit may not take longer than this, however many servers
/// there are and however wedged they may be.
pub const SHUTDOWN_BUDGET: Duration = Duration::from_secs(3);

/// Which server a playground needs: one per language per workspace root, so
/// two Rust playgrounds under one root share a `rust-analyzer`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ServerKey {
    pub language: String,
    /// Empty for a server that needs no root.
    pub root: Option<PathBuf>,
}

impl ServerKey {
    fn of(config: &ServerConfig) -> Self {
        Self {
            language: config.language.clone(),
            root: config.root.clone(),
        }
    }
}

/// A document a tab has open on a server. Dropping it sends `didClose`; the
/// server stays running.
#[derive(Debug)]
pub struct Attachment {
    server: Arc<ServerHandle>,
    uri: String,
}

impl Attachment {
    /// The server this document is attached to, for requests about it.
    pub fn server(&self) -> &Arc<ServerHandle> {
        &self.server
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// The document changed in the editor.
    pub async fn changed(&self, text: &str) -> Result<(), RequestError> {
        self.server.did_change(&self.uri, text).await
    }
}

impl Drop for Attachment {
    fn drop(&mut self) {
        // Drop can't await, and the close should not hold up closing a tab.
        let server = self.server.clone();
        let uri = std::mem::take(&mut self.uri);
        tokio::spawn(async move {
            if let Err(e) = server.did_close(&uri).await {
                debug!(error = %e, uri, "didClose was not delivered");
            }
        });
    }
}

/// Every language server the app has going.
#[derive(Debug)]
pub struct LspPool {
    servers: Mutex<HashMap<ServerKey, Arc<ServerHandle>>>,
    /// State changes for the UI. The app forwards these to its event bus.
    events: Option<mpsc::UnboundedSender<CoreEvent>>,
}

impl Default for LspPool {
    fn default() -> Self {
        Self::new(None)
    }
}

impl LspPool {
    pub fn new(events: Option<mpsc::UnboundedSender<CoreEvent>>) -> Self {
        Self {
            servers: Mutex::default(),
            events,
        }
    }

    /// Open a document on the server for `config`, starting it if this is the
    /// first playground to need it.
    ///
    /// Returns once the document has been sent, which means waiting for the
    /// handshake. A server that could not start fails here instead, having
    /// published why; the editor carries on without LSP.
    pub async fn attach(
        &self,
        config: &ServerConfig,
        uri: &str,
        text: &str,
    ) -> Result<Attachment, RequestError> {
        let server = self.server_for(config);
        let attachment = Attachment {
            server: server.clone(),
            uri: uri.to_owned(),
        };
        server.did_open(uri, &config.language, text).await?;
        Ok(attachment)
    }

    /// Attach a tab whose editor runs an LSP client of its own (02-04).
    ///
    /// The server is started if needed, but the document is not opened here:
    /// the editor's client sends `didOpen` itself, with its own version
    /// numbering, which is the numbering the server echoes back in
    /// diagnostics. The core watches those messages go past (see
    /// [`ServerHandle::sync_from_editor`]) rather than numbering them itself.
    pub fn attach_editor(&self, config: &ServerConfig, uri: &str) -> Attachment {
        Attachment {
            server: self.server_for(config),
            uri: uri.to_owned(),
        }
    }

    /// The URI form of a path, so callers can build a document URI the same
    /// way the pool builds a root one.
    pub fn uri_for(path: &std::path::Path) -> String {
        path_to_uri(path)
    }

    /// The server for `config`, started if it isn't already.
    pub fn server_for(&self, config: &ServerConfig) -> Arc<ServerHandle> {
        let key = ServerKey::of(config);
        let mut servers = self.servers.lock().expect("servers lock");
        if let Some(server) = servers.get(&key) {
            // A crashed server is already on its way back (02-05), so tabs
            // share it as usual and their requests wait. One the pool gave up
            // on gets another go, because opening a tab is someone asking for
            // it again.
            // A server that stopped (idle, or given up on) or is in the
            // middle of stopping starts again: someone is asking for it.
            // `restart` waits for the old process before starting another.
            if matches!(
                server.state(),
                ServerState::Stopped { .. } | ServerState::Stopping
            ) {
                info!(language = %key.language, "starting a server that had stopped");
                server.restart();
            } else {
                debug!(language = %key.language, "sharing the running server");
            }
            return server.clone();
        }
        info!(language = %key.language, root = ?key.root, "starting language server");
        let server = ServerHandle::start(config);
        servers.insert(key, server.clone());
        self.publish(&server);
        server
    }

    /// The server for a language and root, if one is running.
    pub fn get(&self, language: &str, root: Option<&std::path::Path>) -> Option<Arc<ServerHandle>> {
        let key = ServerKey {
            language: language.to_owned(),
            root: root.map(PathBuf::from),
        };
        self.servers
            .lock()
            .expect("servers lock")
            .get(&key)
            .cloned()
    }

    /// Every server, for the status strip (02-08).
    pub fn servers(&self) -> Vec<Arc<ServerHandle>> {
        self.servers
            .lock()
            .expect("servers lock")
            .values()
            .cloned()
            .collect()
    }

    /// How many servers the pool holds, running or not.
    pub fn len(&self) -> usize {
        self.servers.lock().expect("servers lock").len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Stop everything, politely and at once: `shutdown`, `exit`, then kill.
    /// Never takes longer than [`SHUTDOWN_BUDGET`], however many servers there
    /// are, because app exit waits for it.
    pub async fn shutdown(&self) {
        let servers = std::mem::take(&mut *self.servers.lock().expect("servers lock"));
        if servers.is_empty() {
            return;
        }
        let count = servers.len();
        let started = std::time::Instant::now();

        let mut stopping = tokio::task::JoinSet::new();
        for server in servers.values() {
            let server = server.clone();
            stopping.spawn(async move { server.stop().await });
        }
        let all = async { while stopping.join_next().await.is_some() {} };
        if tokio::time::timeout(SHUTDOWN_BUDGET, all).await.is_err() {
            warn!("some servers did not stop in time; killing them");
            for server in servers.values() {
                server.kill();
            }
        }
        info!(
            servers = count,
            ms = started.elapsed().as_millis() as u64,
            "language servers stopped"
        );
    }

    /// Restart one server now, whatever state it is in. The status strip's
    /// Restart button (02-08).
    pub fn restart(&self, language: &str, root: Option<&std::path::Path>) -> bool {
        match self.get(language, root) {
            Some(server) => {
                server.restart();
                true
            }
            None => false,
        }
    }

    /// Forward a server's state changes to the UI until it stops.
    fn publish(&self, server: &Arc<ServerHandle>) {
        let Some(events) = self.events.clone() else {
            return;
        };
        let server = server.clone();
        tokio::spawn(async move {
            let mut states = server.watch_state();
            loop {
                let state = states.borrow_and_update().clone();
                // A server that has given up stays stopped until someone asks
                // for it again, so that is where this ends.
                let stopped = matches!(
                    state,
                    ServerState::Stopped {
                        reason: Some(_),
                        ..
                    }
                );
                // A crash is worth interrupting someone for; the state alone
                // only shows in the status strip.
                if let ServerState::Crashed { restarts, .. } = &state {
                    let _ = events.send(CoreEvent::Toast {
                        level: Level::Warning,
                        message: format!(
                            "{} crashed, restarting… (attempt {restarts})",
                            server.language()
                        ),
                    });
                }
                let event = CoreEvent::Server {
                    language: LanguageId::new(server.language()),
                    root: server
                        .root()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                    state,
                };
                if events.send(event).is_err() || stopped {
                    return;
                }
                if states.changed().await.is_err() {
                    return;
                }
            }
        });
    }
}
