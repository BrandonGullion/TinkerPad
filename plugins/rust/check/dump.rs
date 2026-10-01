// The table `tinkerpad plugin check` expects (docs/PLUGINS.md §8.1).
use serde::Serialize;
use tinkerpad::Dump;

#[derive(Serialize)]
struct Row {
    name: &'static str,
    n: u32,
}

fn main() {
    let rows = [Row { name: "ada", n: 1 }, Row { name: "grace", n: 2 }];
    rows.dump_titled("check");
}
