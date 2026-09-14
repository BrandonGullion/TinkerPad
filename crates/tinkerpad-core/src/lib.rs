//! Domain types, events, and errors shared by every TinkerPad crate.
//!
//! This crate has no IO and no async runtime. It defines *what* the
//! application talks about (playgrounds, runs, servers, rich values) so that
//! every other crate, the UI protocol, and the CLI agree on the vocabulary.
//!
//! See `docs/ARCHITECTURE.md` §3.

pub mod error;
pub mod events;

pub use error::{CommandError, ErrorCode};
pub use events::{CoreEvent, Envelope, Level, RunEvent, Scope};
