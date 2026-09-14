//! The TinkerPad desktop application.
//!
//! Wires the core crates into a Tauri 2 shell: commands the UI can invoke, the
//! single event channel the UI subscribes to, and the `AppState` that owns the
//! pools and the library. Work item 00-02 turns this into a Tauri app; until
//! then it only proves the workspace links.

fn main() {
    println!(
        "tinkerpad {} ({})",
        env!("CARGO_PKG_VERSION"),
        tinkerpad_process::platform::name()
    );
}
