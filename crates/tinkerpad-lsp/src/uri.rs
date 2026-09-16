//! Two namings of the same file, and the translation between them.
//!
//! The editor knows a playground as `tinkerpad://playground/<id>`; the server
//! knows the file it is in, `file:///…/my-playground/main.rs`. Neither has to
//! learn the other's naming, so moving or renaming the library never reaches
//! the editor (`docs/ARCHITECTURE.md` §4.2).
//!
//! A message is rewritten by walking it for the keys that hold URIs, rather
//! than by following a path per method. A server may send fields no version of
//! this core has heard of — `rust-analyzer/…` extensions, say — and those carry
//! URIs too; a walk translates them, while a table of paths would leave them
//! pointing at files the editor can't name. [`REWRITTEN_METHODS`] lists the
//! methods that are known to carry URIs, and every one is covered by a test.

use std::collections::HashMap;
use std::path::Path;

use serde_json::{Map, Value};

use crate::jsonrpc::RawMessage;

/// The scheme the editor sees.
pub const SCHEME: &str = "tinkerpad";

/// Keys whose string value is a URI, anywhere in a message.
const URI_KEYS: &[&str] = &[
    "uri",
    "targetUri",
    "newUri",
    "oldUri",
    "documentUri",
    "externalUri",
];

/// Objects whose *keys* are URIs: `WorkspaceEdit.changes`.
const URI_KEYED: &[&str] = &["changes"];

/// The methods known to carry a URI, with where it sits. Not used to decide
/// what to rewrite — the walk does that — but kept as the checklist this
/// module is tested against, and as documentation of what flows through.
pub const REWRITTEN_METHODS: &[(&str, &str)] = &[
    // Editor → server.
    ("textDocument/didOpen", "textDocument.uri"),
    ("textDocument/didChange", "textDocument.uri"),
    ("textDocument/didClose", "textDocument.uri"),
    ("textDocument/didSave", "textDocument.uri"),
    ("textDocument/completion", "textDocument.uri"),
    ("textDocument/hover", "textDocument.uri"),
    ("textDocument/signatureHelp", "textDocument.uri"),
    ("textDocument/definition", "textDocument.uri"),
    ("textDocument/typeDefinition", "textDocument.uri"),
    ("textDocument/implementation", "textDocument.uri"),
    ("textDocument/references", "textDocument.uri"),
    ("textDocument/documentSymbol", "textDocument.uri"),
    ("textDocument/documentHighlight", "textDocument.uri"),
    ("textDocument/formatting", "textDocument.uri"),
    ("textDocument/rangeFormatting", "textDocument.uri"),
    ("textDocument/codeAction", "textDocument.uri"),
    ("textDocument/rename", "textDocument.uri"),
    ("textDocument/inlayHint", "textDocument.uri"),
    ("textDocument/semanticTokens/full", "textDocument.uri"),
    // Server → editor.
    ("textDocument/publishDiagnostics", "uri"),
    (
        "workspace/applyEdit",
        "edit.changes keys, edit.documentChanges[].textDocument.uri",
    ),
    // Results, which carry no method of their own.
    (
        "(definition result)",
        "Location.uri / LocationLink.targetUri",
    ),
    ("(references result)", "Location.uri"),
    ("(workspace/symbol result)", "location.uri"),
    (
        "(rename result)",
        "changes keys / documentChanges[].textDocument.uri",
    ),
];

/// Which playground each open file belongs to, both ways round.
///
/// Only open playgrounds are in here. A file that isn't one — a dependency a
/// go-to-definition landed in — keeps its `file:///` URI, so 02-09 can open it
/// read-only.
#[derive(Clone, Debug, Default)]
pub struct UriMap {
    /// `tinkerpad://playground/<id>` → `file:///…`
    to_file: HashMap<String, String>,
    /// The file URI, in the form used for comparison → playground URI.
    to_editor: HashMap<String, String>,
}

impl UriMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// The editor's name for a playground.
    pub fn playground_uri(id: &str) -> String {
        format!("{SCHEME}://playground/{id}")
    }

    /// The playground id inside an editor URI, if it is one.
    pub fn playground_id(uri: &str) -> Option<&str> {
        uri.strip_prefix(&format!("{SCHEME}://playground/"))
            .filter(|id| !id.is_empty())
    }

    /// Learn a playground's main file. Returns the editor URI for it.
    pub fn insert(&mut self, id: &str, main_file: &Path) -> String {
        let editor = Self::playground_uri(id);
        let file = crate::server::path_to_uri(main_file);
        self.to_editor.insert(compare_key(&file), editor.clone());
        self.to_file.insert(editor.clone(), file);
        editor
    }

    /// Forget a playground, once its last tab is closed.
    pub fn remove(&mut self, id: &str) {
        let editor = Self::playground_uri(id);
        if let Some(file) = self.to_file.remove(&editor) {
            self.to_editor.remove(&compare_key(&file));
        }
    }

    /// The file URI for an editor URI.
    pub fn file_for(&self, uri: &str) -> Option<&str> {
        self.to_file.get(uri).map(String::as_str)
    }

    /// The editor URI for a file URI, if that file is an open playground.
    pub fn editor_for(&self, uri: &str) -> Option<&str> {
        self.to_editor.get(&compare_key(uri)).map(String::as_str)
    }

    /// The playground a file URI belongs to.
    pub fn playground_for_file(&self, uri: &str) -> Option<&str> {
        self.editor_for(uri).and_then(Self::playground_id)
    }

    /// Rewrite a message on its way to the server: editor URIs become file
    /// URIs. Anything else is left exactly as it was.
    pub fn to_server(&self, message: &mut RawMessage) {
        let Some(object) = message.object_mut() else {
            return;
        };
        walk(object, &|uri| self.file_for(uri).map(str::to_owned));
    }

    /// Rewrite a message on its way to the editor: file URIs that name an open
    /// playground become editor URIs. A file that isn't a playground keeps its
    /// `file:///` URI.
    pub fn to_editor(&self, message: &mut RawMessage) {
        let Some(object) = message.object_mut() else {
            return;
        };
        walk(object, &|uri| self.editor_for(uri).map(str::to_owned));
    }
}

/// Windows paths differ in case and in slash direction between what a server
/// echoes back and what was sent, so compare on a normalised form.
fn compare_key(uri: &str) -> String {
    uri.replace('\\', "/").to_lowercase()
}

/// Rewrite every URI in `object`, in place.
fn walk(object: &mut Map<String, Value>, rewrite: &impl Fn(&str) -> Option<String>) {
    // Keys that are URIs (`WorkspaceEdit.changes`) are rebuilt, since a map
    // key can't be edited in place.
    for key in URI_KEYED {
        if let Some(Value::Object(changes)) = object.get_mut(*key) {
            let renamed = std::mem::take(changes);
            *changes = renamed
                .into_iter()
                .map(|(uri, value)| match rewrite(&uri) {
                    Some(new) => (new, value),
                    None => (uri, value),
                })
                .collect();
        }
    }

    for (key, value) in object.iter_mut() {
        match value {
            Value::String(text) if URI_KEYS.contains(&key.as_str()) => {
                if let Some(new) = rewrite(text) {
                    *text = new;
                }
            }
            Value::Object(child) => walk(child, rewrite),
            Value::Array(items) => {
                for item in items {
                    walk_value(item, rewrite);
                }
            }
            _ => {}
        }
    }
}

fn walk_value(value: &mut Value, rewrite: &impl Fn(&str) -> Option<String>) {
    match value {
        Value::Object(object) => walk(object, rewrite),
        Value::Array(items) => {
            for item in items {
                walk_value(item, rewrite);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map() -> UriMap {
        let mut map = UriMap::new();
        map.insert("01K-ONE", Path::new("/library/one/main.rs"));
        map.insert("01K-TWO", Path::new("/library/two/main.rs"));
        map
    }

    fn one_editor() -> String {
        UriMap::playground_uri("01K-ONE")
    }

    const ONE_FILE: &str = "file:///library/one/main.rs";

    #[test]
    fn maps_both_ways() {
        let map = map();
        assert_eq!(map.file_for(&one_editor()), Some(ONE_FILE));
        assert_eq!(map.editor_for(ONE_FILE).unwrap(), one_editor());
        assert_eq!(map.playground_for_file(ONE_FILE), Some("01K-ONE"));
        assert_eq!(UriMap::playground_id(&one_editor()), Some("01K-ONE"));
        assert_eq!(UriMap::playground_id("file:///x"), None);
    }

    #[test]
    fn forgets_a_closed_playground() {
        let mut map = map();
        map.remove("01K-ONE");
        assert_eq!(map.file_for(&one_editor()), None);
        assert_eq!(map.editor_for(ONE_FILE), None);
        assert!(map.file_for(&UriMap::playground_uri("01K-TWO")).is_some());
    }

    #[test]
    fn a_file_that_is_not_a_playground_is_left_alone() {
        let map = map();
        let mut message = RawMessage::notification(
            "textDocument/publishDiagnostics",
            json!({ "uri": "file:///home/me/.cargo/registry/serde/lib.rs", "diagnostics": [] }),
        );
        let before = message.clone();
        map.to_editor(&mut message);
        assert_eq!(message, before, "a dependency keeps its file URI");
    }

    /// Every method in the table round-trips: editor → server → editor.
    #[test]
    fn every_method_in_the_table_round_trips() {
        let map = map();
        let editor = one_editor();
        let samples: Vec<(&str, Value)> = vec![
            (
                "textDocument/didOpen",
                json!({ "textDocument": { "uri": editor, "languageId": "rust", "version": 1, "text": "" } }),
            ),
            (
                "textDocument/didChange",
                json!({ "textDocument": { "uri": editor, "version": 2 }, "contentChanges": [] }),
            ),
            (
                "textDocument/didClose",
                json!({ "textDocument": { "uri": editor } }),
            ),
            (
                "textDocument/didSave",
                json!({ "textDocument": { "uri": editor } }),
            ),
            (
                "textDocument/completion",
                json!({ "textDocument": { "uri": editor }, "position": { "line": 1, "character": 2 } }),
            ),
            (
                "textDocument/hover",
                json!({ "textDocument": { "uri": editor }, "position": { "line": 0, "character": 0 } }),
            ),
            (
                "textDocument/signatureHelp",
                json!({ "textDocument": { "uri": editor }, "position": { "line": 0, "character": 0 } }),
            ),
            (
                "textDocument/definition",
                json!({ "textDocument": { "uri": editor }, "position": { "line": 0, "character": 0 } }),
            ),
            (
                "textDocument/references",
                json!({ "textDocument": { "uri": editor }, "context": { "includeDeclaration": true } }),
            ),
            (
                "textDocument/rename",
                json!({ "textDocument": { "uri": editor }, "newName": "x" }),
            ),
            (
                "textDocument/publishDiagnostics",
                json!({ "uri": editor, "diagnostics": [] }),
            ),
            (
                "workspace/applyEdit",
                json!({
                    "edit": {
                        "changes": { editor.clone(): [] },
                        "documentChanges": [
                            { "textDocument": { "uri": editor, "version": 1 }, "edits": [] }
                        ],
                    }
                }),
            ),
        ];

        for (method, params) in samples {
            let mut message = RawMessage::notification(method, params.clone());
            map.to_server(&mut message);
            assert!(
                !message.to_string().contains(SCHEME),
                "{method} still had an editor URI",
            );
            assert!(
                String::from_utf8(message.to_vec())
                    .unwrap()
                    .contains(ONE_FILE),
                "{method} lost its URI on the way out",
            );
            map.to_editor(&mut message);
            assert_eq!(
                message.params(),
                Some(&params),
                "{method} did not come back the way it went",
            );
        }
    }

    #[test]
    fn rewrites_uris_inside_results() {
        let map = map();
        let editor = one_editor();
        let two = UriMap::playground_uri("01K-TWO");
        // A definition result: Locations, LocationLinks, and a dependency.
        let mut response = RawMessage::response(
            1,
            json!([
                { "uri": ONE_FILE, "range": {} },
                { "targetUri": "file:///library/two/main.rs", "targetRange": {} },
                { "uri": "file:///usr/lib/rust/core.rs", "range": {} },
            ]),
        );
        map.to_editor(&mut response);
        let result = response.result().unwrap();
        assert_eq!(result[0]["uri"], json!(editor));
        assert_eq!(result[1]["targetUri"], json!(two));
        assert_eq!(
            result[2]["uri"],
            json!("file:///usr/lib/rust/core.rs"),
            "a file outside the library stays a file URI",
        );
    }

    #[test]
    fn matches_a_windows_path_whatever_its_case_or_slashes() {
        let mut map = UriMap::new();
        map.insert("01K-WIN", Path::new(r"C:\Users\Me\TinkerPad\p\main.rs"));
        for spelling in [
            "file:///C:/Users/Me/TinkerPad/p/main.rs",
            "file:///c:/users/me/tinkerpad/p/main.rs",
        ] {
            assert_eq!(
                map.playground_for_file(spelling),
                Some("01K-WIN"),
                "{spelling}",
            );
        }
    }

    #[test]
    fn leaves_everything_that_is_not_a_uri_alone() {
        let map = map();
        let params = json!({
            "textDocument": { "uri": one_editor() },
            "name": "tinkerpad://playground/not-a-uri-key",
            "position": { "line": 3, "character": 7 },
            "data": { "nested": [1, "two", null] },
        });
        let mut message = RawMessage::request(1, "textDocument/hover", params);
        map.to_server(&mut message);
        let out = message.params().unwrap();
        assert_eq!(out["textDocument"]["uri"], json!(ONE_FILE));
        assert_eq!(
            out["name"],
            json!("tinkerpad://playground/not-a-uri-key"),
            "only URI-carrying keys are touched",
        );
        assert_eq!(out["position"], json!({ "line": 3, "character": 7 }));
        assert_eq!(out["data"], json!({ "nested": [1, "two", null] }));
    }
}
