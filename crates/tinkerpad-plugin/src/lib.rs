//! Language plugins: `plugin.toml` schema and loader, discovery from the
//! bundled and user plugin folders, toolchain probing, and the client side of
//! the `tinkerpad-plugin-v1` helper protocol.
//!
//! A plugin is a folder. Nothing language-specific lives in Rust; this crate
//! only knows how to read what a plugin folder declares.
//!
//! Load one with [`Plugin::load`]. Only the validated [`Plugin`] and its parts
//! leave this crate; the raw TOML shape stays private.
//!
//! See `docs/PLUGINS.md` and work items 01-01, 01-02, 01-03, 03-01.

pub mod env;
mod error;
pub mod helper;
mod manifest;
mod os;
pub mod placeholders;
mod plugin;
mod registry;
pub mod scaffold;
mod suggest;
pub mod toolchain;

pub use error::{Location, ManifestError, Problem, ProblemKind, Syntax, Unknown};
pub use helper::{
    HelperClient, HelperError, HelperEvent, HelperRun, HelperStart, HookRequest, HookResult,
    RunRequest,
};
pub use os::{Arch, Os};
pub use plugin::{
    Acquire, AcquireFile, CommandSpec, DepsSpec, HelperSpec, IdleShutdown, Kind, LspSpec,
    MANIFEST_FILE, OutputSpec, Plugin, Probe, Register, SUPPORTED_API, ScaffoldFile, TEMPLATES_DIR,
    Version, Workspace,
};
pub use registry::{Entry, LoadFailure, Override, PluginRegistry, Source};
pub use toolchain::{AcquiredTools, PROBE_TIMEOUT, Resolver, ToolchainOverrides, probe};

/// The annotated C# manifest from `docs/PLUGINS.md` §2, kept identical to the
/// docs by a test. The plugin template repository (work item 05-03) starts
/// from it.
pub fn example_manifest() -> &'static str {
    include_str!("example_manifest.toml")
}
