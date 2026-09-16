//! Events the core sends to the UI.
//!
//! Every event travels inside an [`Envelope`] carrying a [`Scope`] (so the UI
//! can route it without looking at the payload) and a sequence number (so
//! ordering can be checked). The TypeScript mirror lives in
//! `ui/src/protocol.ts`; keep the two in step.

use serde::Serialize;

/// What an event is about. Serialised as a short string such as `"app"` or
/// `"run:7"` so the UI can use it directly as a map key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Application-wide: toasts, settings, startup.
    App,
    /// One execution of a playground.
    Run(u64),
}

impl std::fmt::Display for Scope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Scope::App => f.write_str("app"),
            Scope::Run(id) => write!(f, "run:{id}"),
        }
    }
}

impl Serialize for Scope {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

/// Severity of a user-facing notice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// Something the UI should know about.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CoreEvent {
    /// Reply to the `ping` command, carrying launch-to-ready time.
    #[serde(rename_all = "camelCase")]
    Pong { ready_ms: u64 },
    /// A non-blocking notice for the user.
    Toast { level: Level, message: String },
    /// Execution output and lifecycle; see [`RunEvent`].
    Run(RunEvent),
    /// A language's toolchain was (re)checked.
    Toolchain {
        language: crate::LanguageId,
        state: ToolchainState,
    },
    /// A language server changed state (work item 02-02).
    Server {
        language: crate::LanguageId,
        /// The workspace root it serves, as a path, or empty when it needs none.
        root: String,
        state: ServerState,
    },
    /// Playgrounds were added, removed, renamed, or their metadata changed on
    /// disk. The UI should list them again.
    LibraryChanged,
    /// A playground's main file was changed outside the app. A clean tab
    /// reloads it; a dirty one asks (work item 01-12).
    SourceChangedOnDisk { playground: crate::PlaygroundId },
}

/// Whether a language's toolchain is usable.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ToolchainState {
    /// Probes haven't finished yet.
    Checking,
    /// A probe succeeded.
    Ready {
        /// The program as written in the probe, e.g. `python3`. Commands use
        /// it as `{toolchain}`.
        program: String,
        /// Where it was found.
        path: String,
        /// The version it reported, if one could be read.
        version: Option<String>,
    },
    /// No probe succeeded.
    Missing {
        /// What each probe found, e.g. "found 9.0.3, need 10.0".
        reason: String,
        /// The plugin's `install_hint`.
        hint: Option<String>,
    },
}

/// Whether a language's server is up. One server serves every playground of
/// its language in the same workspace (work item 02-02).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ServerState {
    /// Not running: never started, asked to stop, or unable to start.
    Stopped {
        /// Why, when it was not a plain "nobody has needed it yet".
        reason: Option<String>,
        /// The plugin's `install_hint`, when the server binary is missing.
        hint: Option<String>,
    },
    /// Spawned; the `initialize` handshake hasn't finished. Requests made now
    /// are held until it does.
    Starting,
    /// Handshake done, requests flowing.
    #[serde(rename_all = "camelCase")]
    Ready {
        pid: u32,
        /// What the server called itself in `initialize`, if it said.
        name: Option<String>,
        version: Option<String>,
    },
    /// It exited without being asked to, and is being started again (02-05).
    Crashed {
        reason: String,
        /// How many crashes in a row this is. The wait before each restart
        /// doubles, and after enough of them the pool stops trying.
        restarts: u32,
    },
    /// `shutdown` has been sent and the exit is being waited for.
    Stopping,
}

impl ServerState {
    /// Not running, with nothing to explain.
    pub fn stopped() -> Self {
        Self::Stopped {
            reason: None,
            hint: None,
        }
    }

    /// Not running because something went wrong.
    pub fn failed(reason: impl Into<String>, hint: Option<String>) -> Self {
        Self::Stopped {
            reason: Some(reason.into()),
            hint,
        }
    }

    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready { .. })
    }

    /// Whether this state can still become `Ready` by waiting.
    pub fn is_settling(&self) -> bool {
        matches!(self, Self::Starting)
    }
}
/// What happened during one run. The scope carries the run id.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RunEvent {
    Started,
    /// Output text. Decoded lossily from bytes at this boundary, after the
    /// runner has taken out any dumped values.
    Stdout {
        text: String,
    },
    Stderr {
        text: String,
    },
    /// A value the program dumped, in order with the surrounding output.
    Rich {
        value: crate::RichValue,
    },
    /// Something about the run's output the user should know, such as a dump
    /// that couldn't be read. The run carries on.
    Warning {
        message: String,
    },
    /// A compiler or runtime message with a position, found in the output by
    /// the plugin's `[output].error_regex`. The line it came from has just
    /// been sent as `stdout` or `stderr`.
    Annotation(Annotation),
    /// The process ended on its own.
    #[serde(rename_all = "camelCase")]
    Exited {
        code: Option<i32>,
        duration_ms: u64,
    },
    /// The user cancelled the run.
    #[serde(rename_all = "camelCase")]
    Cancelled {
        duration_ms: u64,
    },
    /// The run could not start at all.
    Failed {
        message: String,
    },
}

/// A position in a file that a run's output pointed at.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// The file as the tool wrote it: absolute, relative, or a `file://` URL.
    /// `None` when the pattern has no `file` group.
    pub file: Option<String>,
    /// One-based, as tools report it.
    pub line: u32,
    /// One-based column, if the pattern captured one.
    pub col: Option<u32>,
    pub message: String,
    /// `Error` unless the pattern captured `severity` as a warning.
    pub severity: Level,
    /// The output line (or lines) the pattern matched.
    pub text: String,
}

/// The wire format for one event.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Envelope {
    /// Strictly increasing per app session.
    pub seq: u64,
    pub scope: Scope,
    pub event: CoreEvent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn envelope_wire_format() {
        let env = Envelope {
            seq: 3,
            scope: Scope::Run(7),
            event: CoreEvent::Run(RunEvent::Exited {
                code: Some(0),
                duration_ms: 12,
            }),
        };
        let json = serde_json::to_string(&env).unwrap();
        assert_eq!(
            json,
            r#"{"seq":3,"scope":"run:7","event":{"type":"run","kind":"exited","code":0,"durationMs":12}}"#
        );
    }

    #[test]
    fn rich_wire_format() {
        let event = CoreEvent::Run(RunEvent::Rich {
            value: crate::RichValue::new(crate::RichNode::Timing {
                label: "q".into(),
                ms: 2.0,
            }),
        });
        assert_eq!(
            serde_json::to_string(&event).unwrap(),
            r#"{"type":"run","kind":"rich","value":{"v":1,"kind":"timing","label":"q","ms":2.0}}"#
        );
    }

    #[test]
    fn toast_wire_format() {
        let env = Envelope {
            seq: 1,
            scope: Scope::App,
            event: CoreEvent::Toast {
                level: Level::Warning,
                message: "hi".into(),
            },
        };
        assert_eq!(
            serde_json::to_string(&env).unwrap(),
            r#"{"seq":1,"scope":"app","event":{"type":"toast","level":"warning","message":"hi"}}"#
        );
    }
}
