//! Linux: process groups plus `PR_SET_PDEATHSIG` so children die if the app is
//! killed hard.
//!
//! `PR_SET_PDEATHSIG` fires when the *thread* that spawned the child exits.
//! Tokio worker threads live for the runtime's lifetime, so spawning from async
//! code means "when the app dies", which is what we want. **Do not call
//! `spawn` from `spawn_blocking` or other short-lived threads**: those threads
//! exit after idling and would take the child with them.

use std::path::PathBuf;

use super::unix;

/// Nothing to hold on Linux; the process group is addressed by pid.
pub struct Guard;

/// Own process group, and `SIGTERM` on parent death.
pub fn prepare(cmd: &mut tokio::process::Command) {
    unix::set_process_group(cmd);
    // SAFETY: `pre_exec` runs in the forked child before exec; prctl and
    // getppid are async-signal-safe.
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // The parent may have died between fork and prctl; if we've been
            // reparented already, don't outlive it.
            if libc::getppid() == 1 {
                libc::_exit(1);
            }
            Ok(())
        });
    }
}

pub fn register_child(_pid: u32, _program: &str) -> Guard {
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

/// No registry needed on Linux; `PR_SET_PDEATHSIG` covers force-quit.
pub fn set_registry(_path: PathBuf) {}

/// Nothing to sweep on Linux.
pub fn sweep_orphans() -> usize {
    0
}
