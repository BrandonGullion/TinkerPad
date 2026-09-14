//! The raw shape of `plugin.toml`, exactly as written.
//!
//! These types have no behaviour and never leave the crate. The loader in
//! `plugin.rs` validates them and produces a [`crate::Plugin`].
//!
//! Every section struct denies unknown fields, so a typo inside a known
//! section is an error. The top level does not, so a plugin can carry a
//! section that only a newer TinkerPad understands; the loader warns about
//! those instead (docs/DEBUGGING.md D6).

use std::collections::BTreeMap;
use std::marker::PhantomData;

use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::os::Os;

/// Top-level sections this version understands.
pub const KNOWN_SECTIONS: &[&str] = &[
    "api",
    "language",
    "toolchain",
    "kind",
    "scaffold",
    "run",
    "helper",
    "lsp",
    "deps",
    "output",
];

#[derive(Debug, Deserialize)]
pub struct Manifest {
    #[allow(dead_code)] // checked before the strict parse
    pub api: u32,
    pub language: Language,
    pub toolchain: Toolchain,
    #[serde(rename = "kind", default)]
    pub kinds: Vec<Kind>,
    #[serde(default)]
    pub scaffold: Scaffold,
    pub run: Option<OneOrMany<Run>>,
    pub helper: Option<OneOrMany<Helper>>,
    pub lsp: Option<OneOrMany<Lsp>>,
    pub deps: Option<Deps>,
    pub output: Option<Output>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Language {
    pub id: String,
    pub name: String,
    pub extension: String,
    pub highlighter: Option<String>,
    /// Where the main file lives inside a playground. Defaults to
    /// `main.<extension>`.
    pub main_path: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    #[serde(default)]
    pub probe: Vec<Probe>,
    pub install_hint: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub min_version: Option<String>,
    pub os: Option<Vec<Os>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Kind {
    pub id: String,
    pub name: String,
    pub template: String,
    #[serde(default)]
    pub default: bool,
    pub prelude: Option<String>,
    pub postlude: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scaffold {
    #[serde(default)]
    pub files: Vec<ScaffoldFile>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScaffoldFile {
    /// Where to write it, relative to the playground folder.
    pub path: String,
    /// Source, relative to the plugin's `templates/` folder.
    pub template: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Run {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub os: Option<Vec<Os>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Helper {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub build: Option<Build>,
    #[serde(default)]
    pub hooks: Vec<String>,
    pub idle_shutdown: Option<String>,
    pub os: Option<Vec<Os>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lsp {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub workspace: Option<Workspace>,
    pub idle_shutdown: Option<String>,
    pub init_options: Option<toml::Table>,
    pub register: Option<Register>,
    pub os: Option<Vec<Os>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Workspace {
    None,
    PlaygroundDir,
    LibraryRoot,
    ScratchWorkspace,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Register {
    pub setting: String,
    pub value: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Deps {
    pub kind: DepsKind,
    pub add: Option<Vec<String>>,
    pub directive: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DepsKind {
    Command,
    InlineDirective,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub error_regex: String,
    #[serde(default)]
    pub error_regex_multiline: bool,
}

/// A section written either once (`[run]`) or repeated per OS (`[[run]]`).
///
/// Implemented by hand rather than with `#[serde(untagged)]` so that errors
/// inside the section (an unknown key, say) keep their message and location.
#[derive(Debug)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::One(t) => vec![t],
            OneOrMany::Many(v) => v,
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for OneOrMany<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct OneOrManyVisitor<T>(PhantomData<T>);

        impl<'de, T: Deserialize<'de>> Visitor<'de> for OneOrManyVisitor<T> {
            type Value = OneOrMany<T>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a table, or an array of tables")
            }

            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Self::Value, A::Error> {
                T::deserialize(MapAccessDeserializer::new(map)).map(OneOrMany::One)
            }

            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Self::Value, A::Error> {
                Vec::<T>::deserialize(SeqAccessDeserializer::new(seq)).map(OneOrMany::Many)
            }
        }

        deserializer.deserialize_any(OneOrManyVisitor(PhantomData))
    }
}
