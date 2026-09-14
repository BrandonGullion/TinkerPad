//! Stands in for the TinkerPad app in the die-with-parent tests: spawns a
//! long-sleeping child through `tinkerpad-process`, prints its pid, and waits
//! to be killed. Not part of the app.
//!
//! If `TINKERPAD_PID_REGISTRY` is set, it is used as the pid registry path
//! (meaningful on macOS only).

use std::time::Duration;

use tinkerpad_process::{SpawnSpec, platform, spawn};

#[tokio::main]
async fn main() {
    if let Some(path) = std::env::var_os("TINKERPAD_PID_REGISTRY") {
        platform::set_registry(path.into());
    }
    let child_exe = std::env::args().nth(1).expect("path to testchild");
    let child = spawn(SpawnSpec::new(child_exe).args(["sleep", "60000"])).expect("spawn");
    println!("child {}", child.pid());
    tokio::time::sleep(Duration::from_secs(60)).await;
    drop(child);
}
