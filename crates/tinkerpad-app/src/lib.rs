//! The TinkerPad desktop application.
//!
//! Wires the core crates into a Tauri 2 shell: commands the UI can invoke, the
//! single event channel the UI subscribes to, and the `AppState` that owns the
//! pools and the library.
//!
//! Adding a feature is "add a command, add an event": commands live in
//! [`commands`], return `Result<T, CommandError>`, and report progress by
//! publishing on the [`EventBus`].

pub mod events;
mod runs;

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tauri::{Emitter, Manager};
use tinkerpad_core::{CoreEvent, Scope};

use events::EventBus;
use runs::Runs;

/// Name of the single Tauri event the UI listens to.
pub const EVENT_NAME: &str = "core-event";

/// When `run` started, for the launch-time measurement.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Everything commands need, held in Tauri's managed state.
pub struct AppState {
    pub events: Arc<EventBus>,
    runs: Runs,
}

mod commands {
    use tauri::State;
    use tinkerpad_core::CommandError;

    use super::*;

    /// The UI calls this once its script has loaded. The reply carries the
    /// time since process start, which is the launch-to-interactive number.
    #[tauri::command]
    pub fn ping(state: State<'_, AppState>) -> Result<(), CommandError> {
        let ready_ms = STARTED
            .get()
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);
        // Printed as well as logged so scripts/smoke-launch.sh can see it.
        println!("ui ready {ready_ms} ms after start");
        state
            .events
            .publish(Scope::App, CoreEvent::Pong { ready_ms });
        Ok(())
    }

    /// Run the editor's code. Returns the run id; output and the outcome
    /// arrive as `run` events scoped to that id.
    #[tauri::command]
    pub async fn run(state: State<'_, AppState>, code: String) -> Result<u64, CommandError> {
        state.runs.start(code).await
    }

    /// Cancel the active run, if any.
    #[tauri::command]
    pub async fn cancel(state: State<'_, AppState>) -> Result<(), CommandError> {
        state.runs.cancel().await;
        Ok(())
    }

    /// Which in-app self-test scenarios to run, from `TINKERPAD_SELFTEST`
    /// (comma-separated). `None` in normal use. See `ui/src/selftest.ts`.
    #[tauri::command]
    pub fn selftest_scenario() -> Option<String> {
        std::env::var("TINKERPAD_SELFTEST")
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// One self-test result line, printed so scripts and CI can read it.
    #[tauri::command]
    pub fn selftest_report(line: String) {
        println!("selftest: {line}");
    }
}

pub fn run() {
    STARTED.get_or_init(Instant::now);

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            // macOS has no parent-death signal; sweep children left behind by a
            // force-quit of the previous session. No-op elsewhere.
            tinkerpad_process::platform::set_registry(data_dir.join("children.pids"));
            let swept = tinkerpad_process::platform::sweep_orphans();
            if swept > 0 {
                tracing::warn!(
                    swept,
                    "terminated orphaned processes from a previous session"
                );
            }

            let bus = Arc::new(EventBus::new());
            let handle = app.handle().clone();
            events::spawn_forwarder(bus.subscribe(), move |env| {
                if let Err(error) = handle.emit(EVENT_NAME, env) {
                    tracing::warn!(%error, seq = env.seq, "failed to emit event to UI");
                }
            });

            let runs = Runs::new(bus.clone(), data_dir.join("scratch"));
            app.manage(AppState { events: bus, runs });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::run,
            commands::cancel,
            commands::selftest_scenario,
            commands::selftest_report,
        ])
        .run(tauri::generate_context!())
        .expect("error while running TinkerPad");
}
