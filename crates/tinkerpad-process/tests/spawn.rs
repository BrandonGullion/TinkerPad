//! Integration tests for spawning and supervision. They use the
//! `tinkerpad-testchild` helper binary so the same tests run on every OS.

use std::time::{Duration, Instant};

use tinkerpad_process::{Exit, ProcessEvent, SpawnSpec, platform, spawn};

const CHILD: &str = env!("CARGO_BIN_EXE_tinkerpad-testchild");

/// Drain a child, returning (stdout, stderr, exit).
async fn collect(mut child: tinkerpad_process::Supervised) -> (Vec<u8>, Vec<u8>, Exit) {
    let mut out = Vec::new();
    let mut err = Vec::new();
    loop {
        match child.recv().await {
            Some(ProcessEvent::Stdout(b)) => out.extend_from_slice(&b),
            Some(ProcessEvent::Stderr(b)) => err.extend_from_slice(&b),
            Some(ProcessEvent::Exited(exit)) => return (out, err, exit),
            None => panic!("stream ended without Exited"),
        }
    }
}

#[tokio::test]
async fn echo_round_trip() {
    let child = spawn(SpawnSpec::new(CHILD).args(["echo", "hello", "world"])).unwrap();
    assert!(child.pid() > 0);
    let (out, err, exit) = collect(child).await;
    assert_eq!(String::from_utf8(out).unwrap().trim(), "hello world");
    assert!(err.is_empty());
    assert_eq!(exit.code, Some(0));
    assert!(!exit.killed);
}

#[tokio::test]
async fn stderr_is_separate() {
    let child = spawn(SpawnSpec::new(CHILD).args(["stderr", "oops"])).unwrap();
    let (out, err, exit) = collect(child).await;
    assert!(out.is_empty());
    assert_eq!(String::from_utf8(err).unwrap().trim(), "oops");
    assert_eq!(exit.code, Some(0));
}

#[tokio::test]
async fn non_zero_exit_code() {
    let child = spawn(SpawnSpec::new(CHILD).args(["exit", "42"])).unwrap();
    let (_, _, exit) = collect(child).await;
    assert_eq!(exit.code, Some(42));
    assert!(!exit.killed);
}

#[tokio::test]
async fn utf8_split_across_chunks_survives_concatenation() {
    let child = spawn(SpawnSpec::new(CHILD).arg("utf8")).unwrap();
    let (out, _, exit) = collect(child).await;
    assert_eq!(exit.code, Some(0));
    assert_eq!(String::from_utf8(out).unwrap(), "héllo wörld");
}

#[tokio::test]
async fn output_streams_without_buffering() {
    let n = 20;
    let mut child = spawn(SpawnSpec::new(CHILD).args(["slow", &n.to_string()])).unwrap();
    let mut chunks = 0;
    let mut bytes = 0;
    let started = Instant::now();
    let mut first_at = None;
    loop {
        match child.recv().await {
            Some(ProcessEvent::Stdout(b)) => {
                first_at.get_or_insert_with(|| started.elapsed());
                chunks += 1;
                bytes += b.len();
            }
            Some(ProcessEvent::Exited(_)) => break,
            Some(_) => {}
            None => panic!("no Exited"),
        }
    }
    assert_eq!(bytes, n);
    // The child flushes each byte 10 ms apart; if we were line- or
    // block-buffering we'd see one chunk. Allow some coalescing under load.
    assert!(chunks >= n / 2, "expected streaming chunks, got {chunks}");
    assert!(
        first_at.unwrap() < Duration::from_millis(500),
        "first byte took {:?}",
        first_at.unwrap()
    );
}

#[tokio::test]
async fn default_env_is_applied_and_overridable() {
    let child = spawn(SpawnSpec::new(CHILD).args(["env", "PYTHONIOENCODING"])).unwrap();
    let (out, _, _) = collect(child).await;
    assert_eq!(String::from_utf8(out).unwrap().trim(), "utf-8");

    let child = spawn(
        SpawnSpec::new(CHILD)
            .args(["env", "PYTHONIOENCODING"])
            .env("PYTHONIOENCODING", "latin-1"),
    )
    .unwrap();
    let (out, _, _) = collect(child).await;
    assert_eq!(String::from_utf8(out).unwrap().trim(), "latin-1");
}

#[tokio::test]
async fn stdin_pipe_when_requested() {
    use tokio::io::AsyncWriteExt;
    let mut child = spawn(SpawnSpec::new(CHILD).arg("cat").stdin(true)).unwrap();
    let mut stdin = child.take_stdin().expect("stdin requested");
    stdin.write_all(b"ping\n").await.unwrap();
    drop(stdin); // EOF
    let (out, _, exit) = collect(child).await;
    assert_eq!(out, b"ping\n");
    assert_eq!(exit.code, Some(0));
}

#[tokio::test]
async fn kill_takes_the_whole_tree() {
    let mut child = spawn(SpawnSpec::new(CHILD).arg("spawn-sleeper")).unwrap();
    let pid = child.pid();

    // Wait for the grandchild's pid.
    let mut line = Vec::new();
    let grandchild: u32 = loop {
        match child.recv().await {
            Some(ProcessEvent::Stdout(b)) => {
                line.extend_from_slice(&b);
                if line.ends_with(b"\n") {
                    let text = String::from_utf8_lossy(&line);
                    break text.trim().strip_prefix("child ").unwrap().parse().unwrap();
                }
            }
            other => panic!("unexpected event before pid line: {other:?}"),
        }
    };
    assert!(platform::is_alive(pid));
    assert!(platform::is_alive(grandchild));

    let handle = child.kill_handle();
    handle.kill();

    let (_, _, exit) = collect(child).await;
    assert!(exit.killed);
    assert!(!platform::is_alive(pid), "direct child still alive");

    // The grandchild is reparented and reaped by the OS; give it a moment.
    let deadline = Instant::now() + Duration::from_secs(3);
    while platform::is_alive(grandchild) && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(!platform::is_alive(grandchild), "grandchild still alive");
}

#[tokio::test]
async fn kill_before_any_output_is_clean() {
    let child = spawn(SpawnSpec::new(CHILD).args(["sleep", "60000"])).unwrap();
    child.kill();
    let (_, _, exit) = collect(child).await;
    assert!(exit.killed);
    assert!(exit.duration < Duration::from_secs(5));
}

#[tokio::test]
async fn program_not_found_is_a_typed_error() {
    let err = spawn(SpawnSpec::new("tinkerpad-definitely-not-a-program")).unwrap_err();
    match err {
        tinkerpad_process::SpawnError::NotFound { program } => {
            assert_eq!(program, "tinkerpad-definitely-not-a-program");
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn sweep_is_a_no_op_without_registry() {
    assert_eq!(platform::sweep_orphans(), 0);
}

/// Children must die when the app is killed hard (Task Manager, `kill -9`).
/// Windows relies on the job object closing; Linux on `PR_SET_PDEATHSIG`.
/// macOS has no such mechanism and relies on the next-launch sweep instead,
/// so the assertion is skipped there.
#[test]
fn children_die_when_parent_is_killed_hard() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let parent_exe = env!("CARGO_BIN_EXE_tinkerpad-testparent");
    let mut parent = Command::new(parent_exe)
        .arg(CHILD)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();

    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let child: u32 = line.trim().strip_prefix("child ").unwrap().parse().unwrap();
    assert!(platform::is_alive(child));

    // Hard kill: TerminateProcess on Windows, SIGKILL on Unix.
    parent.kill().unwrap();
    parent.wait().unwrap();

    if platform::name() == "macos" {
        // Clean up so the test run doesn't leak a sleeper.
        let _ = Command::new("kill").arg(child.to_string()).status();
        return;
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while platform::is_alive(child) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        !platform::is_alive(child),
        "child {child} outlived its hard-killed parent"
    );
}
