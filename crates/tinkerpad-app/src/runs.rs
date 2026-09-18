//! Starting, streaming, and cancelling playground runs for the UI.
//!
//! A run saves the playground, builds the command from its plugin's `[run]`
//! section (`tinkerpad_runner::run_spec`), and runs the file on disk. Each
//! playground has at most one run at a time; different playgrounds can run at
//! once.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use tauri::async_runtime::JoinHandle;
use tinkerpad_core::{
    CommandError, CoreEvent, ErrorCode, Level, Playground, PlaygroundId, RunEvent as UiRunEvent,
    Scope, ToolchainState,
};
use tinkerpad_library::Library;
use tinkerpad_plugin::Plugin;
use tinkerpad_plugin::helper::RunRequest;
use tinkerpad_runner::helper_run::{self, Waiting};
use tinkerpad_runner::spec::scratch_dir;
use tinkerpad_runner::{
    Cancel, HelperConfig, HelperPool, OutputDecoder, RunContext, RunEvent, RunHandle, SpecError,
    run, run_spec,
};
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, info, warn};

use crate::events::EventBus;
use crate::helpers::Toolchain;
use crate::service_settings::ServiceSettings;
use crate::toolchains::Toolchains;

/// How long a run waits for a toolchain check that's still in progress.
const TOOLCHAIN_WAIT: Duration = Duration::from_secs(10);

struct Active {
    id: u64,
    cancel: Cancel,
    /// Finishes after the run's final event has been published.
    forwarder: JoinHandle<()>,
}

/// One playground's run slot. Held for the whole start sequence so two quick
/// runs of the same playground can't overlap.
type Slot = Arc<Mutex<Option<Active>>>;

pub struct Runs {
    events: Arc<EventBus>,
    scratch_root: PathBuf,
    /// `<app-data>`, for the helpers' build stamps.
    data_dir: PathBuf,
    toolchains: Arc<Toolchains>,
    /// The warm runners (03-02). A plugin with a helper that lists `run` gets
    /// its runs sent there instead of a new process each time.
    helpers: Arc<HelperPool>,
    /// `[helpers]` from settings.toml.
    helper_settings: ServiceSettings,
    /// The "Force per-run" command: the user has said to ignore helpers for
    /// the rest of this session, usually to see what a helper is hiding.
    force_per_run: AtomicBool,
    /// Plugins whose helper has already been reported unavailable. Once per
    /// session is enough; the fallback works, and saying so every run is nagging.
    told: Arc<std::sync::Mutex<HashSet<String>>>,
    next_id: AtomicU64,
    slots: std::sync::Mutex<HashMap<PlaygroundId, Slot>>,
}

impl Runs {
    pub fn new(
        events: Arc<EventBus>,
        data_dir: PathBuf,
        toolchains: Arc<Toolchains>,
        helpers: Arc<HelperPool>,
        helper_settings: ServiceSettings,
    ) -> Self {
        Self {
            events,
            scratch_root: data_dir.join("scratch"),
            data_dir,
            toolchains,
            helpers,
            helper_settings,
            force_per_run: AtomicBool::new(false),
            told: Default::default(),
            next_id: AtomicU64::new(1),
            slots: Default::default(),
        }
    }

    /// Whether runs are being kept off helpers for this session.
    pub fn forced_per_run(&self) -> bool {
        self.force_per_run.load(Ordering::Relaxed)
    }

    /// Turn the helper path off (or back on) for the rest of the session.
    pub fn force_per_run(&self, forced: bool) {
        self.force_per_run.store(forced, Ordering::Relaxed);
        info!(forced, "per-run execution forced");
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
            prev.cancel.cancel();
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

        // The warm path: a plugin whose helper runs playgrounds gets them, with
        // the command above kept as the fallback for when it can't (03-03).
        //
        // Nothing here waits for the helper. A build, or another playground's
        // turn, can take a while, and the tab should say what it is waiting for
        // rather than the window sitting on an unanswered command — so the run
        // is registered now and the waiting happens inside it.
        if self.helper_runs(&plugin) {
            let toolchain = Toolchain {
                program: program.clone(),
                version: version.clone(),
            };
            match crate::helpers::config(
                &plugin,
                &resolver,
                &toolchain,
                &self.scratch_root,
                &self.data_dir,
                &self.helper_settings,
            ) {
                Ok(config) => {
                    return Ok(self.start_on_helper(
                        &mut active,
                        id,
                        playground_id,
                        &plugin,
                        &playground,
                        config,
                        spec,
                        program,
                    ));
                }
                Err(e) => self.fell_back(&plugin, &e.message, &program),
            }
        }

        match self.spawn(&mut active, id, playground_id, &plugin, spec) {
            Ok(id) => Ok(id),
            Err(message) => fail(message),
        }
    }

    /// Run `spec` as this playground's current run, replacing any run it
    /// already has, and stream its output as `run` events. Used for a run and
    /// for `[deps]` commands such as `cargo add`.
    pub async fn start_spec(
        &self,
        playground_id: &PlaygroundId,
        plugin: Arc<Plugin>,
        spec: tinkerpad_runner::RunSpec,
    ) -> Result<u64, CommandError> {
        let slot = self.slot(playground_id);
        let mut active = slot.lock().await;
        if let Some(prev) = active.take() {
            prev.cancel.cancel();
            let _ = prev.forwarder.await;
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        match self.spawn(&mut active, id, playground_id, &plugin, spec) {
            Ok(id) => Ok(id),
            Err(message) => {
                warn!(run = id, playground = %playground_id, %message, "command failed to start");
                self.events.publish(
                    Scope::Run(id),
                    CoreEvent::Run(UiRunEvent::Failed { message }),
                );
                Ok(id)
            }
        }
    }

    /// Whether this plugin's runs go to a helper: it has one, the helper says
    /// it runs playgrounds, and the session hasn't been told to use per-run.
    fn helper_runs(&self, plugin: &Plugin) -> bool {
        !self.forced_per_run()
            && plugin
                .helper
                .as_ref()
                .is_some_and(|helper| helper.hooks.iter().any(|hook| hook == "run"))
    }

    /// Run this playground on the plugin's helper, falling back to `spec` if
    /// there turns out to be no helper to run it on.
    ///
    /// Returns as soon as the run exists, which is before it is going: getting
    /// a helper can mean waiting for a build, or for another playground's turn.
    /// Those waits are reported as `waiting` events and the build's output as
    /// the run's own, so the tab says what is happening while it happens.
    #[allow(clippy::too_many_arguments)]
    fn start_on_helper(
        &self,
        active: &mut Option<Active>,
        id: u64,
        playground_id: &PlaygroundId,
        plugin: &Arc<Plugin>,
        playground: &Playground,
        config: HelperConfig,
        fallback: tinkerpad_runner::RunSpec,
        program: String,
    ) -> u64 {
        let scope = Scope::Run(id);
        let request = RunRequest {
            main: playground.main_file.clone(),
            dir: playground.dir.clone(),
            kind: playground.kind.clone(),
        };

        // What the run is waiting for, while it waits: the build's own output,
        // and another playground's turn on the helper.
        let waiting = {
            let events = self.events.clone();
            let scope = scope.clone();
            let name = plugin.name.clone();
            move |what: Waiting| {
                let event = match what {
                    Waiting::Building(line) => UiRunEvent::Stdout {
                        text: format!("{line}\n"),
                    },
                    Waiting::Busy => UiRunEvent::Waiting {
                        message: format!("Waiting for the {name} helper"),
                    },
                };
                events.publish(scope.clone(), CoreEvent::Run(event));
            }
        };

        let (cancel_tx, mut cancel_rx) = mpsc::channel::<()>(1);
        let helpers = self.helpers.clone();
        let events = self.events.clone();
        let told = self.told.clone();
        let plugin = plugin.clone();
        let playground_id = playground_id.clone();
        let output_spec = plugin.output.clone();

        let forwarder = tauri::async_runtime::spawn(async move {
            let starting = helper_run::start(&helpers, &config, request, waiting);
            tokio::pin!(starting);
            // A cancel while the run is still waiting for the helper ends it
            // here: there is nothing running to stop.
            let started = tokio::select! {
                _ = cancel_rx.recv() => {
                    info!(run = id, playground = %playground_id, "cancelled before it started");
                    events.publish(
                        scope.clone(),
                        CoreEvent::Run(UiRunEvent::Cancelled { duration_ms: 0 }),
                    );
                    return;
                }
                started = &mut starting => started,
            };

            let handle = match started {
                Ok(handle) => {
                    info!(run = id, playground = %playground_id, plugin = %plugin.id, "warm run started");
                    handle
                }
                // No helper to run it on: say so once, and run it the old way.
                Err(unavailable) => {
                    fell_back(&events, &told, &plugin, &unavailable.to_string(), &program);
                    match run(fallback) {
                        Ok(handle) => handle,
                        Err(error) => {
                            events.publish(
                                scope.clone(),
                                CoreEvent::Run(UiRunEvent::Failed {
                                    message: error.to_string(),
                                }),
                            );
                            return;
                        }
                    }
                }
            };

            forward(handle, Some(cancel_rx), events, scope, id, output_spec).await;
        });

        *active = Some(Active {
            id,
            cancel: Cancel::Helper(cancel_tx),
            forwarder,
        });
        id
    }

    /// Say once per session that a plugin's helper isn't available and what is
    /// being used instead. The run itself carries on, on the per-run path.
    fn fell_back(&self, plugin: &Plugin, reason: &str, program: &str) {
        fell_back(&self.events, &self.told, plugin, reason, program);
    }

    /// Spawn the process and forward its output. `active` is the playground's
    /// (already cleared) run slot.
    fn spawn(
        &self,
        active: &mut Option<Active>,
        id: u64,
        playground_id: &PlaygroundId,
        plugin: &Plugin,
        spec: tinkerpad_runner::RunSpec,
    ) -> Result<u64, String> {
        let handle = run(spec).map_err(|error| error.to_string())?;
        info!(run = id, pid = handle.pid, playground = %playground_id, plugin = %plugin.id, "run started");
        Ok(self.attach(active, id, playground_id, plugin, handle))
    }

    /// Forward a run's output to the UI and make it this playground's current
    /// run. For a run that already has its process; a helper run registers
    /// itself before it has one (see [`Runs::start_on_helper`]).
    fn attach(
        &self,
        active: &mut Option<Active>,
        id: u64,
        playground_id: &PlaygroundId,
        plugin: &Plugin,
        handle: RunHandle,
    ) -> u64 {
        debug!(run = id, playground = %playground_id, plugin = %plugin.id, "streaming run output");
        let cancel = handle.canceller();
        let forwarder = tauri::async_runtime::spawn(forward(
            handle,
            None,
            self.events.clone(),
            Scope::Run(id),
            id,
            plugin.output.clone(),
        ));

        *active = Some(Active {
            id,
            cancel,
            forwarder,
        });
        id
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
            // Kills the process, or asks the helper to stop the program: the
            // stream reports the ending either way (03-03).
            active.cancel.cancel();
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

/// Say once per session that a plugin's helper isn't available, and what is
/// being used instead. The run itself carries on, on the per-run path.
fn fell_back(
    events: &EventBus,
    told: &std::sync::Mutex<HashSet<String>>,
    plugin: &Plugin,
    reason: &str,
    program: &str,
) {
    warn!(plugin = %plugin.id, %reason, "falling back to the per-run command");
    if !told
        .lock()
        .expect("told lock")
        .insert(plugin.id.to_string())
    {
        return;
    }
    events.publish(
        Scope::App,
        CoreEvent::Toast {
            level: Level::Warning,
            message: format!(
                "The {} helper isn't available ({reason}). Running with {program} instead.",
                plugin.name,
            ),
        },
    );
}

/// Stream one run's output to the UI until it ends, whichever way it is being
/// run.
///
/// `cancel` is the outer channel a helper run is cancelled through: the run may
/// have been registered before it had anything to cancel, so the request comes
/// here and is passed on to whatever it turned out to be.
async fn forward(
    mut handle: RunHandle,
    mut cancel: Option<mpsc::Receiver<()>>,
    events: Arc<EventBus>,
    scope: Scope,
    id: u64,
    output_spec: Option<tinkerpad_plugin::OutputSpec>,
) {
    // Text, dumped values, warnings, and positions found by the plugin's
    // `[output]` pattern, decoded from the output bytes.
    let mut decoder = OutputDecoder::with_output(output_spec);
    let mut out = Vec::new();
    let (mut dumps, mut malformed, mut annotations) = (0u32, 0u32, 0u32);

    loop {
        let event = match cancel.as_mut() {
            Some(cancel) => tokio::select! {
                _ = cancel.recv() => {
                    handle.cancel();
                    continue;
                }
                event = handle.recv() => event,
            },
            None => handle.recv().await,
        };
        let Some(event) = event else { break };

        match event {
            RunEvent::Started { warm, .. } => out.push(UiRunEvent::Started { warm }),
            RunEvent::Stdout(b) => decoder.stdout(&b, &mut out),
            RunEvent::Stderr(b) => decoder.stderr(&b, &mut out),
            RunEvent::Exited { code, duration } => {
                decoder.finish(&mut out);
                out.push(UiRunEvent::Exited {
                    code,
                    duration_ms: duration.as_millis() as u64,
                });
            }
            RunEvent::Cancelled { duration } => {
                decoder.finish(&mut out);
                out.push(UiRunEvent::Cancelled {
                    duration_ms: duration.as_millis() as u64,
                });
            }
            // A helper saying the program never ran: a compile error, with
            // somewhere to point (03-01).
            RunEvent::Failed {
                message,
                diagnostics,
            } => {
                decoder.finish(&mut out);
                for diagnostic in diagnostics {
                    out.push(UiRunEvent::Annotation(diagnostic.into_annotation()));
                }
                out.push(UiRunEvent::Failed { message });
            }
        }

        for ui in out.drain(..) {
            match &ui {
                UiRunEvent::Rich { .. } => dumps += 1,
                UiRunEvent::Annotation(a) => {
                    annotations += 1;
                    debug!(run = id, line = a.line, file = ?a.file, "annotation");
                }
                UiRunEvent::Warning { message } => {
                    malformed += 1;
                    warn!(run = id, %message, "run output warning");
                }
                _ => {}
            }
            events.publish(scope.clone(), CoreEvent::Run(ui));
        }
    }
    info!(run = id, dumps, malformed, annotations, "run finished");
}
