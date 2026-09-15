// Every shape the Rust plugin's dump crate writes. The output is captured in
// tests/dump_cases/15-rust-shapes.txt, so keep it the same on every machine:
// no times, paths, or hash-ordered maps.
use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;
use tinkerpad::Dump;

#[derive(Serialize)]
struct Person {
    name: &'static str,
    born: u16,
    tags: Vec<&'static str>,
}

#[derive(Serialize)]
struct Team {
    team: &'static str,
    members: Vec<Member>,
}

#[derive(Serialize)]
struct Member {
    name: &'static str,
}

#[derive(Serialize)]
enum Level {
    Low,
    High { boost: u8 },
}

#[derive(Serialize)]
struct Unit;

fn main() {
    println!("table:");
    vec![
        Person { name: "Ada", born: 1815, tags: vec!["math"] },
        Person { name: "Grace", born: 1906, tags: vec![] },
    ]
    .dump_titled("people");

    println!("table with a nested table:");
    [Team { team: "core", members: vec![Member { name: "Ada" }, Member { name: "Alan" }] }].dump();

    println!("tree:");
    (Level::High { boost: 2 }, Some("point"), None::<u8>, [1.5, 2.25]).dump_titled("config");
    BTreeMap::from([("b", 2), ("a", 1)]).dump();
    BTreeSet::from(["x", "y"]).dump();

    println!("values:");
    42u8.dump();
    "text".dump();
    true.dump();
    'c'.dump();
    3.5f64.dump();
    f64::NAN.dump();
    u64::MAX.dump();
    i64::MIN.dump();
    ().dump();
    Unit.dump();
    Level::Low.dump();
    None::<String>.dump();

    println!("depth:");
    let deep = vec![vec![vec![vec![vec![vec![1]]]]]];
    deep.dump();

    println!("chained: {}", [1, 2, 3].dump().len());
}
