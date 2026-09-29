//! Platform-specific process handling.
//!
//! Each OS gets one file implementing the same small set of functions. This is
//! the only module in the workspace allowed to use `cfg(target_os)`; CI greps
//! for it elsewhere and fails the build (work item 00-08).
//!
//! | Concern               | Windows            | macOS                      | Linux                              |
//! |-----------------------|--------------------|----------------------------|------------------------------------|
//! | Children die with app | Job object per child | Process group + pid registry swept at launch | Process group + `PR_SET_PDEATHSIG` |
//! | Console window flash  | `CREATE_NO_WINDOW` | n/a                        | n/a                                |
//! | Kill tree             | Terminate job      | `SIGTERM` group, `SIGKILL` | `SIGTERM` group, `SIGKILL`         |
//!
//! Every backend exposes:
//!
//! - `prepare(&mut Command)`: set flags before spawn.
//! - `register_child(pid, program) -> Guard`: after spawn; the guard is held
//!   for the child's lifetime and dropping it releases platform resources.
//! - `terminate(&Guard, pid)`: polite termination of the whole tree.
//! - `terminate_hard(&Guard, pid)`: forceful termination of the whole tree.
//! - `is_alive(pid) -> bool`: liveness check used by tests and the sweep.
//! - `set_registry(path)` / `sweep_orphans()`: the macOS pid registry; no-ops
//!   elsewhere so the app can call them unconditionally.
//! - `print_to_parent_console(text) -> bool`: Windows needs this so a GUI
//!   build's `--version` reaches the console that started it; `false`
//!   elsewhere, and the caller falls back to `println!`.

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(unix)]
mod unix;

/// Human-readable name of the current platform, for logs and the status strip.
pub const fn name() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    }
}
