//! Platform-specific process handling.
//!
//! Each OS gets one file implementing the same small set of functions. This is
//! the only module in the workspace allowed to use `cfg(target_os)`; CI greps
//! for it elsewhere and fails the build (work item 00-08).
//!
//! | Concern                  | Windows          | macOS                     | Linux                              |
//! |--------------------------|------------------|---------------------------|------------------------------------|
//! | Children die with app    | Job object       | Process group + pid sweep | Process group + `PR_SET_PDEATHSIG` |
//! | Console window flash     | `CREATE_NO_WINDOW` | n/a                     | n/a                                |
//! | Kill tree                | Terminate job    | `SIGTERM` group, `SIGKILL` | `SIGTERM` group, `SIGKILL`        |

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
