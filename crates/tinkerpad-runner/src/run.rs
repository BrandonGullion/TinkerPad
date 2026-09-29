//! One execution of a program, start to finish.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use bytes::Bytes;
use tinkerpad_process::{KillHandle, ProcessEvent, SpawnError, SpawnSpec, spawn};
use tokio::sync::mpsc;
use tracing::debug;

/// What to run. Plain data that a caller can build, inspect, and extend before
/// running, which is how a debug launch will add its flags later
/// (`docs/DEBUGGING.md` D13).
///
/// Deliberately has no timeout: a program runs until it exits or is cancelled
/// (`docs/DEBUGGING.md` D12).
#[derive(Clone, Debug, Default)]
pub struct RunSpec {
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub env: Vec<(OsString, OsString)>,
}

impl RunSpec {
    fn into_spawn(self) -> SpawnSpec {
        SpawnSpec {
            program: self.program,
            args: self.args,
            cwd: self.cwd,
            env: self.env,
            // Every run keeps a pipe (work item 04-05). A program that reads
            // input waits for it, which is what a person typing `input()`
            // meant; without one it would read end-of-file and fall over.
            stdin: true,
        }
    }
}

/// Something that happened during a run. Output is raw bytes; decoding to text
/// happens at the UI boundary (see [`crate::Utf8Stream`]).
#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Started {
        pid: u32,
        /// A plugin helper is running it, rather than a process started for
        /// this run (work item 03-03).
        warm: bool,
        /// Whether this run can be typed at (work item 04-05). Always true for
        /// a process of its own; for a helper, only when it says it takes
        /// input.
        stdin: bool,
    },
    Stdout(Bytes),
    Stderr(Bytes),
    /// The program ended on its own.
    Exited {
        code: Option<i32>,
        duration: Duration,
    },
    /// [`RunHandle::cancel`] ended it.
    Cancelled {
        duration: Duration,
    },
    /// The program never ran: a helper reporting a compile error, with
    /// somewhere to point (03-01). The per-run path has no equivalent — a
    /// command that fails to start is a `RunError` instead.
    Failed {
        message: String,
        diagnostics: Vec<tinkerpad_plugin::helper::Diagnostic>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("{program} was not found")]
    NotFound { program: String },
    #[error(transparent)]
    Spawn(#[from] SpawnError),
}

/// How to stop a run, whichever way it is being run.
///
/// Cloneable and cheap, so a caller can keep one beside the run's slot and
/// cancel from anywhere. Both arms are non-blocking: killing a tree and asking
/// a helper to stop are both "say so and let the stream report it".
#[derive(Clone, Debug)]
pub enum Cancel {
    /// A process of this run's own: kill the tree.
    Process(KillHandle),
    /// A run on a plugin helper: the driver sends `cancel` and, if the helper
    /// ignores it, kills the helper (03-01).
    Helper(mpsc::Sender<()>),
}

impl Cancel {
    pub fn cancel(&self) {
        match self {
            Cancel::Process(kill) => kill.kill(),
            // Capacity 1, and asking twice is the same as asking once.
            Cancel::Helper(tx) => {
                let _ = tx.try_send(());
            }
        }
    }
}

/// What to send a run that is waiting to be typed at (work item 04-05).
///
/// One shape for both paths, like [`Cancel`]: the per-run side writes to the
/// child's pipe, the helper side sends `stdin` messages. Cloneable, so the
/// window can keep one beside the run.
#[derive(Clone, Debug)]
pub struct Input(mpsc::Sender<Stdin>);

/// One thing to give a program's standard input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stdin {
    /// Bytes, exactly as typed. A line the user sent ends with a newline.
    Data(Bytes),
    /// No more input is coming: the pipe closes, and a program blocked on a
    /// read gets end-of-file (`Ctrl+D` in the window).
    Eof,
}

impl Input {
    pub fn new(tx: mpsc::Sender<Stdin>) -> Self {
        Self(tx)
    }

    /// Send input. Returns whether the run could still take it.
    pub fn send(&self, data: impl Into<Bytes>) -> bool {
        self.0.try_send(Stdin::Data(data.into())).is_ok()
    }

    /// Close the program's input.
    pub fn eof(&self) -> bool {
        self.0.try_send(Stdin::Eof).is_ok()
    }
}

/// A run in progress. Dropping it does not stop the program; call
/// [`RunHandle::cancel`].
#[derive(Debug)]
pub struct RunHandle {
    /// The process doing the work: the program itself, or the helper running
    /// it.
    pub pid: u32,
    events: mpsc::Receiver<RunEvent>,
    cancel: Cancel,
    /// Where typed input goes, when this run takes any (work item 04-05).
    input: Option<Input>,
}

impl RunHandle {
    /// A handle over a stream someone else is filling, for a run that isn't a
    /// process of its own (03-03).
    pub fn over(pid: u32, events: mpsc::Receiver<RunEvent>, cancel: Cancel) -> Self {
        Self {
            pid,
            events,
            cancel,
            input: None,
        }
    }

    /// The same, for a run that can be typed at (work item 04-05).
    pub fn over_with_input(
        pid: u32,
        events: mpsc::Receiver<RunEvent>,
        cancel: Cancel,
        input: Input,
    ) -> Self {
        Self {
            pid,
            events,
            cancel,
            input: Some(input),
        }
    }

    /// Where to send what the user types, if this run takes input.
    pub fn input(&self) -> Option<Input> {
        self.input.clone()
    }

    /// Next event. `None` after the final `Exited`, `Cancelled` or `Failed`.
    pub async fn recv(&mut self) -> Option<RunEvent> {
        self.events.recv().await
    }

    /// Stop the program and everything it started.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// A cloneable way to cancel from elsewhere.
    pub fn canceller(&self) -> Cancel {
        self.cancel.clone()
    }
}

/// Start a run. Must be called from async code on a long-lived runtime thread
/// (see `tinkerpad-process` platform notes on Linux).
pub fn run(spec: RunSpec) -> Result<RunHandle, RunError> {
    let mut child = spawn(spec.into_spawn()).map_err(|e| match e {
        SpawnError::NotFound { program } => RunError::NotFound { program },
        other => RunError::Spawn(other),
    })?;
    let pid = child.pid();
    let kill = child.kill_handle();
    let (tx, rx) = mpsc::channel(256);

    // What the user types, written to the child's pipe by a task of its own so
    // a program that never reads costs nothing (work item 04-05).
    let (input_tx, mut input_rx) = mpsc::channel::<Stdin>(64);
    if let Some(mut pipe) = child.take_stdin() {
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Some(message) = input_rx.recv().await {
                match message {
                    Stdin::Data(bytes) => {
                        if pipe.write_all(&bytes).await.is_err() || pipe.flush().await.is_err() {
                            debug!(pid, "the program stopped reading its input");
                            break;
                        }
                    }
                    // Dropping the pipe is what closes it.
                    Stdin::Eof => break,
                }
            }
        });
    }

    tokio::spawn(async move {
        if tx
            .send(RunEvent::Started {
                pid,
                warm: false,
                stdin: true,
            })
            .await
            .is_err()
        {
            return;
        }
        while let Some(event) = child.recv().await {
            let event = match event {
                ProcessEvent::Stdout(b) => RunEvent::Stdout(b),
                ProcessEvent::Stderr(b) => RunEvent::Stderr(b),
                ProcessEvent::Exited(exit) if exit.killed => RunEvent::Cancelled {
                    duration: exit.duration,
                },
                ProcessEvent::Exited(exit) => RunEvent::Exited {
                    code: exit.code,
                    duration: exit.duration,
                },
            };
            if tx.send(event).await.is_err() {
                debug!(pid, "run receiver dropped");
                break;
            }
        }
    });

    Ok(RunHandle {
        pid,
        events: rx,
        cancel: Cancel::Process(kill),
        input: Some(Input::new(input_tx)),
    })
}
