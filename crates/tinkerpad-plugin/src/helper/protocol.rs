//! The messages of `tinkerpad-plugin-v1`, as `docs/PLUGINS.md` §5 defines them.
//!
//! Two enums, one per direction, both internally tagged by `type`. Everything
//! a helper sends is optional-tolerant on purpose: a helper is someone else's
//! program, often a script, and a missing `duration_ms` is not a reason to
//! lose a run's output.
//!
//! **Forward compatibility.** Neither side may fail on a field it does not
//! know. `FromHelper` has no `deny_unknown_fields`, so a helper written
//! against a later version can send more than this build reads, and
//! [`FromHelper::Unknown`] catches message types this build has never heard
//! of. Adding an optional field is not a version bump; only removing or
//! re-typing one is (`docs/DEBUGGING.md` D7).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tinkerpad_core::{Annotation, KindId, Level, Playground};

/// The `protocol` field of `hello`. A helper that doesn't recognise it should
/// say so on stderr and exit rather than guess.
pub const PROTOCOL: &str = "tinkerpad-plugin-v1";

/// What to run. No timeout field, and there never will be one: a program runs
/// until it exits or the user cancels it (`docs/DEBUGGING.md` D12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRequest {
    /// Absolute path of the file to run.
    pub main: PathBuf,
    /// The playground folder, which is the run's working directory.
    pub dir: PathBuf,
    /// The playground kind, so the helper knows whether it was handed a
    /// statement list, an expression, or a whole program.
    pub kind: KindId,
}

/// Core to helper.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToHelper {
    /// Always first. The helper answers [`FromHelper::Ready`].
    Hello {
        protocol: &'static str,
        /// The plugin folder, for finding the helper's own resources.
        plugin_dir: PathBuf,
        /// A folder the helper may write to, private to this plugin.
        scratch_dir: PathBuf,
    },
    Run {
        id: String,
        main: PathBuf,
        dir: PathBuf,
        kind: KindId,
    },
    Cancel {
        id: String,
    },
    /// Stop cleanly. The helper has two seconds before it is killed.
    Shutdown,
    PlaygroundCreated {
        id: String,
        playground: Playground,
    },
    PlaygroundOpened {
        id: String,
        playground: Playground,
        /// Every playground open in the app, this one included.
        open: Vec<Playground>,
    },
    PlaygroundClosed {
        id: String,
        playground: Playground,
        open: Vec<Playground>,
    },
    AddDependency {
        id: String,
        playground: Playground,
        name: String,
        /// Absent when the user didn't pin one; the helper picks.
        version: Option<String>,
    },
}

/// Helper to core.
///
/// `id` is missing from `ready` only. Every other message names the request it
/// belongs to, which is how a reply finds the run or hook that is waiting.
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FromHelper {
    Ready {
        /// What the helper calls itself, for the log and the status strip.
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        version: Option<String>,
    },
    /// A chunk of the running program's stdout, base64. Chunks are arbitrary:
    /// a dump sentinel, or a UTF-8 character, may be split across two of them.
    Stdout {
        id: String,
        #[serde(default)]
        data: String,
    },
    Stderr {
        id: String,
        #[serde(default)]
        data: String,
    },
    /// The program ended on its own.
    Exited {
        id: String,
        /// Absent when the platform has no code to give (a signal, or a helper
        /// that doesn't track it).
        #[serde(default)]
        code: Option<i32>,
        #[serde(default)]
        duration_ms: Option<u64>,
    },
    /// The program never ran: a compile error, a missing toolchain.
    Failed {
        id: String,
        #[serde(default)]
        message: String,
        #[serde(default)]
        diagnostics: Vec<Diagnostic>,
    },
    /// The reply to a hook.
    Result {
        id: String,
        #[serde(default)]
        files: Vec<HelperFile>,
        /// Settings to push to the language server for this playground, in the
        /// shape `[lsp.register]` would have sent (work item 02-07).
        #[serde(default)]
        lsp_config: Option<serde_json::Value>,
    },
    /// The helper is about to exit on purpose, and would like to be started
    /// again: its process has accumulated something it can only shed by
    /// restarting — Node's module cache, a leaked handle, a memory figure it
    /// doesn't like. The core starts a fresh one without counting a crash
    /// (work item 03-06).
    Retiring {
        #[serde(default)]
        reason: Option<String>,
    },
    /// A message type this build has never heard of. Logged and ignored, so a
    /// newer helper talking to an older TinkerPad still works.
    #[serde(other)]
    Unknown,
}

/// A place in a file a helper wants pointed at, usually a compile error.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    #[serde(default)]
    pub file: Option<String>,
    /// One-based, as tools report it.
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub col: Option<u32>,
    #[serde(default)]
    pub message: String,
    /// `error` unless the helper says `warning` or `info`. Anything else is an
    /// error: a helper that misspells a severity still has something to say.
    #[serde(default)]
    pub severity: Option<String>,
}

impl Diagnostic {
    /// The same shape the regex annotator produces, so a helper's compile
    /// error reaches the editor by the path a per-run tool's error already
    /// takes (work item 01-08).
    pub fn into_annotation(self) -> Annotation {
        let severity = match self.severity.as_deref() {
            Some(s) if s.eq_ignore_ascii_case("warning") || s.eq_ignore_ascii_case("warn") => {
                Level::Warning
            }
            Some(s) if s.eq_ignore_ascii_case("info") => Level::Info,
            _ => Level::Error,
        };
        Annotation {
            text: self.message.clone(),
            file: self.file,
            line: self.line,
            col: self.col,
            message: self.message,
            severity,
        }
    }
}

/// A file a hook wants written into the playground folder.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
pub struct HelperFile {
    /// Relative to the playground folder. A path that escapes it is refused by
    /// the caller, not here.
    pub path: PathBuf,
    #[serde(default)]
    pub content: String,
}

/// What a hook gave back. Also what the caller gets when the helper has no
/// such hook, or took too long to answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HookResult {
    pub files: Vec<HelperFile>,
    pub lsp_config: Option<serde_json::Value>,
}

/// A hook call. The variants are exactly the hook names a manifest may list.
#[derive(Clone, Debug)]
pub enum HookRequest {
    PlaygroundCreated {
        playground: Playground,
    },
    PlaygroundOpened {
        playground: Playground,
        open: Vec<Playground>,
    },
    PlaygroundClosed {
        playground: Playground,
        open: Vec<Playground>,
    },
    AddDependency {
        playground: Playground,
        name: String,
        version: Option<String>,
    },
}

impl HookRequest {
    /// The name as it appears in the manifest's `hooks` and on the wire.
    pub fn name(&self) -> &'static str {
        match self {
            HookRequest::PlaygroundCreated { .. } => "playground_created",
            HookRequest::PlaygroundOpened { .. } => "playground_opened",
            HookRequest::PlaygroundClosed { .. } => "playground_closed",
            HookRequest::AddDependency { .. } => "add_dependency",
        }
    }

    /// The wire message, once an id has been picked for it.
    pub fn into_message(self, id: String) -> ToHelper {
        match self {
            HookRequest::PlaygroundCreated { playground } => {
                ToHelper::PlaygroundCreated { id, playground }
            }
            HookRequest::PlaygroundOpened { playground, open } => ToHelper::PlaygroundOpened {
                id,
                playground,
                open,
            },
            HookRequest::PlaygroundClosed { playground, open } => ToHelper::PlaygroundClosed {
                id,
                playground,
                open,
            },
            HookRequest::AddDependency {
                playground,
                name,
                version,
            } => ToHelper::AddDependency {
                id,
                playground,
                name,
                version,
            },
        }
    }
}

/// Every message a manifest's `hooks` may list: the four hooks, and `run` for
/// a helper that executes playgrounds. The one list, so the loader and the
/// client can't drift apart.
pub const HOOKS: [&str; 5] = [
    "run",
    "playground_created",
    "playground_opened",
    "playground_closed",
    "add_dependency",
];

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::Value;

    use super::*;

    fn playground() -> Playground {
        Playground {
            id: tinkerpad_core::PlaygroundId::new("pg"),
            name: "Scratch".to_owned(),
            language: tinkerpad_core::LanguageId::new("csharp"),
            kind: KindId::new("statements"),
            dir: PathBuf::from("/tmp/scratch"),
            main_file: PathBuf::from("/tmp/scratch/main.cs"),
            created_ms: 1_700_000_000_000,
            tags: Vec::new(),
            pinned: false,
        }
    }

    fn json(message: &ToHelper) -> Value {
        serde_json::to_value(message).unwrap()
    }

    #[test]
    fn a_run_says_what_to_run_and_nothing_about_how_long_it_may_take() {
        let message = json(&ToHelper::Run {
            id: "r1".to_owned(),
            main: PathBuf::from("/tmp/scratch/main.cs"),
            dir: PathBuf::from("/tmp/scratch"),
            kind: KindId::new("statements"),
        });
        let fields: Vec<&str> = message.as_object().unwrap().keys().map(|k| &**k).collect();
        // No timeout, now or ever (`docs/DEBUGGING.md` D12).
        assert_eq!(fields, ["type", "id", "main", "dir", "kind"]);
        assert_eq!(message["type"], "run");
        assert_eq!(message["kind"], "statements");
    }

    #[test]
    fn hello_carries_the_protocol_and_the_two_folders() {
        let message = json(&ToHelper::Hello {
            protocol: PROTOCOL,
            plugin_dir: PathBuf::from("/plugins/csharp"),
            scratch_dir: PathBuf::from("/scratch/csharp"),
        });
        assert_eq!(message["type"], "hello");
        assert_eq!(message["protocol"], "tinkerpad-plugin-v1");
        assert_eq!(message["plugin_dir"], "/plugins/csharp");
        assert_eq!(message["scratch_dir"], "/scratch/csharp");
    }

    #[test]
    fn a_hook_sends_the_playground_the_way_the_rest_of_the_app_sees_it() {
        let message = json(
            &HookRequest::PlaygroundOpened {
                playground: playground(),
                open: vec![playground()],
            }
            .into_message("h1".to_owned()),
        );
        assert_eq!(message["type"], "playground_opened");
        assert_eq!(message["id"], "h1");
        assert_eq!(message["playground"]["mainFile"], "/tmp/scratch/main.cs");
        assert_eq!(message["playground"]["createdMs"], 1_700_000_000_000u64);
        assert_eq!(message["open"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn every_hook_a_client_can_send_is_one_a_manifest_may_declare() {
        let requests = [
            HookRequest::PlaygroundCreated {
                playground: playground(),
            },
            HookRequest::PlaygroundOpened {
                playground: playground(),
                open: Vec::new(),
            },
            HookRequest::PlaygroundClosed {
                playground: playground(),
                open: Vec::new(),
            },
            HookRequest::AddDependency {
                playground: playground(),
                name: "serde".to_owned(),
                version: None,
            },
        ];
        for request in &requests {
            assert!(HOOKS.contains(&request.name()), "{}", request.name());
            // And the name is the message type, so a manifest that declares a
            // hook declares the message it will be sent.
            let message = json(&request.clone().into_message("h".to_owned()));
            assert_eq!(message["type"], request.name());
        }
        assert_eq!(HOOKS.len(), requests.len() + 1, "plus `run`");
    }

    #[test]
    fn a_diagnostic_becomes_the_annotation_the_editor_already_understands() {
        let diagnostic = Diagnostic {
            file: Some("main.cs".to_owned()),
            line: 3,
            col: Some(9),
            message: "CS1002: ; expected".to_owned(),
            severity: Some("WARNING".to_owned()),
        };
        let annotation = diagnostic.into_annotation();
        assert_eq!(annotation.severity, Level::Warning);
        assert_eq!(annotation.line, 3);
        assert_eq!(annotation.text, "CS1002: ; expected");

        // Anything unrecognised is an error: it still has something to say.
        let odd = Diagnostic {
            severity: Some("catastrophe".to_owned()),
            ..Default::default()
        };
        assert_eq!(odd.into_annotation().severity, Level::Error);
    }
}
