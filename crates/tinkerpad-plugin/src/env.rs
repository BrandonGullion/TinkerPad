//! Environment discovery: where to find toolchains.
//!
//! GUI apps on macOS (and, less severely, Linux) do not inherit the shell's
//! `PATH`, so Homebrew, nvm, and dotnet are invisible unless we ask the login
//! shell. Windows uses the inherited `PATH` plus `PATHEXT`.
//!
//! This module and `tinkerpad-process/src/platform/` are the only places
//! allowed to use platform `cfg` (see `scripts/lint-platform-cfg.sh`).

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::time::Duration;

use tinkerpad_process::{ProcessEvent, SpawnSpec, spawn};
use tracing::{info, warn};

/// How long to wait for the login shell to report its `PATH`.
pub const SHELL_TIMEOUT: Duration = Duration::from_secs(2);

/// Markers around the `PATH` the shell prints, so banners and other output
/// from rc files can't be mistaken for it.
const START: &str = "__TINKERPAD_PATH_START__";
const END: &str = "__TINKERPAD_PATH_END__";

/// The `PATH` to use for every program lookup.
///
/// On macOS and Linux this asks the user's login shell (`$SHELL -ilc`), with a
/// timeout, falling back to the inherited `PATH`. On Windows it is the
/// inherited `PATH`.
pub async fn discover_path() -> OsString {
    let inherited = std::env::var_os("PATH").unwrap_or_default();
    if !uses_login_shell() {
        return inherited;
    }
    let Some(shell) = std::env::var_os("SHELL").filter(|s| !s.is_empty()) else {
        warn!("SHELL is not set; using the inherited PATH");
        return inherited;
    };
    match login_shell_path(&shell, SHELL_TIMEOUT).await {
        Ok(path) => {
            info!(shell = %shell.to_string_lossy(), "using PATH from the login shell");
            path
        }
        Err(reason) => {
            warn!(shell = %shell.to_string_lossy(), %reason, "using the inherited PATH");
            inherited
        }
    }
}

fn uses_login_shell() -> bool {
    cfg!(unix)
}

/// Ask `shell` for the `PATH` a login, interactive session would have.
pub async fn login_shell_path(shell: &OsStr, timeout: Duration) -> Result<OsString, String> {
    let is_fish = Path::new(shell)
        .file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with("fish"));
    let script = if is_fish {
        format!("printf '%s%s%s' '{START}' (string join : $PATH) '{END}'")
    } else {
        format!("printf '%s%s%s' '{START}' \"$PATH\" '{END}'")
    };

    let mut child =
        spawn(SpawnSpec::new(shell).args(["-ilc", script.as_str()])).map_err(|e| e.to_string())?;
    let kill = child.kill_handle();

    let collect = async {
        let mut out = Vec::new();
        while let Some(event) = child.recv().await {
            match event {
                ProcessEvent::Stdout(b) => out.extend_from_slice(&b),
                ProcessEvent::Stderr(_) => {}
                ProcessEvent::Exited(_) => break,
            }
        }
        out
    };
    let out = match tokio::time::timeout(timeout, collect).await {
        Ok(out) => out,
        Err(_) => {
            kill.kill();
            return Err(format!("login shell took longer than {timeout:?}"));
        }
    };

    let text = String::from_utf8_lossy(&out);
    let path = text
        .split_once(START)
        .and_then(|(_, rest)| rest.split_once(END))
        .map(|(path, _)| path.trim())
        .filter(|p| !p.is_empty())
        .ok_or_else(|| "login shell didn't print a PATH".to_owned())?;
    Ok(OsString::from(path))
}

/// Find `program` the way a shell would, using `path` (from
/// [`discover_path`]) instead of the process environment.
///
/// A program containing a path separator is checked as a path and not
/// searched for. On Windows, a name without an extension is tried with each
/// extension in `PATHEXT` (default `.COM;.EXE;.BAT;.CMD`), so `npm` finds
/// `npm.cmd`.
pub fn find_program(program: &str, path: &OsStr) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains(['/', '\\']) {
        return candidates(Path::new(program))
            .into_iter()
            .find(|c| is_executable(c));
    }
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(|dir| candidates(&dir.join(program)))
        .find(|c| is_executable(c))
}

/// The files to try for a program path: itself, plus `PATHEXT` variants on
/// Windows when it has no extension.
fn candidates(base: &Path) -> Vec<PathBuf> {
    let mut out = vec![base.to_path_buf()];
    if cfg!(windows) && base.extension().is_none() {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        for ext in pathext.split(';').filter(|e| !e.is_empty()) {
            let mut name = base.as_os_str().to_os_string();
            name.push(ext.to_ascii_lowercase());
            out.push(PathBuf::from(name));
        }
        // On Windows a bare extensionless file isn't runnable; try it last.
        out.rotate_left(1);
    }
    out
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Mark a file executable (a no-op where there's no such bit). For tests and
/// for plugin helpers built on first use.
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = path.metadata()?.permissions();
        perms.set_mode(perms.mode() | 0o755);
        std::fs::set_permissions(path, perms)?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tinkerpad-env-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        std::fs::write(path, "").unwrap();
        make_executable(path).unwrap();
    }

    #[test]
    fn finds_programs_on_the_given_path_not_the_process_path() {
        let a = scratch("path-a");
        let b = scratch("path-b");
        let name = if cfg!(windows) {
            "tp-tool.exe"
        } else {
            "tp-tool"
        };
        touch(&b.join(name));

        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_program("tp-tool", &path), Some(b.join(name)));
        assert_eq!(
            find_program("tp-tool", &std::env::join_paths([&a]).unwrap()),
            None
        );
        assert_eq!(find_program("", &path), None);

        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    #[test]
    fn first_directory_on_path_wins() {
        let a = scratch("first-a");
        let b = scratch("first-b");
        let name = if cfg!(windows) {
            "tp-dup.exe"
        } else {
            "tp-dup"
        };
        touch(&a.join(name));
        touch(&b.join(name));
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert_eq!(find_program("tp-dup", &path), Some(a.join(name)));
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    #[test]
    fn explicit_paths_are_checked_not_searched() {
        let dir = scratch("explicit");
        let name = if cfg!(windows) {
            "tp-abs.exe"
        } else {
            "tp-abs"
        };
        touch(&dir.join(name));
        let full = dir.join(name);
        assert_eq!(
            find_program(&full.to_string_lossy(), OsStr::new("")),
            Some(full.clone())
        );
        assert_eq!(
            find_program(&dir.join("missing").to_string_lossy(), OsStr::new("")),
            None
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn windows_tries_pathext_so_cmd_shims_resolve() {
        let dir = scratch("pathext");
        touch(&dir.join("tp-npm.cmd"));
        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(find_program("tp-npm", &path), Some(dir.join("tp-npm.cmd")));
        // An explicit extension is used as-is.
        assert_eq!(
            find_program("tp-npm.cmd", &path),
            Some(dir.join("tp-npm.cmd"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn unix_skips_files_without_the_executable_bit() {
        let dir = scratch("noexec");
        std::fs::write(dir.join("tp-plain"), "").unwrap();
        let path = std::env::join_paths([&dir]).unwrap();
        assert_eq!(find_program("tp-plain", &path), None);
        make_executable(&dir.join("tp-plain")).unwrap();
        assert_eq!(find_program("tp-plain", &path), Some(dir.join("tp-plain")));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The login-shell query really returns the shell's PATH. Runs wherever a
    /// POSIX `sh` exists; uses `sh` rather than the user's shell so the test
    /// doesn't depend on anyone's rc files.
    #[cfg(unix)]
    #[tokio::test]
    async fn login_shell_reports_a_path() {
        let path = login_shell_path(OsStr::new("/bin/sh"), Duration::from_secs(10))
            .await
            .expect("sh should report PATH");
        let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
        assert!(!dirs.is_empty(), "{path:?}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn discovered_path_on_unix_comes_from_the_shell() {
        // Whatever the inherited PATH is, discovery should produce a non-empty
        // PATH that can find `sh`.
        let path = discover_path().await;
        assert!(find_program("sh", &path).is_some(), "{path:?}");
    }

    #[tokio::test]
    async fn missing_shell_is_an_error_not_a_hang() {
        let err = login_shell_path(
            OsStr::new("tinkerpad-no-such-shell"),
            Duration::from_secs(2),
        )
        .await
        .unwrap_err();
        assert!(err.contains("not found"), "{err}");
    }
}
