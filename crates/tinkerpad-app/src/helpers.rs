//! Plugin helpers inside the app (work item 03-02).
//!
//! `tinkerpad-runner` owns the pool; this module is what the app has to
//! supply: a `HelperConfig` built from the plugin's `[helper]` section with
//! the same command resolution a `[run]` command gets (01-06), the status the
//! strip shows, and the lifecycle ends — state changes become `helper` events,
//! and quitting stops every helper alongside every language server.
//!
//! Nothing starts a helper yet: running code on one is 03-03 and hooks are
//! 03-07, and none of the bundled plugins has a `[helper]` section until then.
//! What is here is a pool that is ready when they do.

use std::path::PathBuf;
use std::sync::Arc;

use tinkerpad_core::{CommandError, ErrorCode, Scope};
use tinkerpad_plugin::{IdleShutdown, Plugin, Resolver, placeholders};
use tinkerpad_runner::helpers::{HelperConfig, HelperPool};
use tokio::sync::mpsc;

use crate::AppState;
use crate::events::EventBus;
use crate::service_settings::ServiceSettings;

/// One helper, as the status strip sees it (work item 02-08).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HelperStatus {
    /// The plugin id, which is also the language id.
    pub plugin: String,
    /// The plugin's display name — "C#", not "csharp".
    pub name: String,
    pub state: tinkerpad_core::ServerState,
    /// How long the current process has been up.
    pub uptime_ms: u64,
    /// Crashes it has come back from in a row.
    pub restarts: u32,
    /// Runs and hooks using it right now. The idle clock runs at zero.
    pub uses: usize,
    /// How long it has had nothing to do, if it has nothing to do.
    pub idle_ms: Option<u64>,
    /// The program that was started, as resolved.
    pub program: String,
    /// What to do about it when there is no program: the plugin's hint.
    pub install_hint: Option<String>,
}

/// Every helper the pool holds, running or not.
pub fn status(state: &AppState) -> Vec<HelperStatus> {
    let names: std::collections::BTreeMap<String, String> = state
        .plugins
        .read()
        .expect("plugins lock")
        .all()
        .iter()
        .map(|entry| (entry.plugin.id.to_string(), entry.plugin.name.clone()))
        .collect();

    let mut helpers: Vec<HelperStatus> = state
        .helpers
        .helpers()
        .into_iter()
        .map(|helper| HelperStatus {
            name: names
                .get(helper.plugin())
                .cloned()
                .unwrap_or_else(|| helper.plugin().to_owned()),
            plugin: helper.plugin().to_owned(),
            state: helper.state(),
            uptime_ms: helper
                .uptime()
                .map(|up| up.as_millis() as u64)
                .unwrap_or_default(),
            restarts: helper.restarts(),
            uses: helper.uses(),
            idle_ms: helper.idle_for().map(|idle| idle.as_millis() as u64),
            program: helper.config().spawn.program.to_string_lossy().into_owned(),
            install_hint: helper.config().install_hint.clone(),
        })
        .collect();
    helpers.sort_by(|a, b| a.name.cmp(&b.name));
    helpers
}

/// The last few hundred lines a helper wrote, build output included.
pub fn log(state: &AppState, plugin: &str) -> Vec<String> {
    state
        .helpers
        .get_handle(plugin)
        .map(|helper| helper.log())
        .unwrap_or_default()
}

/// Build the pool and forward its state changes to the UI.
pub fn pool(events: Arc<EventBus>) -> Arc<HelperPool> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            events.publish(Scope::App, event);
        }
    });
    Arc::new(HelperPool::new(Some(tx)))
}

/// The helper for a plugin, as the pool wants it.
///
/// The command goes through the same toolchain override and `PATH` resolution
/// a `[run]` command does, so a helper written in the language it serves finds
/// the same runtime the runs do.
pub fn config(
    plugin: &Plugin,
    resolver: &Resolver,
    toolchain: &Toolchain,
    scratch_root: &std::path::Path,
    data_dir: &std::path::Path,
    settings: &ServiceSettings,
) -> Result<HelperConfig, CommandError> {
    let helper = plugin.helper.as_ref().ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidInput,
            format!("{} has no helper ([helper] section)", plugin.name),
        )
    })?;

    // The plugin's scratch folder: the same `{scratch}` a run command gets,
    // which is where a build puts its output and what `hello` calls the
    // helper's `scratch_dir`.
    let scratch = tinkerpad_runner::spec::scratch_dir(scratch_root, plugin);
    if let Err(error) = std::fs::create_dir_all(&scratch) {
        tracing::warn!(dir = %scratch.display(), %error, "could not make the helper's folder");
    }

    let plugin_dir = plugin.dir.to_string_lossy().into_owned();
    let scratch_text = scratch.to_string_lossy().into_owned();
    // A helper serves every playground of its language, so the placeholders
    // that name one aren't available here.
    let values: [(&str, &str); 4] = [
        ("plugin_dir", &plugin_dir),
        ("scratch", &scratch_text),
        ("toolchain", &toolchain.program),
        ("version", &toolchain.version),
    ];
    let fill = |field: &str, template: &str| {
        placeholders::substitute(template, &values).map_err(|e| {
            CommandError::new(ErrorCode::InvalidInput, format!("[helper] {field}: {e}"))
        })
    };

    let spawn = command(
        plugin,
        resolver,
        &helper.command,
        &fill,
        "helper",
        &plugin.dir,
    )?;

    let mut config = HelperConfig::new(plugin.id.as_str(), spawn, &plugin.dir, scratch)
        .with_stamp_dir(data_dir.join("helpers"))
        .with_hooks(helper.hooks.clone());
    config.install_hint = plugin.install_hint.clone();

    // Idle: the plugin says how long, and the user may override it per plugin
    // under `[helpers.<id>]`.
    let from_plugin = match helper.idle_shutdown {
        IdleShutdown::Never => None,
        IdleShutdown::After(idle) => Some(idle),
    };
    config.policy.idle_shutdown = settings.idle_for(plugin.id.as_str(), from_plugin);

    // `[helper.build]`, if the plugin ships its helper as source. The folder
    // it watches is the plugin's own: a change anywhere in it is a reason to
    // build again, and the build's own output folders are skipped.
    if let Some(build) = &helper.build {
        let spec = command(plugin, resolver, build, &fill, "helper.build", &plugin.dir)?;
        config = config.with_build(spec, &plugin.dir);
    }
    Ok(config)
}

/// One command from the manifest, resolved and filled in.
fn command(
    plugin: &Plugin,
    resolver: &Resolver,
    spec: &tinkerpad_plugin::CommandSpec,
    fill: &dyn Fn(&str, &str) -> Result<String, CommandError>,
    what: &str,
    default_cwd: &std::path::Path,
) -> Result<tinkerpad_process::SpawnSpec, CommandError> {
    let program = resolver
        .resolve(plugin.id.as_str(), &fill(what, &spec.command)?)
        .map_err(|reason| {
            CommandError::new(
                ErrorCode::NotFound,
                match &plugin.install_hint {
                    Some(hint) => format!("{}'s helper: {reason}.\n{hint}", plugin.name),
                    None => format!("{}'s helper: {reason}.", plugin.name),
                },
            )
        })?;

    let mut spawn = tinkerpad_process::SpawnSpec::new(program);
    for (i, arg) in spec.args.iter().enumerate() {
        spawn = spawn.arg(fill(&format!("{what}.args[{i}]"), arg)?);
    }
    spawn = spawn.env("PATH", resolver.path());
    for (key, value) in &spec.env {
        spawn = spawn.env(key, fill(&format!("{what}.env.{key}"), value)?);
    }
    spawn = match &spec.cwd {
        Some(cwd) => spawn.cwd(fill(&format!("{what}.cwd"), cwd)?),
        // A helper with no `cwd` runs in its plugin's folder, which is where
        // its own files are.
        None => spawn.cwd(PathBuf::from(default_cwd)),
    };
    Ok(spawn)
}

/// What a plugin's toolchain probe found, for `{toolchain}` and `{version}`.
/// Empty when it hasn't been checked, or isn't there: a helper that names its
/// toolchain then fails to resolve, with the plugin's install hint.
#[derive(Debug, Default)]
pub struct Toolchain {
    pub program: String,
    pub version: String,
}

impl Toolchain {
    fn of(state: &AppState, language: &str) -> Self {
        match state.toolchains.state(language) {
            Some(tinkerpad_core::ToolchainState::Ready {
                program, version, ..
            }) => Self {
                program,
                version: version.unwrap_or_default(),
            },
            _ => Self::default(),
        }
    }
}

/// The config for one plugin by id, for the commands the status strip calls.
pub fn config_for(state: &AppState, plugin_id: &str) -> Result<HelperConfig, CommandError> {
    let plugin = state
        .plugins
        .read()
        .expect("plugins lock")
        .get(plugin_id)
        .cloned()
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidInput,
                format!("no plugin is loaded for `{plugin_id}`"),
            )
        })?;
    config(
        &plugin,
        &state.toolchains.resolver(),
        &Toolchain::of(state, plugin_id),
        &state.scratch_root(),
        &state.data_dir,
        &state.helper_settings,
    )
}
