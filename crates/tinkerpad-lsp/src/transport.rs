//! A language server on the other end of a pipe.
//!
//! One task reads the server's stdout, unframes it, and publishes messages;
//! one task drains a queue into its stdin; the server's stderr goes to the log
//! line by line under the server's span. Both tasks end when the process does
//! (`docs/ARCHITECTURE.md` §4.3).
//!
//! Nothing here interprets a message. Routing responses to the requests that
//! asked for them is the proxy's job (02-03), and keeping the process alive
//! across tabs is the pool's (02-02).

use std::sync::Arc;

use tinkerpad_process::{Exit, KillHandle, ProcessEvent, SpawnError, SpawnSpec, Supervised};
use tokio::io::AsyncWriteExt;
use tokio::process::ChildStdin;
use tokio::sync::mpsc;
use tracing::{Instrument, debug, info, info_span, warn};

use crate::framing::{self, Decoder, FrameError};
use crate::jsonrpc::{ParseError, RawMessage};

/// How many messages may be queued in either direction before a sender waits.
/// A server that stops reading its stdin, or an editor that stops reading its
/// messages, should slow the other side down rather than grow a buffer.
const QUEUE: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("{0}")]
    Frame(#[from] FrameError),
    #[error("{0}")]
    Parse(#[from] ParseError),
}

/// Something that arrived from the server.
#[derive(Debug)]
pub enum TransportEvent {
    Message(RawMessage),
    /// Bytes that were not a message. The reader carries on: a server that
    /// writes something unframed (a panic, a stray line) shouldn't take the
    /// session with it. A [`TransportError::Frame`] at the very end means the
    /// stream stopped mid-message.
    Broken(TransportError),
    /// The server is gone. Always the last event.
    Exited(Exit),
}

/// Sends messages to the server. Cloneable, so the proxy can hand one out per
/// document without sharing the transport itself.
#[derive(Clone, Debug)]
pub struct Sender {
    name: Arc<str>,
    tx: mpsc::Sender<RawMessage>,
}

/// The server is gone, so the message was not sent.
#[derive(Clone, Debug, thiserror::Error)]
#[error("{name} is not running")]
pub struct Closed {
    pub name: String,
}

impl Sender {
    /// Queue a message. Waits if the queue is full; fails once the server is
    /// gone.
    pub async fn send(&self, message: RawMessage) -> Result<(), Closed> {
        self.tx.send(message).await.map_err(|_| Closed {
            name: self.name.to_string(),
        })
    }

    /// Whether the writer is still there.
    pub fn is_open(&self) -> bool {
        !self.tx.is_closed()
    }
}

/// A running language server and the two streams that talk to it.
#[derive(Debug)]
pub struct Transport {
    name: Arc<str>,
    pid: u32,
    kill: KillHandle,
    sender: Sender,
    incoming: mpsc::Receiver<TransportEvent>,
}

impl Transport {
    /// Spawn a server and start talking to it. `name` is what the log calls it
    /// (the plugin's language id, usually).
    ///
    /// The spec is spawned with a stdin pipe whatever it says, since a server
    /// with no stdin can't be spoken to.
    pub fn spawn(name: &str, mut spec: SpawnSpec) -> Result<Self, SpawnError> {
        spec.stdin = true;
        let mut child = tinkerpad_process::spawn(spec)?;
        let stdin = child.take_stdin();
        Ok(Self::over(name, child, stdin))
    }

    /// Talk to a child someone else spawned. The pool uses this after building
    /// the spec its own way.
    pub fn over(name: &str, child: Supervised, stdin: Option<ChildStdin>) -> Self {
        let name: Arc<str> = Arc::from(name);
        let pid = child.pid();
        let kill = child.kill_handle();
        let span = info_span!("lsp", server = %name, pid);

        let (out_tx, out_rx) = mpsc::channel(QUEUE);
        let (in_tx, in_rx) = mpsc::channel(QUEUE);
        tokio::spawn(read(child, in_tx).instrument(span.clone()));
        tokio::spawn(write(stdin, out_rx).instrument(span));

        Self {
            name: name.clone(),
            pid,
            kill,
            sender: Sender { name, tx: out_tx },
            incoming: in_rx,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// A handle that sends without borrowing the transport.
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }

    /// Queue a message for the server.
    pub async fn send(&self, message: RawMessage) -> Result<(), Closed> {
        self.sender.send(message).await
    }

    /// The stream of everything the server says. `None` once it has exited and
    /// its output is drained.
    pub async fn recv(&mut self) -> Option<TransportEvent> {
        self.incoming.recv().await
    }

    /// The same stream, for a caller that wants to `select!` on it.
    pub fn incoming(&mut self) -> &mut mpsc::Receiver<TransportEvent> {
        &mut self.incoming
    }

    /// End the server and its children. The `Exited` event follows.
    pub fn kill(&self) {
        self.kill.kill();
    }

    pub fn kill_handle(&self) -> KillHandle {
        self.kill.clone()
    }
}

/// Unframe stdout into messages, and log stderr.
async fn read(mut child: Supervised, out: mpsc::Sender<TransportEvent>) {
    let mut decoder = Decoder::new();
    let mut stderr = Lines::default();
    let mut messages = 0u64;

    while let Some(event) = child.recv().await {
        match event {
            ProcessEvent::Stdout(bytes) => {
                decoder.push(&bytes);
                if !drain(&mut decoder, &out, &mut messages).await {
                    // Nobody is listening any more; stop reading, but leave the
                    // child to whoever holds its kill handle.
                    return;
                }
            }
            ProcessEvent::Stderr(bytes) => stderr.push(&bytes),
            ProcessEvent::Exited(exit) => {
                stderr.flush();
                decoder.end();
                drain(&mut decoder, &out, &mut messages).await;
                info!(
                    code = ?exit.code,
                    killed = exit.killed,
                    messages,
                    "language server exited"
                );
                let _ = out.send(TransportEvent::Exited(exit)).await;
                return;
            }
        }
    }
}

/// Publish every message the decoder can produce. `false` once the receiver is
/// gone.
async fn drain(
    decoder: &mut Decoder,
    out: &mpsc::Sender<TransportEvent>,
    messages: &mut u64,
) -> bool {
    while let Some(framed) = decoder.take() {
        let event = match framed {
            Ok(body) => match RawMessage::from_slice(&body) {
                Ok(message) => {
                    *messages += 1;
                    debug!(%message, "from server");
                    TransportEvent::Message(message)
                }
                Err(e) => {
                    warn!(error = %e, "server sent a body that is not a message");
                    TransportEvent::Broken(e.into())
                }
            },
            Err(e) => {
                warn!(error = %e, "server output could not be framed");
                TransportEvent::Broken(e.into())
            }
        };
        if out.send(event).await.is_err() {
            return false;
        }
    }
    true
}

/// Frame queued messages into the server's stdin.
async fn write(stdin: Option<ChildStdin>, mut queue: mpsc::Receiver<RawMessage>) {
    let Some(mut stdin) = stdin else {
        warn!("no stdin pipe, so nothing can be sent to this server");
        return;
    };
    let mut buf = Vec::with_capacity(8 * 1024);
    while let Some(message) = queue.recv().await {
        buf.clear();
        framing::encode_into(&mut buf, &message.to_vec());
        debug!(%message, "to server");
        if let Err(e) = stdin.write_all(&buf).await {
            warn!(error = %e, "could not write to the server; it has probably gone");
            return;
        }
        if let Err(e) = stdin.flush().await {
            warn!(error = %e, "could not flush to the server");
            return;
        }
    }
    // The transport was dropped: closing stdin is how an LSP server is told to
    // stop when `shutdown`/`exit` didn't do it.
    debug!("closing the server's stdin");
}

/// Buffers stderr into whole lines for the log. A chunk may hold several lines
/// or half of one, and the last line may have no newline at all.
#[derive(Default)]
struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    /// Log every line `bytes` completes.
    fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if byte == b'\n' {
                self.emit();
            } else {
                self.partial.push(byte);
            }
        }
    }

    /// Log whatever is left, once no more bytes are coming.
    fn flush(&mut self) {
        if !self.partial.is_empty() {
            self.emit();
        }
    }

    fn emit(&mut self) {
        if let Some(line) = Self::clean(std::mem::take(&mut self.partial)) {
            info!(target: "tinkerpad_lsp::server", "{line}");
        }
    }

    /// A line worth logging: valid text, no line ending, not just space.
    fn clean(bytes: Vec<u8>) -> Option<String> {
        let line = String::from_utf8_lossy(&bytes);
        let line = line.trim_end_matches('\r');
        (!line.trim().is_empty()).then(|| line.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same splitting `Lines` does, with the lines collected instead of
    /// logged.
    fn lines(chunks: &[&[u8]]) -> Vec<String> {
        let mut out = Vec::new();
        let mut partial = Vec::new();
        for chunk in chunks {
            for &byte in *chunk {
                if byte == b'\n' {
                    out.extend(Lines::clean(std::mem::take(&mut partial)));
                } else {
                    partial.push(byte);
                }
            }
        }
        out.extend(Lines::clean(partial));
        out
    }

    #[test]
    fn stderr_is_logged_line_by_line() {
        assert_eq!(lines(&[b"one\ntwo\n"]), ["one", "two"]);
        // A line split across chunks is one line.
        assert_eq!(lines(&[b"half", b" a line\n"]), ["half a line"]);
        // Windows line endings, and a last line with no newline at all.
        assert_eq!(lines(&[b"crlf\r\nlast"]), ["crlf", "last"]);
        // Blank lines are not worth a log record.
        assert_eq!(lines(&[b"\n \n\r\n"]), Vec::<String>::new());
        // Bytes that are not text still produce a line rather than a panic.
        assert_eq!(lines(&[&[0xff, b'a', b'\n']]), ["\u{fffd}a"]);
    }
}
