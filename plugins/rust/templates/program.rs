// {{name}}, created {{date}}. Mod+Enter runs it.
// Add crates with `cargo add <crate>` in this folder, or edit Cargo.toml.
use serde::Serialize;
use tinkerpad::Dump;

#[derive(Serialize)]
struct Person {
    name: &'static str,
    born: u16,
}

fn main() {
    println!("Hello from Rust");
    let people = [
        Person { name: "Ada", born: 1815 },
        Person { name: "Grace", born: 1906 },
    ];
    people.dump_titled("people");
}
