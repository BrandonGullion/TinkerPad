//! Checking every plugin's toolchain in the background.
//!
//! At startup (and on "recheck") the app discovers `PATH`, reads overrides from
//! `settings.toml`, and probes all plugins concurrently. Each result is stored
//! and published as a `toolchain` event. The UI never waits: it can ask for
//! the current states at any time with `toolchain_status`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Instant;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tinkerpad_core::{CoreEvent, LanguageId, Scope, ToolchainState};
use tinkerpad_plugin::{PROBE_TIMEOUT, Plugin, Resolver, env::discover_path, probe};
use tracing::info;

use crate::AppState;

pub struct Toolchains {
    resolver: RwLock<Arc<Resolver>>,
    states: RwLock<BTreeMap<LanguageId, ToolchainState>>,
    /// Bumped by each check, so a slow older check can't overwrite newer results.
    generation: AtomicU64,
}

#[derive(Debug, Serialize)]
pub struct LanguageToolchain {
    pub language: LanguageId,
    pub state: ToolchainState,
}

impl Default for Toolchains {
    /// Before the first check finishes, lookups use the inherited `PATH`.
    fn default() -> Self {
        let path = std::env::var_os("PATH").unwrap_or_default();
        Self {
            resolver: RwLock::new(Arc::new(Resolver::new(path, Default::default()))),
            states: RwLock::default(),
            generation: AtomicU64::new(0),
        }
    }
}

impl Toolchains {
    /// The resolver from the latest check. Before the first check finishes it
    /// uses the inherited `PATH` and no overrides.
    pub fn resolver(&self) -> Arc<Resolver> {
        self.resolver.read().expect("resolver lock").clone()
    }

    /// The latest state for one language, if it has been checked at all.
    pub fn state(&self, language: &str) -> Option<ToolchainState> {
        self.states
            .read()
            .expect("toolchain states lock")
            .get(language)
            .cloned()
    }

    pub fn snapshot(&self) -> Vec<LanguageToolchain> {
        self.states
            .read()
            .expect("toolchain states lock")
            .iter()
            .map(|(language, state)| LanguageToolchain {
                language: language.clone(),
                state: state.clone(),
            })
            .collect()
    }
}

/// Start a check of every loaded plugin's toolchain. Returns immediately.
pub fn check_all(app: &AppHandle) {
    let state = app.state::<AppState>();
    let generation = state.toolchains.generation.fetch_add(1, Ordering::SeqCst) + 1;

    let plugins: Vec<Arc<Plugin>> = state
        .plugins
        .read()
        .expect("plugins lock")
        .all()
        .iter()
        .map(|e| e.plugin.clone())
        .collect();

    // Everything starts as Checking, replacing results for removed plugins.
    {
        let mut states = state.toolchains.states.write().expect("states lock");
        states.clear();
        for p in &plugins {
            states.insert(p.id.clone(), ToolchainState::Checking);
        }
    }
    for p in &plugins {
        state.events.publish(
            Scope::App,
            CoreEvent::Toolchain {
                language: p.id.clone(),
                state: ToolchainState::Checking,
            },
        );
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let started = Instant::now();
        let path = discover_path().await;
        let state = app.state::<AppState>();

        let overrides = tinkerpad_plugin::ToolchainOverrides::of(&state.settings.get());
        // Anything TinkerPad has acquired sits between the overrides and PATH,
        // so a downloaded tool is found by probes, runs, servers and helpers
        // alike without the user editing their environment (work item 04-08a).
        let acquired = crate::tools::search_paths(&state.data_dir);
        let resolver = Arc::new(Resolver::new(path, overrides).with_acquired(acquired));
        *state.toolchains.resolver.write().expect("resolver lock") = resolver.clone();

        let mut tasks = tokio::task::JoinSet::new();
        for plugin in plugins {
            let resolver = resolver.clone();
            let scratch: PathBuf = state.data_dir.join("scratch").join(plugin.id.as_str());
            tasks.spawn(async move {
                let result = probe(&plugin, &resolver, &scratch, PROBE_TIMEOUT).await;
                (plugin.id.clone(), result)
            });
        }

        let mut count = 0;
        while let Some(joined) = tasks.join_next().await {
            let Ok((language, result)) = joined else {
                continue;
            };
            count += 1;
            if state.toolchains.generation.load(Ordering::SeqCst) != generation {
                return; // a newer check is running; its results win
            }
            state
                .toolchains
                .states
                .write()
                .expect("states lock")
                .insert(language.clone(), result.clone());
            state.events.publish(
                Scope::App,
                CoreEvent::Toolchain {
                    language,
                    state: result,
                },
            );
        }
        info!(
            plugins = count,
            ms = started.elapsed().as_millis() as u64,
            "toolchain check finished"
        );
    });
}
