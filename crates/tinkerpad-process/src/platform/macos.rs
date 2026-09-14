//! macOS: process groups plus a pid registry swept at launch (no parent-death
//! signal exists on this platform).
//!
//! Filled in by work item 00-04. Stubs only for now.

/// Configure a command so its process starts in its own process group.
pub fn prepare(_cmd: &mut tokio::process::Command) {}

/// Register a spawned child so it can be swept if the app is force-quit.
pub fn register_child(_pid: u32) {}

/// Terminate a process group: `SIGTERM`, wait, `SIGKILL`.
pub fn kill_tree(_pid: u32) {}
