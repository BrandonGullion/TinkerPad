//! The helper protocol against a real process (work item 03-01).
//!
//! Everything here runs the fake helper, which speaks `tinkerpad-plugin-v1`
//! over real pipes and behaves like someone else's program: a banner before
//! `ready`, extra fields in its messages, and a message type this build has
//! never heard of.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;
use tinkerpad_core::{KindId, LanguageId, Playground, PlaygroundId};
use tinkerpad_plugin::helper::{
    CANCEL_GRACE, HelperClient, HelperError, HelperEvent, HelperRun, HelperStart, HookRequest,
    RunRequest,
};
use tinkerpad_process::SpawnSpec;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakehelper");

/// The `tinkerpad-dump-v1` sentinel, byte for byte (`docs/PLUGINS.md` §6).
/// Written out here rather than imported: this crate is below the runner that
/// parses it, and the point of the test is that the bytes survive the trip.
const DUMP_PREFIX: &[u8] = b"\x1b]1337;TinkerPad=";
const BEL: u8 = 0x07;

/// A folder of this test's own.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir()
        .join(format!("tinkerpad-helper-{}", std::process::id()))
        .join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn start(dir: &Path) -> HelperStart {
    HelperStart::new("fake", SpawnSpec::new(FAKE), dir.join("plugin"), dir)
        .with_ready_timeout(Duration::from_secs(20))
}

/// A playground the hooks can be told about.
fn playground(dir: &Path, name: &str) -> Playground {
    Playground {
        id: PlaygroundId::new(format!("pg-{name}")),
        name: name.to_owned(),
        language: LanguageId::new("fake"),
        kind: KindId::new("statements"),
        dir: dir.to_path_buf(),
        main_file: dir.join("main.fake"),
        created_ms: 1_700_000_000_000,
        tags: Vec::new(),
    }
}

/// Write the file a run will be pointed at.
fn main_file(dir: &Path, body: &[u8]) -> PathBuf {
    let path = dir.join("main.fake");
    std::fs::write(&path, body).unwrap();
    path
}

fn request(main: PathBuf, dir: &Path, kind: &str) -> RunRequest {
    RunRequest {
        main,
        dir: dir.to_path_buf(),
        kind: KindId::new(kind),
    }
}

/// How a run ended, skipping whatever output is still in flight. Chunks are
/// small and arbitrary, so the event after a cancel is often another one.
async fn ending(run: &mut HelperRun) -> Option<HelperEvent> {
    while let Some(event) = run.recv().await {
        match event {
            HelperEvent::Stdout(_) | HelperEvent::Stderr(_) => continue,
            other => return Some(other),
        }
    }
    None
}

/// Everything a run said, in order, with output gathered back into bytes.
async fn collect(run: &mut HelperRun) -> (Vec<u8>, Vec<u8>, Option<HelperEvent>) {
    let (mut out, mut err, mut last) = (Vec::new(), Vec::new(), None);
    while let Some(event) = run.recv().await {
        match event {
            HelperEvent::Stdout(bytes) => out.extend_from_slice(&bytes),
            HelperEvent::Stderr(bytes) => err.extend_from_slice(&bytes),
            other => last = Some(other),
        }
    }
    (out, err, last)
}

#[tokio::test]
async fn hello_is_answered_with_ready_through_the_banner_a_runtime_prints() {
    let dir = scratch("hello");
    let client = HelperClient::start(start(&dir)).await.unwrap();

    assert_eq!(client.helper_name(), Some("fakehelper"));
    assert_eq!(client.version(), Some("1.0.0"));
    assert!(client.is_alive());
    assert!(client.pid() > 0);

    // What the helper was told about itself, as it saw it.
    let seen: Value =
        serde_json::from_slice(&std::fs::read(dir.join("hello.json")).unwrap()).unwrap();
    assert_eq!(seen["protocol"], "tinkerpad-plugin-v1");
    assert_eq!(
        seen["plugin_dir"].as_str().unwrap(),
        dir.join("plugin").to_string_lossy(),
    );
    assert_eq!(seen["scratch_dir"].as_str().unwrap(), dir.to_string_lossy());

    client.shutdown().await;
    assert!(!client.is_alive());
}

#[tokio::test]
async fn a_run_streams_its_output_back_with_a_dump_sentinel_intact() {
    let dir = scratch("run");
    // The sentinel is longer than the helper's chunk size, so it can only
    // arrive in one piece if the chunks are stitched back together in order.
    let mut body = b"before\n".to_vec();
    body.extend_from_slice(DUMP_PREFIX);
    body.extend_from_slice(br#"{"v":1,"value":{"kind":"table","rows":2}}"#);
    body.push(BEL);
    body.extend_from_slice("after \u{1f600}\n".as_bytes());
    let main = main_file(&dir, &body);

    let client = HelperClient::start(start(&dir)).await.unwrap();
    let mut run = client.run(request(main, &dir, "echo")).await.unwrap();
    assert_eq!(run.id(), "r1", "runs are numbered for the helper");

    let (out, err, last) = collect(&mut run).await;
    assert_eq!(
        out, body,
        "the bytes arrive exactly as the program wrote them"
    );
    assert!(err.is_empty());
    assert!(
        matches!(last, Some(HelperEvent::Exited { code: Some(0), .. })),
        "{last:?}",
    );
    assert!(client.is_alive(), "the helper is still warm afterwards");
    client.shutdown().await;
}

#[tokio::test]
async fn a_runs_stderr_is_separate_from_the_helpers_own() {
    let dir = scratch("stderr");
    let main = main_file(&dir, b"this went to stderr\n");
    let client = HelperClient::start(start(&dir)).await.unwrap();

    let mut run = client.run(request(main, &dir, "stderr")).await.unwrap();
    let (out, err, last) = collect(&mut run).await;
    assert!(out.is_empty(), "{out:?}");
    assert_eq!(err, b"this went to stderr\n");
    assert!(matches!(last, Some(HelperEvent::Exited { .. })), "{last:?}");
    client.shutdown().await;
}

#[tokio::test]
async fn a_cancel_the_helper_honours_ends_the_run_and_keeps_the_helper() {
    let dir = scratch("cancel");
    let main = main_file(&dir, b"unused\n");
    let client = HelperClient::start(start(&dir)).await.unwrap();

    let mut run = client
        .run(request(main.clone(), &dir, "slow"))
        .await
        .unwrap();
    // Wait for the run to be under way, so the cancel is not a race.
    assert!(matches!(run.recv().await, Some(HelperEvent::Stdout(_))));

    let started = Instant::now();
    run.cancel().await;
    let took = started.elapsed();
    assert!(
        took < CANCEL_GRACE,
        "the cancel was answered, not waited out: {took:?}"
    );
    let last = ending(&mut run).await;
    assert!(
        matches!(last, Some(HelperEvent::Cancelled { .. })),
        "a cancelled run says so rather than reporting an exit code: {last:?}",
    );
    assert!(run.recv().await.is_none(), "and that is the last of it");

    assert!(client.is_alive(), "a helper that cancels cleanly is kept");
    // And it can still run something.
    let mut again = client.run(request(main, &dir, "echo")).await.unwrap();
    let (out, _, last) = collect(&mut again).await;
    assert_eq!(out, b"unused\n");
    assert!(matches!(last, Some(HelperEvent::Exited { .. })), "{last:?}");
    client.shutdown().await;
}

#[tokio::test]
async fn a_helper_that_ignores_a_cancel_is_killed() {
    let dir = scratch("hang");
    let main = main_file(&dir, b"unused\n");
    let mut spec = SpawnSpec::new(FAKE);
    spec.env
        .push(("FAKEHELPER_HANG_ON_CANCEL".into(), "1".into()));
    let client = HelperClient::start(HelperStart {
        spawn: spec,
        ..start(&dir)
    })
    .await
    .unwrap();

    let mut run = client.run(request(main, &dir, "slow")).await.unwrap();
    assert!(matches!(run.recv().await, Some(HelperEvent::Stdout(_))));

    let started = Instant::now();
    run.cancel().await;
    let took = started.elapsed();
    assert!(
        took >= CANCEL_GRACE,
        "it waited the grace out first: {took:?}"
    );
    let last = ending(&mut run).await;
    assert!(
        matches!(last, Some(HelperEvent::Cancelled { .. })),
        "the run is cancelled either way: {last:?}",
    );
    assert!(
        !client.is_alive(),
        "and the helper that would not stop is gone"
    );
}

#[tokio::test]
async fn a_helper_that_dies_mid_run_fails_the_run_rather_than_hanging_it() {
    let dir = scratch("crash");
    let main = main_file(&dir, b"unused\n");
    let mut spec = SpawnSpec::new(FAKE);
    spec.env
        .push(("FAKEHELPER_CRASH_ON_RUN".into(), "1".into()));
    let client = HelperClient::start(HelperStart {
        spawn: spec,
        ..start(&dir)
    })
    .await
    .unwrap();

    let mut run = client.run(request(main, &dir, "echo")).await.unwrap();
    let (_, _, last) = collect(&mut run).await;
    let Some(HelperEvent::Failed { message, .. }) = last else {
        panic!("{last:?}")
    };
    assert!(message.contains("stopped"), "{message}");
    assert!(!client.is_alive());

    // And the helper being gone is an error, not a wait.
    let err = client
        .run(request(dir.join("main.fake"), &dir, "echo"))
        .await
        .unwrap_err();
    assert!(matches!(err, HelperError::Gone { .. }), "{err:?}");
}

#[tokio::test]
async fn running_twice_cancels_what_was_running() {
    let dir = scratch("twice");
    let main = main_file(&dir, b"second run\n");
    let client = HelperClient::start(start(&dir)).await.unwrap();

    let mut first = client
        .run(request(main.clone(), &dir, "slow"))
        .await
        .unwrap();
    assert!(matches!(first.recv().await, Some(HelperEvent::Stdout(_))));

    let mut second = client.run(request(main, &dir, "echo")).await.unwrap();
    let first_ending = ending(&mut first).await;
    assert!(
        matches!(first_ending, Some(HelperEvent::Cancelled { .. })),
        "the first run is cancelled, not abandoned: {first_ending:?}",
    );
    let (out, _, last) = collect(&mut second).await;
    assert_eq!(out, b"second run\n");
    assert!(matches!(last, Some(HelperEvent::Exited { .. })), "{last:?}");
    client.shutdown().await;
}

#[tokio::test]
async fn a_compile_error_comes_back_as_a_failure_with_somewhere_to_point() {
    let dir = scratch("fail");
    let main = main_file(&dir, b"broken\n");
    let client = HelperClient::start(start(&dir)).await.unwrap();

    let mut run = client
        .run(request(main.clone(), &dir, "fail"))
        .await
        .unwrap();
    let (_, _, last) = collect(&mut run).await;
    let Some(HelperEvent::Failed {
        message,
        diagnostics,
    }) = last
    else {
        panic!("{last:?}")
    };
    assert!(message.contains("told to fail"), "{message}");
    assert_eq!(diagnostics.len(), 1);
    let annotation = diagnostics[0].clone().into_annotation();
    assert_eq!(annotation.line, 3);
    assert_eq!(annotation.col, Some(9));
    assert_eq!(
        annotation.file.as_deref(),
        Some(main.to_string_lossy().as_ref())
    );
    assert_eq!(annotation.severity, tinkerpad_core::Level::Error);

    assert!(client.is_alive(), "a failed run does not cost the helper");
    client.shutdown().await;
}

#[tokio::test]
async fn hooks_go_out_and_their_answers_come_back() {
    let dir = scratch("hooks");
    let client = HelperClient::start(start(&dir).hooks([
        "playground_created",
        "playground_opened",
        "add_dependency",
    ]))
    .await
    .unwrap();
    let pg = playground(&dir, "Notes");

    let created = client
        .hook(HookRequest::PlaygroundCreated {
            playground: pg.clone(),
        })
        .await
        .unwrap();
    assert_eq!(created.files.len(), 1);
    assert_eq!(created.files[0].path, PathBuf::from("helper.json"));
    assert!(created.files[0].content.contains("Notes"));
    assert!(created.lsp_config.is_none());

    let opened = client
        .hook(HookRequest::PlaygroundOpened {
            playground: pg.clone(),
            open: vec![pg.clone(), playground(&dir, "Other")],
        })
        .await
        .unwrap();
    assert_eq!(opened.files[0].content, "2 open\n");
    let config = opened.lsp_config.expect("a hook may configure the server");
    assert_eq!(config["fake.setting"]["root"], "Notes");

    let added = client
        .hook(HookRequest::AddDependency {
            playground: pg.clone(),
            name: "serde".to_owned(),
            version: Some("1".to_owned()),
        })
        .await
        .unwrap();
    assert_eq!(added.files[0].content, "serde 1\n");

    // Not in the manifest, so it is never sent: the answer is nothing, and it
    // comes back at once rather than after the five second wait.
    let started = Instant::now();
    let closed = client
        .hook(HookRequest::PlaygroundClosed {
            playground: pg,
            open: Vec::new(),
        })
        .await
        .unwrap();
    assert_eq!(closed, Default::default());
    assert!(started.elapsed() < Duration::from_secs(1));

    client.shutdown().await;
}

#[tokio::test]
async fn a_helper_that_never_says_ready_is_killed_and_reported() {
    let dir = scratch("slow-ready");
    let mut spec = SpawnSpec::new(FAKE);
    spec.env
        .push(("FAKEHELPER_SLOW_READY_MS".into(), "30000".into()));

    let started = Instant::now();
    let error = HelperClient::start(HelperStart {
        spawn: spec,
        ready_timeout: Duration::from_millis(400),
        ..start(&dir)
    })
    .await
    .unwrap_err();
    let took = started.elapsed();

    assert!(matches!(error, HelperError::NoReady { .. }), "{error:?}");
    assert!(
        took < Duration::from_secs(5),
        "it gave up when it said it would: {took:?}",
    );
    assert!(
        error.to_string().contains("did not answer hello"),
        "{error}",
    );
}

#[tokio::test]
async fn a_helper_that_ignores_shutdown_is_killed_after_the_grace() {
    let dir = scratch("stubborn");
    let mut spec = SpawnSpec::new(FAKE);
    spec.env
        .push(("FAKEHELPER_IGNORE_SHUTDOWN".into(), "1".into()));
    let client = HelperClient::start(HelperStart {
        spawn: spec,
        ..start(&dir)
    })
    .await
    .unwrap();

    let started = Instant::now();
    client.shutdown().await;
    let took = started.elapsed();
    assert!(!client.is_alive(), "it went one way or the other");
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(10),
        "the grace was waited out and no longer: {took:?}",
    );
}

/// What warm execution is for: the second run must not cost what the first
/// did. The numbers are printed for the work item; the assertions are loose
/// enough for the slowest CI machine.
#[tokio::test]
async fn a_warm_helper_answers_a_run_in_about_a_millisecond() {
    let dir = scratch("timing");
    let main = main_file(&dir, b"tick\n");

    let started = Instant::now();
    let client = HelperClient::start(start(&dir)).await.unwrap();
    let ready = started.elapsed();

    const RUNS: u32 = 50;
    let started = Instant::now();
    for _ in 0..RUNS {
        let mut run = client
            .run(request(main.clone(), &dir, "echo"))
            .await
            .unwrap();
        let (out, _, last) = collect(&mut run).await;
        assert_eq!(out, b"tick\n");
        assert!(matches!(last, Some(HelperEvent::Exited { .. })), "{last:?}");
    }
    let each = started.elapsed() / RUNS;

    eprintln!("spawn to ready: {ready:?}; {RUNS} runs, {each:?} each");
    assert!(ready < Duration::from_secs(5), "{ready:?}");
    assert!(each < Duration::from_millis(100), "{each:?}");
    client.shutdown().await;
}

/// Both halves of the forward-compatibility rule, from outside the client:
/// fields a side doesn't recognise change nothing, in either direction
/// (`docs/PLUGINS.md` §5, `docs/DEBUGGING.md` D7).
#[tokio::test]
async fn neither_side_minds_a_field_it_has_never_heard_of() {
    use tinkerpad_process::ProcessEvent;
    use tokio::io::AsyncWriteExt;

    let dir = scratch("future");
    let main = main_file(&dir, b"still ran\n");

    let mut spec = SpawnSpec::new(FAKE);
    spec.stdin = true;
    let mut child = tinkerpad_process::spawn(spec).unwrap();
    let mut stdin = child.take_stdin().unwrap();

    // A `hello` and a `run` from a TinkerPad newer than this helper: the extra
    // fields are exactly the sort a later version adds (a `debug` flag, say).
    let hello = format!(
        r#"{{"type":"hello","protocol":"tinkerpad-plugin-v1","plugin_dir":"{dir}","scratch_dir":"{dir}","debug":true,"future":{{"nested":[1,2]}}}}"#,
        dir = dir.to_string_lossy().replace('\\', "\\\\"),
    );
    let run = format!(
        r#"{{"type":"run","id":"r9","main":"{main}","dir":"{dir}","kind":"echo","debug":true,"limits":{{"rss_kb":1}}}}"#,
        main = main.to_string_lossy().replace('\\', "\\\\"),
        dir = dir.to_string_lossy().replace('\\', "\\\\"),
    );
    stdin
        .write_all(format!("{hello}\n{run}\n").as_bytes())
        .await
        .unwrap();
    stdin.flush().await.unwrap();

    // Read until the run is over, or give up.
    let mut text = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !text.contains("\"exited\"") {
        let Ok(Some(event)) = tokio::time::timeout_at(deadline, child.recv()).await else {
            panic!("the helper stopped saying anything: {text}");
        };
        if let ProcessEvent::Stdout(bytes) = event {
            text.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    child.kill();

    let messages: Vec<Value> = text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    assert!(
        messages.iter().any(|m| m["type"] == "ready"),
        "hello was answered despite the extra fields: {text}",
    );
    let output: String = messages
        .iter()
        .filter(|m| m["type"] == "stdout")
        .map(|m| {
            let data = m["data"].as_str().unwrap_or_default();
            String::from_utf8(BASE64.decode(data).unwrap()).unwrap()
        })
        .collect();
    assert_eq!(output, "still ran\n", "and the run happened: {text}");
    assert!(
        messages
            .iter()
            .any(|m| m["type"] == "exited" && m["id"] == "r9"),
    );

    // The other direction: the client read this helper's `ready`, which
    // carries fields no version of the core has ever defined, and its
    // `telemetry` message, which is not a message type at all.
    let client = HelperClient::start(start(&dir)).await.unwrap();
    assert_eq!(client.helper_name(), Some("fakehelper"));
    client.shutdown().await;
}

#[tokio::test]
async fn a_helper_shuts_down_when_asked_without_waiting_for_the_grace() {
    let dir = scratch("polite");
    let client = HelperClient::start(start(&dir)).await.unwrap();

    let started = Instant::now();
    client.shutdown().await;
    assert!(!client.is_alive());
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "it stopped when asked: {:?}",
        started.elapsed(),
    );
    // Asking twice is not an error.
    client.shutdown().await;
}
