//! The `[lsp]` part of `settings.toml` (work item 02-06).
//!
//! ```toml
//! [lsp]
//! # Languages to start at launch, so the first tab is already warm.
//! prewarm = ["typescript", "rust"]
//!
//! [lsp.rust]
//! # Overrides the plugin's own `idle_shutdown`.
//! idle_shutdown = "30m"
//! ```
//!
//! Anything wrong in here is a warning, never a failure to launch: a typo in a
//! duration should cost you the setting, not the app.

use std::collections::BTreeMap;
use std::time::Duration;

use tracing::warn;

/// What `settings.toml` says about language servers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LspSettings {
    /// Language ids to start at launch, in order.
    pub prewarm: Vec<String>,
    /// Per language: how long it may sit idle. `None` inside the map is
    /// "never", which is different from having no entry at all.
    pub idle_shutdown: BTreeMap<String, Option<Duration>>,
}

/// Overrides the pre-warm list, for tests and for automation that wants a
/// known set of servers up. Comma-separated language ids.
pub const PREWARM_ENV: &str = "TINKERPAD_LSP_PREWARM";

impl LspSettings {
    /// Read `[lsp]` from a settings file, then let [`PREWARM_ENV`] override the
    /// pre-warm list.
    pub fn load(text: &str) -> Self {
        let mut settings = Self::from_settings(text);
        if let Ok(list) = std::env::var(PREWARM_ENV) {
            settings.prewarm = list
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect();
        }
        settings
    }

    /// Read `[lsp]` out of a settings file. A missing section is no settings.
    pub fn from_settings(text: &str) -> Self {
        let table: toml::Table = match toml::from_str(text) {
            Ok(table) => table,
            Err(error) => {
                warn!(%error, "settings.toml could not be read; using defaults");
                return Self::default();
            }
        };
        let Some(lsp) = table.get("lsp").and_then(toml::Value::as_table) else {
            return Self::default();
        };

        let mut settings = Self::default();
        for (key, value) in lsp {
            match (key.as_str(), value) {
                ("prewarm", toml::Value::Array(languages)) => {
                    for language in languages {
                        match language.as_str() {
                            Some(id) => settings.prewarm.push(id.to_owned()),
                            None => warn!("[lsp] prewarm should be a list of language ids"),
                        }
                    }
                }
                ("prewarm", _) => {
                    warn!(r#"[lsp] prewarm should be a list, e.g. ["typescript"]"#)
                }
                // `[lsp.<language>]`, a table of that language's settings.
                (language, toml::Value::Table(per_language)) => {
                    for (key, value) in per_language {
                        match key.as_str() {
                            "idle_shutdown" => match value.as_str() {
                                Some(raw) => match parse_idle(raw) {
                                    Some(idle) => {
                                        settings.idle_shutdown.insert(language.to_owned(), idle);
                                    }
                                    None => warn!(
                                        language,
                                        value = raw,
                                        r#"[lsp.{language}] idle_shutdown should be "never" or a duration like "30m""#
                                    ),
                                },
                                None => warn!(language, "idle_shutdown should be a string"),
                            },
                            other => warn!(language, key = other, "unknown [lsp] setting"),
                        }
                    }
                }
                (other, _) => warn!(key = other, "unknown [lsp] setting"),
            }
        }
        settings
    }

    /// How long this language's server may idle: the user's setting if there
    /// is one, otherwise the plugin's.
    pub fn idle_for(&self, language: &str, from_plugin: Option<Duration>) -> Option<Duration> {
        match self.idle_shutdown.get(language) {
            Some(idle) => *idle,
            None => from_plugin,
        }
    }
}

/// `"never"`, or a duration like `"90s"`, `"30m"`, `"2h"` — the same spellings
/// a plugin manifest uses.
fn parse_idle(raw: &str) -> Option<Option<Duration>> {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("never") {
        return Some(None);
    }
    let (digits, unit) = raw.split_at(raw.find(|c: char| !c.is_ascii_digit())?);
    let amount: u64 = digits.parse().ok()?;
    let seconds = match unit {
        "s" => amount,
        "m" => amount * 60,
        "h" => amount * 3600,
        _ => return None,
    };
    Some(Some(Duration::from_secs(seconds)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_prewarm_and_per_language_idle() {
        let settings = LspSettings::from_settings(
            r#"
[lsp]
prewarm = ["typescript", "rust"]

[lsp.rust]
idle_shutdown = "30m"

[lsp.typescript]
idle_shutdown = "never"
"#,
        );
        assert_eq!(settings.prewarm, ["typescript", "rust"]);
        assert_eq!(
            settings.idle_for("rust", None),
            Some(Duration::from_secs(1800)),
        );
        assert_eq!(
            settings.idle_for("typescript", Some(Duration::from_secs(60))),
            None,
            "an explicit `never` overrides the plugin",
        );
        assert_eq!(
            settings.idle_for("csharp", Some(Duration::from_secs(60))),
            Some(Duration::from_secs(60)),
            "and a language with no setting keeps the plugin's",
        );
    }

    #[test]
    fn a_file_without_an_lsp_section_is_no_settings() {
        let settings = LspSettings::from_settings("[toolchains.csharp]\ndotnet = \"x\"\n");
        assert_eq!(settings, LspSettings::default());
    }

    #[test]
    fn nonsense_costs_the_setting_and_nothing_else() {
        let settings = LspSettings::from_settings(
            r#"
[lsp]
prewarm = "typescript"

[lsp.rust]
idle_shutdown = "half an hour"
mystery = 1
"#,
        );
        assert!(settings.prewarm.is_empty(), "a string is not a list");
        assert_eq!(
            settings.idle_for("rust", Some(Duration::from_secs(5))),
            Some(Duration::from_secs(5)),
            "the plugin's value stands when the override can't be read",
        );
    }

    #[test]
    fn durations_are_the_same_spellings_a_manifest_uses() {
        assert_eq!(parse_idle("never"), Some(None));
        assert_eq!(parse_idle("90s"), Some(Some(Duration::from_secs(90))));
        assert_eq!(parse_idle("30m"), Some(Some(Duration::from_secs(1800))));
        assert_eq!(parse_idle("2h"), Some(Some(Duration::from_secs(7200))));
        assert_eq!(parse_idle("2 h"), None);
        assert_eq!(parse_idle("soon"), None);
        assert_eq!(parse_idle("10"), None);
    }
}
