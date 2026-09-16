//! A server that dies on its own: the pool notices, waits, starts it again,
//! and re-opens the documents it had (work item 02-05).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tinkerpad_core::{CoreEvent, Level, ServerState};
use tinkerpad_lsp::jsonrpc::RawMessage;
use tinkerpad_lsp::{EditorSink, LspPool, Proxy, ServerConfig, ServerHandle};
use tinkerpad_process::SpawnSpec;
use tokio::sync::mpsc;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakelsp");

/// The real backoff starts at a second and doubles, which is right for a
/// person watching and wrong for a test sitting through it five times. The
/// ladder itself is checked by a unit test in `server.rs`.
fn config(language: &str) -> ServerConfig {
    ServerConfig::new(language, SpawnSpec::new(FAKE))
        .with_start_timeout(Duration::from_secs(10))
        .with_backoff(Duration::from_millis(20))
}

fn main_file(name: &str) -> PathBuf {
    PathBuf::from(format!("/library/{name}/main.rs"))
}

fn file_uri(name: &str) -> String {
    tinkerpad_lsp::path_to_uri(&main_file(name))
}

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
}

/// Wait for a state matching `ok`, or fail with what it actually is.
async fn wait_state(server: &ServerHandle, what: &str, ok: impl Fn(&ServerState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if ok(&server.state()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("never became {what}; it is {:?}", server.state());
}

/// Ask the server to die, and wait until it has.
async fn crash(server: &ServerHandle) {
    let pid = server.pid();
    let _ = server.request("test/crash", Value::Null).await;
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if server.pid() != pid {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the server did not go away");
}

#[tokio::test]
async fn a_crashed_server_comes_back_and_works_again() {
    let pool = LspPool::default();
    let config = config("fake");
    let attachment = pool
        .attach(&config, &file_uri("one"), "before")
        .await
        .unwrap();
    let server = attachment.server().clone();
    let first = server.pid();
    assert!(server.state().is_ready());

    crash(&server).await;
    wait_state(&server, "crashed", |state| {
        matches!(state, ServerState::Crashed { restarts: 1, .. })
    })
    .await;

    // It comes back on its own, after the first backoff.
    let started = Instant::now();
    wait_state(&server, "ready again", ServerState::is_ready).await;
    let back = started.elapsed();
    assert_ne!(server.pid(), first, "a new process");
    assert_eq!(server.restarts(), 1);
    println!("back up {back:?} after the crash");

    // And it works: same handle, same attachment, no tab involvement.
    let echoed = server.request("test/echo", json!("after")).await.unwrap();
    assert_eq!(echoed, json!("after"));

    pool.shutdown().await;
}

#[tokio::test]
async fn the_documents_are_open_again_afterwards() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let config = config("fake");
    let attachment = pool.attach_editor(&config, &file_uri("doc"));
    let server = attachment.server().clone();
    let editor = Editor::default();
    let uri = proxy.open_tab(
        "doc",
        &main_file("doc"),
        attachment,
        Arc::new(editor.clone()),
    );

    // The editor opens the document and edits it, as in 02-04.
    proxy
        .from_editor(
            "doc",
            RawMessage::notification(
                "textDocument/didOpen",
                json!({
                    "textDocument": {
                        "uri": uri,
                        "languageId": "rust",
                        "version": 1,
                        "text": "fn main() {}",
                    }
                }),
            ),
        )
        .await;
    proxy
        .from_editor(
            "doc",
            RawMessage::notification(
                "textDocument/didChange",
                json!({
                    "textDocument": { "uri": uri, "version": 2 },
                    "contentChanges": [{ "text": "fn main() { edited(); }" }],
                }),
            ),
        )
        .await;
    wait_for_docs(&server, 1).await;

    crash(&server).await;
    wait_state(&server, "ready again", ServerState::is_ready).await;

    // The replay is a `didOpen` with the text as it stands, at the version the
    // editor last used, so the editor's next edit still counts up.
    let docs = wait_for_docs(&server, 1).await;
    assert_eq!(
        docs,
        vec![(file_uri("doc"), 2)],
        "the document is open again at the version the editor knows",
    );
    assert_eq!(
        server.open_docs()[0].text,
        "fn main() { edited(); }",
        "with the text as it was, not the text it was opened with",
    );

    proxy.close_tab("doc").await;
    pool.shutdown().await;
}

/// The fake server's documents, once it has `count` of them.
async fn wait_for_docs(server: &ServerHandle, count: usize) -> Vec<(String, i64)> {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(result) = server.request("test/docs", Value::Null).await {
            let docs: Vec<(String, i64)> = result["docs"]
                .as_array()
                .unwrap()
                .iter()
                .map(|doc| {
                    (
                        doc["uri"].as_str().unwrap().to_owned(),
                        doc["version"].as_i64().unwrap(),
                    )
                })
                .collect();
            if docs.len() == count {
                return docs;
            }
        }
        if Instant::now() > deadline {
            panic!("the server never had {count} documents");
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_request_in_flight_when_the_server_dies_is_answered() {
    let pool = LspPool::default();
    let proxy = Proxy::new(None);
    let config = config("fake");
    let attachment = pool.attach_editor(&config, &file_uri("flight"));
    let server = attachment.server().clone();
    let editor = Editor::default();
    proxy.open_tab(
        "flight",
        &main_file("flight"),
        attachment,
        Arc::new(editor.clone()),
    );
    // Make sure the handshake is done before anything else.
    server.ready().await.unwrap();

    // A request that will never be answered, because the next one kills the
    // server before it gets there.
    proxy
        .from_editor(
            "flight",
            RawMessage::request(11, "test/wait", json!({ "ms": 30_000 })),
        )
        .await;
    for _ in 0..200 {
        if proxy.in_flight() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(proxy.in_flight(), 1);
    server.kill();

    // The editor gets an error, not silence: -32000 says "transient, ask
    // again", which is what its client does on the next keystroke.
    let deadline = Instant::now() + Duration::from_secs(30);
    let response = loop {
        if let Some(message) = editor
            .messages()
            .into_iter()
            .find(|m| m.id() == Some(&json!(11)))
        {
            break message;
        }
        assert!(Instant::now() < deadline, "no answer to the lost request");
        tokio::time::sleep(Duration::from_millis(10)).await;
    };
    let error = response.error_object().expect("an error response");
    assert_eq!(error["code"], json!(-32000), "{response}");
    assert!(
        error["message"]
            .as_str()
            .unwrap_or_default()
            .contains("restarted"),
        "{response}",
    );
    assert_eq!(proxy.in_flight(), 0, "nothing is left waiting");

    pool.shutdown().await;
}

#[tokio::test]
async fn a_server_that_keeps_crashing_is_left_alone_until_asked_again() {
    let (tx, mut events) = mpsc::unbounded_channel();
    let pool = LspPool::new(Some(tx));
    let config = config("fake");
    let server = pool.server_for(&config);
    server.ready().await.unwrap();

    // Crash it until the pool gives up. The waits double (1, 2, 4, 8, 16 s),
    // so this only goes as far as the give-up state without sitting through
    // the later ones: each crash happens while the server is up again.
    for expected in 1..=5 {
        crash(&server).await;
        wait_state(
            &server,
            "crashed",
            |state| matches!(state, ServerState::Crashed { restarts, .. } if *restarts == expected),
        )
        .await;
        wait_state(&server, "ready again", ServerState::is_ready).await;
    }
    crash(&server).await;

    wait_state(&server, "given up", |state| {
        matches!(
            state,
            ServerState::Stopped {
                reason: Some(_),
                ..
            }
        )
    })
    .await;
    match server.state() {
        ServerState::Stopped { reason, hint } => {
            assert!(
                reason.unwrap_or_default().contains("6 times in a row"),
                "the reason says how often it crashed",
            );
            assert!(
                hint.unwrap_or_default().contains("Restart"),
                "and how to get it back",
            );
        }
        other => panic!("expected Stopped, got {other:?}"),
    }
    assert!(
        server.request("test/echo", json!("x")).await.is_err(),
        "requests fail rather than wait for a server nobody is restarting",
    );

    // The user hears about the crashes.
    let mut toasts = Vec::new();
    while let Ok(event) = events.try_recv() {
        if let CoreEvent::Toast { level, message } = event {
            assert_eq!(level, Level::Warning);
            toasts.push(message);
        }
    }
    assert!(
        toasts.iter().any(|t| t.contains("crashed, restarting")),
        "{toasts:?}",
    );

    // Opening a tab is asking for it again, which gives it another go.
    let again = pool.server_for(&config);
    assert!(Arc::ptr_eq(&again, &server), "the same handle, restarted");
    wait_state(&server, "ready again", ServerState::is_ready).await;
    assert_eq!(server.restarts(), 0, "the count starts over");
    assert_eq!(
        server.request("test/echo", json!("alive")).await.unwrap(),
        json!("alive"),
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn restarting_by_hand_replaces_the_process() {
    let pool = LspPool::default();
    let config = config("fake");
    let attachment = pool
        .attach(&config, &file_uri("manual"), "text")
        .await
        .unwrap();
    let server = attachment.server().clone();
    let first = server.pid();

    assert!(pool.restart("fake", None), "the pool knows this server");
    // A restart is asynchronous: the old process has to go first, so wait for
    // a different one to be ready rather than for "ready", which it still is.
    let deadline = Instant::now() + Duration::from_secs(60);
    while (server.pid() == first || !server.state().is_ready()) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_ne!(server.pid(), first, "a new process");
    assert!(server.state().is_ready(), "{:?}", server.state());
    assert_eq!(
        server.request("test/echo", json!("hello")).await.unwrap(),
        json!("hello"),
    );
    assert_eq!(
        wait_for_docs(&server, 1).await,
        vec![(file_uri("manual"), 1)],
        "the document is open on the new process",
    );

    assert!(!pool.restart("nosuch", None), "and says when it doesn't");

    pool.shutdown().await;
}
