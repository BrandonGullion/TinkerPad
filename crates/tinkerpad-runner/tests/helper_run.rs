//! Running a playground on a helper (work item 03-03).
//!
//! The thing being checked throughout is sameness: a helper run must reach the
//! rest of the app as the same `RunEvent` stream a per-run command produces,
//! dumped values and all, or every layer above would need to know which kind
//! of run it was looking at.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tinkerpad_core::{KindId, RunEvent as UiEvent};
use tinkerpad_plugin::helper::RunRequest;
use tinkerpad_process::SpawnSpec;
use tinkerpad_process::supervise::Policy;
use tinkerpad_runner::dump::PREFIX;
use tinkerpad_runner::helper_run::{self, Waiting};
use tinkerpad_runner::helpers::{HelperConfig, HelperPool, Unavailable};
use tinkerpad_runner::{OutputDecoder, RunEvent, RunHandle};

/// The fake helper from `tinkerpad-plugin`; see the note in `helper_pool.rs`.
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
        .join(format!("tinkerpad-helper-run-{}", std::process::id()))
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config(dir: &PathBuf) -> HelperConfig {
    HelperConfig::new(
        "fake",
        SpawnSpec::new(fake_helper()),
        dir.join("plugin"),
        dir,
    )
    .with_ready_timeout(Duration::from_secs(20))
    .with_policy(Policy {
        backoff: Duration::from_millis(20),
        watch_tick: Duration::from_millis(50),
        ..Policy::default()
    })
}

/// A playground: a folder with a main file in it.
fn playground(dir: &Path, name: &str, body: &[u8], kind: &str) -> RunRequest {
    let folder = dir.join(name);
    std::fs::create_dir_all(&folder).unwrap();
    let main = folder.join("main.fake");
    std::fs::write(&main, body).unwrap();
    RunRequest {
        main,
        dir: folder,
        kind: KindId::new(kind),
    }
}

/// Collects what the run was waiting for, as the app's status bar would.
#[derive(Clone, Default)]
struct Waits(Arc<Mutex<Vec<Waiting>>>);

impl Waits {
    fn watcher(&self) -> impl FnMut(Waiting) + Send + 'static {
        let seen = self.0.clone();
        move |what| seen.lock().expect("waits").push(what)
    }

    fn all(&self) -> Vec<Waiting> {
        self.0.lock().expect("waits").clone()
    }

    fn busy(&self) -> bool {
        self.all().contains(&Waiting::Busy)
    }
}

/// Everything a run said, decoded the way the app decodes it.
async fn collect(handle: &mut RunHandle) -> (Vec<UiEvent>, Option<RunEvent>) {
    let mut decoder = OutputDecoder::new();
    let (mut ui, mut last) = (Vec::new(), None);
    while let Some(event) = handle.recv().await {
        match &event {
            RunEvent::Stdout(bytes) => decoder.stdout(bytes, &mut ui),
            RunEvent::Stderr(bytes) => decoder.stderr(bytes, &mut ui),
            _ => {}
        }
        match event {
            RunEvent::Stdout(_) | RunEvent::Stderr(_) => {}
            other => {
                decoder.finish(&mut ui);
                last = Some(other);
            }
        }
    }
    (ui, last)
}

fn text(events: &[UiEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            UiEvent::Stdout { text } | UiEvent::Stderr { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn a_run_on_a_helper_looks_exactly_like_a_run_on_a_process() {
    let dir = scratch("same");
    let pool = HelperPool::default();
    let config = config(&dir);
    let request = playground(&dir, "one", b"hello from the helper\n", "echo");
    let waits = Waits::default();

    let mut handle = helper_run::start(&pool, &config, request, waits.watcher())
        .await
        .unwrap();

    let first = handle.recv().await;
    assert!(
        matches!(first, Some(RunEvent::Started { warm: true, .. })),
        "the only thing that says a helper ran it: {first:?}",
    );
    let (ui, last) = collect(&mut handle).await;
    assert_eq!(text(&ui), "hello from the helper\n");
    assert!(
        matches!(last, Some(RunEvent::Exited { code: Some(0), .. })),
        "{last:?}",
    );
    assert!(
        waits.all().is_empty(),
        "nothing to wait for: {:?}",
        waits.all()
    );
    pool.shutdown().await;
}

#[tokio::test]
async fn a_value_dumped_on_a_helper_arrives_as_a_dumped_value() {
    let dir = scratch("dump");
    let pool = HelperPool::default();
    let config = config(&dir);

    // A dump sentinel in the middle of ordinary output, split across the
    // helper's chunks on its way here.
    let mut body = b"before\n".to_vec();
    body.extend_from_slice(PREFIX);
    body.extend_from_slice(br#"{"v":1,"kind":"value","value":42}"#);
    body.push(0x07);
    body.extend_from_slice(b"after\n");
    let request = playground(&dir, "dump", &body, "echo");

    let mut handle = helper_run::start(&pool, &config, request, Waits::default().watcher())
        .await
        .unwrap();
    let (ui, last) = collect(&mut handle).await;

    assert_eq!(text(&ui), "before\nafter\n", "the sentinel is not output");
    let rich = ui.iter().find_map(|event| match event {
        UiEvent::Rich { value } => Some(value),
        _ => None,
    });
    assert!(
        rich.is_some(),
        "the dump came through the same parser: {ui:?}"
    );
    assert!(matches!(last, Some(RunEvent::Exited { .. })), "{last:?}");
    pool.shutdown().await;
}

#[tokio::test]
async fn cancelling_asks_the_helper_to_stop_and_leaves_it_warm() {
    let dir = scratch("cancel");
    let pool = HelperPool::default();
    let config = config(&dir);
    let request = playground(&dir, "slow", b"unused\n", "slow");

    let mut handle = helper_run::start(&pool, &config, request, Waits::default().watcher())
        .await
        .unwrap();
    // Wait for it to be under way, so the cancel isn't a race.
    assert!(matches!(
        handle.recv().await,
        Some(RunEvent::Started { warm: true, .. })
    ));
    assert!(matches!(handle.recv().await, Some(RunEvent::Stdout(_))));

    handle.cancel();
    let (_, last) = collect(&mut handle).await;
    assert!(matches!(last, Some(RunEvent::Cancelled { .. })), "{last:?}",);

    let helper = pool.get_handle("fake").unwrap();
    assert!(
        helper.state().is_ready(),
        "the helper is still warm: {:?}",
        helper.state(),
    );
    assert_eq!(helper.uses(), 0, "and free for the next run");
    pool.shutdown().await;
}

#[tokio::test]
async fn two_playgrounds_queue_on_the_one_helper_rather_than_cancelling_each_other() {
    let dir = scratch("queue");
    let pool = HelperPool::default();
    let config = config(&dir);
    let waits = Waits::default();

    let slow = playground(&dir, "slow", b"unused\n", "slow");
    let mut first = helper_run::start(&pool, &config, slow, Waits::default().watcher())
        .await
        .unwrap();
    assert!(matches!(
        first.recv().await,
        Some(RunEvent::Started { warm: true, .. })
    ));
    assert!(matches!(first.recv().await, Some(RunEvent::Stdout(_))));

    // The second playground waits for the helper instead of taking it from
    // the first, which is what the protocol's one-run-at-a-time means here.
    let quick = playground(&dir, "quick", b"second\n", "echo");
    let waiting = waits.clone();
    let queued = helper_run::start(&pool, &config, quick, waiting.watcher());
    tokio::pin!(queued);
    let not_yet = tokio::time::timeout(Duration::from_millis(300), &mut queued).await;
    assert!(not_yet.is_err(), "it waits for the helper to be free");
    assert!(waiting.busy(), "and says so: {:?}", waiting.all());

    // The first run ends; the second gets the helper.
    first.cancel();
    let (_, last) = collect(&mut first).await;
    assert!(matches!(last, Some(RunEvent::Cancelled { .. })), "{last:?}");

    let mut second = tokio::time::timeout(Duration::from_secs(20), queued)
        .await
        .expect("the queued run started")
        .expect("on the helper");
    assert!(matches!(
        second.recv().await,
        Some(RunEvent::Started { warm: true, .. })
    ));
    let (ui, last) = collect(&mut second).await;
    assert_eq!(text(&ui), "second\n");
    assert!(matches!(last, Some(RunEvent::Exited { .. })), "{last:?}");
    pool.shutdown().await;
}

#[tokio::test]
async fn a_compile_error_from_the_helper_ends_the_run_with_somewhere_to_point() {
    let dir = scratch("fail");
    let pool = HelperPool::default();
    let config = config(&dir);
    let request = playground(&dir, "fail", b"broken\n", "fail");

    let mut handle = helper_run::start(&pool, &config, request, Waits::default().watcher())
        .await
        .unwrap();
    let (_, last) = collect(&mut handle).await;
    let Some(RunEvent::Failed {
        message,
        diagnostics,
    }) = last
    else {
        panic!("{last:?}")
    };
    assert!(message.contains("told to fail"), "{message}");
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].line, 3);
    pool.shutdown().await;
}

#[tokio::test]
async fn a_helper_that_keeps_dying_is_reported_so_the_caller_can_fall_back() {
    let dir = scratch("give-up");
    let pool = HelperPool::default();
    let mut config = config(&dir);
    config.policy.max_restarts = 2;
    config
        .spawn
        .env
        .push(("FAKEHELPER_CRASH_ON_RUN".into(), "1".into()));

    // The first run reaches the helper and dies with it.
    let request = playground(&dir, "one", b"unused\n", "echo");
    let mut handle = helper_run::start(&pool, &config, request, Waits::default().watcher())
        .await
        .unwrap();
    let (_, last) = collect(&mut handle).await;
    let Some(RunEvent::Failed { message, .. }) = last else {
        panic!("{last:?}")
    };
    assert!(message.contains("stopped"), "{message}");

    // It is restarted and dies again, until the pool gives up — and then the
    // run path is told there is no helper, which is its cue to use `[run]`.
    let mut reason = None;
    for _ in 0..8 {
        let request = playground(&dir, "again", b"unused\n", "echo");
        match helper_run::start(&pool, &config, request, Waits::default().watcher()).await {
            Ok(mut handle) => {
                let _ = collect(&mut handle).await;
            }
            Err(e) => {
                reason = Some(e);
                break;
            }
        }
    }
    let reason = reason.expect("the pool gave up in the end");
    assert!(
        matches!(
            reason,
            Unavailable::GaveUp { .. } | Unavailable::Start { .. }
        ),
        "{reason:?}",
    );
    assert_eq!(reason.plugin(), "fake");
    pool.shutdown().await;
}

#[tokio::test]
async fn a_build_is_shown_to_the_run_that_is_waiting_for_it() {
    let dir = scratch("build");
    let source = dir.join("helper-src");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(source.join("helper.cs"), "class Helper {}").unwrap();

    let mut build = SpawnSpec::new(fake_helper());
    build.env.push((
        "FAKEHELPER_BUILD_INTO".into(),
        dir.join("builds.txt").into_os_string(),
    ));
    let config = config(&dir).with_build(build, &source);
    let pool = HelperPool::default();
    let waits = Waits::default();

    let request = playground(&dir, "one", b"after the build\n", "echo");
    let mut handle = helper_run::start(&pool, &config, request, waits.watcher())
        .await
        .unwrap();
    let (ui, last) = collect(&mut handle).await;
    assert_eq!(text(&ui), "after the build\n");
    assert!(matches!(last, Some(RunEvent::Exited { .. })), "{last:?}");

    let lines: Vec<String> = waits
        .all()
        .into_iter()
        .filter_map(|what| match what {
            Waiting::Building(line) => Some(line),
            Waiting::Busy => None,
        })
        .collect();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("Building the fake helper")),
        "the header the run shows: {lines:?}",
    );
    assert!(
        lines.iter().any(|l| l == "fakehelper: building"),
        "and the build's own output, live: {lines:?}",
    );
    pool.shutdown().await;
}
