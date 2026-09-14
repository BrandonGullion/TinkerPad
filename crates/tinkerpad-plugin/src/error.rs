//! What can go wrong loading a plugin, phrased for the person writing it.

use std::path::PathBuf;

/// Why a plugin failed to load.
///
/// Rendering an error with `Display` gives a message meant for a plugin author:
/// the file, where in it, what's wrong, and a suggestion when there is one.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The manifest couldn't be read.
    #[error("could not read {}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// A key inside a known section isn't part of the schema.
    #[error("{}", .0.render("key"))]
    UnknownKey(Box<Unknown>),

    /// A value must be one of a fixed set (such as `workspace`) and isn't.
    #[error("{}", .0.render("value"))]
    UnknownValue(Box<Unknown>),

    /// The TOML is malformed, a required key is missing, or a value has the
    /// wrong type.
    #[error("{}{}", .0.location, .0.message)]
    Syntax(Box<Syntax>),

    /// The manifest parsed but breaks one or more rules.
    #[error("{}", render_problems(file, problems))]
    Invalid {
        file: PathBuf,
        problems: Vec<Problem>,
    },
}

impl ManifestError {
    /// The problems, if this is an [`ManifestError::Invalid`].
    pub fn problems(&self) -> &[Problem] {
        match self {
            ManifestError::Invalid { problems, .. } => problems,
            _ => &[],
        }
    }
}

/// Where in a manifest something went wrong.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub file: PathBuf,
    /// 1-based.
    pub line: usize,
    /// 1-based, in characters.
    pub column: usize,
    /// The `[section]` header in effect, if any.
    pub section: Option<String>,
}

impl std::fmt::Display for Location {
    /// `path:line:column: in [section]: `, ready to prefix a message.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}:{}: ", self.file.display(), self.line, self.column)?;
        if let Some(section) = &self.section {
            write!(f, "in [{section}]: ")?;
        }
        Ok(())
    }
}

/// An unknown key or value, with what was expected instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unknown {
    pub location: Location,
    /// The key or value as written.
    pub name: String,
    /// What the schema allows here.
    pub expected: Vec<String>,
    /// The closest allowed name, if it looks like a typo.
    pub suggestion: Option<String>,
}

impl Unknown {
    fn render(&self, what: &str) -> String {
        let mut s = format!("{}unknown {what} `{}`", self.location, self.name);
        if let Some(suggestion) = &self.suggestion {
            s.push_str(&format!("; did you mean `{suggestion}`?"));
        }
        if !self.expected.is_empty() {
            let list = self
                .expected
                .iter()
                .map(|e| format!("`{e}`"))
                .collect::<Vec<_>>()
                .join(", ");
            s.push_str(&format!(" (expected one of {list})"));
        }
        s
    }
}

/// A TOML or type error at a location.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Syntax {
    pub location: Location,
    pub message: String,
}

/// One rule a manifest breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub kind: ProblemKind,
    /// Where, as a key path such as `kind[1].template` or `run.args[0]`.
    pub key: String,
    pub message: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProblemKind {
    /// `api` is not a version this TinkerPad supports.
    UnsupportedApi,
    /// `language.id` or a kind id isn't lowercase letters, digits, and hyphens.
    InvalidId,
    /// No `[[kind]]` entries.
    NoKinds,
    /// Zero or several kinds marked `default = true`.
    DefaultKind,
    /// Two kinds share an id.
    DuplicateKind,
    /// A kind's template file doesn't exist.
    MissingTemplate,
    /// A `[scaffold].files` template doesn't exist.
    MissingScaffoldSource,
    /// A path is absolute, escapes its folder with `..`, or is empty.
    InvalidPath,
    /// Neither `[run]` nor `[helper]` applies on this OS.
    NoRunner,
    /// No `[toolchain].probe` entry applies on this OS.
    NoProbe,
    /// `[output].error_regex` doesn't compile.
    InvalidRegex,
    /// `[output].error_regex` lacks a required named group.
    MissingRegexGroup,
    /// A placeholder isn't allowed here, or braces are malformed.
    UnknownPlaceholder,
    /// Per-OS variants of a section overlap or are ambiguous.
    OsConflict,
    /// `[deps]` has the wrong keys for its `kind`.
    DepsShape,
    /// Any other malformed value: an empty command, a bad duration or
    /// version, an extension with a leading dot.
    InvalidValue,
}

fn render_problems(file: &std::path::Path, problems: &[Problem]) -> String {
    let mut s = format!(
        "{}: {} problem{}",
        file.display(),
        problems.len(),
        if problems.len() == 1 { "" } else { "s" }
    );
    for p in problems {
        // Indent continuation lines (a regex error spans several) so each
        // problem stays visually one list item.
        let message = p.message.replace('\n', "\n    ");
        s.push_str(&format!("\n  - {}: {message}", p.key));
    }
    s
}
