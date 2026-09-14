//! macOS: process groups plus a pid registry swept at launch.
//!
//! macOS has no parent-death signal. Normal exit and panic paths kill every
//! group, but a force-quit (`kill -9` on the app) leaves children running. To
//! cover that, every spawned pid and program name is appended to a registry
//! file, and [`sweep_orphans`] at the next launch terminates any listed pid
//! that is still alive *and* still runs the same program (guarding against pid
//! reuse).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use tracing::{debug, warn};

use super::unix;

static REGISTRY: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Nothing to hold on macOS; the process group is addressed by pid.
pub struct Guard;

/// Own process group.
pub fn prepare(cmd: &mut tokio::process::Command) {
    unix::set_process_group(cmd);
}

/// Record the child in the registry (if one is configured).
pub fn register_child(pid: u32, program: &str) -> Guard {
    if let Some(path) = REGISTRY.lock().ok().and_then(|r| r.clone()) {
        let name = program_basename(program);
        let result = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| writeln!(f, "{pid}\t{name}"));
        if let Err(error) = result {
            warn!(pid, program, %error, "could not write pid registry");
        }
    }
    Guard
}

/// `SIGTERM` to the group.
pub fn terminate(_guard: &Guard, pid: u32) {
    unix::signal_group(pid, libc::SIGTERM);
}

/// `SIGKILL` to the group.
pub fn terminate_hard(_guard: &Guard, pid: u32) {
    unix::signal_group(pid, libc::SIGKILL);
}

pub fn is_alive(pid: u32) -> bool {
    unix::is_alive(pid)
}

/// Where to record spawned pids. Call once at startup, before any spawn.
pub fn set_registry(path: PathBuf) {
    if let Ok(mut r) = REGISTRY.lock() {
        *r = Some(path);
    }
}

/// Terminate leftovers from a previous, force-quit session. Returns how many
/// processes were signalled. Rewrites the registry empty.
pub fn sweep_orphans() -> usize {
    let Some(path) = REGISTRY.lock().ok().and_then(|r| r.clone()) else {
        return 0;
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return 0;
    };

    let mut swept = 0;
    for line in contents.lines() {
        let Some((pid, name)) = line.split_once('\t') else {
            continue;
        };
        let Ok(pid) = pid.parse::<u32>() else {
            continue;
        };
        if !is_alive(pid) {
            continue;
        }
        if running_program(pid).as_deref() != Some(name) {
            debug!(pid, name, "pid reused by another program; not sweeping");
            continue;
        }
        warn!(pid, name, "terminating orphan from a previous session");
        unix::signal_group(pid, libc::SIGTERM);
        swept += 1;
    }

    if let Err(error) = std::fs::write(&path, "") {
        warn!(%error, "could not clear pid registry");
    }
    swept
}

fn program_basename(program: &str) -> String {
    std::path::Path::new(program)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_owned())
}

/// The basename of the executable a pid is running, via `ps`.
fn running_program(pid: u32) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let comm = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if comm.is_empty() {
        None
    } else {
        Some(program_basename(&comm))
    }
}
