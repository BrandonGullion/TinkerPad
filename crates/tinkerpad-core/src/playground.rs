//! Playgrounds: the thing a user creates, edits, and runs.
//!
//! A playground is a folder in the library with a `playground.toml` and a main
//! source file. These types describe one; `tinkerpad-library` stores them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::LanguageId;

/// A playground's permanent id: a ULID, stable across renames and moves.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaygroundId(String);

impl PlaygroundId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PlaygroundId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<&str> for PlaygroundId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// A playground kind within a language, such as `statements` or `program`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct KindId(String);

impl KindId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for KindId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PartialEq<&str> for KindId {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

/// One playground.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playground {
    pub id: PlaygroundId,
    /// Display name, as the user typed it. The folder name is a slug of it.
    pub name: String,
    pub language: LanguageId,
    pub kind: KindId,
    /// The playground's folder.
    pub dir: PathBuf,
    /// Absolute path of the main source file inside `dir`.
    pub main_file: PathBuf,
    /// Milliseconds since the Unix epoch.
    pub created_ms: u64,
    pub tags: Vec<String>,
}

/// A playground as listed in the library.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaygroundSummary {
    #[serde(flatten)]
    pub playground: Playground,
    /// When the main file (or, failing that, `playground.toml`) last changed,
    /// in milliseconds since the Unix epoch.
    pub modified_ms: u64,
    /// The first non-empty line of the main file, trimmed and shortened.
    pub preview: String,
}

/// A folder that has a `playground.toml` but can't be used as a playground.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrokenPlayground {
    pub dir: PathBuf,
    pub message: String,
}
