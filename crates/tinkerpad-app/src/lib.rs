//! The TinkerPad desktop application.
//!
//! Wires the core crates into a Tauri 2 shell: commands the UI can invoke, the
//! single event channel the UI subscribes to, and the `AppState` that owns the
//! pools and the library.

use std::sync::OnceLock;
use std::time::Instant;

use tauri::{AppHandle, Emitter};

/// When `run` started, for the launch-time measurement.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Seed round-trip for work item 00-05. The UI calls this once its script has
/// loaded; the reply carries the time since process start, which is the
/// launch-to-interactive number work item 00-02 asks for.
#[tauri::command]
fn ping(app: AppHandle) -> Result<(), String> {
    let ms = STARTED.get().map(|s| s.elapsed().as_millis()).unwrap_or(0);
    println!("ui ready {ms} ms after start");
    app.emit("pong", format!("ready in {ms} ms"))
        .map_err(|e| e.to_string())
}

pub fn run() {
    STARTED.get_or_init(Instant::now);

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![ping])
        .run(tauri::generate_context!())
        .expect("error while running TinkerPad");
}
