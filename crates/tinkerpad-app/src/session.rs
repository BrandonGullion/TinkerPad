//! The open tabs, restored at launch: `<app-data>/session.json`.
//!
//! The UI owns what's in it and saves it whenever tabs change; the core only
//! stores it. A missing or unreadable file is an empty session, never an
//! error: losing tab positions must not stop the app from starting.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tracing::warn;

pub const FILE_NAME: &str = "session.json";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Session {
    /// Open tabs, left to right.
    pub tabs: Vec<SessionTab>,
    /// The active tab's playground id.
    pub active: Option<String>,
    /// The language `Mod+N` uses when no tab is open.
    pub last_language: Option<String>,
    /// Playgrounds opened lately, most recent first (work item 04-01). The
    /// window keeps the list; the core only stores it.
    pub recent: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SessionTab {
    /// Playground id.
    pub playground: String,
    /// Editor scroll position in CSS pixels.
    pub scroll_top: f64,
}

/// Where the session lives. `TINKERPAD_SESSION` overrides it; a self-test run
/// without that override uses a file in the temp folder, so tests never touch
/// the real session.
pub fn path(data_dir: &Path) -> PathBuf {
    if let Some(path) = std::env::var_os("TINKERPAD_SESSION").filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    if std::env::var_os("TINKERPAD_SELFTEST").is_some_and(|s| !s.is_empty()) {
        return std::env::temp_dir().join("tinkerpad-selftest-session.json");
    }
    data_dir.join(FILE_NAME)
}

/// Read the session at `path`, or an empty one.
pub fn load(path: &Path) -> Session {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
            warn!(file = %path.display(), error = %e, "session file unreadable; starting empty");
            Session::default()
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Session::default(),
        Err(e) => {
            warn!(file = %path.display(), error = %e, "could not read session file");
            Session::default()
        }
    }
}

/// Write the session atomically: a crash mid-write leaves the old file.
pub fn save(path: &Path, session: &Session) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(session).map_err(std::io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-session-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join(FILE_NAME)
    }

    #[test]
    fn round_trip_and_overwrite() {
        let path = temp("round-trip");
        let session = Session {
            tabs: vec![
                SessionTab {
                    playground: "A".into(),
                    scroll_top: 120.5,
                },
                SessionTab {
                    playground: "B".into(),
                    scroll_top: 0.0,
                },
            ],
            active: Some("B".into()),
            last_language: Some("rust".into()),
            recent: vec!["B".into(), "A".into()],
        };
        save(&path, &session).unwrap();
        assert_eq!(load(&path), session);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.contains("\"scrollTop\": 120.5") && text.contains("\"lastLanguage\""),
            "{text}"
        );

        save(&path, &Session::default()).unwrap();
        assert_eq!(load(&path), Session::default());
        assert!(!path.with_extension("json.tmp").exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_corrupt_and_partial_files() {
        let path = temp("broken");
        assert_eq!(load(&path), Session::default(), "missing");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Session::default(), "corrupt");
        std::fs::write(&path, r#"{"tabs":[{"playground":"A"}],"future":1}"#).unwrap();
        assert_eq!(
            load(&path),
            Session {
                tabs: vec![SessionTab {
                    playground: "A".into(),
                    scroll_top: 0.0
                }],
                ..Session::default()
            },
            "missing fields default and unknown ones are ignored"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
