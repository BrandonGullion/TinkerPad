//! Child process supervision.
//!
//! The one place in TinkerPad that spawns child processes. Per-run commands,
//! language servers, plugin helpers, and (later) debug adapters all go through
//! here. It provides spawning with streaming stdout/stderr, killing a whole
//! process tree, and the guarantee that children die with the app.
//!
//! Platform-specific behaviour lives only in [`platform`]; nothing outside that
//! module uses `cfg(target_os)`.
//!
//! See `docs/ARCHITECTURE.md` §1, §5.1, §10 and work item 00-04.

pub mod platform;
