fn main() {
    // Bundled plugins are copied next to the binary as a resource. Cargo scans
    // a watched directory recursively, so editing any plugin file re-copies them.
    println!("cargo:rerun-if-changed=../../plugins");
    tauri_build::build();
}
