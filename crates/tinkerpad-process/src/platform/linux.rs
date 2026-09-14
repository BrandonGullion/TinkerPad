//! Linux: process groups plus `PR_SET_PDEATHSIG` so children die if the app is
//! killed hard.
//!
//! Filled in by work item 00-04. Stubs only for now.

/// Configure a command so its process starts in its own process group and
/// receives `SIGTERM` when the parent dies.
pub fn prepare(_cmd: &mut tokio::process::Command) {}

/// Register a spawned child (no-op on Linux; `PR_SET_PDEATHSIG` covers it).
pub fn register_child(_pid: u32) {}

/// Terminate a process group: `SIGTERM`, wait, `SIGKILL`.
pub fn kill_tree(_pid: u32) {}
