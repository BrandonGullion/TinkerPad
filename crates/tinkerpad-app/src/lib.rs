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

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use tauri::{Emitter, Manager};
use tinkerpad_core::{CoreEvent, Scope};

use events::EventBus;

/// Name of the single Tauri event the UI listens to.
pub const EVENT_NAME: &str = "core-event";

/// When `run` started, for the launch-time measurement.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Everything commands need, held in Tauri's managed state.
pub struct AppState {
    pub events: Arc<EventBus>,
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
}

pub fn run() {
    STARTED.get_or_init(Instant::now);

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .setup(|app| {
            let bus = Arc::new(EventBus::new());
            let handle = app.handle().clone();
            events::spawn_forwarder(bus.subscribe(), move |env| {
                if let Err(error) = handle.emit(EVENT_NAME, env) {
                    tracing::warn!(%error, seq = env.seq, "failed to emit event to UI");
                }
            });
            app.manage(AppState { events: bus });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![commands::ping])
        .run(tauri::generate_context!())
        .expect("error while running TinkerPad");
}
