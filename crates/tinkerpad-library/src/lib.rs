//! The playground library on disk.
//!
//! One folder per playground under a user-chosen root (default `~/TinkerPad/`),
//! each with a `playground.toml` and a main source file. This crate creates,
//! opens, renames, deletes, and lists them, keeps an in-memory index, and
//! watches the root for changes made outside the app.
//!
//! See `docs/ARCHITECTURE.md` §6 and work item 01-04.
