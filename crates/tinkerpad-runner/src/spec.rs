//! Turning a plugin's `[run]` section and a playground into a [`RunSpec`].
//!
//! This is one pure function on purpose (docs/DEBUGGING.md D13): the caller
//! gets a plain `RunSpec` back before anything is spawned, so a debug launch
//! can add its own arguments and environment to the same spec later.
//!
//! Today a run is a single command. `[run]` may later split into `[build]`
//! then `[run]` (DEBUGGING.md D14), where the run step launches an artifact the
//! build produced; nothing here should assume there is only ever one step.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tinkerpad_core::Playground;
use tinkerpad_plugin::placeholders::{self, PlaceholderError};
use tinkerpad_plugin::{Plugin, Resolver};

use crate::RunSpec;

/// What a run needs beyond the plugin and the playground.
#[derive(Clone, Debug)]
pub struct RunContext<'a> {
    pub resolver: &'a Resolver,
    /// `<app-data>/scratch`; the plugin's scratch folder is `<this>/<plugin-id>`.
    pub scratch_root: &'a Path,
    /// The program of the toolchain probe that succeeded (`{toolchain}`).
    pub toolchain_program: &'a str,
    /// The version that probe reported (`{version}`), or empty if unknown.
    pub toolchain_version: &'a str,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    #[error("{plugin} has no [run] command for this platform")]
    NoRunCommand { plugin: String },
    #[error("{message}")]
    NotFound {
        message: String,
        install_hint: Option<String>,
    },
    #[error("[run] {field}: {error}")]
    Placeholder {
        field: String,
        error: PlaceholderError,
    },
}

/// The plugin's scratch folder for playgrounds of its language.
pub fn scratch_dir(scratch_root: &Path, plugin: &Plugin) -> PathBuf {
    scratch_root.join(plugin.id.as_str())
}

/// Build the command that runs `playground` with `plugin`'s `[run]` section.
///
/// Placeholders are filled in every string. The program is resolved through
/// the resolver (settings override, then `PATH`), and the child's `PATH` is
/// set to the discovered one so its own lookups match; `[run].env` is applied
/// on top. The working directory defaults to the playground folder. There is
/// no timeout (DEBUGGING.md D12).
pub fn run_spec(
    plugin: &Plugin,
    playground: &Playground,
    ctx: &RunContext<'_>,
) -> Result<RunSpec, SpecError> {
    let run = plugin.run.as_ref().ok_or_else(|| SpecError::NoRunCommand {
        plugin: plugin.name.clone(),
    })?;

    let main = playground.main_file.to_string_lossy();
    let dir = playground.dir.to_string_lossy();
    let scratch = scratch_dir(ctx.scratch_root, plugin);
    let scratch = scratch.to_string_lossy();
    let plugin_dir = plugin.dir.to_string_lossy();
    let values: [(&str, &str); 8] = [
        ("main", &main),
        ("dir", &dir),
        ("id", playground.id.as_str()),
        ("name", &playground.name),
        ("scratch", &scratch),
        ("plugin_dir", &plugin_dir),
        ("version", ctx.toolchain_version),
        ("toolchain", ctx.toolchain_program),
    ];
    let fill = |field: &str, template: &str| {
        placeholders::substitute(template, &values).map_err(|error| SpecError::Placeholder {
            field: field.to_owned(),
            error,
        })
    };

    let command = fill("command", &run.command)?;
    let program = ctx
        .resolver
        .resolve(plugin.id.as_str(), &command)
        .map_err(|reason| SpecError::NotFound {
            message: format!("Can't run {}: {reason}.", plugin.name),
            install_hint: plugin.install_hint.clone(),
        })?;

    let args = run
        .args
        .iter()
        .enumerate()
        .map(|(i, a)| fill(&format!("args[{i}]"), a).map(OsString::from))
        .collect::<Result<Vec<_>, _>>()?;

    let cwd = match &run.cwd {
        Some(cwd) => PathBuf::from(fill("cwd", cwd)?),
        None => playground.dir.clone(),
    };

    let mut env: Vec<(OsString, OsString)> =
        vec![("PATH".into(), ctx.resolver.path().to_os_string())];
    for (key, value) in &run.env {
        env.push((key.into(), fill(&format!("env.{key}"), value)?.into()));
    }

    Ok(RunSpec {
        program: program.into_os_string(),
        args,
        cwd: Some(cwd),
        env,
    })
}
