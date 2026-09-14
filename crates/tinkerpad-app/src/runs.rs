//! Starting, streaming, and cancelling runs on behalf of the UI.
//!
//! Work item 00-06: a single hard-coded runtime (`node`) and a single active
//! run. Plugins (01-06) replace the hard-coded spec; tabs (01-12) allow one
//! run per playground.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use tauri::async_runtime::JoinHandle;
use tinkerpad_core::{CommandError, CoreEvent, ErrorCode, RunEvent as UiRunEvent, Scope};
use tinkerpad_plugin::env::find_program;
use tinkerpad_process::KillHandle;
use tinkerpad_runner::{RunError, RunEvent, RunSpec, Utf8Stream, run};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::events::EventBus;
use crate::toolchains::Toolchains;

struct Active {
    id: u64,
    kill: KillHandle,
    /// Finishes after the run's final event has been published.
    forwarder: JoinHandle<()>,
}

pub struct Runs {
    events: Arc<EventBus>,
    scratch: PathBuf,
    /// For finding programs on the discovered PATH (the login shell's on macOS
    /// and Linux), not just the PATH the app was launched with.
    toolchains: Arc<Toolchains>,
    next_id: AtomicU64,
    /// Held for the whole start sequence so two quick Runs can't interleave.
    active: Mutex<Option<Active>>,
}

impl Runs {
    pub fn new(events: Arc<EventBus>, scratch: PathBuf, toolchains: Arc<Toolchains>) -> Self {
        Self {
            events,
            scratch,
            toolchains,
            next_id: AtomicU64::new(1),
            active: Mutex::new(None),
        }
    }

    /// Run `code` with the hard-coded runtime. Cancels any active run first and
    /// waits for its final event, so output from two runs never interleaves.
    pub async fn start(&self, code: String) -> Result<u64, CommandError> {
        let mut active = self.active.lock().await;
        if let Some(prev) = active.take() {
            info!(run = prev.id, "replacing previous run");
            prev.kill.kill();
            let _ = prev.forwarder.await;
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let scope = Scope::Run(id);

        tokio::fs::create_dir_all(&self.scratch)
            .await
            .map_err(|e| {
                CommandError::new(
                    ErrorCode::Io,
                    format!("could not create scratch folder: {e}"),
                )
            })?;
        let file = self.scratch.join("scratch.js");
        tokio::fs::write(&file, code).await.map_err(|e| {
            CommandError::new(
                ErrorCode::Io,
                format!("could not write {}: {e}", file.display()),
            )
        })?;

        // Still hard-coded to node until plugins drive runs (01-06), but found
        // on the discovered PATH, which children also inherit.
        let resolver = self.toolchains.resolver();
        let program = find_program("node", resolver.path())
            .map(PathBuf::into_os_string)
            .unwrap_or_else(|| "node".into());
        let spec = RunSpec {
            program,
            args: vec![file.into_os_string()],
            cwd: Some(self.scratch.clone()),
            env: vec![("PATH".into(), resolver.path().to_os_string())],
        };

        let mut handle = match run(spec) {
            Ok(handle) => handle,
            Err(error) => {
                let message = match &error {
                    RunError::NotFound { .. } => {
                        "Node.js was not found on PATH. Install it from https://nodejs.org and restart TinkerPad.".to_owned()
                    }
                    other => other.to_string(),
                };
                warn!(run = id, %error, "run failed to start");
                self.events
                    .publish(scope, CoreEvent::Run(UiRunEvent::Failed { message }));
                return Ok(id);
            }
        };

        info!(run = id, pid = handle.pid, "run started");
        let kill = handle.canceller();
        let events = self.events.clone();
        let forwarder = tauri::async_runtime::spawn(async move {
            let mut stdout = Utf8Stream::new();
            let mut stderr = Utf8Stream::new();
            while let Some(event) = handle.recv().await {
                let ui = match event {
                    RunEvent::Started { .. } => UiRunEvent::Started,
                    RunEvent::Stdout(b) => UiRunEvent::Stdout {
                        text: stdout.push(&b),
                    },
                    RunEvent::Stderr(b) => UiRunEvent::Stderr {
                        text: stderr.push(&b),
                    },
                    end @ (RunEvent::Exited { .. } | RunEvent::Cancelled { .. }) => {
                        for (text, is_err) in [(stdout.finish(), false), (stderr.finish(), true)] {
                            if !text.is_empty() {
                                let e = if is_err {
                                    UiRunEvent::Stderr { text }
                                } else {
                                    UiRunEvent::Stdout { text }
                                };
                                events.publish(scope.clone(), CoreEvent::Run(e));
                            }
                        }
                        match end {
                            RunEvent::Exited { code, duration } => UiRunEvent::Exited {
                                code,
                                duration_ms: duration.as_millis() as u64,
                            },
                            RunEvent::Cancelled { duration } => UiRunEvent::Cancelled {
                                duration_ms: duration.as_millis() as u64,
                            },
                            _ => unreachable!(),
                        }
                    }
                };
                let skip = matches!(&ui, UiRunEvent::Stdout { text } | UiRunEvent::Stderr { text } if text.is_empty());
                if !skip {
                    events.publish(scope.clone(), CoreEvent::Run(ui));
                }
            }
            info!(run = id, "run finished");
        });

        *active = Some(Active {
            id,
            kill,
            forwarder,
        });
        Ok(id)
    }

    /// Cancel the active run, if any. Returns immediately; a `cancelled` event
    /// follows.
    pub async fn cancel(&self) {
        if let Some(active) = self.active.lock().await.as_ref() {
            info!(run = active.id, "cancel requested");
            active.kill.kill();
        }
    }
}
