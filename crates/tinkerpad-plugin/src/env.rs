//! Environment discovery: where to find toolchains.
//!
//! GUI apps on macOS (and, less severely, Linux) do not inherit the shell's
//! `PATH`, so Homebrew, nvm, and dotnet are invisible unless we ask the login
//! shell. Windows uses the inherited `PATH` plus `PATHEXT`.
//!
//! Work item 01-03 fills in the login-shell query. For now this returns the
//! inherited `PATH` on every platform.

use std::ffi::OsString;

/// The `PATH` to use for every program lookup.
pub fn discover_path() -> OsString {
    std::env::var_os("PATH").unwrap_or_default()
}
