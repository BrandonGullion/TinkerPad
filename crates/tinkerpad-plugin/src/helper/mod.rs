//! The `tinkerpad-plugin-v1` helper protocol, core side.
//!
//! A helper keeps a language's runtime warm so a run costs a message instead
//! of a process start, and gives a plugin somewhere to answer questions that
//! need the language's own tools: what a new playground should contain, how to
//! add a dependency, what to tell the language server.
//!
//! [`protocol`] is the wire format, [`codec`] turns a byte stream into
//! messages, and [`client`] is one running helper. The pool that keeps them
//! warm and restarts them is work item 03-02; the run path that turns
//! [`HelperEvent`]s into what the UI shows is 03-03.
//!
//! See `docs/PLUGINS.md` §5, which this module is the reference
//! implementation of.

pub mod client;
pub mod codec;
pub mod protocol;

pub use client::{
    CANCEL_GRACE, HOOK_TIMEOUT, HelperClient, HelperError, HelperEvent, HelperRun, HelperStart,
    READY_TIMEOUT, SHUTDOWN_GRACE,
};
pub use codec::{Decoder, Garbled, Reason};
pub use protocol::{
    Diagnostic, FromHelper, HOOKS, HelperFile, HookRequest, HookResult, PROTOCOL, RunRequest,
    ToHelper,
};
