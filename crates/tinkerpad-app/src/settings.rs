//! `settings.toml`: created on first launch, read when needed.
//!
//! Only `[toolchains]` is read so far (work item 01-03). The full typed
//! settings schema, hot reload, and a settings UI arrive in 04-02.

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
