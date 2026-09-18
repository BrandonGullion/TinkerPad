//! The pool: one server per language and workspace root, shared by every
//! playground that needs it (`docs/ARCHITECTURE.md` §4.1, work item 02-02).
//!
//! Servers start on first use and outlive the tabs that used them. Closing a
//! tab drops its [`Attachment`], which sends `textDocument/didClose`; the
//! process stays up, so the next tab in that language is a `didOpen` and
//! nothing else. Only [`LspPool::shutdown`] (app exit) stops them.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;
use tinkerpad_core::{CoreEvent, LanguageId, Level, ServerState};
use tinkerpad_process::supervise::Pool;
use tokio::sync::mpsc;
use tracing::{debug, info};

/// The whole of app exit may not take longer than this, however many servers
/// there are and however wedged they may be.
pub use tinkerpad_process::supervise::SHUTDOWN_BUDGET;

use crate::server::{RequestError, ServerConfig, ServerHandle, path_to_uri};

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
    /// Whether this playground was added to the server's registered setting,
    /// and so has to be taken out again (work item 02-07).
    registered: bool,
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
        // Taken out of the registered list here and now, so a tab opening
        // straight after this one closes sees the list as it will be; the
        // server is told from the task below.
        let forgotten = self.registered && server.forget_registration(&uri);
        tokio::spawn(async move {
            if let Err(e) = server.did_close(&uri).await {
                debug!(error = %e, uri, "didClose was not delivered");
            }
            if forgotten {
                server.push_registrations().await;
            }
        });
    }
}

/// Every language server the app has going.
///
/// The holding, sharing, and stopping-at-exit is [`Pool`], which the helper
/// pool uses too (03-02); what is here is what makes these servers rather than
/// some other long-lived child.
#[derive(Debug)]
pub struct LspPool {
    servers: Pool<ServerKey, ServerHandle>,
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
            servers: Pool::new(),
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
            registered: false,
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
        self.attach_registered(config, uri, None)
    }

    /// [`attach_editor`](Self::attach_editor), plus what this playground adds
    /// to the server's registered setting — the path to its `Cargo.toml`, for
    /// rust-analyzer's `linkedProjects` (work item 02-07).
    ///
    /// The setting is pushed as soon as the server is ready, before the editor
    /// opens the document, so the server knows which project the file belongs
    /// to when it arrives.
    pub fn attach_registered(
        &self,
        config: &ServerConfig,
        uri: &str,
        registration: Option<Value>,
    ) -> Attachment {
        let registered = registration.is_some() && config.register_setting.is_some();
        let value = registration.filter(|_| registered);

        // A server starting for the first time is told about this playground
        // before it starts, so its handshake carries it: one that reads its
        // projects once would otherwise find none (02-07). An already-running
        // server is told the usual way, and asked to reload.
        let (server, fresh) = self.server_or_new(config, |server| {
            if let Some(value) = value.clone() {
                server.note_registration(uri, value);
            }
        });
        if let Some(value) = value
            && !fresh
        {
            // A running server with nothing registered has nothing loaded, and
            // one that found no projects when it started does not reliably
            // pick them up from a configuration change (rust-analyzer does
            // not). Starting it again is cheap when it is doing nothing, and
            // its handshake then carries the project.
            if server.note_registration(uri, value) {
                let server = server.clone();
                tokio::spawn(async move { server.push_registrations().await });
            }
        }
        Attachment {
            server,
            uri: uri.to_owned(),
            registered,
        }
    }

    /// The URI form of a path, so callers can build a document URI the same
    /// way the pool builds a root one.
    pub fn uri_for(path: &std::path::Path) -> String {
        path_to_uri(path)
    }

    /// The server for `config`, started if it isn't already, and whether this
    /// call is what started it. `before_start` runs on a new server before it
    /// is connected, which is the only moment its handshake can be influenced.
    fn server_or_new(
        &self,
        config: &ServerConfig,
        before_start: impl FnOnce(&Arc<ServerHandle>),
    ) -> (Arc<ServerHandle>, bool) {
        let key = ServerKey::of(config);
        let (server, fresh) = self.servers.get_or_make(
            &key,
            || {
                info!(language = %key.language, root = ?key.root, "starting language server");
                let server = ServerHandle::prepare(config);
                before_start(&server);
                server.begin();
                server
            },
            |server| {
                // A crashed server is already on its way back (02-05), so tabs
                // share it as usual and their requests wait. One that stopped —
                // idle, or given up on — starts again, because opening a tab is
                // someone asking for it.
                if matches!(
                    server.state(),
                    ServerState::Stopped { .. } | ServerState::Stopping
                ) {
                    info!(language = %key.language, "starting a server that had stopped");
                    server.restart();
                } else {
                    debug!(language = %key.language, "sharing the running server");
                }
            },
        );
        if fresh {
            self.publish(&server);
        }
        (server, fresh)
    }

    /// The server for `config`, started if it isn't already.
    pub fn server_for(&self, config: &ServerConfig) -> Arc<ServerHandle> {
        self.server_or_new(config, |_| {}).0
    }

    /// The server for a language and root, if one is running.
    pub fn get(&self, language: &str, root: Option<&std::path::Path>) -> Option<Arc<ServerHandle>> {
        let key = ServerKey {
            language: language.to_owned(),
            root: root.map(PathBuf::from),
        };
        self.servers.get(&key)
    }

    /// Every server, for the status strip (02-08).
    pub fn servers(&self) -> Vec<Arc<ServerHandle>> {
        self.servers.all()
    }

    /// How many servers the pool holds, running or not.
    pub fn len(&self) -> usize {
        self.servers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Stop everything, politely and at once: `shutdown`, `exit`, then kill.
    /// Never takes longer than [`SHUTDOWN_BUDGET`], however many servers there
    /// are, because app exit waits for it.
    pub async fn shutdown(&self) {
        self.servers.shutdown(SHUTDOWN_BUDGET).await;
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
