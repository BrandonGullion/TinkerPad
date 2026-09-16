//! The proxy between editor and server: URI rewriting, id ownership, and what
//! happens to a tab's requests when it goes away (work item 02-03).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tinkerpad_core::{CoreEvent, Level};
use tinkerpad_lsp::jsonrpc::RawMessage;
use tinkerpad_lsp::{EditorSink, LspPool, Proxy, ServerConfig, UriMap};
use tinkerpad_process::SpawnSpec;
use tokio::sync::mpsc;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakelsp");

/// One tab's editor, collecting what the proxy sends it.
#[derive(Clone, Default)]
struct Editor(Arc<Mutex<Vec<RawMessage>>>);

impl EditorSink for Editor {
    fn send(&self, message: RawMessage) {
        self.0.lock().expect("editor lock").push(message);
    }
}

impl Editor {
    fn messages(&self) -> Vec<RawMessage> {
        self.0.lock().expect("editor lock").clone()
    }

    /// Wait for a message matching `ok`, or fail.
    async fn wait(&self, what: &str, ok: impl Fn(&RawMessage) -> bool) -> RawMessage {
        for _ in 0..600 {
            if let Some(message) = self.messages().into_iter().find(|m| ok(m)) {
                return message;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no {what} arrived; got {:?}", self.messages());
    }

    async fn response_to(&self, id: Value) -> RawMessage {
        self.wait(&format!("response to {id}"), |m| m.id() == Some(&id))
            .await
    }
}

fn main_file(name: &str) -> PathBuf {
    PathBuf::from(format!("/library/{name}/main.rs"))
}

fn file_uri(name: &str) -> String {
    tinkerpad_lsp::path_to_uri(&main_file(name))
}

/// A proxy, a pool, and one tab attached to the fake server.
async fn tab(pool: &LspPool, proxy: &Arc<Proxy>, playground: &str) -> (Editor, String) {
    let config = ServerConfig::new("fake", SpawnSpec::new(FAKE));
    let attachment = pool
        .attach(&config, &file_uri(playground), "source")
        .await
        .unwrap();
    let editor = Editor::default();
    let uri = proxy.open_tab(
        playground,
        &main_file(playground),
        attachment,
        Arc::new(editor.clone()),
    );
    (editor, uri)
}

#[tokio::test]
async fn the_editor_names_a_playground_and_the_server_sees_a_file() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (editor, uri) = tab(&pool, &proxy, "one").await;
    assert_eq!(uri, UriMap::playground_uri("one"));

    // A request from the editor, in the editor's naming.
    proxy
        .from_editor(
            "one",
            RawMessage::request(
                7,
                "test/echo",
                json!({ "textDocument": { "uri": uri }, "position": { "line": 1, "character": 0 } }),
            ),
        )
        .await;

    // The fake server echoes its params, so the response shows what it saw.
    let response = editor.response_to(json!(7)).await;
    let result = response.result().unwrap();
    assert_eq!(
        response.id(),
        Some(&json!(7)),
        "the answer carries the editor's own id",
    );
    assert_eq!(
        result["textDocument"]["uri"],
        json!(uri),
        "and comes back in the editor's naming",
    );

    // What the server was actually told is the file path.
    let seen = proxy.stats();
    assert_eq!(seen.to_server, 1);
    assert_eq!(seen.requests, 1);

    proxy.close_tab("one").await;
    pool.shutdown().await;
}

#[tokio::test]
async fn a_file_outside_the_library_stays_a_file_uri() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (editor, _) = tab(&pool, &proxy, "one").await;

    // A definition result pointing into a dependency: the editor gets the
    // file URI, so 02-09 can open it read-only.
    let dependency = "file:///home/me/.cargo/registry/serde/src/lib.rs";
    proxy
        .from_editor(
            "one",
            RawMessage::request(1, "test/echo", json!([{ "uri": dependency, "range": {} }])),
        )
        .await;
    let response = editor.response_to(json!(1)).await;
    assert_eq!(response.result().unwrap()[0]["uri"], json!(dependency));

    proxy.close_tab("one").await;
    pool.shutdown().await;
}

#[tokio::test]
async fn two_tabs_interleave_without_their_answers_crossing() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (one, one_uri) = tab(&pool, &proxy, "one").await;
    let (two, two_uri) = tab(&pool, &proxy, "two").await;
    assert_eq!(proxy.tabs(), 2);

    // Both tabs use id 1, which is exactly the case core-owned ids exist for.
    for n in 0..20 {
        proxy
            .from_editor(
                "one",
                RawMessage::request(
                    1,
                    "test/echo",
                    json!({ "tab": "one", "n": n, "uri": one_uri }),
                ),
            )
            .await;
        proxy
            .from_editor(
                "two",
                RawMessage::request(
                    1,
                    "test/echo",
                    json!({ "tab": "two", "n": n, "uri": two_uri }),
                ),
            )
            .await;
    }

    for (editor, name, uri) in [(&one, "one", &one_uri), (&two, "two", &two_uri)] {
        editor
            .wait("20 answers", |m| m.id() == Some(&json!(1)))
            .await;
        for _ in 0..100 {
            if editor.messages().len() == 20 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let messages = editor.messages();
        assert_eq!(messages.len(), 20, "{name} got its own answers and no more");
        for (n, message) in messages.iter().enumerate() {
            assert_eq!(message.id(), Some(&json!(1)), "under the editor's id");
            let result = message.result().unwrap();
            assert_eq!(
                result["tab"],
                json!(name),
                "no answer went to the wrong tab"
            );
            assert_eq!(result["n"], json!(n), "and they arrived in order");
            assert_eq!(result["uri"], json!(uri), "in the editor's naming");
        }
    }

    proxy.close_tab("one").await;
    proxy.close_tab("two").await;
    pool.shutdown().await;
}

#[tokio::test]
async fn a_tab_that_goes_away_mid_request_leaves_nothing_behind() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (editor, _) = tab(&pool, &proxy, "slow").await;

    // The fake server answers `test/wait` only when told to.
    proxy
        .from_editor(
            "slow",
            RawMessage::request(3, "test/wait", json!({ "ms": 3000 })),
        )
        .await;
    for _ in 0..100 {
        if proxy.in_flight() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(proxy.in_flight(), 1, "the request is on its way");

    proxy.close_tab("slow").await;
    assert_eq!(proxy.in_flight(), 0, "closing the tab forgets it");
    assert_eq!(proxy.tabs(), 0);

    // The answer, when it comes, goes nowhere: no stray response, no panic.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        editor.messages().is_empty(),
        "a closed tab hears nothing: {:?}",
        editor.messages(),
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn diagnostics_go_only_to_the_tab_that_owns_the_document() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (one, one_uri) = tab(&pool, &proxy, "one").await;
    let (two, _) = tab(&pool, &proxy, "two").await;

    // The server publishes diagnostics for one playground's file.
    proxy
        .from_editor(
            "one",
            RawMessage::request(
                1,
                "test/publish",
                json!({ "uri": file_uri("one"), "message": "unused variable" }),
            ),
        )
        .await;

    let published = one
        .wait("diagnostics", |m| {
            m.method() == Some("textDocument/publishDiagnostics")
        })
        .await;
    let params = published.params().unwrap();
    assert_eq!(params["uri"], json!(one_uri), "in the editor's naming");
    assert_eq!(
        params["diagnostics"][0]["message"],
        json!("unused variable")
    );
    assert!(
        !two.messages()
            .iter()
            .any(|m| m.method() == Some("textDocument/publishDiagnostics")),
        "the other tab is not told about a file it doesn't show",
    );

    proxy.close_tab("one").await;
    proxy.close_tab("two").await;
    pool.shutdown().await;
}

#[tokio::test]
async fn the_core_answers_the_questions_a_server_asks_it() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (editor, _) = tab(&pool, &proxy, "one").await;

    // The fake server asks us the three things real servers ask, and reports
    // what we replied.
    let answers = request(&proxy, &editor, "test/ask", json!({})).await;
    assert_eq!(
        answers["workspace/configuration"],
        json!([null, null]),
        "one null per item asked about",
    );
    assert_eq!(answers["client/registerCapability"], json!(null));
    assert_eq!(answers["window/workDoneProgress/create"], json!(null));
    assert!(
        answers["server/unknownRequest"]["error"]["code"] == json!(-32601),
        "a question the core doesn't answer gets a proper error: {answers}",
    );
    assert!(
        !editor
            .messages()
            .iter()
            .any(|m| m.method() == Some("workspace/configuration")),
        "none of it reached the editor",
    );

    proxy.close_tab("one").await;
    pool.shutdown().await;
}

#[tokio::test]
async fn a_message_for_the_user_becomes_a_toast_and_a_log_line() {
    let (tx, mut events) = mpsc::unbounded_channel();
    let pool = LspPool::default();
    let proxy = Proxy::new(Some(tx));
    let (editor, _) = tab(&pool, &proxy, "one").await;

    proxy
        .from_editor(
            "one",
            RawMessage::request(
                1,
                "test/tell",
                json!({ "type": 1, "message": "workspace failed to load" }),
            ),
        )
        .await;

    let event = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .expect("a toast")
        .expect("a toast");
    match event {
        CoreEvent::Toast { level, message } => {
            assert_eq!(level, Level::Error, "showMessage type 1 is an error");
            assert!(message.contains("workspace failed to load"), "{message}");
            assert!(
                message.starts_with("fake:"),
                "it says which server: {message}"
            );
        }
        other => panic!("expected a toast, got {other:?}"),
    }
    assert!(
        !editor
            .messages()
            .iter()
            .any(|m| m.method() == Some("window/showMessage")),
        "the editor doesn't have to handle it",
    );

    proxy.close_tab("one").await;
    pool.shutdown().await;
}

#[tokio::test]
async fn a_request_to_a_server_that_is_not_there_is_an_error_not_a_silence() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let config = ServerConfig::new("nosuch", SpawnSpec::new("tinkerpad-no-such-server"));
    let server = pool.server_for(&config);
    assert!(!server.state().is_ready());

    // Attaching fails, so build the tab the way the app would if it went ahead
    // anyway: a request must still come back, or the editor waits forever.
    let editor = Editor::default();
    let attachment = pool.attach(&config, &file_uri("gone"), "").await;
    assert!(attachment.is_err(), "there is no server to attach to");

    // With no tab registered, a message is dropped rather than panicking.
    proxy
        .from_editor("gone", RawMessage::request(1, "test/echo", json!({})))
        .await;
    assert!(editor.messages().is_empty());
    assert_eq!(proxy.tabs(), 0);
}

#[tokio::test]
async fn traffic_is_counted_and_timed_for_the_status_strip() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let (editor, _) = tab(&pool, &proxy, "one").await;

    for n in 0..10 {
        proxy
            .from_editor("one", RawMessage::request(n, "test/echo", json!(n)))
            .await;
    }
    editor.response_to(json!(9)).await;

    let stats = proxy.stats();
    assert_eq!(stats.requests, 10);
    assert_eq!(stats.to_server, 10);
    assert!(stats.to_editor >= 10, "{stats:?}");
    assert!(stats.p50_us > 0, "requests are timed: {stats:?}");
    assert!(stats.p95_us >= stats.p50_us, "{stats:?}");
    println!(
        "10 round trips through the proxy: p50 {} µs, p95 {} µs",
        stats.p50_us, stats.p95_us
    );

    proxy.close_tab("one").await;
    pool.shutdown().await;
}

/// Send a request through the proxy and return its result.
async fn request(proxy: &Arc<Proxy>, editor: &Editor, method: &str, params: Value) -> Value {
    proxy
        .from_editor("one", RawMessage::request(99, method, params))
        .await;
    let response = editor.response_to(json!(99)).await;
    response.result().cloned().unwrap_or(Value::Null)
}

/// The URI helper is the same one the app will use for a playground's file.
#[test]
fn a_playground_uri_is_built_from_its_id() {
    assert_eq!(
        UriMap::playground_uri("01K5"),
        "tinkerpad://playground/01K5"
    );
    assert_eq!(
        tinkerpad_lsp::path_to_uri(Path::new("/library/one/main.rs")),
        "file:///library/one/main.rs"
    );
}
