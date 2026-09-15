//! `tinkerpad-dump-v1`: rich values a program writes to stdout with `dump()`.
//!
//! A program emits `ESC ] 1337 ; TinkerPad= <json> BEL` (or `ESC \` instead of
//! `BEL`). `tinkerpad-runner` finds those sequences and parses the JSON into a
//! [`RichValue`]; the UI renders it. See `docs/PLUGINS.md` §6.

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// The schema version this build understands.
pub const VERSION: u32 = 1;

/// One dumped value.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RichValue {
    /// Schema version, `1`.
    pub v: u32,
    /// A caption shown above the value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Reserved for lazy expansion of large values (not used yet).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(flatten)]
    pub node: RichNode,
}

/// What kind of value, and its data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RichNode {
    /// A scalar, shown inline with a type badge.
    Value {
        value: Value,
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        ty: Option<String>,
    },
    /// Rows of records. A cell may itself be a rich value object.
    Table {
        columns: Vec<Column>,
        rows: Vec<Vec<Value>>,
        #[serde(default)]
        truncated: bool,
    },
    /// Any JSON structure, shown collapsible.
    Tree {
        value: Value,
        #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
        ty: Option<String>,
    },
    /// A block of text, optionally highlighted as `lang`.
    Text {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
    },
    /// HTML, sanitised before display.
    Html { html: String },
    /// An image, base64-encoded.
    Image { mime: String, base64: String },
    /// A labelled duration.
    Timing { label: String, ms: f64 },
    /// A kind this build doesn't know (from a newer plugin library). Kept so the
    /// UI can show it as raw JSON instead of losing it.
    Unknown { raw: Value },
    /// A dump larger than the payload limit. Only the size is kept.
    Truncated { bytes: u64 },
}

/// A table column.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub ty: Option<String>,
}

const KNOWN_KINDS: &[&str] = &["value", "table", "tree", "text", "html", "image", "timing"];

impl RichValue {
    pub fn new(node: RichNode) -> Self {
        Self {
            v: VERSION,
            title: None,
            id: None,
            node,
        }
    }

    pub fn titled(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Parse a dump payload. A missing `v` means 1. An unknown `kind` becomes
    /// [`RichNode::Unknown`]; a known kind with the wrong fields is an error.
    pub fn from_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl<'de> Deserialize<'de> for RichValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Value::deserialize(deserializer)?;
        let object = raw
            .as_object()
            .ok_or_else(|| D::Error::custom("a dump must be a JSON object"))?;

        let v = match object.get("v") {
            None => VERSION,
            Some(v) => v
                .as_u64()
                .and_then(|v| u32::try_from(v).ok())
                .ok_or_else(|| D::Error::custom("`v` must be a small whole number"))?,
        };
        let title = optional_string::<D>(object, "title")?;
        let id = optional_string::<D>(object, "id")?;

        let kind = object
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| D::Error::custom("a dump needs a string `kind`"))?;
        let node = if KNOWN_KINDS.contains(&kind) {
            RichNode::deserialize(&raw).map_err(D::Error::custom)?
        } else {
            RichNode::Unknown { raw: raw.clone() }
        };
        Ok(Self { v, title, id, node })
    }
}

fn optional_string<'de, D: Deserializer<'de>>(
    object: &serde_json::Map<String, Value>,
    key: &str,
) -> Result<Option<String>, D::Error> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(D::Error::custom(format!("`{key}` must be a string"))),
    }
}

/// One sample of every kind, for renderer tests and documentation. The table
/// also carries the awkward cells a renderer must handle: nested dump values,
/// plain JSON structures, nulls, and a long string.
pub fn examples() -> Vec<RichValue> {
    use serde_json::json;
    let column = |name: &str, ty: &str| Column {
        name: name.into(),
        ty: Some(ty.into()),
    };
    vec![
        RichValue::new(RichNode::Value {
            value: json!(42),
            ty: Some("int".into()),
        })
        .titled("answer"),
        RichValue::new(RichNode::Table {
            columns: vec![
                column("id", "int"),
                column("name", "string"),
                column("born", "int"),
                column("tags", "list"),
                column("bio", "string"),
            ],
            rows: vec![
                vec![
                    json!(3),
                    json!("grace"),
                    json!(1906),
                    json!({"v": 1, "kind": "tree", "value": ["cobol", "compilers"]}),
                    json!("Rear admiral, wrote the first compiler, and popularised the word \"debugging\" after a moth was found in a relay."),
                ],
                vec![
                    json!(1),
                    json!("ada"),
                    json!(1815),
                    json!({"v": 1, "kind": "table", "columns": [{"name": "topic"}], "rows": [["math"], ["engines"]], "truncated": false}),
                    json!("Wrote the first published algorithm for a machine."),
                ],
                vec![json!(2), json!("alan"), json!(null), json!(["logic", "codes"]), json!(null)],
            ],
            truncated: false,
        })
        .titled("users"),
        RichValue::new(RichNode::Tree {
            value: json!({
                "name": "TinkerPad",
                "version": [0, 1, 0],
                "stable": false,
                "license": null,
                "nested": {"deep": {"deeper": {"deepest": true}}}
            }),
            ty: Some("object".into()),
        })
        .titled("config"),
        RichValue::new(RichNode::Text {
            text: "{\n  \"name\": \"TinkerPad\",\n  \"plugins\": [\"csharp\", \"typescript\"],\n  \"warm\": true\n}\n".into(),
            lang: Some("json".into()),
        }),
        RichValue::new(RichNode::Html {
            html: "<p>Hello <strong>world</strong>, from <em>sanitised</em> HTML.</p><ul><li>one</li><li>two</li></ul>".into(),
        }),
        RichValue::new(RichNode::Image {
            mime: "image/svg+xml".into(),
            // A 160x90 SVG: a blue card with a yellow circle and "dump".
            base64: "PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSIxNjAiIGhlaWdodD0iOTAiIHZpZXdCb3g9IjAgMCAxNjAgOTAiPjxyZWN0IHdpZHRoPSIxNjAiIGhlaWdodD0iOTAiIHJ4PSI4IiBmaWxsPSIjM2I4MmY2Ii8+PGNpcmNsZSBjeD0iNDUiIGN5PSI0NSIgcj0iMjQiIGZpbGw9IiNmYmJmMjQiLz48dGV4dCB4PSI4MCIgeT0iNTIiIGZvbnQtZmFtaWx5PSJzYW5zLXNlcmlmIiBmb250LXNpemU9IjE4IiBmaWxsPSIjZmZmIj5kdW1wPC90ZXh0Pjwvc3ZnPg==".into(),
        }),
        RichValue::new(RichNode::Timing {
            label: "query".into(),
            ms: 12.5,
        }),
    ]
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn table_from_the_docs() {
        let v = RichValue::from_json(
            br#"{"v":1,"kind":"table","title":"users",
                "columns":[{"name":"id","type":"int"},{"name":"name","type":"string"}],
                "rows":[[1,"ada"],[2,"grace"]],"truncated":false}"#,
        )
        .unwrap();
        assert_eq!(v.title.as_deref(), Some("users"));
        match v.node {
            RichNode::Table {
                columns,
                rows,
                truncated,
            } => {
                assert_eq!(columns[1].name, "name");
                assert_eq!(rows[1], vec![json!(2), json!("grace")]);
                assert!(!truncated);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_example_round_trips_through_json() {
        let examples = examples();
        let kinds: Vec<String> = examples
            .iter()
            .map(|e| {
                serde_json::to_value(e).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(kinds, KNOWN_KINDS);
        for example in examples {
            let json = serde_json::to_vec(&example).unwrap();
            assert_eq!(RichValue::from_json(&json).unwrap(), example);
        }
    }

    #[test]
    fn unknown_kind_is_kept_raw() {
        let v = RichValue::from_json(br#"{"v":1,"kind":"chart","series":[1,2,3]}"#).unwrap();
        assert_eq!(
            v.node,
            RichNode::Unknown {
                raw: json!({"v":1,"kind":"chart","series":[1,2,3]})
            }
        );
    }

    #[test]
    fn version_defaults_to_one_and_title_is_optional() {
        let v = RichValue::from_json(br#"{"kind":"value","value":"hi"}"#).unwrap();
        assert_eq!(v.v, 1);
        assert_eq!(v.title, None);
        assert_eq!(
            v.node,
            RichNode::Value {
                value: json!("hi"),
                ty: None
            }
        );
    }

    #[test]
    fn malformed_payloads_are_errors() {
        for bad in [
            &b"not json"[..],
            br#"[1,2,3]"#,
            br#"{"v":1}"#,
            br#"{"v":1,"kind":7}"#,
            br#"{"v":1,"kind":"table","columns":[]}"#,
            br#"{"v":"one","kind":"value","value":1}"#,
            br#"{"v":1,"kind":"value","value":1,"title":5}"#,
        ] {
            assert!(
                RichValue::from_json(bad).is_err(),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
    }

    #[test]
    fn object_keys_keep_the_programs_order() {
        let v =
            RichValue::from_json(br#"{"kind":"tree","value":{"zeta":1,"alpha":{"b":2,"a":3}}}"#)
                .unwrap();
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            r#"{"v":1,"kind":"tree","value":{"zeta":1,"alpha":{"b":2,"a":3}}}"#
        );
    }

    #[test]
    fn wire_format_is_flat() {
        let json = serde_json::to_value(RichValue::new(RichNode::Timing {
            label: "q".into(),
            ms: 1.0,
        }))
        .unwrap();
        assert_eq!(json, json!({"v":1,"kind":"timing","label":"q","ms":1.0}));
    }
}
