// Release builds on Windows are GUI apps with no console window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tinkerpad_app::run();
}
