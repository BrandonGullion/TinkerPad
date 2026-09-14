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
            stdin: false,
        }
    }
}

/// Something that happened during a run. Output is raw bytes; decoding to text
/// happens at the UI boundary (see [`crate::Utf8Stream`]).
#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    Started {
        pid: u32,
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
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("{program} was not found")]
    NotFound { program: String },
    #[error(transparent)]
    Spawn(#[from] SpawnError),
}

/// A run in progress. Dropping it does not stop the program; call
/// [`RunHandle::cancel`].
#[derive(Debug)]
pub struct RunHandle {
    pub pid: u32,
    events: mpsc::Receiver<RunEvent>,
    kill: KillHandle,
}

impl RunHandle {
    /// Next event. `None` after the final `Exited` or `Cancelled`.
    pub async fn recv(&mut self) -> Option<RunEvent> {
        self.events.recv().await
    }

    /// Stop the program and everything it started.
    pub fn cancel(&self) {
        self.kill.kill();
    }

    /// A cloneable way to cancel from elsewhere.
    pub fn canceller(&self) -> KillHandle {
        self.kill.clone()
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

    tokio::spawn(async move {
        if tx.send(RunEvent::Started { pid }).await.is_err() {
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
        kill,
    })
}
