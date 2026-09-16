//! The pool against real processes: one server shared by many playgrounds,
//! kept alive as tabs come and go, and stopped only on app exit
//! (work item 02-02).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tinkerpad_core::{CoreEvent, ServerState};
use tinkerpad_lsp::{LspPool, ServerConfig, ServerHandle};
use tinkerpad_process::SpawnSpec;
use tokio::sync::mpsc;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakelsp");

fn config(language: &str) -> ServerConfig {
    ServerConfig::new(language, SpawnSpec::new(FAKE))
}

/// A URI for a playground's main file, the way the app will build one.
fn uri(name: &str) -> String {
    tinkerpad_lsp::path_to_uri(&PathBuf::from(format!("/tmp/tinkerpad/{name}/main.rs")))
}

/// Wait for a state that satisfies `ok`, or fail.
async fn wait_state(server: &ServerHandle, ok: impl Fn(&ServerState) -> bool) -> ServerState {
    let mut states = server.watch_state();
    let wait = states.wait_for(|state| ok(state));
    match tokio::time::timeout(Duration::from_secs(30), wait).await {
        Ok(Ok(state)) => state.clone(),
        Ok(Err(_)) => panic!("the server handle went away"),
        Err(_) => panic!("state stayed {:?}", server.state()),
    }
}

/// What the fake server says it has open, and which process answered.
async fn docs(server: &ServerHandle) -> (u32, Vec<(String, i64)>) {
    let result: Value = server.request("test/docs", Value::Null).await.unwrap();
    let pid = result["pid"].as_u64().unwrap() as u32;
    let docs = result["docs"]
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
    (pid, docs)
}

#[tokio::test]
async fn two_playgrounds_share_one_server_that_outlives_their_tabs() {
    let pool = LspPool::default();
    let config = config("rust");

    let started = Instant::now();
    let first = pool
        .attach(&config, &uri("one"), "fn main() {}")
        .await
        .unwrap();
    let cold = started.elapsed();
    let started = Instant::now();
    let second = pool
        .attach(&config, &uri("two"), "fn main() {}")
        .await
        .unwrap();
    let warm = started.elapsed();
    println!("first attach (spawn + handshake) {cold:?}, second attach {warm:?}");

    // One process, one entry in the pool, both documents on it.
    let server = first.server().clone();
    assert!(std::sync::Arc::ptr_eq(first.server(), second.server()));
    assert_eq!(pool.len(), 1);
    let (pid, open) = docs(&server).await;
    assert_eq!(
        pid,
        match server.state() {
            ServerState::Ready { pid, .. } => pid,
            other => panic!("expected Ready, got {other:?}"),
        }
    );
    assert_eq!(
        open,
        vec![(uri("one"), 1), (uri("two"), 1)],
        "the server was told about both, each at version 1",
    );

    // Closing one tab closes its document and nothing else.
    drop(first);
    let open = wait_for_docs(&server, 1).await;
    assert_eq!(open, vec![(uri("two"), 1)]);
    assert!(server.state().is_ready(), "the server stays up");

    // Closing the last tab leaves the server running: this is the whole point.
    drop(second);
    let open = wait_for_docs(&server, 0).await;
    assert!(open.is_empty());
    assert!(server.state().is_ready());
    assert_eq!(pool.len(), 1);

    // A third playground reuses it, with no second process.
    let third = pool
        .attach(&config, &uri("three"), "fn main() {}")
        .await
        .unwrap();
    assert!(std::sync::Arc::ptr_eq(third.server(), &server));
    let (again, open) = docs(&server).await;
    assert_eq!(again, pid, "still the same process");
    assert_eq!(open, vec![(uri("three"), 1)]);

    pool.shutdown().await;
}

/// `didClose` is sent from a drop, so give it a moment to arrive.
async fn wait_for_docs(server: &ServerHandle, count: usize) -> Vec<(String, i64)> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let (_, open) = docs(server).await;
        if open.len() == count || Instant::now() > deadline {
            return open;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_different_root_is_a_different_server() {
    let pool = LspPool::default();
    let one = config("rust").with_root("/tmp/tinkerpad/a");
    let two = config("rust").with_root("/tmp/tinkerpad/b");

    let first = pool.attach(&one, &uri("a"), "").await.unwrap();
    let second = pool.attach(&two, &uri("b"), "").await.unwrap();
    assert_eq!(pool.len(), 2, "one server per (language, root)");
    assert_ne!(
        docs(first.server()).await.0,
        docs(second.server()).await.0,
        "two processes",
    );

    // And the same root, twice, is one server.
    let third = pool.attach(&one, &uri("c"), "").await.unwrap();
    assert!(std::sync::Arc::ptr_eq(third.server(), first.server()));
    assert_eq!(pool.len(), 2);

    pool.shutdown().await;
}

#[tokio::test]
async fn the_core_owns_the_document_version() {
    let pool = LspPool::default();
    let config = config("rust");
    let attachment = pool.attach(&config, &uri("versions"), "one").await.unwrap();

    for (n, text) in ["two", "three", "four"].iter().enumerate() {
        attachment.changed(text).await.unwrap();
        let (_, open) = docs(attachment.server()).await;
        assert_eq!(
            open,
            vec![(uri("versions"), n as i64 + 2)],
            "versions increase by one, whatever the editor thinks",
        );
    }
    assert_eq!(
        attachment.server().open_docs()[0].text,
        "four",
        "the text is kept for replay after a restart",
    );

    // Opening the same document again is a change, not a second open: the
    // fake server complains on stderr if it ever sees two opens for one URI.
    let again = pool
        .attach(&config, &uri("versions"), "five")
        .await
        .unwrap();
    let (_, open) = docs(again.server()).await;
    assert_eq!(open, vec![(uri("versions"), 5)]);

    pool.shutdown().await;
}

#[tokio::test]
async fn requests_made_while_starting_wait_for_the_handshake() {
    let pool = LspPool::default();
    let mut spawn = SpawnSpec::new(FAKE);
    spawn
        .env
        .push(("TINKERPAD_FAKELSP_INIT_DELAY_MS".into(), "300".into()));
    let config = ServerConfig::new("slow", spawn);

    let server = pool.server_for(&config);
    assert!(
        matches!(server.state(), ServerState::Starting),
        "it starts in Starting, not Ready",
    );

    // The request is made now and answered later, without the caller knowing
    // the handshake was in the way.
    let started = Instant::now();
    let echoed = server.request("test/echo", json!("queued")).await.unwrap();
    assert_eq!(echoed, json!("queued"));
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "it really did wait: {:?}",
        started.elapsed(),
    );
    assert!(server.state().is_ready());

    pool.shutdown().await;
}

#[tokio::test]
async fn a_server_that_never_finishes_starting_fails_the_request() {
    let pool = LspPool::default();
    let mut spawn = SpawnSpec::new(FAKE);
    spawn
        .env
        .push(("TINKERPAD_FAKELSP_INIT_DELAY_MS".into(), "60000".into()));
    let config = ServerConfig::new("stuck", spawn).with_start_timeout(Duration::from_millis(300));

    let server = pool.server_for(&config);
    let error = server
        .request("test/echo", json!("never"))
        .await
        .expect_err("a request can't wait forever");
    assert!(
        error.to_string().contains("did not finish starting"),
        "{error}",
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn a_missing_server_binary_is_reported_and_the_editor_carries_on() {
    let (tx, mut events) = mpsc::unbounded_channel();
    let pool = LspPool::new(Some(tx));
    let config = ServerConfig::new(
        "nosuch",
        SpawnSpec::new("tinkerpad-no-such-language-server"),
    )
    .with_install_hint("Install it from example.com");

    let error = pool
        .attach(&config, &uri("nosuch"), "")
        .await
        .expect_err("there is no server to attach to");
    assert!(error.to_string().contains("not running"), "{error}");

    let server = pool.get("nosuch", None).expect("the pool remembers it");
    match server.state() {
        ServerState::Stopped { reason, hint } => {
            assert!(
                reason.unwrap_or_default().contains("not found"),
                "the reason says what went wrong",
            );
            assert_eq!(hint.as_deref(), Some("Install it from example.com"));
        }
        other => panic!("expected Stopped, got {other:?}"),
    }

    // The UI hears about it.
    let event = events.recv().await.expect("a state event");
    match event {
        CoreEvent::Server {
            language, state, ..
        } => {
            assert_eq!(language.as_str(), "nosuch");
            assert!(matches!(state, ServerState::Stopped { .. }));
        }
        other => panic!("expected a server event, got {other:?}"),
    }
}

#[tokio::test]
async fn state_changes_reach_the_ui_in_order() {
    let (tx, mut events) = mpsc::unbounded_channel();
    let pool = LspPool::new(Some(tx));
    let config = config("rust");
    let attachment = pool.attach(&config, &uri("events"), "").await.unwrap();
    assert!(attachment.server().state().is_ready());

    let mut kinds = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
        if let CoreEvent::Server { state, .. } = event {
            let last = matches!(state, ServerState::Stopped { .. });
            kinds.push(state);
            if last {
                break;
            }
        }
        if kinds.len() == 2 {
            // Starting and Ready have arrived; now stop it and expect the rest.
            pool.shutdown().await;
        }
    }

    assert!(
        matches!(kinds.first(), Some(ServerState::Starting)),
        "{kinds:?}",
    );
    assert!(kinds.iter().any(ServerState::is_ready), "{kinds:?}");
    assert!(
        matches!(kinds.last(), Some(ServerState::Stopped { .. })),
        "{kinds:?}",
    );
}

#[tokio::test]
async fn shutdown_stops_every_server_and_is_quick() {
    let pool = LspPool::default();
    let mut servers = Vec::new();
    for language in ["rust", "typescript", "csharp"] {
        let attachment = pool
            .attach(&config(language), &uri(language), "")
            .await
            .unwrap();
        servers.push(attachment.server().clone());
    }
    assert_eq!(pool.len(), 3);

    let started = Instant::now();
    pool.shutdown().await;
    let elapsed = started.elapsed();
    println!("stopped {} servers in {elapsed:?}", servers.len());

    assert!(
        elapsed < tinkerpad_lsp::SHUTDOWN_BUDGET,
        "app exit is never held up: {elapsed:?}",
    );
    for server in &servers {
        let state = wait_state(server, |state| matches!(state, ServerState::Stopped { .. })).await;
        assert!(matches!(state, ServerState::Stopped { .. }));
    }
    assert!(pool.is_empty(), "the pool is empty afterwards");
}

#[tokio::test]
async fn a_server_that_ignores_shutdown_is_killed_inside_the_budget() {
    // The fake server exits on `exit`; to prove the kill path, stop a server
    // whose stdin has been closed by killing the transport's writer first.
    let pool = LspPool::default();
    let attachment = pool
        .attach(&config("rust"), &uri("wedged"), "")
        .await
        .unwrap();
    let server = attachment.server().clone();

    // Simulate a wedged server: it is still alive, but its stdin is gone, so
    // neither `shutdown` nor `exit` can reach it.
    drop(attachment);
    server.kill();
    wait_state(&server, |state| !state.is_ready()).await;

    let started = Instant::now();
    pool.shutdown().await;
    assert!(
        started.elapsed() < tinkerpad_lsp::SHUTDOWN_BUDGET,
        "shutdown still returns promptly: {:?}",
        started.elapsed(),
    );
}
