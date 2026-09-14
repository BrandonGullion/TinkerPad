//! Spawning and supervising a single child process.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::{Bytes, BytesMut};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use crate::platform;

/// How long to wait after a polite termination before killing hard.
const GRACE: Duration = Duration::from_secs(2);

/// How long to wait for stdout/stderr to drain after the child exits before
/// giving up (a grandchild may be holding the pipes open).
const DRAIN: Duration = Duration::from_secs(1);

/// Read chunk size for stdout/stderr.
const CHUNK: usize = 8 * 1024;

/// Environment applied to every child before the caller's own additions.
/// UTF-8 output everywhere; English tool messages so error regexes match.
const DEFAULT_ENV: &[(&str, &str)] = &[
    ("PYTHONIOENCODING", "utf-8"),
    ("PYTHONUTF8", "1"),
    ("DOTNET_CLI_UI_LANGUAGE", "en"),
    ("TERM", "xterm-256color"),
];

/// What to spawn. Plain data on purpose: a caller may clone and extend it
/// (extra args, extra env) before spawning, which is how a debug launch will
/// be built later (see `docs/DEBUGGING.md` D3, D13).
#[derive(Clone, Debug, Default)]
pub struct SpawnSpec {
    /// Program name or path. Resolved through `PATH` by the OS; callers that
    /// need overrides resolve to a full path first.
    pub program: OsString,
    /// Arguments, passed as an array. There is never a shell in between.
    pub args: Vec<OsString>,
    /// Working directory. Inherited when `None`.
    pub cwd: Option<PathBuf>,
    /// Extra environment, applied over [`DEFAULT_ENV`].
    pub env: Vec<(OsString, OsString)>,
    /// Keep a stdin pipe open. When `false`, stdin is `/dev/null`.
    pub stdin: bool,
}

impl SpawnSpec {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            ..Self::default()
        }
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub fn stdin(mut self, keep_open: bool) -> Self {
        self.stdin = keep_open;
        self
    }
}

/// How a child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Exit {
    /// Exit code, if the process exited normally. `None` when killed by a
    /// signal (Unix) or when the status could not be read.
    pub code: Option<i32>,
    /// `true` when [`Supervised::kill`] (or a [`KillHandle`]) ended it.
    pub killed: bool,
    /// Wall time from spawn to exit.
    pub duration: Duration,
}

/// Something the child did.
#[derive(Clone, Debug)]
pub enum ProcessEvent {
    /// A chunk of stdout, as it arrived. Not line-buffered; may split a UTF-8
    /// sequence across chunks.
    Stdout(Bytes),
    /// A chunk of stderr, same rules.
    Stderr(Bytes),
    /// The child is gone and both pipes are drained (or abandoned). Always the
    /// last event.
    Exited(Exit),
}

#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("program not found: {program}")]
    NotFound { program: String },
    #[error("failed to spawn {program}: {source}")]
    Io {
        program: String,
        #[source]
        source: std::io::Error,
    },
}

/// A cloneable handle that can kill a child from anywhere.
#[derive(Clone, Debug)]
pub struct KillHandle {
    notify: Arc<Notify>,
}

impl KillHandle {
    /// Request termination of the child and its whole process tree. Returns
    /// immediately; the [`ProcessEvent::Exited`] event follows.
    pub fn kill(&self) {
        self.notify.notify_one();
    }
}

/// A running (or finished) child process and its event stream.
pub struct Supervised {
    pid: u32,
    events: mpsc::Receiver<ProcessEvent>,
    stdin: Option<ChildStdin>,
    kill: KillHandle,
}

impl std::fmt::Debug for Supervised {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Supervised")
            .field("pid", &self.pid)
            .field("stdin", &self.stdin.is_some())
            .finish_non_exhaustive()
    }
}

impl Supervised {
    /// OS process id of the direct child.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// Next event. `None` after [`ProcessEvent::Exited`] has been delivered.
    pub async fn recv(&mut self) -> Option<ProcessEvent> {
        self.events.recv().await
    }

    /// The child's stdin, if the spec asked for one. Can be taken once.
    pub fn take_stdin(&mut self) -> Option<ChildStdin> {
        self.stdin.take()
    }

    /// Request termination. See [`KillHandle::kill`].
    pub fn kill(&self) {
        self.kill.kill();
    }

    /// A handle that can kill this child from another task.
    pub fn kill_handle(&self) -> KillHandle {
        self.kill.clone()
    }
}

/// Spawn a child under supervision.
///
/// The child is hidden (no console window on Windows), placed in its own
/// process group or job object so [`Supervised::kill`] takes its descendants
/// with it, and registered so it dies with the app.
pub fn spawn(spec: SpawnSpec) -> Result<Supervised, SpawnError> {
    let program = spec.program.to_string_lossy().into_owned();

    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    for (k, v) in DEFAULT_ENV {
        cmd.env(k, v);
    }
    for (k, v) in &spec.env {
        cmd.env(k, v);
    }
    cmd.stdin(if spec.stdin {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);
    platform::prepare(&mut cmd);

    let mut child = cmd.spawn().map_err(|source| {
        if source.kind() == std::io::ErrorKind::NotFound {
            SpawnError::NotFound {
                program: program.clone(),
            }
        } else {
            SpawnError::Io {
                program: program.clone(),
                source,
            }
        }
    })?;

    let pid = child.id().unwrap_or(0);
    let guard = platform::register_child(pid, &program);
    debug!(pid, program, "spawned");

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let stdin = child.stdin.take();

    let (tx, rx) = mpsc::channel(256);
    let started = Instant::now();
    let out = tokio::spawn(pump(stdout, tx.clone(), ProcessEvent::Stdout));
    let err = tokio::spawn(pump(stderr, tx.clone(), ProcessEvent::Stderr));

    let notify = Arc::new(Notify::new());
    let kill = KillHandle { notify };
    tokio::spawn(supervise(
        child,
        pid,
        program,
        guard,
        kill.clone(),
        out,
        err,
        tx,
        started,
    ));

    Ok(Supervised {
        pid,
        events: rx,
        stdin,
        kill,
    })
}

async fn pump<R>(mut reader: R, tx: mpsc::Sender<ProcessEvent>, wrap: fn(Bytes) -> ProcessEvent)
where
    R: AsyncRead + Unpin,
{
    let mut buf = BytesMut::with_capacity(CHUNK);
    loop {
        buf.reserve(CHUNK);
        match reader.read_buf(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let chunk = buf.split().freeze();
                if tx.send(wrap(chunk)).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    mut child: Child,
    pid: u32,
    program: String,
    guard: platform::Guard,
    kill: KillHandle,
    out: JoinHandle<()>,
    err: JoinHandle<()>,
    tx: mpsc::Sender<ProcessEvent>,
    started: Instant,
) {
    let mut killed = false;
    let status = tokio::select! {
        status = child.wait() => status,
        _ = kill.notify.notified() => {
            killed = true;
            debug!(pid, program, "terminating");
            platform::terminate(&guard, pid);
            match tokio::time::timeout(GRACE, child.wait()).await {
                Ok(status) => status,
                Err(_) => {
                    warn!(pid, program, "did not exit after SIGTERM/terminate; killing hard");
                    platform::terminate_hard(&guard, pid);
                    let _ = child.kill().await;
                    child.wait().await
                }
            }
        }
    };

    // Drain the pipes, but don't wait forever: a grandchild may still hold
    // them open after the direct child is gone.
    if tokio::time::timeout(DRAIN, async {
        let _ = out.await;
        let _ = err.await;
    })
    .await
    .is_err()
    {
        warn!(
            pid,
            program, "stdout/stderr still open after exit; abandoning pipes"
        );
    }

    let code = match status {
        Ok(status) => status.code(),
        Err(error) => {
            warn!(pid, program, %error, "could not read exit status");
            None
        }
    };
    let exit = Exit {
        code,
        killed,
        duration: started.elapsed(),
    };
    debug!(pid, program, ?exit, "exited");
    let _ = tx.send(ProcessEvent::Exited(exit)).await;
    drop(guard);
}
