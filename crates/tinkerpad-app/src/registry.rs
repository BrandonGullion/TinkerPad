//! The plugin index: what is out there, and fetching it (work item 05-05).
//!
//! A JSON file in a repository, listing plugins. A submission is a pull request
//! adding an entry, which is a fine process for a hobby community and needs no
//! server at all. Nothing is hosted here: an entry is a name and a git URL, and
//! installing one goes through the same path as typing that URL by hand
//! (05-02), trust notice and all.
//!
//! **Fetching it is the app's first network call**, so it never happens on
//! launch and never without being asked. The window asks once, the answer is
//! remembered, and the index is cached for a day — an index nobody can reach is
//! an empty list and a sentence, not an error.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How long a fetched index is used before it is fetched again.
pub const CACHE_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// How long the fetch may take before it is given up on. A browse tab that
/// hangs is worse than one that says it couldn't reach anything.
const TIMEOUT: Duration = Duration::from_secs(15);

/// Where the index is fetched from, unless `TINKERPAD_REGISTRY` says otherwise.
///
/// Not reachable yet: the repository holding it is private until the project
/// is. Until then the window shows what is in the checkout, and the Browse tab
/// says why it has nothing new.
pub const DEFAULT_URL: &str =
    "https://raw.githubusercontent.com/BrandonGullion/tinkerpad-plugins/main/index.json";

/// Overrides the index location. A path works as well as a URL, which is how
/// the tests and a checkout read the one in `registry/`.
pub const REGISTRY_ENV: &str = "TINKERPAD_REGISTRY";

/// The catalogue.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Index {
    /// Index format version. `1` is this one.
    pub version: u32,
    /// Every plugin anyone has submitted, bundled ones included.
    #[serde(default)]
    pub plugins: Vec<Entry>,
}

/// One plugin in the index.
///
/// Deliberately small: a name, where to get it, and who to ask about it. Any
/// field that would need maintaining — a download count, a rating, a mirror of
/// the code — is a field that goes stale and misleads.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Entry {
    /// The `[language].id` from its manifest. Unique in the index.
    pub id: String,
    /// What it calls itself.
    pub name: String,
    /// One line. Shown in the list.
    pub description: String,
    /// What `tinkerpad plugin install` is given. Absent for a bundled plugin,
    /// which is already here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<String>,
    /// The languages it adds, for searching. Usually one.
    #[serde(default)]
    pub languages: Vec<String>,
    /// Who to open an issue with.
    pub maintainer: String,
    /// The last TinkerPad version its `plugin check` was seen to pass on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verified_with: Option<String>,
    /// Shipped with TinkerPad. Listed so the index is the whole catalogue
    /// rather than only the parts that aren't here.
    #[serde(default)]
    pub bundled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the index could not be reached: {0}")]
    Fetch(String),
    #[error("the index isn't valid: {0}")]
    Parse(String),
    #[error("this build reads index version 1, and that one is version {0}")]
    Version(u32),
}

impl Index {
    /// Read an index from text, and check what a schema can't.
    pub fn parse(text: &str) -> Result<Index, Error> {
        let index: Index = serde_json::from_str(text).map_err(|e| Error::Parse(e.to_string()))?;
        if index.version != 1 {
            return Err(Error::Version(index.version));
        }
        Ok(index)
    }

    /// Everything wrong with it that is worth refusing a pull request over.
    ///
    /// Returned rather than logged: the registry's own CI prints these, and a
    /// contributor should see all of them at once rather than one per push.
    pub fn problems(&self) -> Vec<String> {
        let mut problems = Vec::new();
        let mut seen: Vec<&str> = Vec::new();
        for entry in &self.plugins {
            let at = &entry.id;
            if entry.id.trim().is_empty() {
                problems.push("an entry has no id".to_owned());
                continue;
            }
            if !entry
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            {
                problems.push(format!(
                    "{at}: an id is lowercase letters, digits and hyphens"
                ));
            }
            if seen.contains(&entry.id.as_str()) {
                problems.push(format!("{at}: listed twice"));
            }
            seen.push(&entry.id);

            for (what, value) in [
                ("name", &entry.name),
                ("description", &entry.description),
                ("maintainer", &entry.maintainer),
            ] {
                if value.trim().is_empty() {
                    problems.push(format!("{at}: {what} is empty"));
                }
            }
            if entry.description.len() > 120 {
                problems.push(format!(
                    "{at}: the description is {} characters; one line is 120 or fewer",
                    entry.description.len()
                ));
            }
            match (&entry.git, entry.bundled) {
                (None, false) => {
                    problems.push(format!("{at}: needs a `git` URL, unless it is `bundled`"))
                }
                (Some(url), _) if !is_fetchable(url) => {
                    problems.push(format!("{at}: `{url}` should be an https:// or ssh:// URL"))
                }
                _ => {}
            }
        }
        problems
    }
}

/// Whether a URL is one `git clone` can be given without surprises.
fn is_fetchable(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("ssh://") || url.starts_with("git@")
}

/// Where the index comes from: the environment, else the default URL.
pub fn source() -> String {
    std::env::var(REGISTRY_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_URL.to_owned())
}

/// Where a fetched copy is kept.
pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("registry.json")
}

/// How old the cached copy is, if there is one.
pub fn cached_age(data_dir: &Path) -> Option<Duration> {
    let modified = std::fs::metadata(cache_path(data_dir))
        .ok()?
        .modified()
        .ok()?;
    SystemTime::now().duration_since(modified).ok()
}

/// The index, from the cache if it is fresh enough, else from `source()`.
///
/// Never called on launch. The window calls it when the Browse tab is opened
/// and the user has said it may.
pub async fn load(data_dir: &Path, force: bool) -> Result<Index, Error> {
    let cache = cache_path(data_dir);
    if !force
        && cached_age(data_dir).is_some_and(|age| age < CACHE_FOR)
        && let Ok(text) = std::fs::read_to_string(&cache)
        && let Ok(index) = Index::parse(&text)
    {
        return Ok(index);
    }

    let from = source();
    let text = fetch(&from).await?;
    let index = Index::parse(&text)?;
    // Only a good one is cached: a 404 page should not become tomorrow's
    // catalogue.
    if let Some(parent) = cache.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&cache, &text);
    Ok(index)
}

/// Read the index from a URL, or from a file when `source()` is a path — which
/// is how a checkout reads the one in `registry/` and how the tests work
/// without a network.
async fn fetch(from: &str) -> Result<String, Error> {
    if !from.starts_with("http://") && !from.starts_with("https://") {
        return std::fs::read_to_string(from).map_err(|e| Error::Fetch(format!("{from}: {e}")));
    }
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .user_agent(concat!("TinkerPad/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Fetch(e.to_string()))?;
    let response = client
        .get(from)
        .send()
        .await
        .map_err(|e| Error::Fetch(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Fetch(format!(
            "{from} answered {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|e| Error::Fetch(e.to_string()))
}

/// The JSON Schema for `index.json`, generated from the types above so the
/// registry's CI checks what this build actually reads (work item 05-04's
/// rule, applied here).
pub fn schema() -> serde_json::Value {
    let schema = schemars::schema_for!(Index);
    serde_json::to_value(&schema).expect("a schema is JSON")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            name: "A Language".to_owned(),
            description: "Adds a language.".to_owned(),
            git: Some("https://example.invalid/plugin".to_owned()),
            languages: vec![id.to_owned()],
            maintainer: "someone".to_owned(),
            verified_with: None,
            bundled: false,
        }
    }

    #[test]
    fn a_sound_index_has_nothing_wrong_with_it() {
        let index = Index {
            version: 1,
            plugins: vec![entry("elm"), entry("zig")],
        };
        assert!(index.problems().is_empty(), "{:?}", index.problems());
    }

    #[test]
    fn an_index_from_the_future_is_refused_rather_than_guessed_at() {
        let text = r#"{"version": 2, "plugins": []}"#;
        let error = Index::parse(text).expect_err("version 2");
        assert!(error.to_string().contains("version 2"), "{error}");
    }

    #[test]
    fn an_unknown_field_is_refused() {
        // The index is a file people edit by hand in pull requests, so a typo
        // should fail the check rather than be dropped in silence.
        let text = r#"{"version": 1, "plugins": [{"id":"x","name":"X",
            "description":"d","maintainer":"m","gti":"https://x.invalid"}]}"#;
        assert!(Index::parse(text).is_err());
    }

    #[test]
    fn every_problem_is_reported_at_once() {
        let mut twice = entry("elm");
        twice.description = String::new();
        let index = Index {
            version: 1,
            plugins: vec![
                entry("elm"),
                twice,
                Entry {
                    git: None,
                    ..entry("no-url")
                },
                Entry {
                    id: "Shouty".to_owned(),
                    ..entry("shouty")
                },
            ],
        };
        let problems = index.problems();
        assert!(
            problems.iter().any(|p| p.contains("listed twice")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("description is empty")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("needs a `git` URL")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("lowercase letters")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_bundled_entry_needs_no_url() {
        let index = Index {
            version: 1,
            plugins: vec![Entry {
                git: None,
                bundled: true,
                ..entry("python")
            }],
        };
        assert!(index.problems().is_empty(), "{:?}", index.problems());
    }

    #[test]
    fn a_description_that_is_a_paragraph_is_refused() {
        let index = Index {
            version: 1,
            plugins: vec![Entry {
                description: "x".repeat(200),
                ..entry("wordy")
            }],
        };
        assert!(
            index.problems().iter().any(|p| p.contains("one line")),
            "{:?}",
            index.problems()
        );
    }
}
