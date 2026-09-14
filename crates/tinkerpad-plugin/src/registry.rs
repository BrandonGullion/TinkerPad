//! Finding and loading every plugin the app can use.
//!
//! Plugins come from two folders, scanned in order: the plugins bundled with
//! the app, then the user's plugin folder. Both use the same format. A user
//! plugin with the same id as a bundled one replaces it. A folder that fails to
//! load is recorded and skipped; it never stops the others.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tinkerpad_core::LanguageId;
use tracing::{debug, info, warn};

use crate::os::Os;
use crate::plugin::{MANIFEST_FILE, Plugin};

/// Where a plugin was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// Shipped inside the app.
    Bundled,
    /// Installed in the user's plugin folder.
    User,
}

/// A plugin folder that couldn't be used.
#[derive(Clone, Debug)]
pub struct LoadFailure {
    pub dir: PathBuf,
    pub source: Source,
    /// The rendered error, ready to show a person.
    pub message: String,
}

/// A user plugin that replaced a bundled one with the same id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Override {
    pub id: LanguageId,
    pub bundled_dir: PathBuf,
    pub user_dir: PathBuf,
}

/// A loaded plugin and where it came from.
#[derive(Clone, Debug)]
pub struct Entry {
    pub plugin: Arc<Plugin>,
    pub source: Source,
}

/// Every usable plugin, read-only after loading.
#[derive(Debug)]
pub struct PluginRegistry {
    bundled_dir: Option<PathBuf>,
    user_dir: PathBuf,
    os: Os,
    /// Sorted by id.
    entries: Vec<Entry>,
    failures: Vec<LoadFailure>,
    overrides: Vec<Override>,
    load_time: Duration,
}

impl PluginRegistry {
    /// Scan `bundled_dir` (if any) then `user_dir`, loading plugins for the
    /// current OS. `user_dir` is created if it doesn't exist.
    pub fn load(bundled_dir: Option<PathBuf>, user_dir: PathBuf) -> Self {
        Self::load_for(bundled_dir, user_dir, Os::current())
    }

    /// As [`PluginRegistry::load`], resolving plugins for `os`.
    pub fn load_for(bundled_dir: Option<PathBuf>, user_dir: PathBuf, os: Os) -> Self {
        let mut registry = Self {
            bundled_dir,
            user_dir,
            os,
            entries: Vec::new(),
            failures: Vec::new(),
            overrides: Vec::new(),
            load_time: Duration::ZERO,
        };
        registry.reload();
        registry
    }

    /// Scan both folders again from scratch. Meant for the plugins panel and
    /// CLI; the app doesn't watch for changes.
    pub fn reload(&mut self) {
        let started = Instant::now();
        self.entries.clear();
        self.failures.clear();
        self.overrides.clear();

        if let Err(error) = std::fs::create_dir_all(&self.user_dir) {
            warn!(dir = %self.user_dir.display(), %error, "could not create user plugin folder");
        }
        info!(
            bundled = %self.bundled_dir.as_deref().map_or("<none>".into(), |d| d.display().to_string()),
            user = %self.user_dir.display(),
            "loading plugins"
        );

        if let Some(dir) = self.bundled_dir.clone() {
            self.scan(&dir, Source::Bundled);
        }
        let user_dir = self.user_dir.clone();
        self.scan(&user_dir, Source::User);

        self.entries.sort_by(|a, b| a.plugin.id.cmp(&b.plugin.id));
        self.load_time = started.elapsed();
        info!(
            loaded = self.entries.len(),
            failed = self.failures.len(),
            ms = format_args!("{:.2}", self.load_time.as_secs_f64() * 1000.0),
            "plugins loaded"
        );
    }

    fn scan(&mut self, root: &Path, source: Source) {
        let mut dirs: Vec<PathBuf> = match std::fs::read_dir(root) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(error) => {
                // A missing bundled folder is worth a warning; the user folder
                // was just created, so failing to read it is too.
                warn!(dir = %root.display(), %error, "could not read plugin folder");
                return;
            }
        };
        dirs.sort();

        // Ids loaded from this root, to catch two folders claiming one id.
        let mut seen_here: Vec<(LanguageId, PathBuf)> = Vec::new();

        for dir in dirs {
            if !dir.join(MANIFEST_FILE).is_file() {
                debug!(dir = %dir.display(), "no plugin.toml; not a plugin");
                continue;
            }
            let plugin = match Plugin::load_for(&dir, self.os) {
                Ok(plugin) => plugin,
                Err(error) => {
                    let message = error.to_string();
                    warn!(dir = %dir.display(), ?source, "plugin failed to load: {message}");
                    self.failures.push(LoadFailure {
                        dir,
                        source,
                        message,
                    });
                    continue;
                }
            };

            if let Some((_, first)) = seen_here.iter().find(|(id, _)| *id == plugin.id) {
                let message = format!(
                    "plugin id `{}` is already used by {}; rename one of them",
                    plugin.id,
                    first.display()
                );
                warn!(dir = %dir.display(), ?source, "{message}");
                self.failures.push(LoadFailure {
                    dir,
                    source,
                    message,
                });
                continue;
            }
            seen_here.push((plugin.id.clone(), dir.clone()));

            if let Some(existing) = self.entries.iter_mut().find(|e| e.plugin.id == plugin.id) {
                // Only possible for a user plugin shadowing a bundled one.
                info!(
                    id = %plugin.id,
                    user = %dir.display(),
                    bundled = %existing.plugin.dir.display(),
                    "user plugin overrides bundled plugin"
                );
                self.overrides.push(Override {
                    id: plugin.id.clone(),
                    bundled_dir: existing.plugin.dir.clone(),
                    user_dir: dir,
                });
                *existing = Entry {
                    plugin: Arc::new(plugin),
                    source,
                };
                continue;
            }

            debug!(id = %plugin.id, dir = %dir.display(), ?source, "plugin loaded");
            self.entries.push(Entry {
                plugin: Arc::new(plugin),
                source,
            });
        }
    }

    /// Every loaded plugin, sorted by id.
    pub fn all(&self) -> &[Entry] {
        &self.entries
    }

    /// The plugin with this id.
    pub fn get(&self, id: &str) -> Option<&Arc<Plugin>> {
        self.entries
            .iter()
            .find(|e| e.plugin.id == id)
            .map(|e| &e.plugin)
    }

    /// The plugin for a file extension, with or without the leading dot,
    /// ignoring case. If several plugins claim an extension, the one with the
    /// lowest id wins.
    pub fn by_extension(&self, ext: &str) -> Option<&Arc<Plugin>> {
        let ext = ext.trim_start_matches('.');
        self.entries
            .iter()
            .find(|e| e.plugin.extension.eq_ignore_ascii_case(ext))
            .map(|e| &e.plugin)
    }

    /// Plugin folders that were skipped, in scan order.
    pub fn failures(&self) -> &[LoadFailure] {
        &self.failures
    }

    /// User plugins that replaced bundled ones.
    pub fn overrides(&self) -> &[Override] {
        &self.overrides
    }

    pub fn bundled_dir(&self) -> Option<&Path> {
        self.bundled_dir.as_deref()
    }

    pub fn user_dir(&self) -> &Path {
        &self.user_dir
    }

    /// How long the last load or reload took.
    pub fn load_time(&self) -> Duration {
        self.load_time
    }
}
