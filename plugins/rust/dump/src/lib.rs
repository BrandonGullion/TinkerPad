//! TinkerPad's dump library for Rust playgrounds.
//!
//! `dump(&value)` writes one `tinkerpad-dump-v1` value to stdout as
//! `ESC ] 1337 ; TinkerPad= <json> BEL` (docs/PLUGINS.md §6) and returns the
//! reference, so it chains. Anything that implements `serde::Serialize` works.
//! This crate decides the shape; the app only renders it:
//!
//! - a non-empty sequence whose items all serialise as maps (structs, maps)
//!   -> `table`, columns from the union of their keys in first-seen order,
//!   at most [`MAX_ROWS`] rows;
//! - any other sequence, struct, or map -> `tree`;
//! - everything else -> `value`.
//!
//! Nesting deeper than [`MAX_DEPTH`] becomes a short summary string.
//! Serialisation can't form cycles, so none are handled.
//!
//! ```ignore
//! use tinkerpad::Dump;
//!
//! #[derive(serde::Serialize)]
//! struct Person { name: &'static str, born: u16 }
//!
//! let people = vec![Person { name: "Ada", born: 1815 }];
//! let count = people.dump_titled("people").len();
//! tinkerpad::dump(&count);
//! ```

use std::io::Write;

use serde::Serialize;
use serde_json::{Map, Value};

/// Levels of nesting shown; deeper structures are summarised.
pub const MAX_DEPTH: usize = 5;
/// Rows kept in a table, and items kept in a sequence or map.
pub const MAX_ROWS: usize = 1000;

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// Integers beyond this can't be represented exactly in JavaScript.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Show `value` in TinkerPad's output pane and return it.
pub fn dump<T: Serialize + ?Sized>(value: &T) -> &T {
    write(&to_node(value, None));
    value
}

/// [`dump`] with a caption.
pub fn dump_titled<'a, T: Serialize + ?Sized>(value: &'a T, title: &str) -> &'a T {
    write(&to_node(value, Some(title)));
    value
}

/// `.dump()` and `.dump_titled(title)` on anything serialisable.
pub trait Dump: Serialize {
    /// Show `self` in TinkerPad's output pane and return it.
    fn dump(&self) -> &Self {
        dump(self)
    }

    /// Show `self` with a caption and return it.
    fn dump_titled(&self, title: &str) -> &Self {
        dump_titled(self, title)
    }
}

impl<T: Serialize + ?Sized> Dump for T {}

/// The dump node for `value`, without writing it.
pub fn to_node<T: Serialize + ?Sized>(value: &T, title: Option<&str>) -> Value {
    let type_name = short_type_name(std::any::type_name::<T>());
    let mut node = match serde_json::to_value(value) {
        Ok(json) => describe(json, Some(&type_name), 0),
        Err(e) => node("value", [
            ("value", Value::String(format!("[error: {e}]"))),
            ("type", Value::String(type_name)),
        ]),
    };
    if let (Some(title), Value::Object(map)) = (title, &mut node) {
        map.insert("title".into(), Value::String(title.into()));
    }
    node
}

fn write(node: &Value) {
    let json = node.to_string();
    let mut bytes = Vec::with_capacity(json.len() + 20);
    bytes.push(ESC);
    bytes.extend_from_slice(b"]1337;TinkerPad=");
    bytes.extend_from_slice(json.as_bytes());
    bytes.push(BEL);
    // One write, so the sequence can't interleave with other output.
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(&bytes);
    let _ = out.flush();
}

fn node<const N: usize>(kind: &str, fields: [(&str, Value); N]) -> Value {
    let mut map = Map::new();
    map.insert("v".into(), Value::from(1));
    map.insert("kind".into(), Value::String(kind.into()));
    for (k, v) in fields {
        map.insert(k.into(), v);
    }
    Value::Object(map)
}

fn describe(json: Value, type_name: Option<&str>, depth: usize) -> Value {
    let type_value = |fallback: &str| Value::String(type_name.unwrap_or(fallback).to_owned());
    match json {
        Value::Array(items) if !items.is_empty() && items.iter().all(Value::is_object) => {
            table(items, depth)
        }
        container @ (Value::Array(_) | Value::Object(_)) => {
            let kind_name = json_kind(&container);
            node("tree", [
                ("value", tree(container, depth)),
                ("type", type_value(kind_name)),
            ])
        }
        scalar => {
            let kind_name = json_kind(&scalar);
            node("value", [("value", safe_scalar(scalar)), ("type", type_value(kind_name))])
        }
    }
}

fn table(mut items: Vec<Value>, depth: usize) -> Value {
    let truncated = items.len() > MAX_ROWS;
    items.truncate(MAX_ROWS);

    let mut columns: Vec<String> = Vec::new();
    let mut kinds: Vec<Vec<&'static str>> = Vec::new();
    for item in &items {
        if let Value::Object(map) = item {
            for (key, value) in map {
                let index = match columns.iter().position(|c| c == key) {
                    Some(i) => i,
                    None => {
                        columns.push(key.clone());
                        kinds.push(Vec::new());
                        columns.len() - 1
                    }
                };
                let kind = json_kind(value);
                if kind != "null" && !kinds[index].contains(&kind) {
                    kinds[index].push(kind);
                }
            }
        }
    }

    let rows: Vec<Value> = items
        .into_iter()
        .map(|item| {
            let Value::Object(mut map) = item else {
                unreachable!("table rows are objects")
            };
            Value::Array(
                columns
                    .iter()
                    .map(|c| cell(map.remove(c).unwrap_or(Value::Null), depth + 2))
                    .collect(),
            )
        })
        .collect();

    let columns: Vec<Value> = columns
        .into_iter()
        .zip(kinds)
        .map(|(name, kinds)| {
            let mut column = Map::new();
            column.insert("name".into(), Value::String(name));
            match kinds.as_slice() {
                [] => {}
                [one] => {
                    column.insert("type".into(), Value::String((*one).into()));
                }
                _ => {
                    column.insert("type".into(), Value::String("mixed".into()));
                }
            }
            Value::Object(column)
        })
        .collect();

    node("table", [
        ("columns", Value::Array(columns)),
        ("rows", Value::Array(rows)),
        ("truncated", Value::Bool(truncated)),
    ])
}

/// A table cell: scalars inline, structures as nested nodes the app can open.
fn cell(value: Value, depth: usize) -> Value {
    match value {
        Value::Array(_) | Value::Object(_) if depth >= MAX_DEPTH => Value::String(summary(&value)),
        Value::Array(_) | Value::Object(_) => describe(value, None, depth),
        scalar => safe_scalar(scalar),
    }
}

/// `value` as plain JSON for a tree, with depth and size limits applied.
fn tree(value: Value, depth: usize) -> Value {
    match value {
        Value::Array(_) | Value::Object(_) if depth >= MAX_DEPTH => Value::String(summary(&value)),
        Value::Array(mut items) => {
            let more = items.len().saturating_sub(MAX_ROWS);
            items.truncate(MAX_ROWS);
            let mut out: Vec<Value> = items.into_iter().map(|v| tree(v, depth + 1)).collect();
            if more > 0 {
                out.push(Value::String(format!("… {more} more")));
            }
            Value::Array(out)
        }
        Value::Object(map) => {
            let more = map.len().saturating_sub(MAX_ROWS);
            let mut out: Map<String, Value> = map
                .into_iter()
                .take(MAX_ROWS)
                .map(|(k, v)| (k, tree(v, depth + 1)))
                .collect();
            if more > 0 {
                out.insert("…".into(), Value::String(format!("… {more} more")));
            }
            Value::Object(out)
        }
        scalar => safe_scalar(scalar),
    }
}

/// Integers JavaScript can't hold exactly become strings.
fn safe_scalar(value: Value) -> Value {
    match &value {
        Value::Number(n) if n.as_u64().is_some_and(|u| u > MAX_SAFE_INTEGER) => {
            Value::String(n.to_string())
        }
        Value::Number(n) if n.as_i64().is_some_and(|i| i.unsigned_abs() > MAX_SAFE_INTEGER) => {
            Value::String(n.to_string())
        }
        _ => value,
    }
}

fn json_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(n) if n.is_f64() => "float",
        Value::Number(_) => "int",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn summary(value: &Value) -> String {
    match value {
        Value::Array(items) => format!("[array, {} items]", items.len()),
        Value::Object(map) => format!("[object, {} fields]", map.len()),
        _ => value.to_string(),
    }
}

/// `alloc::vec::Vec<my_crate::Person>` becomes `Vec<Person>`.
pub fn short_type_name(full: &str) -> String {
    let mut out = String::with_capacity(full.len());
    let mut segment = String::new();
    let flush = |segment: &mut String, out: &mut String| {
        let last = segment.rsplit("::").next().unwrap_or("");
        out.push_str(last);
        segment.clear();
    };
    for c in full.chars() {
        if c.is_alphanumeric() || c == '_' || c == ':' {
            segment.push(c);
        } else {
            flush(&mut segment, &mut out);
            out.push(c);
        }
    }
    flush(&mut segment, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct Person {
        name: &'static str,
        born: u16,
        tags: Vec<&'static str>,
    }

    #[test]
    fn records_become_a_table_with_columns_in_field_order() {
        let people = vec![
            Person { name: "Ada", born: 1815, tags: vec!["math"] },
            Person { name: "Grace", born: 1906, tags: vec![] },
        ];
        let node = to_node(&people, Some("people"));
        assert_eq!(node["kind"], "table");
        assert_eq!(node["title"], "people");
        let names: Vec<&str> = node["columns"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["name", "born", "tags"]);
        assert_eq!(node["columns"][1]["type"], "int");
        assert_eq!(node["rows"][0][2]["kind"], "tree", "nested structures become nodes");
    }

    #[test]
    fn scalars_trees_and_limits() {
        assert_eq!(to_node(&42u8, None)["type"], "u8");
        assert_eq!(to_node("text", None)["kind"], "value");
        assert_eq!(to_node(&u64::MAX, None)["value"], "18446744073709551615");
        assert_eq!(to_node(&vec![1, 2, 3], None)["type"], "Vec<i32>");

        let rows: Vec<_> = (0..1005).map(|i| std::collections::BTreeMap::from([("i", i)])).collect();
        let table = to_node(&rows, None);
        assert_eq!(table["rows"].as_array().unwrap().len(), MAX_ROWS);
        assert_eq!(table["truncated"], true);

        let deep = serde_json::json!({"a": {"b": {"c": {"d": {"e": {"f": 1}}}}}});
        assert_eq!(to_node(&deep, None)["value"]["a"]["b"]["c"]["d"]["e"], "[object, 1 fields]");
    }

    #[test]
    fn short_type_names() {
        assert_eq!(short_type_name("alloc::vec::Vec<my_crate::Person>"), "Vec<Person>");
        assert_eq!(short_type_name("&str"), "&str");
        assert_eq!(
            short_type_name("std::collections::hash::map::HashMap<alloc::string::String, (i32, core::option::Option<u8>)>"),
            "HashMap<String, (i32, Option<u8>)>"
        );
    }
}
