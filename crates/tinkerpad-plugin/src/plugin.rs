//! Loading a plugin folder into a validated [`Plugin`].

use std::collections::{BTreeMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use regex::Regex;
use tracing::warn;

use tinkerpad_core::LanguageId;

use crate::error::{Location, ManifestError, Problem, ProblemKind, Syntax, Unknown};
use crate::manifest::{self, KNOWN_SECTIONS, Manifest};
use crate::os::{Arch, Os};
use crate::placeholders::{self, PlaceholderError};
use crate::suggest;

/// The manifest's file name inside a plugin folder.
pub const MANIFEST_FILE: &str = "plugin.toml";
/// Folder inside a plugin that templates are read from.
pub const TEMPLATES_DIR: &str = "templates";
/// The manifest `api` version this build reads.
pub const SUPPORTED_API: i64 = 1;

/// Hook names a helper may declare: the protocol's own list, so a manifest
/// can't be validated against one set of names and served by another
/// (docs/PLUGINS.md §5).
const KNOWN_HOOKS: &[&str] = &crate::helper::protocol::HOOKS;

/// Highlighters the UI bundles (docs/PLUGINS.md §9). Kept in step with
/// `ui/src/highlight.ts`, which is where each one is actually loaded.
const KNOWN_HIGHLIGHTERS: &[&str] = &[
    "cpp",
    "csharp",
    "go",
    "java",
    "javascript",
    "json",
    "jsx",
    "kotlin",
    "lua",
    "markdown",
    "python",
    "ruby",
    "rust",
    "scala",
    "shell",
    "sql",
    "swift",
    "toml",
    "typescript",
    "yaml",
];

/// Named groups `[output].error_regex` may use.
const REGEX_GROUPS: &[&str] = &["file", "line", "col", "message", "severity"];

/// A language plugin, loaded and validated for one OS.
///
/// Paths to plugin files are absolute. Command strings still contain their
/// `{placeholders}`; substitute them with [`crate::placeholders::substitute`]
/// when the values are known.
#[derive(Clone, Debug)]
pub struct Plugin {
    /// The plugin folder.
    pub dir: PathBuf,
    /// The OS this plugin was resolved for.
    pub os: Os,
    pub id: LanguageId,
    pub name: String,
    /// File extension without the dot.
    pub extension: String,
    pub highlighter: Option<String>,
    /// Main file inside a playground folder, relative to it.
    pub main_path: PathBuf,
    /// Toolchain probes that apply on this OS, in order.
    pub probes: Vec<Probe>,
    pub install_hint: Option<String>,
    /// How to get what the probes look for, when it isn't there (04-08a).
    pub toolchain_acquire: Option<Acquire>,
    /// Always non-empty, exactly one marked default.
    pub kinds: Vec<Kind>,
    pub scaffold: Vec<ScaffoldFile>,
    pub run: Option<CommandSpec>,
    pub helper: Option<HelperSpec>,
    pub lsp: Option<LspSpec>,
    pub deps: Option<DepsSpec>,
    pub output: Option<OutputSpec>,
    /// Non-fatal findings, such as unknown top-level sections. Also logged.
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Probe {
    pub command: String,
    pub args: Vec<String>,
    pub min_version: Option<Version>,
}

/// A dotted version such as `10.0` or `22.6.1`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub Vec<u64>);

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let parts: Option<Vec<u64>> = s.split('.').map(|p| p.parse().ok()).collect();
        parts.filter(|p| (1..=4).contains(&p.len())).map(Version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let parts: Vec<String> = self.0.iter().map(u64::to_string).collect();
        f.write_str(&parts.join("."))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kind {
    pub id: String,
    pub name: String,
    /// Absolute path to the template file.
    pub template: PathBuf,
    pub default: bool,
    pub prelude: Option<String>,
    pub postlude: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScaffoldFile {
    /// Where to write it, relative to the playground folder.
    pub path: PathBuf,
    /// Absolute path to the source template.
    pub template: PathBuf,
}

/// A program to start. Strings may contain placeholders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandSpec {
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HelperSpec {
    pub command: CommandSpec,
    pub build: Option<CommandSpec>,
    pub hooks: Vec<String>,
    pub idle_shutdown: IdleShutdown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LspSpec {
    pub command: CommandSpec,
    /// How to get the server when it isn't installed (work item 04-08a).
    pub acquire: Option<Acquire>,
    pub workspace: Workspace,
    pub idle_shutdown: IdleShutdown,
    /// `initializationOptions`, converted from TOML.
    pub init_options: Option<serde_json::Value>,
    pub register: Option<Register>,
    /// Files written into the server's workspace the first time it starts,
    /// and never overwritten (work item 02-07).
    pub workspace_files: Vec<ScaffoldFile>,
}

pub use manifest::Workspace;

/// Whether a path names something inside an archive: relative, no `..`, no
/// drive, no root. What comes out of a download must land where it is meant to
/// (work item 04-08a).
fn inside_archive(path: &str) -> bool {
    let normalised = path.replace('\\', "/");
    !normalised.is_empty()
        && !normalised.starts_with('/')
        && normalised.as_bytes().get(1) != Some(&b':')
        && normalised
            .split('/')
            .all(|part| part != ".." && !part.is_empty())
}

/// How to get a tool the plugin needs, validated (work item 04-08a).
///
/// Nothing here happens on its own: the app asks, having told the user exactly
/// what is about to run or be fetched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Acquire {
    /// Ask a package manager the user already has.
    Command {
        /// Program first, then its arguments. Never a shell.
        command: Vec<String>,
        /// What must be on PATH for this to work, such as `rustup`.
        needs: Option<String>,
        /// What the user is shown as the thing about to happen.
        label: String,
    },
    /// Fetch a published release.
    Download {
        /// Part of where it lands, and what the checksums belong to.
        version: String,
        /// One per platform; the machine's own is chosen when it runs.
        files: Vec<AcquireFile>,
    },
}

/// One platform's download, validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcquireFile {
    pub os: Os,
    pub arch: Arch,
    pub url: String,
    /// Lowercase hex, 64 characters.
    pub sha256: String,
    /// What to take out of the archive; the download itself when absent.
    pub member: Option<String>,
    /// What to call it once it is out.
    pub rename_to: Option<String>,
}

impl Acquire {
    /// What the user is told will happen, in one line.
    pub fn label(&self) -> String {
        match self {
            Acquire::Command { label, .. } => label.clone(),
            Acquire::Download { version, .. } => format!("download version {version}"),
        }
    }

    /// The download for this machine, if this is a download and there is one.
    pub fn file_for(&self, os: Os, arch: Arch) -> Option<&AcquireFile> {
        match self {
            Acquire::Command { .. } => None,
            Acquire::Download { files, .. } => files.iter().find(|f| f.os == os && f.arch == arch),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Register {
    pub setting: String,
    pub value: String,
    /// Sent after the setting changes, if the plugin names one.
    pub reload: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdleShutdown {
    Never,
    After(Duration),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DepsSpec {
    /// Run a command in the playground folder, e.g. `cargo add {name}@{version}`.
    Command { add: Vec<String> },
    /// Insert a line at the top of the main file, e.g. `#:package {name}@{version}`.
    InlineDirective { directive: String },
}

#[derive(Clone, Debug)]
pub struct OutputSpec {
    pub error_regex: Regex,
    pub multiline: bool,
}

impl Plugin {
    /// Load and validate the plugin in `dir` for the current OS.
    pub fn load(dir: impl AsRef<Path>) -> Result<Plugin, ManifestError> {
        Self::load_for(dir, Os::current())
    }

    /// Load and validate the plugin in `dir`, resolving per-OS sections for
    /// `os`. Rules that involve other operating systems (overlapping `os`
    /// lists, for example) are still checked for all of them.
    pub fn load_for(dir: impl AsRef<Path>, os: Os) -> Result<Plugin, ManifestError> {
        let dir = dir.as_ref();
        let file = dir.join(MANIFEST_FILE);
        let source = std::fs::read_to_string(&file).map_err(|source| ManifestError::Io {
            path: file.clone(),
            source,
        })?;

        // Loose pass: the api version and unknown top-level sections.
        let loose: toml::Table =
            toml::from_str(&source).map_err(|e| parse_error(&file, &source, e))?;
        match loose.get("api") {
            Some(toml::Value::Integer(SUPPORTED_API)) => {}
            other => {
                let message = match other {
                    Some(v) => format!(
                        "api = {v} is not supported; this version of TinkerPad reads api = {SUPPORTED_API}"
                    ),
                    None => format!("missing `api = {SUPPORTED_API}` at the top of the file"),
                };
                return Err(ManifestError::Invalid {
                    file,
                    problems: vec![Problem {
                        kind: ProblemKind::UnsupportedApi,
                        key: "api".into(),
                        message,
                    }],
                });
            }
        }
        let mut warnings = Vec::new();
        for key in loose.keys() {
            if !KNOWN_SECTIONS.contains(&key.as_str()) {
                let hint = suggest::nearest(key, KNOWN_SECTIONS)
                    .map(|s| format!("; did you mean `{s}`?"))
                    .unwrap_or_default();
                warnings.push(format!(
                    "unknown top-level section `{key}` is ignored{hint}"
                ));
            }
        }

        // Strict pass.
        let manifest: Manifest =
            toml::from_str(&source).map_err(|e| parse_error(&file, &source, e))?;

        let mut v = Validator {
            dir,
            os,
            problems: Vec::new(),
            warnings,
        };
        let plugin = v.build(manifest);
        if !v.problems.is_empty() {
            return Err(ManifestError::Invalid {
                file,
                problems: v.problems,
            });
        }
        for warning in &plugin.warnings {
            warn!(plugin = %plugin.id, "{warning}");
        }
        Ok(plugin)
    }

    /// The kind marked `default = true`.
    pub fn default_kind(&self) -> &Kind {
        self.kinds
            .iter()
            .find(|k| k.default)
            .expect("validated: exactly one default kind")
    }

    pub fn kind(&self, id: &str) -> Option<&Kind> {
        self.kinds.iter().find(|k| k.id == id)
    }
}

struct Validator<'a> {
    dir: &'a Path,
    os: Os,
    problems: Vec<Problem>,
    warnings: Vec<String>,
}

impl Validator<'_> {
    fn problem(&mut self, kind: ProblemKind, key: impl Into<String>, message: impl Into<String>) {
        self.problems.push(Problem {
            kind,
            key: key.into(),
            message: message.into(),
        });
    }

    fn build(&mut self, mut m: Manifest) -> Plugin {
        let lang = m.language;
        self.id("language.id", &lang.id);
        self.non_empty("language.name", &lang.name);
        if lang.extension.is_empty()
            || lang.extension.starts_with('.')
            || lang.extension.contains(['/', '\\'])
        {
            self.problem(
                ProblemKind::InvalidValue,
                "language.extension",
                format!(
                    "`{}` should be a bare extension such as `cs`, without a dot",
                    lang.extension
                ),
            );
        }
        if let Some(h) = &lang.highlighter
            && !KNOWN_HIGHLIGHTERS.contains(&h.as_str())
        {
            let hint = suggest::nearest(h, KNOWN_HIGHLIGHTERS)
                .map(|s| format!("; did you mean `{s}`?"))
                .unwrap_or_default();
            self.warnings.push(format!(
                "highlighter `{h}` isn't bundled; the editor will show plain text{hint}"
            ));
        }
        let main_path = lang
            .main_path
            .clone()
            .unwrap_or_else(|| format!("main.{}", lang.extension));
        let main_path = self
            .relative_path("language.main_path", &main_path)
            .unwrap_or_default();

        let toolchain_acquire = self.acquire("toolchain.acquire", m.toolchain.acquire.take());
        let (probes, install_hint) = self.toolchain(m.toolchain);
        let kinds = self.kinds(m.kinds);
        let scaffold = self.scaffold(m.scaffold);

        let run = self.per_os("run", m.run.map(manifest::OneOrMany::into_vec), |r| {
            r.os.as_deref()
        });
        let helper = self.per_os("helper", m.helper.map(manifest::OneOrMany::into_vec), |h| {
            h.os.as_deref()
        });
        let lsp = self.per_os("lsp", m.lsp.map(manifest::OneOrMany::into_vec), |l| {
            l.os.as_deref()
        });

        let run_lists: Vec<Option<Vec<Os>>> = run.iter().map(|(_, r, _)| r.os.clone()).collect();
        let helper_lists: Vec<Option<Vec<Os>>> =
            helper.iter().map(|(_, h, _)| h.os.clone()).collect();
        let covers = |lists: &[Option<Vec<Os>>], os: Os| {
            lists
                .iter()
                .any(|l| l.as_ref().is_none_or(|l| l.contains(&os)))
        };
        self.check_runners(&|os| covers(&run_lists, os), &|os| {
            covers(&helper_lists, os)
        });

        let run = self.runs(run);
        let helper = self.helpers(helper);
        let lsp = self.lsps(lsp);

        let deps = m.deps.and_then(|d| self.deps(d));
        let output = m.output.and_then(|o| self.output(o));

        Plugin {
            dir: self.dir.to_path_buf(),
            os: self.os,
            id: LanguageId::new(lang.id),
            name: lang.name,
            extension: lang.extension,
            highlighter: lang.highlighter,
            main_path,
            probes,
            install_hint,
            toolchain_acquire,
            kinds,
            scaffold,
            run,
            helper,
            lsp,
            deps,
            output,
            warnings: std::mem::take(&mut self.warnings),
        }
    }

    fn id(&mut self, key: &str, id: &str) {
        let ok = !id.is_empty()
            && id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            self.problem(
                ProblemKind::InvalidId,
                key,
                format!("`{id}` must use only lowercase letters, digits, and hyphens"),
            );
        }
    }

    /// Check an `acquire` section and turn it into what the app can act on
    /// (work item 04-08a).
    ///
    /// A plugin that gets this wrong fails to load rather than downloading
    /// something unverified: an empty command, a missing checksum, a checksum
    /// that isn't a SHA-256, or a URL that isn't HTTPS are all refusals.
    fn acquire(&mut self, key: &str, acquire: Option<manifest::Acquire>) -> Option<Acquire> {
        match acquire? {
            manifest::Acquire::Command {
                command,
                needs,
                label,
                os,
            } => {
                self.os_list(key, os.as_deref());
                if command.iter().all(|part| part.trim().is_empty()) {
                    self.problem(
                        ProblemKind::InvalidValue,
                        format!("{key}.command"),
                        "needs a program to run, as an array: [\"rustup\", \"component\", \"add\", …]",
                    );
                    return None;
                }
                // A section that applies to no OS here is a section this
                // machine simply doesn't have; the plugin is still fine.
                if os.as_ref().is_some_and(|list| !list.contains(&self.os)) {
                    return None;
                }
                let label = label.unwrap_or_else(|| command.join(" "));
                Some(Acquire::Command {
                    command,
                    needs: needs.filter(|n| !n.trim().is_empty()),
                    label,
                })
            }
            manifest::Acquire::Download { version, file } => {
                self.non_empty(&format!("{key}.version"), &version);
                if file.is_empty() {
                    self.problem(
                        ProblemKind::InvalidValue,
                        format!("{key}.file"),
                        "a download needs at least one [[….file]] with an os, arch, url and sha256",
                    );
                    return None;
                }
                let mut files = Vec::new();
                let mut seen: Vec<(Os, Arch)> = Vec::new();
                for (i, f) in file.into_iter().enumerate() {
                    let at = format!("{key}.file[{i}]");
                    if !f.url.starts_with("https://") {
                        self.problem(
                            ProblemKind::InvalidValue,
                            format!("{at}.url"),
                            format!("`{}` should be an https:// URL", f.url),
                        );
                        continue;
                    }
                    let sha = f.sha256.trim().to_ascii_lowercase();
                    if sha.len() != 64 || !sha.chars().all(|c| c.is_ascii_hexdigit()) {
                        self.problem(
                            ProblemKind::InvalidValue,
                            format!("{at}.sha256"),
                            "should be a SHA-256 as 64 hex characters",
                        );
                        continue;
                    }
                    if seen.contains(&(f.os, f.arch)) {
                        self.problem(
                            ProblemKind::InvalidValue,
                            at.clone(),
                            format!("{:?}/{} is named twice", f.os, f.arch),
                        );
                        continue;
                    }
                    seen.push((f.os, f.arch));
                    if let Some(member) = &f.member
                        && !inside_archive(member)
                    {
                        self.problem(
                            ProblemKind::InvalidValue,
                            format!("{at}.member"),
                            format!("`{member}` must be a relative path inside the archive"),
                        );
                        continue;
                    }
                    files.push(AcquireFile {
                        os: f.os,
                        arch: f.arch,
                        url: f.url,
                        sha256: sha,
                        member: f.member,
                        rename_to: f.rename_to,
                    });
                }
                if files.is_empty() {
                    return None;
                }
                Some(Acquire::Download { version, files })
            }
        }
    }

    fn non_empty(&mut self, key: &str, value: &str) {
        if value.trim().is_empty() {
            self.problem(ProblemKind::InvalidValue, key, "must not be empty");
        }
    }

    fn relative_path(&mut self, key: &str, raw: &str) -> Option<PathBuf> {
        let normalised = raw.replace('\\', "/");
        let path = PathBuf::from(&normalised);
        // Checked textually as well as with Path, so a Windows drive path is
        // rejected when validating on Unix and vice versa.
        let drive = normalised.as_bytes().get(1) == Some(&b':');
        let bad = normalised.is_empty()
            || path.is_absolute()
            || normalised.starts_with('/')
            || drive
            || path.components().any(|c| {
                matches!(
                    c,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            });
        if bad {
            self.problem(
                ProblemKind::InvalidPath,
                key,
                format!("`{raw}` must be a relative path that stays inside its folder"),
            );
            None
        } else {
            Some(path)
        }
    }

    fn placeholders(&mut self, key: &str, value: &str, allowed: &[&str]) {
        match placeholders::validate(value, allowed) {
            Ok(()) => {}
            Err(PlaceholderError::Unknown { name }) => {
                let hint = suggest::nearest(&name, allowed)
                    .map(|s| format!("did you mean `{{{s}}}`? "))
                    .unwrap_or_default();
                let list = allowed
                    .iter()
                    .map(|a| format!("{{{a}}}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                self.problem(
                    ProblemKind::UnknownPlaceholder,
                    key,
                    format!("unknown placeholder `{{{name}}}`; {hint}allowed here: {list}"),
                );
            }
            Err(e @ PlaceholderError::Malformed { .. }) => {
                self.problem(ProblemKind::UnknownPlaceholder, key, e.to_string());
            }
        }
    }

    fn idle_shutdown(&mut self, key: &str, raw: Option<&str>) -> IdleShutdown {
        let Some(raw) = raw else {
            return IdleShutdown::Never;
        };
        match parse_idle(raw) {
            Some(idle) => idle,
            None => {
                self.problem(
                    ProblemKind::InvalidValue,
                    key,
                    format!(
                        "`{raw}` should be \"never\" or a duration such as \"90s\", \"30m\", \"2h\""
                    ),
                );
                IdleShutdown::Never
            }
        }
    }

    fn os_list(&mut self, key: &str, os: Option<&[Os]>) {
        if os.is_some_and(<[Os]>::is_empty) {
            self.problem(
                ProblemKind::OsConflict,
                format!("{key}.os"),
                "an empty `os` list applies nowhere; remove it or name at least one OS",
            );
        }
    }

    fn toolchain(&mut self, t: manifest::Toolchain) -> (Vec<Probe>, Option<String>) {
        let mut applying = Vec::new();
        let mut covered: HashSet<Os> = HashSet::new();
        for (i, p) in t.probe.into_iter().enumerate() {
            let key = format!("toolchain.probe[{i}]");
            self.os_list(&key, p.os.as_deref());
            self.non_empty(&format!("{key}.command"), &p.command);
            self.placeholders(&format!("{key}.command"), &p.command, placeholders::PROBE);
            for (j, a) in p.args.iter().enumerate() {
                self.placeholders(&format!("{key}.args[{j}]"), a, placeholders::PROBE);
            }
            let min_version = p.min_version.as_deref().and_then(|raw| {
                let v = Version::parse(raw);
                if v.is_none() {
                    self.problem(
                        ProblemKind::InvalidValue,
                        format!("{key}.min_version"),
                        format!("`{raw}` should be a dotted version such as \"10.0\""),
                    );
                }
                v
            });
            let applies_to = |os: Os| p.os.as_ref().is_none_or(|list| list.contains(&os));
            for os in Os::ALL {
                if applies_to(os) {
                    covered.insert(os);
                }
            }
            if applies_to(self.os) {
                applying.push(Probe {
                    command: p.command,
                    args: p.args,
                    min_version,
                });
            }
        }
        for os in Os::ALL {
            if covered.contains(&os) {
                continue;
            }
            if os == self.os {
                self.problem(
                    ProblemKind::NoProbe,
                    "toolchain.probe",
                    format!("no probe applies on {os}; add one without `os`, or list {os}"),
                );
            } else {
                self.warnings
                    .push(format!("no toolchain probe applies on {os}"));
            }
        }
        (applying, t.install_hint)
    }

    fn kinds(&mut self, raw: Vec<manifest::Kind>) -> Vec<Kind> {
        if raw.is_empty() {
            self.problem(
                ProblemKind::NoKinds,
                "kind",
                "add at least one [[kind]] (for example a \"statements\" kind)",
            );
            return Vec::new();
        }
        let defaults = raw.iter().filter(|k| k.default).count();
        if defaults != 1 {
            let message = if defaults == 0 {
                "mark exactly one kind with `default = true`".to_owned()
            } else {
                format!("{defaults} kinds are marked `default = true`; mark exactly one")
            };
            self.problem(ProblemKind::DefaultKind, "kind", message);
        }
        let mut seen = HashSet::new();
        let templates = self.dir.join(TEMPLATES_DIR);
        raw.into_iter()
            .enumerate()
            .map(|(i, k)| {
                let key = format!("kind[{i}]");
                self.id(&format!("{key}.id"), &k.id);
                if !seen.insert(k.id.clone()) {
                    self.problem(
                        ProblemKind::DuplicateKind,
                        format!("{key}.id"),
                        format!("another kind already uses id `{}`", k.id),
                    );
                }
                self.non_empty(&format!("{key}.name"), &k.name);
                let template = match self.relative_path(&format!("{key}.template"), &k.template) {
                    Some(rel) => {
                        let path = templates.join(&rel);
                        if !path.is_file() {
                            self.problem(
                                ProblemKind::MissingTemplate,
                                format!("{key}.template"),
                                format!("{TEMPLATES_DIR}/{} does not exist", k.template),
                            );
                        }
                        path
                    }
                    None => PathBuf::new(),
                };
                Kind {
                    id: k.id,
                    name: k.name,
                    template,
                    default: k.default,
                    prelude: k.prelude,
                    postlude: k.postlude,
                }
            })
            .collect()
    }

    fn scaffold(&mut self, raw: manifest::Scaffold) -> Vec<ScaffoldFile> {
        self.template_files("scaffold.files", raw.files)
    }

    /// A list of `path` + `template` pairs: where to write it, and what to
    /// write there. Used by `[scaffold].files` and `[lsp].workspace_files`.
    fn template_files(
        &mut self,
        section: &str,
        files: Vec<manifest::ScaffoldFile>,
    ) -> Vec<ScaffoldFile> {
        let templates = self.dir.join(TEMPLATES_DIR);
        files
            .into_iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let key = format!("{section}[{i}]");
                let path = self.relative_path(&format!("{key}.path"), &f.path);
                let template = self
                    .relative_path(&format!("{key}.template"), &f.template)
                    .map(|rel| templates.join(rel));
                if let Some(t) = &template
                    && !t.is_file()
                {
                    self.problem(
                        ProblemKind::MissingScaffoldSource,
                        format!("{key}.template"),
                        format!("{TEMPLATES_DIR}/{} does not exist", f.template),
                    );
                }
                Some(ScaffoldFile {
                    path: path?,
                    template: template?,
                })
            })
            .collect()
    }

    /// Choose the variant of a per-OS section for `self.os`, checking every OS
    /// for overlaps. Returns the chosen variant and its key path.
    fn per_os<T>(
        &mut self,
        section: &str,
        variants: Option<Vec<T>>,
        os_of: impl Fn(&T) -> Option<&[Os]>,
    ) -> Vec<(String, T, bool)> {
        let Some(variants) = variants else {
            return Vec::new();
        };
        let single = variants.len() == 1;
        let keyed: Vec<(String, T)> = variants
            .into_iter()
            .enumerate()
            .map(|(i, v)| {
                let key = if single {
                    section.to_owned()
                } else {
                    format!("{section}[{i}]")
                };
                (key, v)
            })
            .collect();

        for (key, v) in &keyed {
            self.os_list(key, os_of(v));
        }
        let generic = keyed.iter().filter(|(_, v)| os_of(v).is_none()).count();
        if generic > 1 {
            self.problem(
                ProblemKind::OsConflict,
                section,
                format!(
                    "{generic} [[{section}]] entries have no `os` list, so it's ambiguous which applies; give all but one an `os` list"
                ),
            );
        }
        for os in Os::ALL {
            let specific = keyed
                .iter()
                .filter(|(_, v)| os_of(v).is_some_and(|l| l.contains(&os)))
                .count();
            if specific > 1 {
                self.problem(
                    ProblemKind::OsConflict,
                    section,
                    format!("{specific} [[{section}]] entries list `{os}`; each OS may appear in only one"),
                );
            }
        }

        // Every variant is returned for field validation; the flag marks the
        // one chosen for this OS (a specific match beats a generic one).
        let target = self.os;
        let chosen = keyed
            .iter()
            .position(|(_, v)| os_of(v).is_some_and(|l| l.contains(&target)))
            .or_else(|| keyed.iter().position(|(_, v)| os_of(v).is_none()));
        keyed
            .into_iter()
            .enumerate()
            .map(|(i, (key, v))| (key, v, Some(i) == chosen))
            .collect()
    }

    fn command(
        &mut self,
        key: &str,
        command: String,
        args: Vec<String>,
        cwd: Option<String>,
        env: BTreeMap<String, String>,
    ) -> CommandSpec {
        self.non_empty(&format!("{key}.command"), &command);
        self.placeholders(&format!("{key}.command"), &command, placeholders::COMMAND);
        for (i, a) in args.iter().enumerate() {
            self.placeholders(&format!("{key}.args[{i}]"), a, placeholders::COMMAND);
        }
        if let Some(cwd) = &cwd {
            self.placeholders(&format!("{key}.cwd"), cwd, placeholders::COMMAND);
        }
        for (k, v) in &env {
            self.non_empty(&format!("{key}.env"), k);
            self.placeholders(&format!("{key}.env.{k}"), v, placeholders::COMMAND);
        }
        CommandSpec {
            command,
            args,
            cwd,
            env,
        }
    }

    fn runs(&mut self, variants: Vec<(String, manifest::Run, bool)>) -> Option<CommandSpec> {
        let mut chosen = None;
        for (key, r, is_chosen) in variants {
            let spec = self.command(&key, r.command, r.args, r.cwd, r.env);
            if is_chosen {
                chosen = Some(spec);
            }
        }
        chosen
    }

    fn helpers(&mut self, variants: Vec<(String, manifest::Helper, bool)>) -> Option<HelperSpec> {
        let mut chosen = None;
        for (key, h, is_chosen) in variants {
            let command = self.command(&key, h.command, h.args, None, h.env);
            let build = h.build.map(|b| {
                self.command(
                    &format!("{key}.build"),
                    b.command,
                    b.args,
                    b.cwd,
                    BTreeMap::new(),
                )
            });
            for hook in &h.hooks {
                if !KNOWN_HOOKS.contains(&hook.as_str()) {
                    let hint = suggest::nearest(hook, KNOWN_HOOKS)
                        .map(|s| format!("; did you mean `{s}`?"))
                        .unwrap_or_default();
                    self.warnings.push(format!(
                        "{key}.hooks: `{hook}` isn't a hook this version sends{hint}"
                    ));
                }
            }
            let idle_shutdown =
                self.idle_shutdown(&format!("{key}.idle_shutdown"), h.idle_shutdown.as_deref());
            let spec = HelperSpec {
                command,
                build,
                hooks: h.hooks,
                idle_shutdown,
            };
            if is_chosen {
                chosen = Some(spec);
            }
        }
        chosen
    }

    fn lsps(&mut self, variants: Vec<(String, manifest::Lsp, bool)>) -> Option<LspSpec> {
        let mut chosen = None;
        for (key, mut l, is_chosen) in variants {
            let command = self.command(&key, l.command, l.args, None, l.env);
            let idle_shutdown =
                self.idle_shutdown(&format!("{key}.idle_shutdown"), l.idle_shutdown.as_deref());
            let init_options = l
                .init_options
                .map(|t| serde_json::to_value(t).expect("TOML tables always convert to JSON"));
            let register = l.register.map(|r| {
                self.non_empty(&format!("{key}.register.setting"), &r.setting);
                self.placeholders(
                    &format!("{key}.register.value"),
                    &r.value,
                    placeholders::COMMAND,
                );
                Register {
                    setting: r.setting,
                    value: r.value,
                    reload: r.reload,
                }
            });
            let workspace_files = self.template_files(
                &format!("{key}.workspace_files"),
                std::mem::take(&mut l.workspace_files),
            );
            let spec = LspSpec {
                command,
                acquire: self.acquire(&format!("{key}.acquire"), l.acquire.take()),
                workspace: l.workspace.unwrap_or(Workspace::PlaygroundDir),
                idle_shutdown,
                init_options,
                register,
                workspace_files,
            };
            if is_chosen {
                chosen = Some(spec);
            }
        }
        chosen
    }

    fn deps(&mut self, d: manifest::Deps) -> Option<DepsSpec> {
        match (d.kind, d.add, d.directive) {
            (manifest::DepsKind::Command, Some(add), None) if !add.is_empty() => {
                for (i, a) in add.iter().enumerate() {
                    self.placeholders(&format!("deps.add[{i}]"), a, placeholders::DEPS);
                }
                Some(DepsSpec::Command { add })
            }
            (manifest::DepsKind::InlineDirective, None, Some(directive)) => {
                self.non_empty("deps.directive", &directive);
                self.placeholders("deps.directive", &directive, placeholders::DEPS);
                Some(DepsSpec::InlineDirective { directive })
            }
            (manifest::DepsKind::Command, ..) => {
                self.problem(
                    ProblemKind::DepsShape,
                    "deps",
                    "kind = \"command\" needs a non-empty `add` list and no `directive`",
                );
                None
            }
            (manifest::DepsKind::InlineDirective, ..) => {
                self.problem(
                    ProblemKind::DepsShape,
                    "deps",
                    "kind = \"inline-directive\" needs `directive` and no `add`",
                );
                None
            }
        }
    }

    fn output(&mut self, o: manifest::Output) -> Option<OutputSpec> {
        let regex = match Regex::new(&o.error_regex) {
            Ok(r) => r,
            Err(e) => {
                self.problem(
                    ProblemKind::InvalidRegex,
                    "output.error_regex",
                    format!("doesn't compile: {e}"),
                );
                return None;
            }
        };
        let names: Vec<&str> = regex.capture_names().flatten().collect();
        for required in ["line", "message"] {
            if !names.contains(&required) {
                self.problem(
                    ProblemKind::MissingRegexGroup,
                    "output.error_regex",
                    format!("needs a named group `(?<{required}>…)`"),
                );
            }
        }
        for name in &names {
            if !REGEX_GROUPS.contains(name) {
                self.warnings.push(format!(
                    "output.error_regex: group `{name}` is ignored; recognised groups are file, line, col, message, severity"
                ));
            }
        }
        Some(OutputSpec {
            error_regex: regex,
            multiline: o.error_regex_multiline,
        })
    }
}

impl Validator<'_> {
    /// After building: neither a run command nor a helper on some OS.
    fn check_runners(&mut self, run_os: &dyn Fn(Os) -> bool, helper_os: &dyn Fn(Os) -> bool) {
        for os in Os::ALL {
            if run_os(os) || helper_os(os) {
                continue;
            }
            if os == self.os {
                self.problem(
                    ProblemKind::NoRunner,
                    "run",
                    format!("neither [run] nor [helper] applies on {os}, so playgrounds can't run"),
                );
            } else {
                self.warnings.push(format!(
                    "neither [run] nor [helper] applies on {os}; the plugin won't run there"
                ));
            }
        }
    }
}

fn parse_idle(raw: &str) -> Option<IdleShutdown> {
    if raw == "never" {
        return Some(IdleShutdown::Never);
    }
    let (number, unit) = raw.split_at(raw.len().checked_sub(1)?);
    let n: u64 = number.parse().ok().filter(|n| *n > 0)?;
    let secs = match unit {
        "s" => n,
        "m" => n.checked_mul(60)?,
        "h" => n.checked_mul(3600)?,
        _ => return None,
    };
    Some(IdleShutdown::After(Duration::from_secs(secs)))
}

/// Turn a TOML error into a [`ManifestError`] with a location, the section it
/// happened in, and a suggestion for unknown keys and values.
fn parse_error(file: &Path, source: &str, err: toml::de::Error) -> ManifestError {
    let offset = err.span().map(|s| s.start).unwrap_or(0);
    let (line, column) = line_col(source, offset);
    let location = Location {
        file: file.to_path_buf(),
        line,
        column,
        section: section_at(source, offset),
    };
    let message = err.message().trim().to_owned();

    let unknown = |rest: &str| {
        let (name, expected) = split_unknown(rest);
        let suggestion = nearest_owned(&name, &expected);
        Box::new(Unknown {
            location: location.clone(),
            name,
            expected,
            suggestion,
        })
    };
    if let Some(rest) = message.strip_prefix("unknown field `") {
        return ManifestError::UnknownKey(unknown(rest));
    }
    if let Some(rest) = message.strip_prefix("unknown variant `") {
        return ManifestError::UnknownValue(unknown(rest));
    }
    ManifestError::Syntax(Box::new(Syntax { location, message }))
}

/// Split serde's "`name`, expected one of `a`, `b`" (after the opening
/// backtick) into the name and the expected list.
fn split_unknown(rest: &str) -> (String, Vec<String>) {
    let (name, tail) = rest.split_once('`').unwrap_or((rest, ""));
    let expected = tail
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect();
    (name.to_owned(), expected)
}

fn nearest_owned(word: &str, candidates: &[String]) -> Option<String> {
    let refs: Vec<&str> = candidates.iter().map(String::as_str).collect();
    suggest::nearest(word, &refs).map(str::to_owned)
}

/// 1-based line and column of a byte offset.
fn line_col(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let before = &source[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rfind('\n').map_or(before.chars().count(), |nl| {
        before[nl + 1..].chars().count()
    }) + 1;
    (line, column)
}

/// The `[section]` or `[[section]]` header governing a byte offset.
fn section_at(source: &str, offset: usize) -> Option<String> {
    let mut current = None;
    let mut pos = 0;
    for line in source.split_inclusive('\n') {
        if pos > offset {
            break;
        }
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            let header = trimmed
                .split('#')
                .next()
                .unwrap_or("")
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim();
            current = Some(header.to_owned());
        }
        pos += line.len();
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_durations() {
        assert_eq!(parse_idle("never"), Some(IdleShutdown::Never));
        assert_eq!(
            parse_idle("90s"),
            Some(IdleShutdown::After(Duration::from_secs(90)))
        );
        assert_eq!(
            parse_idle("30m"),
            Some(IdleShutdown::After(Duration::from_secs(1800)))
        );
        assert_eq!(
            parse_idle("2h"),
            Some(IdleShutdown::After(Duration::from_secs(7200)))
        );
        for bad in ["", "m", "0m", "30", "30d", "-5m", "forever"] {
            assert_eq!(parse_idle(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn versions() {
        assert_eq!(Version::parse("10.0"), Some(Version(vec![10, 0])));
        assert_eq!(Version::parse("22.6.1"), Some(Version(vec![22, 6, 1])));
        assert!(Version::parse("10.0") < Version::parse("10.1"));
        for bad in ["", "v10", "10.x", "1.2.3.4.5"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn locations() {
        let src = "a = 1\n[lsp]\nidle = 2\n";
        assert_eq!(line_col(src, 0), (1, 1));
        assert_eq!(line_col(src, 12), (3, 1));
        assert_eq!(line_col(src, 14), (3, 3));
        assert_eq!(section_at(src, 2), None);
        assert_eq!(section_at(src, 12), Some("lsp".into()));
        assert_eq!(section_at("[[run]] # win\nx = 1", 15), Some("run".into()));
    }

    #[test]
    fn unknown_field_messages() {
        let (name, expected) = split_unknown("idle_shutdwon`, expected one of `command`, `args`");
        assert_eq!(name, "idle_shutdwon");
        assert_eq!(expected, vec!["command", "args"]);
        let (name, expected) = split_unknown("x`, expected `a` or `b`");
        assert_eq!(name, "x");
        assert_eq!(expected, vec!["a", "b"]);
        let (_, expected) = split_unknown("x`, there are no fields");
        assert!(expected.is_empty());
    }
}
