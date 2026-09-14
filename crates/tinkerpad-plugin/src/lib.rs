//! Language plugins: `plugin.toml` schema and loader, discovery from the
//! bundled and user plugin folders, toolchain probing, and the client side of
//! the `tinkerpad-plugin-v1` helper protocol.
//!
//! A plugin is a folder. Nothing language-specific lives in Rust; this crate
//! only knows how to read what a plugin folder declares.
//!
//! See `docs/PLUGINS.md` and work items 01-01, 01-02, 01-03, 03-01.

pub mod env;
