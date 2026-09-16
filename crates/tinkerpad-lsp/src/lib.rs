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
mod pool;
mod proxy;
mod server;
mod transport;
pub mod uri;

pub use framing::{Decoder, FrameError, encode, encode_into};
pub use jsonrpc::{MessageKind, ParseError, RawMessage};
pub use pool::{Attachment, LspPool, SHUTDOWN_BUDGET, ServerKey};
pub use proxy::{EditorSink, Proxy, Stats};
pub use server::{
    DocState, RequestError, START_TIMEOUT, STOP_GRACE, ServerConfig, ServerHandle, path_to_uri,
};
pub use transport::{Closed, Sender, Transport, TransportError, TransportEvent};
pub use uri::{REWRITTEN_METHODS, SCHEME, UriMap};
