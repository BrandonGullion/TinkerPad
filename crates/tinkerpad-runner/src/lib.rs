//! Running playgrounds.
//!
//! Per-run execution from a plugin's `[run]` section, the pool of persistent
//! plugin helpers for warm execution, and the parser that pulls
//! `tinkerpad-dump-v1` rich values out of a program's stdout.
//!
//! See `docs/ARCHITECTURE.md` §5 and work items 00-06, 01-06, 01-07, 03-02, 03-03.
