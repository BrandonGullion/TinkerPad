//! The `[lsp]` and `[helpers]` parts of `settings.toml` (work items 02-06,
//! 03-02).
//!
//! ```toml
//! [lsp]
//! # Languages to start at launch, so the first tab is already warm.
//! prewarm = ["typescript", "rust"]
//!
//! [lsp.rust]
//! # Overrides the plugin's own `idle_shutdown`.
//! idle_shutdown = "30m"
//!
//! [helpers.csharp]
//! # The same, for a plugin's warm helper.
//! idle_shutdown = "never"
//! ```
//!
//! The two sections have the same shape because the two things have the same
//! life (`tinkerpad_process::supervise`), so one reader serves both. The text
//! is parsed once into `tinkerpad_core::Settings` (work item 04-02); what is
//! left here is turning what it says into what a supervisor needs.
//!
//! Anything wrong in here is a warning, never a failure to launch: a typo in a
//! duration should cost you the setting, not the app.

use std::collections::BTreeMap;
use std::time::Duration;

use tracing::warn;

/// What `settings.toml` says about one kind of long-lived child: language
/// servers under `[lsp]`, plugin helpers under `[helpers]`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ServiceSettings {
    /// Language ids to start at launch, in order.
    pub prewarm: Vec<String>,
    /// Per language: how long it may sit idle. `None` inside the map is
    /// "never", which is different from having no entry at all.
    pub idle_shutdown: BTreeMap<String, Option<Duration>>,
}

/// Overrides the pre-warm list, for tests and for automation that wants a
/// known set of servers up. Comma-separated language ids.
pub const PREWARM_ENV: &str = "TINKERPAD_LSP_PREWARM";

/// The `settings.toml` section a kind of child is configured under.
pub const LSP: &str = "lsp";
pub const HELPERS: &str = "helpers";

impl ServiceSettings {
    /// The `[lsp]` part of already-parsed settings (work item 04-02), with
    /// [`PREWARM_ENV`] still able to override the pre-warm list.
    pub fn lsp_of(settings: &tinkerpad_core::Settings) -> Self {
        let mut service = Self {
            prewarm: settings.lsp.prewarm.clone(),
            idle_shutdown: idle_of(&settings.lsp.languages, LSP),
        };
        if let Ok(list) = std::env::var(PREWARM_ENV) {
            service.prewarm = list
                .split(',')
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
                .collect();
        }
        service
    }

    /// The `[helpers]` part. Helpers have no pre-warm list: a helper is
    /// started by the first run that wants it, and a language whose server is
    /// pre-warmed usually isn't the one you run first.
    pub fn helpers_of(settings: &tinkerpad_core::Settings) -> Self {
        Self {
            prewarm: Vec::new(),
            idle_shutdown: idle_of(&settings.helpers, HELPERS),
        }
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
/// The idle overrides of one section, complaining about the ones that make no
/// sense and keeping the rest.
fn idle_of(
    overrides: &BTreeMap<String, tinkerpad_core::ServiceOverride>,
    section: &str,
) -> BTreeMap<String, Option<Duration>> {
    let mut idle = BTreeMap::new();
    for (id, over) in overrides {
        let Some(raw) = over.idle_shutdown.as_deref() else {
            continue;
        };
        match parse_idle(raw) {
            Some(parsed) => {
                idle.insert(id.clone(), parsed);
            }
            None => warn!(
                id,
                value = raw,
                r#"[{section}.{id}] idle_shutdown should be "never" or a duration like "30m""#
            ),
        }
    }
    idle
}

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
    use tinkerpad_core::Settings;

    use super::*;

    fn settings(text: &str) -> Settings {
        toml::from_str(text).expect("settings")
    }

    #[test]
    fn reads_prewarm_and_per_language_idle() {
        let service = ServiceSettings::lsp_of(&settings(
            r#"
[lsp]
prewarm = ["typescript", "rust"]

[lsp.rust]
idle_shutdown = "30m"

[lsp.typescript]
idle_shutdown = "never"
"#,
        ));
        assert_eq!(service.prewarm, ["typescript", "rust"]);
        assert_eq!(
            service.idle_for("rust", None),
            Some(Duration::from_secs(1800)),
        );
        assert_eq!(
            service.idle_for("typescript", Some(Duration::from_secs(60))),
            None,
            "an explicit `never` overrides the plugin",
        );
        assert_eq!(
            service.idle_for("csharp", Some(Duration::from_secs(60))),
            Some(Duration::from_secs(60)),
            "and a language with no setting keeps the plugin's",
        );
    }

    #[test]
    fn a_file_without_an_lsp_section_is_no_settings() {
        let service = ServiceSettings::lsp_of(&settings(
            "[toolchains.csharp]
dotnet = \"x\"
",
        ));
        assert_eq!(service, ServiceSettings::default());
    }

    #[test]
    fn helpers_are_read_from_their_own_section() {
        let service = ServiceSettings::helpers_of(&settings(
            "[helpers.csharp]
idle_shutdown = \"never\"
",
        ));
        assert!(service.prewarm.is_empty(), "helpers are never pre-warmed");
        assert_eq!(
            service.idle_for("csharp", Some(Duration::from_secs(60))),
            None
        );
    }

    #[test]
    fn nonsense_costs_the_setting_and_nothing_else() {
        let service = ServiceSettings::lsp_of(&settings(
            r#"
[lsp.rust]
idle_shutdown = "half an hour"
"#,
        ));
        assert_eq!(
            service.idle_for("rust", Some(Duration::from_secs(5))),
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
