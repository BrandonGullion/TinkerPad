//! `keybindings.toml`: the shortcuts the user has changed (work item 04-03).
//!
//! ```toml
//! # Command id = the keys it answers to.
//! "run.start" = "mod+shift+r"
//! "playground.new" = ["mod+n", "mod+t"]   # more than one
//! "tab.close" = ""            # unbound
//! ```
//!
//! Flat on purpose: one line per command, nothing nested, so it reads like the
//! list it is. The window owns what the ids mean and which keys are sensible;
//! the core only carries the file, watches it, and hands over the map. An
//! unreadable file keeps the bindings that were working and says what is
//! wrong, the same way settings do (04-02).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use tinkerpad_core::{CoreEvent, Level, Scope};
use tracing::{debug, info, warn};

use crate::events::EventBus;
use crate::settings::Problem;

pub const FILE_NAME: &str = "keybindings.toml";

const DEBOUNCE: Duration = Duration::from_millis(150);

/// What the file says: command id to the bindings it answers to. An empty
/// list means the command has no key, which is how a default is taken away.
pub type Bindings = BTreeMap<String, Vec<String>>;

const TEMPLATE: &str = r#"# TinkerPad keybindings.
#
# One line per command: its id, then the keys it should answer to. `mod` is
# Cmd on macOS and Ctrl everywhere else; use `ctrl` or `cmd` when a binding is
# meant for one platform only. A list gives a command several keys, and an
# empty string takes its keys away.
#
# Every command and its default keys are in docs/KEYBINDINGS.md, and the
# command palette shows them too.
#
# "run.start" = "mod+shift+r"
# "playground.new" = ["mod+n", "mod+t"]
# "tab.close" = ""
"#;

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// Create `keybindings.toml` with its comments if it isn't there.
pub fn ensure_exists(data_dir: &Path) {
    let file = path(data_dir);
    if file.exists() {
        return;
    }
    match std::fs::write(&file, TEMPLATE) {
        Ok(()) => info!(file = %file.display(), "created keybindings file"),
        Err(error) => warn!(file = %file.display(), %error, "could not create keybindings file"),
    }
}

/// Read the file. A missing one is no overrides at all.
pub fn read(file: &Path) -> Result<Bindings, Problem> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Bindings::new()),
        Err(e) => {
            return Err(Problem {
                message: format!("could not read {}: {e}", file.display()),
                line: None,
            });
        }
    };
    parse(&text)
}

/// Parse keybindings text, with the line number when the parser knows it.
pub fn parse(text: &str) -> Result<Bindings, Problem> {
    let table: toml::Table = toml::from_str(text).map_err(|error| Problem {
        message: error.message().to_owned(),
        line: error
            .span()
            .map(|span| text[..span.start.min(text.len())].lines().count().max(1)),
    })?;
    let mut bindings = Bindings::new();
    for (id, value) in table {
        match value {
            toml::Value::String(binding) => {
                bindings.insert(id, tidy([binding]));
            }
            toml::Value::Array(items) => {
                let mut keys = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        toml::Value::String(binding) => keys.push(binding),
                        other => {
                            return Err(Problem {
                                message: format!(
                                    "`{id}` should list keys in quotes, like [\"mod+enter\", \"f5\"], not {}",
                                    other.type_str()
                                ),
                                line: None,
                            });
                        }
                    }
                }
                bindings.insert(id, tidy(keys));
            }
            // A table means someone wrote `[run]` and then `start = "..."`.
            // Say so rather than quietly ignoring half their file.
            other => {
                return Err(Problem {
                    message: format!(
                        "`{id}` should be keys in quotes, like \"mod+shift+r\", not {}",
                        other.type_str()
                    ),
                    line: None,
                });
            }
        }
    }
    Ok(bindings)
}

/// Lowercase and trimmed, with blanks dropped: `""` is no keys at all.
fn tidy(keys: impl IntoIterator<Item = String>) -> Vec<String> {
    keys.into_iter()
        .map(|key| key.trim().to_lowercase())
        .filter(|key| !key.is_empty())
        .collect()
}

/// The bindings as they are now, kept in step with the file.
pub struct Live {
    file: PathBuf,
    current: RwLock<Arc<Bindings>>,
    events: Arc<EventBus>,
    _watcher: Mutex<Option<Debouncer<RecommendedWatcher>>>,
}

impl Live {
    pub fn open(file: PathBuf, events: Arc<EventBus>) -> Arc<Self> {
        let (bindings, problem) = match read(&file) {
            Ok(bindings) => (bindings, None),
            Err(problem) => (Bindings::new(), Some(problem)),
        };
        let live = Arc::new(Self {
            file,
            current: RwLock::new(Arc::new(bindings)),
            events,
            _watcher: Mutex::new(None),
        });
        if let Some(problem) = problem {
            live.complain(&problem);
        }
        live.watch();
        live
    }

    pub fn get(&self) -> Arc<Bindings> {
        self.current.read().expect("bindings lock").clone()
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    fn reload(&self) {
        let bindings = match read(&self.file) {
            Ok(bindings) => bindings,
            Err(problem) => {
                self.complain(&problem);
                return;
            }
        };
        let bindings = Arc::new(bindings);
        {
            let mut current = self.current.write().expect("bindings lock");
            if *current == bindings {
                return;
            }
            *current = bindings.clone();
        }
        debug!(count = bindings.len(), "keybindings changed");
        self.events.publish(
            Scope::App,
            CoreEvent::Keybindings {
                bindings: (*bindings).clone(),
            },
        );
    }

    fn complain(&self, problem: &Problem) {
        warn!(file = %self.file.display(), %problem, "keybindings not applied");
        self.events.publish(
            Scope::App,
            CoreEvent::Toast {
                level: Level::Warning,
                message: format!(
                    "{FILE_NAME} has a problem, so your last working keys are still in use.\n{problem}"
                ),
            },
        );
    }

    /// Watch the folder, not the file: editors save by rename (see 04-02).
    fn watch(self: &Arc<Self>) {
        let Some(folder) = self.file.parent().map(Path::to_path_buf) else {
            return;
        };
        let live = Arc::downgrade(self);
        let wanted = self.file.clone();
        let watcher = new_debouncer(DEBOUNCE, move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            if !events
                .iter()
                .any(|e| e.path == wanted || e.path.file_name() == wanted.file_name())
            {
                return;
            }
            if let Some(live) = live.upgrade() {
                live.reload();
            }
        })
        .and_then(|mut debouncer| {
            debouncer
                .watcher()
                .watch(&folder, RecursiveMode::NonRecursive)?;
            Ok(debouncer)
        });
        match watcher {
            Ok(watcher) => *self._watcher.lock().expect("watcher lock") = Some(watcher),
            Err(error) => warn!(%error, "keybindings file will not be watched"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_is_no_overrides() {
        let dir = std::env::temp_dir().join("tinkerpad-keys-missing");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(read(&path(&dir)).unwrap().is_empty());
    }

    #[test]
    fn bindings_are_read_and_tidied() {
        let bindings = parse("\"run.start\" = \"Mod+Shift+R\"\n\"tab.close\" = \"\"\n").unwrap();
        assert_eq!(
            bindings["run.start"],
            ["mod+shift+r"],
            "case doesn't matter"
        );
        assert!(
            bindings["tab.close"].is_empty(),
            "an empty binding is a removal"
        );
    }

    #[test]
    fn a_list_gives_a_command_several_keys() {
        let bindings =
            parse("\"run.start\" = [\"Mod+Enter\", \"F5\", \"\"]\n\"tab.close\" = []\n").unwrap();
        assert_eq!(
            bindings["run.start"],
            ["mod+enter", "f5"],
            "tidied, blanks dropped"
        );
        assert!(
            bindings["tab.close"].is_empty(),
            "an empty list is a removal"
        );
    }

    #[test]
    fn a_list_of_something_else_is_refused() {
        let problem = parse("\"run.start\" = [\"f5\", 5]\n").unwrap_err();
        assert!(problem.message.contains("list keys in quotes"), "{problem}");
    }

    #[test]
    fn the_template_is_all_comments() {
        assert!(parse(TEMPLATE).unwrap().is_empty());
    }

    #[test]
    fn a_broken_file_says_which_line() {
        let problem = parse("\"run.start\" = \"mod+r\"\n\"tab.close\" =\n").unwrap_err();
        assert_eq!(problem.line, Some(2), "{problem}");
    }

    #[test]
    fn a_section_is_refused_with_an_explanation() {
        let problem = parse("[run]\nstart = \"mod+r\"\n").unwrap_err();
        assert!(problem.message.contains("keys in quotes"), "{problem}");
    }
}
