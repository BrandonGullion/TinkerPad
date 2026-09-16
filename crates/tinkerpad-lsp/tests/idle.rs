//! Servers that nobody is using stop on their own, and come back when someone
//! wants one again (work item 02-06).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tinkerpad_core::ServerState;
use tinkerpad_lsp::{LspPool, ServerConfig, ServerHandle};
use tinkerpad_process::SpawnSpec;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakelsp");

/// Idle timers are minutes in the app; these are milliseconds, so the tests
/// take about as long as they look like they should.
fn config(language: &str, idle: Duration) -> ServerConfig {
    ServerConfig::new(language, SpawnSpec::new(FAKE))
        .with_idle_shutdown(Some(idle))
        .with_watch_tick(Duration::from_millis(20))
        .with_backoff(Duration::from_millis(20))
}

fn uri(name: &str) -> String {
    tinkerpad_lsp::path_to_uri(&PathBuf::from(format!("/library/{name}/main.rs")))
}

async fn wait_state(server: &ServerHandle, what: &str, ok: impl Fn(&ServerState) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < deadline {
        if ok(&server.state()) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("never became {what}; it is {:?}", server.state());
}

#[tokio::test]
async fn a_server_with_no_documents_left_stops_on_its_own() {
    let pool = LspPool::default();
    let config = config("fake", Duration::from_millis(200));
    let attachment = pool.attach(&config, &uri("one"), "text").await.unwrap();
    let server = attachment.server().clone();
    let first = server.pid();

    // While a document is open, the clock doesn't run at all.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        server.state().is_ready(),
        "a server with an open document is not idle",
    );
    assert_eq!(server.idle_for(), None);

    // Closing the last tab starts it.
    drop(attachment);
    wait_state(&server, "stopped", |state| {
        matches!(state, ServerState::Stopped { .. })
    })
    .await;
    assert_eq!(server.pid(), 0, "the process is gone");

    // And the next tab brings it back, transparently.
    let again = pool.attach(&config, &uri("two"), "text").await.unwrap();
    assert!(
        std::sync::Arc::ptr_eq(again.server(), &server),
        "the same handle, a new process",
    );
    assert_ne!(server.pid(), first);
    assert_eq!(
        server.request("test/echo", json!("back")).await.unwrap(),
        json!("back"),
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn opening_a_document_again_stops_the_clock() {
    let pool = LspPool::default();
    let config = config("fake", Duration::from_millis(400));
    let first = pool.attach(&config, &uri("a"), "text").await.unwrap();
    let server = first.server().clone();

    // Close one document and open another before the timer is up: the clock
    // resets, so the server is still there well after the limit.
    drop(first);
    tokio::time::sleep(Duration::from_millis(150)).await;
    let second = pool.attach(&config, &uri("b"), "text").await.unwrap();
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        server.state().is_ready(),
        "the clock restarted when the document opened: {:?}",
        server.state(),
    );

    drop(second);
    wait_state(&server, "stopped", |state| {
        matches!(state, ServerState::Stopped { .. })
    })
    .await;

    pool.shutdown().await;
}

#[tokio::test]
async fn a_server_that_never_gets_a_document_stops_too() {
    let pool = LspPool::default();
    // "Never" idle, as a plugin might say — but nothing was ever opened on it,
    // which is the pre-warmed-and-unused case, so the shorter limit applies.
    let config = ServerConfig::new("prewarmed", SpawnSpec::new(FAKE))
        .with_idle_shutdown(None)
        .with_idle_unused(Duration::from_millis(200))
        .with_watch_tick(Duration::from_millis(20));

    let server = pool.server_for(&config);
    server.ready().await.unwrap();
    wait_state(&server, "stopped", |state| {
        matches!(state, ServerState::Stopped { .. })
    })
    .await;

    pool.shutdown().await;
}

#[tokio::test]
async fn a_server_told_never_to_idle_stays_once_it_is_used() {
    let pool = LspPool::default();
    let config = ServerConfig::new("kept", SpawnSpec::new(FAKE))
        .with_idle_shutdown(None)
        .with_idle_unused(Duration::from_millis(200))
        .with_watch_tick(Duration::from_millis(20));

    let attachment = pool.attach(&config, &uri("kept"), "text").await.unwrap();
    let server = attachment.server().clone();
    drop(attachment);

    // Well past the unused limit, and past any plausible idle limit: a server
    // that has done real work and is told "never" is kept.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        server.state().is_ready(),
        "`never` means never once the server has been used: {:?}",
        server.state(),
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn memory_is_sampled_for_the_status_strip() {
    let pool = LspPool::default();
    let config = ServerConfig::new("memory", SpawnSpec::new(FAKE))
        .with_watch_tick(Duration::from_millis(20));
    let attachment = pool.attach(&config, &uri("memory"), "text").await.unwrap();
    let server = attachment.server().clone();

    let deadline = Instant::now() + Duration::from_secs(30);
    let rss = loop {
        if let ServerState::Ready {
            rss_bytes: Some(rss),
            ..
        } = server.state()
        {
            break rss;
        }
        assert!(Instant::now() < deadline, "no memory sample arrived");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    println!("the fake server is using {} KiB", rss / 1024);
    assert!(rss > 0, "a running process uses some memory");
    assert!(
        rss < 4 * 1024 * 1024 * 1024,
        "and the number is bytes, not something else: {rss}",
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn an_idle_stop_is_clean_and_quiet() {
    let (tx, mut events) = tokio::sync::mpsc::unbounded_channel();
    let pool = LspPool::new(Some(tx));
    let config = config("quiet", Duration::from_millis(150));
    let attachment = pool.attach(&config, &uri("quiet"), "text").await.unwrap();
    let server = attachment.server().clone();
    drop(attachment);

    wait_state(&server, "stopped", |state| {
        matches!(state, ServerState::Stopped { .. })
    })
    .await;
    match server.state() {
        // No reason: this was not a failure, and the status strip should say
        // "stopped", not explain itself.
        ServerState::Stopped { reason, hint } => {
            assert_eq!(reason, None, "an idle stop has nothing to explain");
            assert_eq!(hint, None);
        }
        other => panic!("expected Stopped, got {other:?}"),
    }

    let mut toasts = 0;
    while let Ok(event) = events.try_recv() {
        if matches!(event, tinkerpad_core::CoreEvent::Toast { .. }) {
            toasts += 1;
        }
    }
    assert_eq!(toasts, 0, "nobody asked, so nobody is told");

    pool.shutdown().await;
}

#[tokio::test]
async fn a_request_after_an_idle_stop_starts_the_server_again() {
    let pool = LspPool::default();
    let config = config("revive", Duration::from_millis(150));
    let attachment = pool.attach(&config, &uri("revive"), "text").await.unwrap();
    let server = attachment.server().clone();
    drop(attachment);
    wait_state(&server, "stopped", |state| {
        matches!(state, ServerState::Stopped { .. })
    })
    .await;

    // Asking the pool for the server is what a new tab does.
    let again = pool.server_for(&config);
    assert!(std::sync::Arc::ptr_eq(&again, &server));
    wait_state(&server, "ready again", ServerState::is_ready).await;
    assert_eq!(
        server.request("test/echo", json!("hello")).await.unwrap(),
        json!("hello"),
    );
    assert!(server.pid() > 0);

    pool.shutdown().await;
}

/// The fake server reports its own pid, which is how the tests above know a
/// process was replaced rather than reused.
#[tokio::test]
async fn the_replacement_is_a_different_process() {
    let pool = LspPool::default();
    let config = config("replaced", Duration::from_millis(150));
    let attachment = pool
        .attach(&config, &uri("replaced"), "text")
        .await
        .unwrap();
    let server = attachment.server().clone();
    let before: Value = server.request("test/docs", Value::Null).await.unwrap();
    drop(attachment);
    wait_state(&server, "stopped", |state| {
        matches!(state, ServerState::Stopped { .. })
    })
    .await;

    pool.server_for(&config);
    wait_state(&server, "ready again", ServerState::is_ready).await;
    let after: Value = server.request("test/docs", Value::Null).await.unwrap();
    assert_ne!(before["pid"], after["pid"], "a new process answered");
    assert_eq!(
        after["docs"].as_array().map(Vec::len),
        Some(0),
        "and it starts with no documents, because the tab had closed",
    );

    pool.shutdown().await;
}
