//! The playground library on disk.
//!
//! One folder per playground under a user-chosen root (default `~/TinkerPad/`),
//! each with a `playground.toml` and a main source file. This crate creates,
//! opens, renames, deletes, and lists them, keeps an in-memory index, and
//! watches the root for changes made outside the app.
//!
//! It knows nothing about plugins: callers pass the main file path and any
//! files to write when creating a playground.
//!
//! See `docs/ARCHITECTURE.md` §6 and work item 01-04.

mod error;
mod fsutil;
mod library;
mod meta;
mod platform;
mod slug;

pub use error::LibraryError;
pub use library::{
    Claimed, DEBOUNCE, Library, LibraryEvent, Listing, NewPlayground, Scaffold, Watcher,
};
pub use meta::FILE_NAME as METADATA_FILE;
pub use slug::slugify;
