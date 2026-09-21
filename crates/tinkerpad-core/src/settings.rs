//! What the user can change: the schema of `settings.toml` (work item 04-02).
//!
//! Every field has a default, so a missing file, a missing section, and a
//! missing key all mean the same thing: whatever TinkerPad would have done
//! anyway. Nothing here reads a file — the app owns that, along with watching
//! it and saying what is wrong with it — but the shape, the defaults, and the
//! comments that end up in the file all live here, so the schema is one thing
//! in one place.
//!
//! Unknown keys are kept rather than rejected: a settings file written by a
//! newer build, or by a plugin that hasn't been installed yet, should survive
//! being read and written by this one.
//!
//! Nothing here skips a field when it serializes: this type is what the
//! settings window is handed, and a window missing half its fields is worse
//! than a JSON object with some nulls in it. What the *file* holds is decided
//! when it is written, key by key, by the app.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Everything in `settings.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct Settings {
    /// The colour theme, by name. Real themes arrive in 04-04; until then
    /// anything but `dark` falls back to it.
    pub theme: String,
    pub library: LibrarySettings,
    pub editor: EditorSettings,
    pub run: RunSettings,
    /// Per plugin: the programs it should use, by the name the plugin's
    /// `[toolchain]` probe asks for.
    pub toolchains: BTreeMap<String, BTreeMap<String, String>>,
    pub lsp: LspSettings,
    /// Per plugin: how long its warm helper may sit idle.
    pub helpers: BTreeMap<String, ServiceOverride>,
}

/// Where playgrounds live.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LibrarySettings {
    /// Absent means `TinkerPad` in the user's home folder.
    pub root: Option<PathBuf>,
}

/// The editor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct EditorSettings {
    /// Points. Clamped to something readable when it is applied.
    pub font_size: u16,
    /// Absent means the platform's monospace stack.
    pub font_family: Option<String>,
    /// How wide a tab is, and how many spaces indenting inserts.
    pub tab_size: u8,
}

/// Running code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct RunSettings {
    /// Empty the output pane when a run starts.
    pub clear_output: bool,
}

/// Language servers: which to start early, and how long each may idle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct LspSettings {
    /// Languages whose server starts once the window is up.
    pub prewarm: Vec<String>,
    /// Per language, under `[lsp.<id>]`.
    #[serde(flatten)]
    pub languages: BTreeMap<String, ServiceOverride>,
}

/// What can be said about one long-lived child process.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "snake_case")]
pub struct ServiceOverride {
    /// `"never"`, or a duration such as `"30m"`. Overrides the plugin's own.
    pub idle_shutdown: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: Self::DEFAULT_THEME.to_owned(),
            library: LibrarySettings::default(),
            editor: EditorSettings::default(),
            run: RunSettings::default(),
            toolchains: BTreeMap::new(),
            lsp: LspSettings::default(),
            helpers: BTreeMap::new(),
        }
    }
}

impl Settings {
    /// The only theme there is until 04-04.
    pub const DEFAULT_THEME: &'static str = "dark";

    /// How big the editor's text may be, whatever the file says.
    pub const FONT_SIZE: std::ops::RangeInclusive<u16> = 8..=40;
    /// How wide a tab may be.
    pub const TAB_SIZE: std::ops::RangeInclusive<u8> = 1..=16;

    /// The editor's font size, kept inside [`Settings::FONT_SIZE`]. A file
    /// asking for 2-point text is a file asking for a mistake.
    pub fn font_size(&self) -> u16 {
        self.editor
            .font_size
            .clamp(*Self::FONT_SIZE.start(), *Self::FONT_SIZE.end())
    }

    /// The editor's tab size, kept inside [`Settings::TAB_SIZE`].
    pub fn tab_size(&self) -> u8 {
        self.editor
            .tab_size
            .clamp(*Self::TAB_SIZE.start(), *Self::TAB_SIZE.end())
    }

    /// The program overrides for one plugin: probe name to path.
    pub fn toolchain(&self, plugin: &str) -> Option<&BTreeMap<String, String>> {
        self.toolchains.get(plugin)
    }
}

impl Default for EditorSettings {
    fn default() -> Self {
        Self {
            font_size: 13,
            font_family: None,
            tab_size: 2,
        }
    }
}

impl Default for RunSettings {
    fn default() -> Self {
        Self { clear_output: true }
    }
}

/// The file TinkerPad writes on first launch: every setting, at its default,
/// commented out, with a line saying what it does.
///
/// Written rather than generated from the struct because a good settings file
/// reads like prose and a derived one reads like a schema. The test below
/// keeps it honest: what this template says the defaults are has to be what
/// the defaults are.
pub const TEMPLATE: &str = r#"# TinkerPad settings.
#
# Everything here is optional: a line you delete goes back to its default.
# Changes apply as soon as you save, except `library.root`, which asks first.

# The colour theme. Only "dark" so far.
# theme = "dark"

[editor]
# Text size in points, between 8 and 40.
# font_size = 13
# A font family for code. Left out, TinkerPad uses the platform's own.
# font_family = "Cascadia Code"
# How wide a tab is, and how many spaces indenting inserts (1 to 16).
# tab_size = 2

[run]
# Empty the output pane when a run starts.
# clear_output = true

[library]
# Where playgrounds are stored. Default: a TinkerPad folder in your home folder.
# root = "D:/Code/TinkerPad"

# Program overrides per plugin. When set, TinkerPad uses this path instead of
# searching PATH, both when checking the toolchain and when running code.
#
# [toolchains.csharp]
# dotnet = "C:/Program Files/dotnet/dotnet.exe"
#
# [toolchains.python]
# python3 = "/opt/homebrew/bin/python3"

[lsp]
# Languages whose server starts in the background once the window is up, so the
# first tab of the day is as quick as the hundredth.
# prewarm = ["typescript"]

# A server stops on its own once it has had no open files for a while; the
# plugin decides how long, and you can override it per language with "never"
# or a duration such as "30m".
#
# [lsp.rust]
# idle_shutdown = "30m"

# The same, for a plugin's warm helper (the process that runs your code).
#
# [helpers.csharp]
# idle_shutdown = "never"
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn everything_has_a_default() {
        let settings = Settings::default();
        assert_eq!(settings.theme, "dark");
        assert_eq!(settings.library.root, None);
        assert_eq!(settings.font_size(), 13);
        assert_eq!(settings.tab_size(), 2);
        assert!(settings.run.clear_output);
        assert!(settings.toolchains.is_empty());
        assert!(settings.lsp.prewarm.is_empty());
        assert!(settings.helpers.is_empty());
    }

    #[test]
    fn silly_sizes_are_brought_back_to_something_readable() {
        let mut settings = Settings::default();
        settings.editor.font_size = 2;
        settings.editor.tab_size = 100;
        assert_eq!(settings.font_size(), 8);
        assert_eq!(settings.tab_size(), 16);
    }

    /// The template's settings lines, uncommented, ready to parse. Its prose
    /// lines are dropped; `examples` says whether the per-plugin sections
    /// (which name plugins that may not be installed) come along.
    fn uncommented(examples: bool) -> String {
        let mut text = String::new();
        let mut in_example = false;
        for line in TEMPLATE.lines() {
            let line = line.trim_start_matches("# ").trim_start_matches('#').trim();
            if line.starts_with('[') {
                in_example = line.starts_with("[toolchains")
                    || line.starts_with("[lsp.")
                    || line.starts_with("[helpers");
            }
            let wanted = examples || !in_example;
            if !wanted || !(line.contains('=') || line.starts_with('[')) {
                continue;
            }
            text.push_str(line);
            text.push('\n');
        }
        text
    }

    #[test]
    fn the_template_says_what_the_defaults_are() {
        let text = uncommented(false);
        let parsed: Settings = toml::from_str(&text).expect(&text);
        let expected = Settings {
            editor: EditorSettings {
                font_family: Some("Cascadia Code".to_owned()),
                ..EditorSettings::default()
            },
            library: LibrarySettings {
                root: Some(PathBuf::from("D:/Code/TinkerPad")),
            },
            lsp: LspSettings {
                prewarm: vec!["typescript".to_owned()],
                ..LspSettings::default()
            },
            ..Settings::default()
        };
        assert_eq!(parsed, expected, "in:\n{text}");
    }

    #[test]
    fn the_per_plugin_examples_in_the_template_are_the_shape_they_claim() {
        let text = uncommented(true);
        let settings: Settings = toml::from_str(&text).expect(&text);
        assert_eq!(
            settings.toolchain("csharp").and_then(|t| t.get("dotnet")),
            Some(&"C:/Program Files/dotnet/dotnet.exe".to_owned()),
        );
        assert_eq!(
            settings.lsp.languages["rust"].idle_shutdown.as_deref(),
            Some("30m"),
        );
        assert_eq!(
            settings.helpers["csharp"].idle_shutdown.as_deref(),
            Some("never"),
        );
    }

    #[test]
    fn a_file_from_a_newer_build_still_reads() {
        // Unknown keys are not an error: the user keeps their file, and this
        // build keeps the settings it knows.
        let settings: Settings = toml::from_str(
            "theme = \"dark\"\nsomething_new = 3\n\n[editor]\nfont_size = 16\nwrap = true\n",
        )
        .expect("unknown keys");
        assert_eq!(settings.font_size(), 16);
        assert_eq!(settings.theme, "dark");
    }
}
