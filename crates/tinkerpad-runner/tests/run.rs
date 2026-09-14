//! Integration tests for runs. They use `node` when it is installed (CI
//! installs it on every platform) and skip with a message otherwise.

use std::time::{Duration, Instant};

use tinkerpad_runner::{RunError, RunEvent, RunSpec, run};

fn node() -> Option<RunSpec> {
    let ok = std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("node not installed; skipping");
        return None;
    }
    Some(RunSpec {
        program: "node".into(),
        ..RunSpec::default()
    })
}

fn eval(mut spec: RunSpec, code: &str) -> RunSpec {
    spec.args = vec!["-e".into(), code.into()];
    spec
}

#[tokio::test]
async fn console_log_one() {
    let Some(spec) = node() else { return };
    let started = Instant::now();
    let mut handle = run(eval(spec, "console.log(1)")).unwrap();

    let mut out = Vec::new();
    let mut first_byte = None;
    let mut saw_started = false;
    let end = loop {
        match handle.recv().await.expect("stream ended early") {
            RunEvent::Started { pid } => {
                assert!(pid > 0);
                saw_started = true;
            }
            RunEvent::Stdout(b) => {
                first_byte.get_or_insert_with(|| started.elapsed());
                out.extend_from_slice(&b);
            }
            RunEvent::Stderr(b) => panic!("unexpected stderr: {}", String::from_utf8_lossy(&b)),
            other => break other,
        }
    };

    assert!(saw_started);
    assert_eq!(String::from_utf8(out).unwrap().trim(), "1");
    match end {
        RunEvent::Exited { code, .. } => assert_eq!(code, Some(0)),
        other => panic!("expected Exited, got {other:?}"),
    }
    eprintln!(
        "node console.log(1): first byte after {:?}",
        first_byte.unwrap()
    );
    assert!(handle.recv().await.is_none(), "events after the end");
}

#[tokio::test]
async fn infinite_loop_can_be_cancelled() {
    let Some(spec) = node() else { return };
    let mut handle = run(eval(spec, "while(true){}")).unwrap();
    assert!(matches!(
        handle.recv().await,
        Some(RunEvent::Started { .. })
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;
    handle.cancel();
    let end = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match handle.recv().await {
                Some(RunEvent::Cancelled { duration }) => break duration,
                Some(RunEvent::Exited { .. }) => panic!("expected Cancelled, got Exited"),
                Some(_) => {}
                None => panic!("stream ended without Cancelled"),
            }
        }
    })
    .await
    .expect("cancel took too long");
    assert!(end >= Duration::from_millis(300));
    assert!(!tinkerpad_process::platform::is_alive(handle.pid));
}

#[tokio::test]
async fn non_zero_exit_and_stderr() {
    let Some(spec) = node() else { return };
    let mut handle = run(eval(spec, "console.error('bad'); process.exit(3)")).unwrap();
    let mut err = Vec::new();
    let code = loop {
        match handle.recv().await.unwrap() {
            RunEvent::Stderr(b) => err.extend_from_slice(&b),
            RunEvent::Exited { code, .. } => break code,
            RunEvent::Cancelled { .. } => panic!("not cancelled"),
            _ => {}
        }
    };
    assert_eq!(code, Some(3));
    assert_eq!(String::from_utf8(err).unwrap().trim(), "bad");
}

#[tokio::test]
async fn missing_program_is_not_found() {
    let spec = RunSpec {
        program: "tinkerpad-no-such-runtime".into(),
        ..RunSpec::default()
    };
    match run(spec) {
        Err(RunError::NotFound { program }) => assert_eq!(program, "tinkerpad-no-such-runtime"),
        other => panic!("expected NotFound, got {other:?}"),
    }
}
