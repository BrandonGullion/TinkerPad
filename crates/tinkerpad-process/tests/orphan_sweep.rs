//! The macOS force-quit path: a hard-killed app leaves children behind, and
//! the next launch sweeps them using the pid registry.
//!
//! Lives in its own test binary because the registry is process-global and
//! would otherwise capture pids from tests running in parallel. On Windows and
//! Linux the registry is a no-op (children already died with the parent), so
//! the test only asserts that sweeping finds nothing to do.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tinkerpad_process::platform;

const CHILD: &str = env!("CARGO_BIN_EXE_tinkerpad-testchild");
const PARENT: &str = env!("CARGO_BIN_EXE_tinkerpad-testparent");

#[test]
fn next_launch_sweeps_orphans_from_a_force_quit() {
    let registry = std::env::temp_dir().join(format!("tinkerpad-pids-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&registry);

    let mut parent = Command::new(PARENT)
        .arg(CHILD)
        .env("TINKERPAD_PID_REGISTRY", &registry)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(parent.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let child: u32 = line.trim().strip_prefix("child ").unwrap().parse().unwrap();

    parent.kill().unwrap();
    parent.wait().unwrap();

    // "Next launch": configure the same registry and sweep.
    platform::set_registry(registry.clone());
    let swept = platform::sweep_orphans();

    if platform::name() == "macos" {
        assert_eq!(swept, 1, "expected the orphaned child to be swept");
    } else {
        assert_eq!(swept, 0, "registry is a no-op on this platform");
    }

    let deadline = Instant::now() + Duration::from_secs(5);
    while platform::is_alive(child) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!platform::is_alive(child), "child {child} survived");

    let _ = std::fs::remove_file(&registry);
}
