//! Where the app finds plugins, and what it tells the UI about them.

use std::path::PathBuf;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tinkerpad_core::Level;
use tinkerpad_plugin::{PluginRegistry, Source};
use tracing::{info, warn};

/// Overrides the user plugin folder. Meant for tests and portable setups.
pub const USER_PLUGINS_ENV: &str = "TINKERPAD_USER_PLUGINS";

/// The plugins bundled with the app.
///
/// Installed and `tauri build` apps find them in the resource directory, where
/// `bundle.resources` in `tauri.conf.json` copies `plugins/`. A debug build
/// run straight from Cargo may not have that copy, so debug builds fall back
/// to the repository's `plugins/` folder. This is the only place that knows
/// the difference.
pub fn bundled_plugins_dir(app: &AppHandle) -> Option<PathBuf> {
    let from_resources = app
        .path()
        .resource_dir()
        .ok()
        .map(|dir| without_verbatim_prefix(dir.join("plugins")))
        .filter(|dir| dir.is_dir());
    if let Some(dir) = from_resources {
        return Some(dir);
    }

    if cfg!(debug_assertions) {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
        if repo.is_dir() {
            let repo = repo.canonicalize().unwrap_or(repo);
            info!(dir = %repo.display(), "using the repository's plugins folder (debug build)");
            return Some(repo);
        }
    }

    warn!("bundled plugins folder not found; only user plugins will load");
    None
}

/// The user's plugin folder: `<app-data>/plugins`, or `TINKERPAD_USER_PLUGINS`.
pub fn user_plugins_dir(app: &AppHandle) -> tauri::Result<PathBuf> {
    if let Some(dir) = std::env::var_os(USER_PLUGINS_ENV).filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    Ok(app.path().app_data_dir()?.join("plugins"))
}

/// Load the registry at startup, returning notices for the UI (one per plugin
/// that failed to load).
pub fn load(app: &AppHandle) -> tauri::Result<(PluginRegistry, Vec<(Level, String)>)> {
    let registry = PluginRegistry::load(bundled_plugins_dir(app), user_plugins_dir(app)?);
    let notices = registry
        .failures()
        .iter()
        .map(|f| {
            let folder = f.dir.file_name().map_or_else(
                || f.dir.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            );
            (
                Level::Error,
                format!(
                    "Plugin \"{folder}\" failed to load and was skipped.\n{}",
                    f.message
                ),
            )
        })
        .collect();
    Ok((registry, notices))
}

/// What the UI needs to know about the plugin setup.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginsStatus {
    pub bundled_dir: Option<String>,
    pub user_dir: String,
    pub load_ms: f64,
    pub plugins: Vec<PluginSummary>,
    pub failures: Vec<FailureSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSummary {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub source: Source,
    pub dir: String,
    pub kinds: Vec<KindSummary>,
    /// The CodeMirror language the UI highlights with (docs/PLUGINS.md §9).
    pub highlighter: Option<String>,
    /// Whether the plugin declares a language server.
    pub lsp: bool,
    /// Whether it declares a warm helper (work item 03-02).
    pub helper: bool,
    /// The programs its `[toolchain]` probes look for, which are the ones the
    /// settings window offers a path box for (work item 04-02).
    pub programs: Vec<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KindSummary {
    pub id: String,
    pub name: String,
    pub default: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailureSummary {
    pub dir: String,
    pub source: Source,
    pub message: String,
}

impl PluginsStatus {
    pub fn from_registry(r: &PluginRegistry) -> Self {
        Self {
            bundled_dir: r.bundled_dir().map(|d| d.display().to_string()),
            user_dir: r.user_dir().display().to_string(),
            load_ms: r.load_time().as_secs_f64() * 1000.0,
            plugins: r
                .all()
                .iter()
                .map(|e| PluginSummary {
                    id: e.plugin.id.to_string(),
                    name: e.plugin.name.clone(),
                    extension: e.plugin.extension.clone(),
                    source: e.source,
                    dir: e.plugin.dir.display().to_string(),
                    kinds: e
                        .plugin
                        .kinds
                        .iter()
                        .map(|k| KindSummary {
                            id: k.id.clone(),
                            name: k.name.clone(),
                            default: k.default,
                        })
                        .collect(),
                    highlighter: e.plugin.highlighter.clone(),
                    lsp: e.plugin.lsp.is_some(),
                    helper: e.plugin.helper.is_some(),
                    programs: {
                        let mut programs: Vec<String> = e
                            .plugin
                            .probes
                            .iter()
                            .map(|probe| probe.command.clone())
                            .collect();
                        programs.dedup();
                        programs
                    },
                })
                .collect(),
            failures: r
                .failures()
                .iter()
                .map(|f| FailureSummary {
                    dir: f.dir.display().to_string(),
                    source: f.source,
                    message: f.message.clone(),
                })
                .collect(),
        }
    }
}

/// Drop the `\\?\` verbatim prefix Windows sometimes puts on resolved paths
/// (`\\?\C:\...`), so paths read normally in logs, errors, and the UI. Paths
/// without the prefix, and UNC verbatim paths, are returned unchanged.
fn without_verbatim_prefix(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    match text.strip_prefix(r"\\?\") {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_drive_verbatim_prefix_only() {
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(r"\\?\C:\Apps\TinkerPad\plugins")),
            PathBuf::from(r"C:\Apps\TinkerPad\plugins")
        );
        assert_eq!(
            without_verbatim_prefix(PathBuf::from(r"\\?\UNC\server\share")),
            PathBuf::from(r"\\?\UNC\server\share")
        );
        assert_eq!(
            without_verbatim_prefix(PathBuf::from("/usr/lib/tinkerpad/plugins")),
            PathBuf::from("/usr/lib/tinkerpad/plugins")
        );
    }
}
