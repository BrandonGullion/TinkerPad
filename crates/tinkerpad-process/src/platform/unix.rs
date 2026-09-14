//! Shared Unix behaviour: process groups and signals.

use tracing::debug;

/// Put the child in its own process group so signals reach its descendants.
pub fn set_process_group(cmd: &mut tokio::process::Command) {
    cmd.process_group(0);
}

/// Send a signal to the child's whole process group.
pub fn signal_group(pid: u32, signal: libc::c_int) {
    let Ok(pid) = i32::try_from(pid) else {
        return;
    };
    if pid <= 0 {
        return;
    }
    // SAFETY: plain libc call; a negative pid addresses the process group.
    let rc = unsafe { libc::kill(-pid, signal) };
    if rc != 0 {
        debug!(
            pid,
            signal, "kill(-pgid) failed (group may already be gone)"
        );
    }
}

/// Whether a process with this pid exists (zombies count as alive until reaped).
pub fn is_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: signal 0 performs only the existence/permission check.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
