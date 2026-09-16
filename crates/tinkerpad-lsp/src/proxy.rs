//! The core in the middle of the editor and the server
//! (`docs/ARCHITECTURE.md` §4.2, work item 02-03).
//!
//! Every message passes through here, which buys three things: the editor only
//! ever names a playground (`tinkerpad://playground/<id>`) while the server
//! only ever sees files; the core owns request ids, so a tab that goes away
//! can't leave a server answering into nothing; and there is one place that
//! logs, counts, and times what flows.
//!
//! What the proxy does *not* do is rewrite positions. Kinds that need wrapper
//! code keep it in the file as locked lines (`docs/PLUGINS.md` §3), so the
//! server sees exactly the text the editor shows.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use serde_json::{Value, json};
use tinkerpad_core::{CoreEvent, Level};
use tokio::sync::mpsc;
use tracing::{debug, trace, warn};

use crate::jsonrpc::RawMessage;
use crate::pool::Attachment;
use crate::server::{SERVER_RESTARTED, ServerHandle};
use crate::uri::UriMap;

/// JSON-RPC's "no such method", for a server request the core won't answer.
const METHOD_NOT_FOUND: i64 = -32601;

/// How many request timings are kept per server for the percentiles the status
/// strip shows (02-08).
const TIMINGS: usize = 512;

/// Where messages for one tab's editor go. The app implements this over a
/// Tauri channel; tests collect into a vector.
pub trait EditorSink: Send + Sync + 'static {
    fn send(&self, message: RawMessage);
}

/// One tab's connection: which playground it shows and where its messages go.
struct Tab {
    playground: String,
    /// Holds the document open on the server. Dropping it sends `didClose`.
    attachment: Attachment,
    sink: Arc<dyn EditorSink>,
    /// Core ids of requests this tab is waiting on.
    in_flight: Vec<i64>,
}

/// A request on its way through: which tab asked, and under what id.
struct Pending {
    tab: String,
    editor_id: Value,
    method: String,
    started: Instant,
}

/// What one server's traffic has looked like, for the status strip.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub to_server: u64,
    pub to_editor: u64,
    pub requests: u64,
    /// Request round trips, in microseconds.
    pub p50_us: u64,
    pub p95_us: u64,
}

#[derive(Default)]
struct Timings {
    to_server: u64,
    to_editor: u64,
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

    fn stats(&self) -> Stats {
        let mut sorted: Vec<u64> = self.recent.iter().copied().collect();
        sorted.sort_unstable();
        let at = |fraction: f64| -> u64 {
            if sorted.is_empty() {
                return 0;
            }
            let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
            sorted[index]
        };
        Stats {
            to_server: self.to_server,
            to_editor: self.to_editor,
            requests: self.requests,
            p50_us: at(0.5),
            p95_us: at(0.95),
        }
    }
}

#[derive(Default)]
struct Inner {
    tabs: HashMap<String, Tab>,
    /// Core request id → who is waiting for it.
    pending: HashMap<i64, Pending>,
    uris: UriMap,
    stats: Timings,
}

/// The editor's side of every open tab.
pub struct Proxy {
    inner: Mutex<Inner>,
    /// Toasts and other app-level events (`window/showMessage`).
    events: Option<mpsc::UnboundedSender<CoreEvent>>,
    /// Servers whose messages are already being listened to.
    listening: Mutex<Vec<Arc<ServerHandle>>>,
    forwarded: AtomicU64,
}

impl std::fmt::Debug for Proxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock().expect("proxy lock");
        f.debug_struct("Proxy")
            .field("tabs", &inner.tabs.len())
            .field("pending", &inner.pending.len())
            .finish_non_exhaustive()
    }
}

impl Proxy {
    pub fn new(events: Option<mpsc::UnboundedSender<CoreEvent>>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::default(),
            events,
            listening: Mutex::default(),
            forwarded: AtomicU64::new(0),
        })
    }

    /// Connect a tab: its document is already open on the server (that is what
    /// `attachment` means), and from now on its messages flow both ways.
    ///
    /// Returns the URI the editor should use for the document.
    pub fn open_tab(
        self: &Arc<Self>,
        playground: &str,
        main_file: &std::path::Path,
        attachment: Attachment,
        sink: Arc<dyn EditorSink>,
    ) -> String {
        let server = attachment.server().clone();
        let uri = {
            let mut inner = self.inner.lock().expect("proxy lock");
            let uri = inner.uris.insert(playground, main_file);
            inner.tabs.insert(
                playground.to_owned(),
                Tab {
                    playground: playground.to_owned(),
                    attachment,
                    sink,
                    in_flight: Vec::new(),
                },
            );
            uri
        };
        self.listen(server);
        debug!(playground, %uri, "tab connected to its language server");
        uri
    }

    /// Disconnect a tab: in-flight requests are cancelled, its document is
    /// closed (the attachment drops), and the server stays up.
    pub async fn close_tab(&self, playground: &str) {
        let (tab, cancelled) = {
            let mut inner = self.inner.lock().expect("proxy lock");
            let Some(tab) = inner.tabs.remove(playground) else {
                return;
            };
            inner.uris.remove(playground);
            let cancelled: Vec<i64> = tab.in_flight.clone();
            for id in &cancelled {
                inner.pending.remove(id);
            }
            (tab, cancelled)
        };
        // Nothing is waiting for these answers any more, so tell the server to
        // stop working on them.
        let server = tab.attachment.server().clone();
        for id in cancelled {
            server.cancel_request(id).await;
        }
        debug!(playground, "tab disconnected");
        drop(tab);
    }

    /// A message from one tab's editor.
    pub async fn from_editor(self: &Arc<Self>, playground: &str, mut message: RawMessage) {
        let Some((server, editor_id)) = self.prepare(playground, &mut message) else {
            return;
        };
        let method = message.method().unwrap_or_default().to_owned();
        trace!(playground, %method, bytes = message.to_vec().len(), "editor → server");

        // The editor runs a client of its own, which wants a handshake and
        // wants to sync its document. Neither goes to the server as-is: the
        // core did the handshake once for every tab sharing this server, and
        // it owns document versions (02-04).
        match method.as_str() {
            "initialize" => {
                if let Some(id) = editor_id {
                    // Waiting here is the point: the handshake may still be
                    // running, and an editor that hears "no capabilities" acts
                    // on that for as long as the tab is open.
                    let answer = match server.ready().await {
                        Ok(capabilities) => RawMessage::response(
                            id,
                            json!({
                                "capabilities": capabilities,
                                "serverInfo": { "name": server.language() },
                            }),
                        ),
                        Err(e) => RawMessage::error(id, METHOD_NOT_FOUND, &e.to_string()),
                    };
                    self.to_editor(playground, answer);
                }
                return;
            }
            // The core already sent these, or will when the tab closes.
            "initialized" | "shutdown" | "exit" => {
                if let Some(id) = editor_id {
                    self.to_editor(playground, RawMessage::response(id, Value::Null));
                }
                return;
            }
            "textDocument/didOpen" | "textDocument/didChange" | "textDocument/didClose" => {
                let Some(renumbered) = server.sync_from_editor(&message) else {
                    return;
                };
                self.inner.lock().expect("proxy lock").stats.to_server += 1;
                if let Err(e) = server.send(renumbered).await {
                    debug!(playground, %method, error = %e, "document sync not delivered");
                }
                return;
            }
            _ => {}
        }

        match editor_id {
            // A request: the core gives it an id of its own and remembers who
            // is waiting, so the answer finds its way back to this tab.
            Some(editor_id) => {
                let params = message.params().cloned().unwrap_or(Value::Null);
                let answer = match server.begin_request(&method, params).await {
                    Ok((core_id, answer)) => {
                        let mut inner = self.inner.lock().expect("proxy lock");
                        inner.stats.to_server += 1;
                        inner.pending.insert(
                            core_id,
                            Pending {
                                tab: playground.to_owned(),
                                editor_id: editor_id.clone(),
                                method: method.clone(),
                                started: Instant::now(),
                            },
                        );
                        if let Some(tab) = inner.tabs.get_mut(playground) {
                            tab.in_flight.push(core_id);
                        }
                        Some((core_id, answer))
                    }
                    Err(e) => {
                        // A server that isn't there is an error response, not
                        // a silence: the editor must never wait forever.
                        self.to_editor(
                            playground,
                            RawMessage::error(editor_id, METHOD_NOT_FOUND, &e.to_string()),
                        );
                        None
                    }
                };
                if let Some((core_id, answer)) = answer {
                    let proxy = self.clone();
                    tokio::spawn(async move {
                        let response = answer.await.ok();
                        proxy.answered(core_id, response);
                    });
                }
            }
            // A notification: straight through, once the URIs are rewritten.
            None => {
                self.inner.lock().expect("proxy lock").stats.to_server += 1;
                if let Err(e) = server.send(message).await {
                    debug!(playground, %method, error = %e, "notification not delivered");
                }
            }
        }
    }

    /// Rewrite a message for the server and find the server it belongs to.
    /// `None` when the tab is unknown.
    fn prepare(
        &self,
        playground: &str,
        message: &mut RawMessage,
    ) -> Option<(Arc<ServerHandle>, Option<Value>)> {
        let inner = self.inner.lock().expect("proxy lock");
        let tab = inner.tabs.get(playground)?;
        let server = tab.attachment.server().clone();
        inner.uris.to_server(message);
        let editor_id = message
            .is_request()
            .then(|| message.id().cloned())
            .flatten();
        Some((server, editor_id))
    }

    /// A server answered a request. Give the answer back under the id the
    /// editor used.
    fn answered(&self, core_id: i64, response: Option<RawMessage>) {
        let (tab, mut message) = {
            let mut inner = self.inner.lock().expect("proxy lock");
            let Some(pending) = inner.pending.remove(&core_id) else {
                // The tab went away, so there is nobody to tell.
                return;
            };
            inner
                .stats
                .record(pending.started.elapsed().as_micros() as u64);
            if let Some(tab) = inner.tabs.get_mut(&pending.tab) {
                tab.in_flight.retain(|id| *id != core_id);
            }
            let message = match response {
                Some(mut response) => {
                    inner.uris.to_editor(&mut response);
                    replace_id(response, pending.editor_id)
                }
                // The server went away before answering. The editor gets an
                // error rather than a request that never completes, with the
                // code that says "transient": ask again (02-05).
                None => RawMessage::error(
                    pending.editor_id,
                    SERVER_RESTARTED,
                    &format!(
                        "the language server restarted before answering {}",
                        pending.method
                    ),
                ),
            };
            (pending.tab, message)
        };
        self.inner
            .lock()
            .expect("proxy lock")
            .uris
            .to_editor(&mut message);
        self.to_editor(&tab, message);
    }

    /// Start listening to a server's notifications and requests, if this is
    /// the first tab to use it.
    fn listen(self: &Arc<Self>, server: Arc<ServerHandle>) {
        {
            let mut listening = self.listening.lock().expect("listening lock");
            if listening.iter().any(|s| Arc::ptr_eq(s, &server)) {
                return;
            }
            listening.push(server.clone());
        }
        let proxy = self.clone();
        tokio::spawn(async move {
            let mut messages = server.subscribe();
            loop {
                match messages.recv().await {
                    Ok(message) => proxy.from_server(&server, message).await,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(
                            language = server.language(),
                            missed = n,
                            "proxy fell behind"
                        );
                    }
                    Err(_) => break,
                }
            }
            proxy
                .listening
                .lock()
                .expect("listening lock")
                .retain(|s| !Arc::ptr_eq(s, &server));
        });
    }

    /// Something the server said that wasn't a reply.
    async fn from_server(self: &Arc<Self>, server: &Arc<ServerHandle>, message: RawMessage) {
        let method = message.method().unwrap_or_default().to_owned();
        trace!(language = server.language(), %method, bytes = message.to_vec().len(), "server → editor");

        // Requests the server makes of us are answered here, not sent to the
        // editor: they are about the session, which the core owns.
        if message.is_request() {
            let id = message.id().cloned().unwrap_or(Value::Null);
            let answer = match method.as_str() {
                // One value per item asked about: whatever the plugin's
                // `[lsp.register]` has put there, and null for anything else
                // (per-workspace settings arrive in 04-02).
                "workspace/configuration" => {
                    let items = message
                        .params()
                        .and_then(|p| p.get("items"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    let answers: Vec<Value> = items
                        .iter()
                        .map(|item| {
                            let section = item.get("section").and_then(Value::as_str).unwrap_or("");
                            server.configuration_section(section)
                        })
                        .collect();
                    server.note_configuration_request();
                    RawMessage::response(id, Value::Array(answers))
                }
                // Accepted: the core's capabilities don't change per document.
                "client/registerCapability" | "client/unregisterCapability" => {
                    RawMessage::response(id, Value::Null)
                }
                // Progress is welcome; the status strip shows it (02-08).
                "window/workDoneProgress/create" => RawMessage::response(id, Value::Null),
                // Pull diagnostics are not used here — the server publishes
                // them — so there is nothing to refresh, and saying so is
                // friendlier than an error.
                "workspace/diagnostic/refresh"
                | "workspace/semanticTokens/refresh"
                | "workspace/inlayHint/refresh"
                | "workspace/codeLens/refresh" => RawMessage::response(id, Value::Null),
                other => RawMessage::error(
                    id,
                    METHOD_NOT_FOUND,
                    &format!("TinkerPad does not answer {other}"),
                ),
            };
            if let Err(e) = server.send(answer).await {
                debug!(error = %e, %method, "could not answer the server");
            }
            return;
        }

        match method.as_str() {
            // Diagnostics belong to one document, so they go to that tab only.
            "textDocument/publishDiagnostics" => {
                let mut message = message;
                let uri = message
                    .params()
                    .and_then(|p| p.get("uri"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let owner = {
                    let inner = self.inner.lock().expect("proxy lock");
                    inner.uris.playground_for_file(&uri).map(str::to_owned)
                };
                match owner {
                    Some(playground) => {
                        self.inner
                            .lock()
                            .expect("proxy lock")
                            .uris
                            .to_editor(&mut message);
                        self.to_editor(&playground, message);
                    }
                    None => trace!(%uri, "diagnostics for a file no tab shows"),
                }
            }
            // A message for the user, which the app shows as a toast.
            "window/showMessage" => {
                let params = message.params().cloned().unwrap_or(Value::Null);
                let text = params["message"].as_str().unwrap_or_default().to_owned();
                let level = match params["type"].as_i64().unwrap_or(3) {
                    1 => Level::Error,
                    2 => Level::Warning,
                    _ => Level::Info,
                };
                self.publish(CoreEvent::Toast {
                    level,
                    message: format!("{}: {text}", server.language()),
                });
            }
            // Servers are chatty; this is what the log is for.
            "window/logMessage" => {
                let params = message.params().cloned().unwrap_or(Value::Null);
                debug!(
                    language = server.language(),
                    "{}",
                    params["message"].as_str().unwrap_or_default()
                );
            }
            // Anything else a server sends goes to every tab it serves, with
            // its URIs translated.
            _ => {
                let mut message = message;
                let tabs: Vec<String> = {
                    let inner = self.inner.lock().expect("proxy lock");
                    inner.uris.to_editor(&mut message);
                    inner
                        .tabs
                        .values()
                        .filter(|tab| Arc::ptr_eq(tab.attachment.server(), server))
                        .map(|tab| tab.playground.clone())
                        .collect()
                };
                for playground in tabs {
                    self.to_editor(&playground, message.clone());
                }
            }
        }
    }

    /// Hand a message to one tab's editor.
    fn to_editor(&self, playground: &str, message: RawMessage) {
        let inner = self.inner.lock().expect("proxy lock");
        let Some(tab) = inner.tabs.get(playground) else {
            return;
        };
        tab.sink.send(message);
        self.forwarded.fetch_add(1, Ordering::Relaxed);
    }

    fn publish(&self, event: CoreEvent) {
        if let Some(events) = &self.events {
            let _ = events.send(event);
        }
    }

    /// How many tabs are connected.
    pub fn tabs(&self) -> usize {
        self.inner.lock().expect("proxy lock").tabs.len()
    }

    /// Requests waiting for an answer.
    pub fn in_flight(&self) -> usize {
        self.inner.lock().expect("proxy lock").pending.len()
    }

    /// Traffic and request latency, for the status strip (02-08).
    pub fn stats(&self) -> Stats {
        let inner = self.inner.lock().expect("proxy lock");
        let mut stats = inner.stats.stats();
        stats.to_editor = self.forwarded.load(Ordering::Relaxed);
        stats
    }
}

/// The same response under the id the editor used.
fn replace_id(mut response: RawMessage, editor_id: Value) -> RawMessage {
    if let Some(object) = response.object_mut() {
        object.insert("id".into(), editor_id);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn percentiles_come_from_the_recent_requests() {
        let mut timings = Timings::default();
        assert_eq!(timings.stats().p50_us, 0, "no requests, no percentile");
        for micros in 0..100 {
            timings.record(micros);
        }
        // Nearest rank over 0..99: the middle sample and the 95th of them.
        let stats = timings.stats();
        assert_eq!(stats.requests, 100);
        assert_eq!(stats.p50_us, 50);
        assert_eq!(stats.p95_us, 94);
    }

    #[test]
    fn only_the_most_recent_timings_are_kept() {
        let mut timings = Timings::default();
        for micros in 0..(TIMINGS as u64 * 2) {
            timings.record(micros);
        }
        assert_eq!(timings.requests, TIMINGS as u64 * 2);
        assert_eq!(timings.recent.len(), TIMINGS, "the window does not grow");
        assert!(
            timings.stats().p50_us > TIMINGS as u64,
            "the window holds the later half",
        );
    }

    #[test]
    fn a_response_comes_back_under_the_editors_id() {
        let response = RawMessage::response(41, json!({ "ok": true }));
        let swapped = replace_id(response, json!("editor-7"));
        assert_eq!(swapped.id(), Some(&json!("editor-7")));
        assert_eq!(swapped.result(), Some(&json!({ "ok": true })));
    }
}
