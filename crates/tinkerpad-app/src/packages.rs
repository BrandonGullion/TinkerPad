//! Adding a package to a playground, the way its plugin's `[deps]` says
//! (docs/PLUGINS.md §2), or the way its helper does it (03-07).
//!
//! The helper comes first. A plugin that declares an `add_dependency` hook is
//! saying that adding a package is more than a command line and a line of
//! text — it may have a lock file to rewrite, or a manifest to parse — and
//! what it says goes.
//!
//! Failing that, two shapes. Either a command runs in the playground folder (`cargo add`,
//! `npm install`) and its output streams into the tab like a run, or a line is
//! inserted at the top of the main file (`#:package Foo@1.0`). The line comes
//! back to the UI rather than being written here, so the editor, the file, and
//! undo stay in step, and nothing is half-applied.

use serde::Serialize;
use tinkerpad_core::{CommandError, ErrorCode, PlaygroundId};
use tinkerpad_runner::{Package, RunContext, deps_command, deps_directive};

use crate::AppState;

/// What adding a package did.
#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum AddPackage {
    /// A command is running; its output arrives as `run` events for this id.
    Command { run: u64, command: String },
    /// The UI inserts `line` at the top of the main file. A line already
    /// starting with `prefix` plus this package's name is replaced.
    Directive { line: String, prefix: String },
    /// The plugin's helper did it, and these files of the playground changed.
    Handled { files: Vec<String> },
}

/// Add `name` (optionally at `version`) to a playground.
pub async fn add(
    state: &AppState,
    id: &PlaygroundId,
    name: &str,
    version: &str,
) -> Result<AddPackage, CommandError> {
    let name = check("A package name", name)?;
    let version = check("A version", version.trim())?;
    let playground = state
        .library
        .get(id)
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("no playground {id}")))?;
    let plugin = state
        .plugins
        .read()
        .expect("plugins lock")
        .get(playground.language.as_str())
        .cloned()
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidInput,
                format!("no plugin is loaded for language `{}`", playground.language),
            )
        })?;

    // The hook first, and only if the plugin declared one (03-07).
    let asked = (!version.is_empty()).then_some(version);
    if let Some(written) =
        crate::hooks::add_dependency(state, &plugin, &playground, name, asked).await
    {
        let files = written
            .iter()
            .map(|path| {
                path.strip_prefix(&playground.dir)
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        return Ok(AddPackage::Handled { files });
    }

    let package = Package { name, version };
    let invalid =
        |e: tinkerpad_runner::SpecError| CommandError::new(ErrorCode::InvalidInput, e.to_string());
    if let Some((line, prefix)) = deps_directive(&plugin, package).map_err(invalid)? {
        return Ok(AddPackage::Directive { line, prefix });
    }

    // A `[deps]` command needs no toolchain probe: `npm` and `cargo add` are
    // resolved through the same settings overrides and PATH a run uses.
    let resolver = state.toolchains.resolver();
    let ctx = RunContext {
        resolver: &resolver,
        scratch_root: &state.data_dir.join("scratch"),
        toolchain_program: "",
        toolchain_version: "",
    };
    let spec = deps_command(&plugin, &playground, &ctx, package)
        .map_err(invalid)?
        .expect("a [deps] section is either a directive or a command");
    let command = std::iter::once(spec.program.to_string_lossy().into_owned())
        .chain(spec.args.iter().map(|a| a.to_string_lossy().into_owned()))
        .collect::<Vec<_>>()
        .join(" ");

    let run = state.runs.start_spec(id, plugin, spec).await?;
    Ok(AddPackage::Command { run, command })
}

/// Names and versions go straight into a command line, so keep them plain:
/// no spaces, no control characters, and nothing that could read as a flag.
fn check<'a>(what: &str, value: &'a str) -> Result<&'a str, CommandError> {
    let bad = value.starts_with('-')
        || value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '\'');
    if bad {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            format!("{what} can't contain spaces, quotes, or start with a dash: `{value}`"),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_values_that_would_change_the_command() {
        for bad in ["--global", "-g", "a b", "a\"b", "a'b", "a\nb"] {
            assert!(check("A package name", bad).is_err(), "{bad}");
        }
        for good in ["zod", "Newtonsoft.Json", "serde", "@scope/pkg", "1.2.3", ""] {
            assert_eq!(check("A version", good).unwrap(), good);
        }
    }
}
