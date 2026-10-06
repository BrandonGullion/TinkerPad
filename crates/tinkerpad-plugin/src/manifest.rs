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

use crate::os::{Arch, Os};

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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct Manifest {
    /// Manifest schema version. The core supports this one and the one before
    /// it; see `docs/VERSIONING.md`.
    #[allow(dead_code)] // checked before the strict parse
    pub api: u32,
    /// Who the language is: its id, name and file extension.
    pub language: Language,
    /// How to tell the language is installed, and what to say when it isn't.
    pub toolchain: Toolchain,
    /// The shapes a playground can take. Exactly one is the default.
    #[serde(rename = "kind", default)]
    pub kinds: Vec<Kind>,
    /// Files written into every new playground besides the main one.
    #[serde(default)]
    pub scaffold: Scaffold,
    /// How to run one playground. Required unless a `helper` runs them.
    pub run: Option<OneOrMany<Run>>,
    /// A long-lived process that runs playgrounds and answers hooks.
    pub helper: Option<OneOrMany<Helper>>,
    /// The language server, if the language has one.
    pub lsp: Option<OneOrMany<Lsp>>,
    /// How "Add package" adds a dependency.
    pub deps: Option<Deps>,
    /// How to find a file and line in the output, so an error is clickable.
    pub output: Option<Output>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Language {
    /// Stable, lowercase, `[a-z0-9-]`. Names the plugin folder.
    pub id: String,
    /// Shown in menus and the status bar.
    pub name: String,
    /// Playground files are `main.<extension>`.
    pub extension: String,
    pub highlighter: Option<String>,
    /// Where the main file lives inside a playground. Defaults to
    /// `main.<extension>`.
    pub main_path: Option<String>,
    /// The plugin's own version, for the plugins panel and `plugin list`
    /// (work item 05-02). Optional, and never interpreted: `api` is the
    /// compatibility contract, not this.
    pub version: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Toolchain {
    #[serde(default)]
    pub probe: Vec<Probe>,
    pub install_hint: Option<String>,
    /// How to get what the probes look for, when it isn't there (04-08a).
    pub acquire: Option<Acquire>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub min_version: Option<String>,
    pub os: Option<Vec<Os>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Scaffold {
    #[serde(default)]
    pub files: Vec<ScaffoldFile>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScaffoldFile {
    /// Where to write it, relative to the playground folder.
    pub path: String,
    /// Source, relative to the plugin's `templates/` folder.
    pub template: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Build {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Lsp {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub workspace: Option<Workspace>,
    pub idle_shutdown: Option<String>,
    // Whatever the server wants; TinkerPad passes it through without reading
    // it, so the schema says so rather than pretending to know.
    #[schemars(with = "Option<std::collections::BTreeMap<String, serde_json::Value>>")]
    pub init_options: Option<toml::Table>,
    pub register: Option<Register>,
    /// Files the server's workspace needs, written there once: a
    /// `tsconfig.json` for a server that insists on one.
    #[serde(default)]
    pub workspace_files: Vec<ScaffoldFile>,
    pub os: Option<Vec<Os>>,
    /// How to get the server, when it isn't installed (work item 04-08a).
    pub acquire: Option<Acquire>,
    /// A path inside the workspace the server needs to find there, such as
    /// `node_modules/typescript` (work item 04-08b).
    pub workspace_needs: Option<String>,
    /// How to put it there. Runs in the workspace root.
    pub workspace_acquire: Option<Acquire>,
}

/// How to get a tool this plugin needs (work item 04-08a).
///
/// Two kinds. `command` asks a package manager the user already has; `download`
/// fetches a published release when there is none to ask. Nothing here runs
/// unless the user asks for it.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Acquire {
    Command {
        /// The program and its arguments, as an array: there is never a shell.
        command: Vec<String>,
        /// Environment for the command, which is how a package manager is
        /// pointed at `{tools_dir}` instead of wherever it installs by
        /// default (work item 04-08b).
        #[serde(default)]
        env: BTreeMap<String, String>,
        /// What must already exist for this to work, such as `rustup`. Shown
        /// as the reason when it doesn't.
        needs: Option<String>,
        /// What to show the user as the thing about to happen. The command
        /// line itself, when absent.
        label: Option<String>,
        os: Option<Vec<Os>>,
    },
    Download {
        /// Required, and part of where the tool lands: a checksum means
        /// nothing without the version it belongs to.
        version: String,
        /// One entry per platform. The one matching this machine is used; a
        /// machine with no entry is told there is nothing to download.
        #[serde(default)]
        file: Vec<AcquireFile>,
    },
}

/// One platform's download.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AcquireFile {
    pub os: Os,
    pub arch: Arch,
    pub url: String,
    /// Hex SHA-256 of the file at `url`, checked before anything is unpacked.
    pub sha256: String,
    /// What to take out of the archive. Absent when the download is the tool
    /// itself rather than an archive of one.
    pub member: Option<String>,
    /// What to call it once it is out, when that isn't the member's own name.
    pub rename_to: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Workspace {
    None,
    PlaygroundDir,
    LibraryRoot,
    ScratchWorkspace,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Register {
    pub setting: String,
    pub value: String,
    /// A request to send after the setting changes, for a server that reads
    /// its projects once rather than on every change:
    /// `rust-analyzer/reloadWorkspace`.
    pub reload: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Deps {
    pub kind: DepsKind,
    pub add: Option<Vec<String>>,
    pub directive: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum DepsKind {
    Command,
    InlineDirective,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
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

/// A table, or an array of them — which is how TOML spells "repeat
/// this section per OS" (docs/PLUGINS.md §2.1). Written by hand because the
/// shape is a union, which no derive can see from the custom `Deserialize`
/// below.
impl<T: schemars::JsonSchema> schemars::JsonSchema for OneOrMany<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        format!("OneOrMany_{}", T::schema_name()).into()
    }

    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        let one = generator.subschema_for::<T>();
        let many = generator.subschema_for::<Vec<T>>();
        schemars::json_schema!({
            "anyOf": [one, many],
            "description": "A table, or an array of tables: one entry per OS.",
        })
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
