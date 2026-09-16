//! Language server support.
//!
//! `Content-Length` framing (shared later with the debug adapter protocol),
//! JSON-RPC routing, the server pool that spawns one server per language and
//! keeps it alive across tabs, and the proxy that sits between the editor and
//! the server to rewrite URIs and own request ids.
//!
//! See `docs/ARCHITECTURE.md` §4 and work items 02-01 through 02-07.

pub mod framing;
pub mod jsonrpc;
mod transport;

pub use framing::{Decoder, FrameError, encode, encode_into};
pub use jsonrpc::{MessageKind, ParseError, RawMessage};
pub use transport::{Closed, Sender, Transport, TransportError, TransportEvent};
