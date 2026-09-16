//! What a server is told about its workspace: the root it is given, and the
//! playgrounds it is told to serve (work item 02-07).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tinkerpad_lsp::{LspPool, ServerConfig, ServerHandle};
use tinkerpad_process::SpawnSpec;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakelsp");

fn uri(name: &str) -> String {
    tinkerpad_lsp::path_to_uri(&PathBuf::from(format!("/library/{name}/main.rs")))
}

/// What the fake server was told at `initialize`, and since.
async fn seen(server: &ServerHandle) -> Value {
    server.request("test/seen", Value::Null).await.unwrap()
}

/// The projects the server has been told about, wherever it heard them: a
/// server learns its settings unprefixed in `initializationOptions` at
/// startup, and under their namespace in `didChangeConfiguration` later.
fn projects(seen: &Value) -> Value {
    let configuration = &seen["configuration"];
    match &configuration["rust-analyzer"]["linkedProjects"] {
        Value::Null => configuration["linkedProjects"].clone(),
        prefixed => prefixed.clone(),
    }
}

/// Wait until `ok` holds for what the server has seen.
async fn wait_seen(server: &ServerHandle, what: &str, ok: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let value = seen(server).await;
        if ok(&value) {
            return value;
        }
        assert!(Instant::now() < deadline, "never {what}: {value}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn a_server_with_no_workspace_is_given_no_root() {
    let pool = LspPool::default();
    // `workspace = "none"`: the app leaves the root unset.
    let config = ServerConfig::new("rootless", SpawnSpec::new(FAKE));
    let server = pool.server_for(&config);
    server.ready().await.unwrap();

    let seen = seen(&server).await;
    assert_eq!(seen["rootUri"], Value::Null, "{seen}");
    assert_eq!(
        seen["workspaceFolders"],
        Value::Null,
        "and no folders either: {seen}",
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn a_server_is_rooted_where_it_was_told() {
    let pool = LspPool::default();
    let root = std::env::temp_dir().join("tinkerpad-workspace-test");
    std::fs::create_dir_all(&root).unwrap();
    let config = ServerConfig::new("rooted", SpawnSpec::new(FAKE)).with_root(root.clone());
    let server = pool.server_for(&config);
    server.ready().await.unwrap();

    let expected = tinkerpad_lsp::path_to_uri(&root);
    let seen = seen(&server).await;
    assert_eq!(seen["rootUri"], json!(expected), "{seen}");
    assert_eq!(
        seen["workspaceFolders"][0]["uri"],
        json!(expected),
        "a workspace folder as well, which is what servers read now: {seen}",
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn two_playgrounds_on_a_shared_server_are_both_registered() {
    let pool = LspPool::default();
    let root = std::env::temp_dir().join("tinkerpad-shared-workspace");
    std::fs::create_dir_all(&root).unwrap();
    let config = ServerConfig::new("shared", SpawnSpec::new(FAKE))
        .with_root(root)
        .with_register_setting("rust-analyzer.linkedProjects");

    // The first playground: the server is told about it before anything else.
    let first =
        pool.attach_registered(&config, &uri("one"), Some(json!("/library/one/Cargo.toml")));
    let server = first.server().clone();
    let seen = wait_seen(&server, "told about one project", |seen| {
        projects(seen)
            .as_array()
            .is_some_and(|list| list.len() == 1)
    })
    .await;
    assert_eq!(
        projects(&seen),
        json!(["/library/one/Cargo.toml"]),
        "{seen}",
    );

    // A second playground is added without restarting anything.
    let pid = seen["pid"].clone();
    let second =
        pool.attach_registered(&config, &uri("two"), Some(json!("/library/two/Cargo.toml")));
    assert!(std::sync::Arc::ptr_eq(second.server(), &server));
    let seen = wait_seen(&server, "told about both projects", |seen| {
        projects(seen)
            .as_array()
            .is_some_and(|list| list.len() == 2)
    })
    .await;
    assert_eq!(
        projects(&seen),
        json!(["/library/one/Cargo.toml", "/library/two/Cargo.toml"]),
        "both, in a stable order: {seen}",
    );
    assert_eq!(seen["pid"], pid, "and the same process throughout");

    // Closing one tab takes its project out again.
    drop(first);
    let seen = wait_seen(&server, "told about one project again", |seen| {
        projects(seen)
            .as_array()
            .is_some_and(|list| list.len() == 1)
    })
    .await;
    assert_eq!(
        projects(&seen),
        json!(["/library/two/Cargo.toml"]),
        "{seen}",
    );

    drop(second);
    pool.shutdown().await;
}

#[tokio::test]
async fn a_server_that_asks_for_its_configuration_gets_the_same_answer() {
    let pool = LspPool::default();
    let config = ServerConfig::new("asks", SpawnSpec::new(FAKE))
        .with_register_setting("rust-analyzer.linkedProjects");
    let attachment = pool.attach_registered(
        &config,
        &uri("asks"),
        Some(json!("/library/asks/Cargo.toml")),
    );
    let server = attachment.server().clone();
    server.ready().await.unwrap();

    // Some servers wait to be told; others ask. Both get the same thing.
    assert_eq!(
        server.configuration_section("rust-analyzer.linkedProjects"),
        json!(["/library/asks/Cargo.toml"]),
    );
    assert_eq!(
        server.configuration_section("rust-analyzer"),
        json!({ "linkedProjects": ["/library/asks/Cargo.toml"] }),
    );
    assert_eq!(
        server.configuration_section("something.else"),
        Value::Null,
        "and nothing for a section nobody registered",
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn the_registered_projects_survive_a_restart() {
    let pool = LspPool::default();
    let config = ServerConfig::new("restarted", SpawnSpec::new(FAKE))
        .with_register_setting("rust-analyzer.linkedProjects")
        .with_backoff(Duration::from_millis(20));
    let attachment = pool.attach_registered(
        &config,
        &uri("restarted"),
        Some(json!("/library/restarted/Cargo.toml")),
    );
    let server = attachment.server().clone();
    wait_seen(&server, "told about the project", |seen| {
        !projects(seen).is_null()
    })
    .await;

    // The server dies and comes back: it has to be told again, or it would
    // serve the file without knowing which crate it belongs to.
    let before = seen(&server).await["pid"].clone();
    let _ = server.request("test/crash", Value::Null).await;
    let seen = wait_seen(&server, "told again after the restart", |seen| {
        seen["pid"] != before && !projects(seen).is_null()
    })
    .await;
    assert_eq!(
        projects(&seen),
        json!(["/library/restarted/Cargo.toml"]),
        "{seen}",
    );

    pool.shutdown().await;
}

#[tokio::test]
async fn a_plugin_that_registers_nothing_sends_nothing() {
    let pool = LspPool::default();
    let config = ServerConfig::new("plain", SpawnSpec::new(FAKE));
    let attachment = pool.attach_registered(&config, &uri("plain"), Some(json!("ignored")));
    let server = attachment.server().clone();
    server.ready().await.unwrap();

    assert_eq!(server.configuration(), json!({}));
    let seen = seen(&server).await;
    assert_eq!(
        seen["configuration"],
        Value::Null,
        "no configuration was ever pushed: {seen}",
    );

    pool.shutdown().await;
}
