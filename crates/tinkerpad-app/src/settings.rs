//! `settings.toml`: created on first launch, read when needed.
//!
//! Only `[toolchains]` (01-03), `[library]` (01-04), and `[lsp]` (02-06) are read
//! so far. The full typed settings schema, hot reload, and a settings UI arrive
//! in 04-02.

use std::path::{Path, PathBuf};

use tinkerpad_plugin::ToolchainOverrides;
use tracing::{info, warn};

pub const FILE_NAME: &str = "settings.toml";

const TEMPLATE: &str = r#"# TinkerPad settings.
#
# Program overrides per plugin. When set, TinkerPad uses this path instead of
# searching PATH, both when checking the toolchain and when running code.
# Use "Recheck toolchains" (or restart) after editing.
#
# [toolchains.csharp]
# dotnet = "C:/Program Files/dotnet/dotnet.exe"
#
# [toolchains.python]
# python3 = "/opt/homebrew/bin/python3"
#
# Where playgrounds are stored. Default: a TinkerPad folder in your home folder.
# Restart after changing it.
#
# [library]
# root = "D:/Code/TinkerPad"
#
# Language servers. Pinned languages start in the background once the window is
# up, so the first tab of the day is as quick as the hundredth. A server stops
# on its own once it has had no open files for a while; the plugin decides how
# long, and you can override it per language with "never" or a duration.
#
# [lsp]
# prewarm = ["typescript"]
#
# [lsp.rust]
# idle_shutdown = "30m"
"#;

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Create `settings.toml` with a commented template if it doesn't exist.
pub fn ensure_exists(data_dir: &Path) {
    let file = path(data_dir);
    if file.exists() {
        return;
    }
    match std::fs::write(&file, TEMPLATE) {
        Ok(()) => info!(file = %file.display(), "created settings file"),
        Err(error) => warn!(file = %file.display(), %error, "could not create settings file"),
    }
}

/// Toolchain overrides from `settings.toml`. A missing file means none; a
/// broken file is an error the caller should show.
pub fn toolchain_overrides(data_dir: &Path) -> Result<ToolchainOverrides, String> {
    let file = path(data_dir);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
        Err(e) => return Err(format!("could not read {}: {e}", file.display())),
    };
    ToolchainOverrides::from_settings(&text).map_err(|e| format!("{}: {e}", file.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_parses_to_no_overrides() {
        assert!(
            ToolchainOverrides::from_settings(TEMPLATE)
                .unwrap()
                .is_empty()
        );
    }
}

/// `[library].root` from `settings.toml`, if set.
pub fn library_root(data_dir: &Path) -> Result<Option<PathBuf>, String> {
    let file = path(data_dir);
    let text = match std::fs::read_to_string(&file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {e}", file.display())),
    };
    let table: toml::Table =
        toml::from_str(&text).map_err(|e| format!("{}: {}", file.display(), e.message()))?;
    match table.get("library").and_then(|l| l.get("root")) {
        None => Ok(None),
        Some(toml::Value::String(root)) if !root.trim().is_empty() => Ok(Some(PathBuf::from(root))),
        Some(_) => Err(format!(
            "{}: [library] root should be a folder path in quotes",
            file.display()
        )),
    }
}

#[cfg(test)]
mod library_root_tests {
    use super::*;

    #[test]
    fn reads_library_root() {
        let dir = std::env::temp_dir().join(format!("tinkerpad-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(library_root(&dir).unwrap(), None, "no file");

        std::fs::write(path(&dir), TEMPLATE).unwrap();
        assert_eq!(
            library_root(&dir).unwrap(),
            None,
            "template has it commented out"
        );

        std::fs::write(path(&dir), "[library]\nroot = \"/data/pads\"\n").unwrap();
        assert_eq!(
            library_root(&dir).unwrap(),
            Some(PathBuf::from("/data/pads"))
        );

        std::fs::write(path(&dir), "[library]\nroot = 3\n").unwrap();
        assert!(library_root(&dir).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
