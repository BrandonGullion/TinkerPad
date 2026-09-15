//! Starting, streaming, and cancelling playground runs for the UI.
//!
//! A run saves the playground, builds the command from its plugin's `[run]`
//! section (`tinkerpad_runner::run_spec`), and runs the file on disk. Each
//! playground has at most one run at a time; different playgrounds can run at
//! once.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::async_runtime::JoinHandle;
use tinkerpad_core::{
    CommandError, CoreEvent, ErrorCode, PlaygroundId, RunEvent as UiRunEvent, Scope, ToolchainState,
};
use tinkerpad_library::Library;
use tinkerpad_plugin::Plugin;
use tinkerpad_process::KillHandle;
use tinkerpad_runner::spec::scratch_dir;
use tinkerpad_runner::{RunContext, RunEvent, SpecError, Utf8Stream, run, run_spec};
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::events::EventBus;
use crate::toolchains::Toolchains;

/// How long a run waits for a toolchain check that's still in progress.
const TOOLCHAIN_WAIT: Duration = Duration::from_secs(10);

struct Active {
    id: u64,
    kill: KillHandle,
    /// Finishes after the run's final event has been published.
    forwarder: JoinHandle<()>,
}

/// One playground's run slot. Held for the whole start sequence so two quick
/// runs of the same playground can't overlap.
type Slot = Arc<Mutex<Option<Active>>>;

pub struct Runs {
    events: Arc<EventBus>,
    scratch_root: PathBuf,
    toolchains: Arc<Toolchains>,
    next_id: AtomicU64,
    slots: std::sync::Mutex<HashMap<PlaygroundId, Slot>>,
}

impl Runs {
    pub fn new(events: Arc<EventBus>, scratch_root: PathBuf, toolchains: Arc<Toolchains>) -> Self {
        Self {
            events,
            scratch_root,
            toolchains,
            next_id: AtomicU64::new(1),
            slots: Default::default(),
        }
    }

    fn slot(&self, playground: &PlaygroundId) -> Slot {
        self.slots
            .lock()
            .expect("slots lock")
            .entry(playground.clone())
            .or_default()
            .clone()
    }

    /// Save `text` as the playground's main file, then run it. Returns the run
    /// id; output and the outcome arrive as `run` events scoped to it. A run
    /// already going for this playground is cancelled first, and its final
    /// event is published before this run starts.
    pub async fn start(
        &self,
        library: &Library,
        plugin: Arc<Plugin>,
        playground_id: &PlaygroundId,
        text: String,
    ) -> Result<u64, CommandError> {
        let slot = self.slot(playground_id);
        let mut active = slot.lock().await;
        if let Some(prev) = active.take() {
            info!(run = prev.id, playground = %playground_id, "replacing previous run");
            prev.kill.kill();
            let _ = prev.forwarder.await;
        }

        // Always save first (01-12 adds dirty tracking). The runner reads the
        // file, never the text.
        library
            .write_source(playground_id, &text)
            .map_err(crate::library::to_command_error)?;
        let playground = library.get(playground_id).ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                format!("no playground {playground_id}"),
            )
        })?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let scope = Scope::Run(id);
        let fail = |message: String| {
            warn!(run = id, playground = %playground_id, %message, "run failed to start");
            self.events.publish(
                scope.clone(),
                CoreEvent::Run(UiRunEvent::Failed { message }),
            );
            Ok(id)
        };

        let (program, version) = match self.wait_for_toolchain(&plugin).await {
            ToolchainState::Ready {
                program, version, ..
            } => (program, version.unwrap_or_default()),
            ToolchainState::Missing { reason, hint } => {
                return fail(with_hint(
                    format!("{} isn't ready: {reason}.", plugin.name),
                    hint.as_deref(),
                ));
            }
            ToolchainState::Checking => {
                return fail(format!(
                    "{}'s toolchain check hasn't finished. Try again in a moment.",
                    plugin.name
                ));
            }
        };

        let resolver = self.toolchains.resolver();
        let ctx = RunContext {
            resolver: &resolver,
            scratch_root: &self.scratch_root,
            toolchain_program: &program,
            toolchain_version: &version,
        };
        let spec = match run_spec(&plugin, &playground, &ctx) {
            Ok(spec) => spec,
            Err(SpecError::NotFound {
                message,
                install_hint,
            }) => return fail(with_hint(message, install_hint.as_deref())),
            Err(other) => return fail(other.to_string()),
        };

        let scratch = scratch_dir(&self.scratch_root, &plugin);
        if let Err(e) = tokio::fs::create_dir_all(&scratch).await {
            return fail(format!("Couldn't create {}: {e}", scratch.display()));
        }

        let mut handle = match run(spec) {
            Ok(handle) => handle,
            Err(error) => return fail(error.to_string()),
        };

        info!(run = id, pid = handle.pid, playground = %playground_id, plugin = %plugin.id, "run started");
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

    /// Cancel a playground's run, if it has one. Returns immediately; a
    /// `cancelled` event follows.
    pub async fn cancel(&self, playground: &PlaygroundId) {
        let slot = self
            .slots
            .lock()
            .expect("slots lock")
            .get(playground)
            .cloned();
        if let Some(slot) = slot
            && let Some(active) = slot.lock().await.as_ref()
        {
            info!(run = active.id, %playground, "cancel requested");
            active.kill.kill();
        }
    }

    /// The plugin's toolchain state, waiting briefly if the startup check is
    /// still running.
    async fn wait_for_toolchain(&self, plugin: &Plugin) -> ToolchainState {
        let deadline = tokio::time::Instant::now() + TOOLCHAIN_WAIT;
        loop {
            match self.toolchains.state(plugin.id.as_str()) {
                Some(ToolchainState::Checking) | None if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Some(state) => return state,
                None => return ToolchainState::Checking,
            }
        }
    }
}

fn with_hint(message: String, hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!("{message}\n{hint}"),
        None => message,
    }
}
