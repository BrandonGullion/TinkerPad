//! Windows: job objects and `CREATE_NO_WINDOW`.
//!
//! Filled in by work item 00-04. Stubs only for now.

/// Configure a command so its process is hidden and joins the app job object.
pub fn prepare(_cmd: &mut tokio::process::Command) {}

/// Register a spawned child so it dies with the app.
pub fn register_child(_pid: u32) {}

/// Terminate a process and all of its descendants.
pub fn kill_tree(_pid: u32) {}
