//! Running playgrounds.
//!
//! Per-run execution from a plugin's `[run]` section, the pool of persistent
//! plugin helpers for warm execution, and the parser that pulls
//! `tinkerpad-dump-v1` rich values out of a program's stdout.
//!
//! Today: [`run`] executes a [`RunSpec`] and streams [`RunEvent`]s, and
//! [`OutputDecoder`] turns their bytes into UI events, with dumped values
//! found by [`DumpParser`].
//!
//! See `docs/ARCHITECTURE.md` §5 and work items 00-06, 01-06, 01-07, 03-02, 03-03.

pub mod annotate;
pub mod dump;
pub mod helper_run;
pub mod helpers;
mod output;
mod run;
pub mod spec;
mod utf8;

pub use annotate::Annotator;
pub use dump::{DumpParser, Dumped};
pub use helper_run::Waiting;
pub use helpers::{HelperConfig, HelperHandle, HelperPool, Unavailable, Use};
pub use output::OutputDecoder;
pub use run::{Cancel, Input, RunError, RunEvent, RunHandle, RunSpec, Stdin, run};
pub use spec::{Package, RunContext, SpecError, deps_command, deps_directive, run_spec};
pub use utf8::Utf8Stream;
