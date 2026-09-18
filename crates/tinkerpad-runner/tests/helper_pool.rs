//! The helper pool against real processes (work item 03-02): one helper per
//! plugin, built before its first use, restarted when it dies, stopped when
//! nothing has wanted it for a while.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use tinkerpad_core::ServerState;
use tinkerpad_process::SpawnSpec;
use tinkerpad_process::supervise::Policy;
use tinkerpad_runner::helpers::{HelperConfig, HelperPool, Unavailable};

/// The fake helper from `tinkerpad-plugin`.
///
/// Cargo tells a test where its *own* package's binaries are and no others, so
/// this looks where they all land: beside the test executable. `cargo test
/// --workspace` (and `cargo build --workspace`) builds it.
fn fake_helper() -> PathBuf {
    let mut dir = std::env::current_exe().expect("this test's own path");
    dir.pop();
    if dir.ends_with("deps") {
        dir.pop();
    }
    let exe = dir.join(format!(
        "tinkerpad-fakehelper{}",
        std::env::consts::EXE_SUFFIX
    ));
    assert!(
        exe.exists(),
        "the fake helper isn't built: run `cargo build --workspace` ({})",
        exe.display(),
    );
    exe
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("tinkerpad-pool-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A helper that starts quickly and never idles out on its own.
fn config(plugin: &str, dir: &PathBuf) -> HelperConfig {
    HelperConfig::new(
        plugin,
        SpawnSpec::new(fake_helper()),
        dir.join("plugin"),
        dir,
    )
    .with_ready_timeout(Duration::from_secs(20))
    .with_policy(Policy {
        backoff: Duration::from_millis(50),
        idle_shutdown: None,
        idle_unused: Duration::from_secs(3600),
        watch_tick: Duration::from_millis(50),
        ..Policy::default()
    })
}

/// Wait for something to become true, or give up.
async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test]
async fn one_helper_serves_everything_that_asks_for_it() {
    let dir = scratch("shared");
    let pool = HelperPool::default();
    let config = config("fake", &dir);

    let first = pool.get(&config).await.unwrap();
    let second = pool.get(&config).await.unwrap();
    assert_eq!(first.pid(), second.pid(), "the same process serves both");
    assert_eq!(pool.len(), 1);

    let handle = pool.get_handle("fake").expect("the pool knows it");
    assert!(matches!(handle.state(), ServerState::Ready { .. }));
    assert_eq!(handle.plugin(), "fake");

    pool.shutdown().await;
    assert!(pool.is_empty(), "app exit empties the pool");
    assert!(!first.is_alive());
}

#[tokio::test]
async fn two_callers_racing_for_a_cold_helper_start_one_between_them() {
    let dir = scratch("race");
    let pool = HelperPool::default();
    let config = config("fake", &dir);

    let (first, second, third) =
        tokio::join!(pool.get(&config), pool.get(&config), pool.get(&config));
    let pids = [
        first.unwrap().pid(),
        second.unwrap().pid(),
        third.unwrap().pid(),
    ];
    assert_eq!(pids[0], pids[1]);
    assert_eq!(pids[1], pids[2], "one start, not three: {pids:?}");
    assert_eq!(pool.len(), 1);
    pool.shutdown().await;
}

#[tokio::test]
async fn a_helper_that_dies_on_its_own_comes_back_warm() {
    let dir = scratch("crash");
    let pool = HelperPool::default();
    let config = config("fake", &dir);

    let first = pool.get(&config).await.unwrap();
    let was = first.pid();
    let handle = pool.get_handle("fake").unwrap();

    // Not a stop we asked for: the process dies under the pool.
    first.kill();
    until(
        "the helper to be running again",
        || matches!(handle.state(), ServerState::Ready { pid, .. } if pid != was),
    )
    .await;

    let second = pool.get(&config).await.unwrap();
    assert_ne!(second.pid(), was, "a new process");
    assert!(second.is_alive());
    assert_eq!(pool.len(), 1, "still one helper, not two");
    pool.shutdown().await;
}

#[tokio::test]
async fn a_helper_that_keeps_dying_is_given_up_on_until_someone_asks_again() {
    let dir = scratch("give-up");
    let pool = HelperPool::default();
    let mut config = config("fake", &dir);
    config.policy.max_restarts = 2;
    // Dies the moment it is spoken to, which for this helper is at once.
    config
        .spawn
        .env
        .push(("FAKEHELPER_SLOW_READY_MS".into(), "50".into()));
    config.ready_timeout = Duration::from_millis(1);

    let error = pool.get(&config).await.unwrap_err();
    assert!(matches!(error, Unavailable::Start { .. }), "{error:?}");
    let handle = pool.get_handle("fake").unwrap();
    assert!(
        matches!(
            handle.state(),
            ServerState::Stopped {
                reason: Some(_),
                ..
            }
        ),
        "{:?}",
        handle.state(),
    );

    // The run path can say why, which is what it falls back to `[run]` with.
    assert!(
        error.to_string().contains("did not answer hello"),
        "{error}"
    );
    pool.shutdown().await;
}

#[tokio::test]
async fn the_build_runs_before_the_first_use_and_then_only_when_the_source_changes() {
    let dir = scratch("build");
    let source = dir.join("helper-src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("helper.cs"), "class Helper {}").unwrap();
    let marker = dir.join("builds.txt");

    let mut build = SpawnSpec::new(fake_helper());
    build.env.push((
        "FAKEHELPER_BUILD_INTO".into(),
        marker.clone().into_os_string(),
    ));
    let config = config("fake", &dir).with_build(build, &source);
    let pool = HelperPool::default();
    let builds = || {
        std::fs::read_to_string(&marker)
            .map(|t| t.lines().count())
            .unwrap_or(0)
    };

    // The build's output is there for a run that is waiting on it.
    let handle = pool.handle(&config);
    let mut output = handle.build_output();

    pool.get(&config).await.unwrap();
    assert_eq!(builds(), 1, "built before the first use");
    let mut lines = Vec::new();
    while let Ok(line) = output.try_recv() {
        lines.push(line);
    }
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Building the fake helper")),
        "the header a waiting run shows: {lines:?}",
    );
    assert!(
        lines.iter().any(|l| l == "fakehelper: building"),
        "and the build's own output: {lines:?}",
    );
    assert!(
        dir.join("fake.build").exists(),
        "the stamp is kept beside the app's data",
    );

    // Starting again changes nothing: the source is what it was.
    handle.stop().await;
    pool.get(&config).await.unwrap();
    assert_eq!(builds(), 1, "no rebuild for an unchanged helper");

    // Touch the source, and it is built again.
    handle.stop().await;
    std::fs::write(source.join("helper.cs"), "class Helper { int x; }").unwrap();
    pool.get(&config).await.unwrap();
    assert_eq!(builds(), 2, "rebuilt when the source changed");

    pool.shutdown().await;
}

#[tokio::test]
async fn a_build_that_fails_leaves_the_helper_unavailable_rather_than_half_started() {
    let dir = scratch("build-fails");
    let source = dir.join("helper-src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("helper.cs"), "not going to compile").unwrap();

    let mut build = SpawnSpec::new(fake_helper());
    build.env.push((
        "FAKEHELPER_BUILD_INTO".into(),
        dir.join("builds.txt").into_os_string(),
    ));
    build
        .env
        .push(("FAKEHELPER_BUILD_FAILS".into(), "1".into()));
    let config = config("fake", &dir).with_build(build, &source);
    let pool = HelperPool::default();

    let error = pool.get(&config).await.unwrap_err();
    let Unavailable::Build { reason, .. } = &error else {
        panic!("{error:?}")
    };
    assert!(reason.contains("exited 3"), "{reason}");
    let handle = pool.get_handle("fake").unwrap();
    assert!(
        matches!(
            handle.state(),
            ServerState::Stopped {
                reason: Some(_),
                ..
            }
        ),
        "{:?}",
        handle.state(),
    );
    assert!(
        !dir.join("fake.build").exists(),
        "a failed build is not recorded as a build",
    );

    // And it is tried again next time, rather than being written off.
    let again = pool.get(&config).await.unwrap_err();
    assert!(matches!(again, Unavailable::Build { .. }), "{again:?}");
    pool.shutdown().await;
}

#[tokio::test]
async fn a_helper_nothing_has_used_stops_on_its_own() {
    let dir = scratch("idle");
    let pool = HelperPool::default();
    let mut config = config("fake", &dir);
    config.policy.idle_shutdown = Some(Duration::from_millis(100));
    config.policy.idle_unused = Duration::from_millis(100);

    let client = pool.get(&config).await.unwrap();
    let handle = pool.get_handle("fake").unwrap();
    until("the idle helper to stop", || {
        matches!(handle.state(), ServerState::Stopped { .. })
    })
    .await;
    assert!(!client.is_alive());

    // Asking for it again starts it: an idle stop is not a give-up.
    let again = pool.get(&config).await.unwrap();
    assert!(again.is_alive());
    assert_ne!(again.pid(), client.pid());
    pool.shutdown().await;
}

#[tokio::test]
async fn a_helper_in_use_is_not_stopped_for_being_idle() {
    let dir = scratch("in-use");
    let pool = HelperPool::default();
    let mut config = config("fake", &dir);
    config.policy.idle_shutdown = Some(Duration::from_millis(100));
    config.policy.idle_unused = Duration::from_millis(100);

    let handle = pool.handle(&config);
    let held = handle.using();
    let client = pool.get(&config).await.unwrap();
    assert_eq!(handle.uses(), 1);

    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(client.is_alive(), "something is using it");
    assert!(matches!(handle.state(), ServerState::Ready { .. }));

    drop(held);
    assert_eq!(handle.uses(), 0);
    until("the helper to stop once the run is over", || {
        matches!(handle.state(), ServerState::Stopped { .. })
    })
    .await;
    pool.shutdown().await;
}

#[tokio::test]
async fn every_helper_stops_together_at_app_exit() {
    let dir = scratch("exit");
    let pool = HelperPool::default();
    let first = pool.get(&config("one", &dir.join("one"))).await.unwrap();
    let second = pool.get(&config("two", &dir.join("two"))).await.unwrap();
    assert_eq!(pool.len(), 2);
    assert_eq!(pool.helpers().len(), 2);

    let started = Instant::now();
    pool.shutdown().await;
    assert!(!first.is_alive());
    assert!(!second.is_alive());
    assert!(pool.is_empty());
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "within the budget: {:?}",
        started.elapsed(),
    );
}

#[tokio::test]
async fn restarting_by_hand_replaces_the_process() {
    let dir = scratch("restart");
    let pool = HelperPool::default();
    let config = config("fake", &dir);
    let was = pool.get(&config).await.unwrap().pid();
    let handle = pool.get_handle("fake").unwrap();

    assert!(pool.restart("fake"));
    until(
        "the helper to be running again",
        || matches!(handle.state(), ServerState::Ready { pid, .. } if pid != was),
    )
    .await;
    assert!(!pool.restart("nobody"), "a plugin with no helper");
    pool.shutdown().await;
}
