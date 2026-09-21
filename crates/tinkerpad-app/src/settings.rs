//! `settings.toml`: read, watched, and written back without losing a word of
//! it (work item 04-02).
//!
//! The schema and its defaults live in `tinkerpad_core::settings`. This module
//! owns the file: creating it with its comments on first launch, reading it,
//! watching it so an edit in any editor applies within a second, and writing
//! it back when the settings window saves.
//!
//! Two rules shape the whole module.
//!
//! **A broken file costs you nothing but the edit.** Invalid TOML keeps the
//! last settings that worked and says what is wrong, with the line, in a
//! toast. It is never a failure to start and never a reason to overwrite what
//! the user wrote.
//!
//! **Saving keeps the file yours.** The settings window writes through
//! `toml_edit`, so comments, ordering, and anything a newer build wrote all
//! survive. A setting put back to its default has its line removed rather than
//! written out, so the file stays as short as what you actually changed.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use tinkerpad_core::settings::TEMPLATE;
use tinkerpad_core::{CoreEvent, Level, Scope, Settings};
use toml_edit::{DocumentMut, Item, value};
use tracing::{debug, info, warn};

use crate::events::EventBus;

pub const FILE_NAME: &str = "settings.toml";

/// How long file events are collected before the file is re-read. The same
/// reason as the library's: one save is often several events.
const DEBOUNCE: Duration = Duration::from_millis(150);

pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE_NAME)
}

/// What is wrong with a settings file, and where.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub message: String,
    /// 1-based, when the parser could say.
    pub line: Option<usize>,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(line) => write!(f, "line {line}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

/// Create `settings.toml` with its commented template if it isn't there.
pub fn ensure_exists(data_dir: &Path) {
    let file = path(data_dir);
    if file.exists() {
        return;
    }
    match std::fs::write(&file, TEMPLATE) {
        Ok(()) => info!(file = %file.display(), "created settings file"),
        Err(error) => warn!(file = %file.display(), %error, "could not create settings file"),
    }
}

/// Read a settings file. A missing file is the defaults; an unreadable one is
/// a problem the caller shows.
pub fn read(file: &Path) -> Result<Settings, Problem> {
    let text = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(e) => {
            return Err(Problem {
                message: format!("could not read {}: {e}", file.display()),
                line: None,
            });
        }
    };
    parse(&text)
}

/// Parse settings text, with the line number when the parser knows it.
pub fn parse(text: &str) -> Result<Settings, Problem> {
    toml::from_str(text).map_err(|error| Problem {
        message: error.message().to_owned(),
        line: error
            .span()
            .map(|span| text[..span.start.min(text.len())].lines().count().max(1)),
    })
}

/// Write `settings` into the file at `file`, keeping everything about it that
/// isn't a setting: comments, blank lines, key order, and keys this build
/// doesn't know about.
pub fn write(file: &Path, settings: &Settings) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(file).unwrap_or_else(|_| TEMPLATE.to_owned());
    let mut doc: DocumentMut = existing.parse().unwrap_or_else(|error| {
        warn!(%error, "settings file could not be edited in place; writing a fresh one");
        TEMPLATE.parse().expect("the template is valid TOML")
    });
    let defaults = Settings::default();

    set(
        &mut doc,
        &["theme"],
        (settings.theme != defaults.theme).then(|| value(&settings.theme)),
    );
    set(
        &mut doc,
        &["library", "root"],
        settings
            .library
            .root
            .as_ref()
            .map(|root| value(root.to_string_lossy().replace('\\', "/"))),
    );
    set(
        &mut doc,
        &["editor", "font_size"],
        (settings.editor.font_size != defaults.editor.font_size)
            .then(|| value(i64::from(settings.editor.font_size))),
    );
    set(
        &mut doc,
        &["editor", "font_family"],
        settings.editor.font_family.as_ref().map(value),
    );
    set(
        &mut doc,
        &["editor", "tab_size"],
        (settings.editor.tab_size != defaults.editor.tab_size)
            .then(|| value(i64::from(settings.editor.tab_size))),
    );
    set(
        &mut doc,
        &["run", "clear_output"],
        (settings.run.clear_output != defaults.run.clear_output)
            .then(|| value(settings.run.clear_output)),
    );

    // The per-plugin tables. Only what the settings hold is written; a plugin
    // the user has cleared loses its table.
    write_toolchains(&mut doc, settings);
    write_prewarm(&mut doc, settings);
    write_idle(&mut doc, "lsp", &settings.lsp.languages);
    write_idle(&mut doc, "helpers", &settings.helpers);
    // A `[toolchains.python]` with nothing left in it is not a setting, it is
    // a leftover. The sections themselves stay: the template's comments live
    // under them.
    for section in ["toolchains", "lsp", "helpers"] {
        prune_empty(&mut doc, section);
    }

    write_atomic(file, doc.to_string().as_bytes())
}

/// Write the file through a temporary one, so an interrupted write leaves the
/// settings that were there.
fn write_atomic(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = file.with_extension("toml.tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, file).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Set (or, with `None`, remove) one key, making the tables above it as
/// needed. An empty table left behind by a removal goes too, unless the user
/// wrote something in it.
fn set(doc: &mut DocumentMut, keys: &[&str], item: Option<Item>) {
    let (last, parents) = keys.split_last().expect("a key to set");
    let mut table = doc.as_table_mut();
    for parent in parents {
        if item.is_none() && !table.contains_key(parent) {
            return;
        }
        table = table
            .entry(parent)
            .or_insert_with(|| Item::Table(toml_edit::Table::new()))
            .as_table_mut()
            .expect("a settings section is a table");
    }
    match item {
        Some(item) => {
            table.insert(last, item);
        }
        None => {
            table.remove(last);
        }
    }
}

fn write_toolchains(doc: &mut DocumentMut, settings: &Settings) {
    for (plugin, programs) in &settings.toolchains {
        for (program, path) in programs {
            set(
                doc,
                &["toolchains", plugin, program],
                Some(value(path.replace('\\', "/"))),
            );
        }
    }
    // A plugin whose overrides were all removed.
    let present: Vec<String> = doc
        .get("toolchains")
        .and_then(Item::as_table)
        .map(|t| t.iter().map(|(k, _)| k.to_owned()).collect())
        .unwrap_or_default();
    for plugin in present {
        let wanted = settings.toolchains.get(&plugin);
        let programs: Vec<String> = doc["toolchains"][&plugin]
            .as_table()
            .map(|t| t.iter().map(|(k, _)| k.to_owned()).collect())
            .unwrap_or_default();
        for program in programs {
            if wanted.is_none_or(|w| !w.contains_key(&program)) {
                set(doc, &["toolchains", &plugin, &program], None);
            }
        }
    }
}

fn write_prewarm(doc: &mut DocumentMut, settings: &Settings) {
    if settings.lsp.prewarm.is_empty() {
        set(doc, &["lsp", "prewarm"], None);
        return;
    }
    let mut array = toml_edit::Array::new();
    for language in &settings.lsp.prewarm {
        array.push(language.as_str());
    }
    set(doc, &["lsp", "prewarm"], Some(value(array)));
}

/// The `[lsp.<id>]` and `[helpers.<id>]` tables, which hold one key each.
fn write_idle(
    doc: &mut DocumentMut,
    section: &str,
    wanted: &std::collections::BTreeMap<String, tinkerpad_core::ServiceOverride>,
) {
    for (id, over) in wanted {
        match &over.idle_shutdown {
            Some(idle) => set(doc, &[section, id, "idle_shutdown"], Some(value(idle))),
            None => set(doc, &[section, id, "idle_shutdown"], None),
        }
    }
    let present: Vec<String> = doc
        .get(section)
        .and_then(Item::as_table)
        .map(|t| {
            t.iter()
                .filter(|(_, item)| item.is_table())
                .map(|(k, _)| k.to_owned())
                .collect()
        })
        .unwrap_or_default();
    for id in present {
        if !wanted.contains_key(&id) {
            set(doc, &[section, &id, "idle_shutdown"], None);
        }
    }
}

/// Drop the empty tables under a section, and the section itself if the user
/// has nothing left in it and the template doesn't document it.
fn prune_empty(doc: &mut DocumentMut, section: &str) {
    let Some(table) = doc.get_mut(section).and_then(Item::as_table_mut) else {
        return;
    };
    table.retain(|_, item| !item.as_table().is_some_and(toml_edit::Table::is_empty));
    if section == "toolchains" && table.is_empty() {
        doc.as_table_mut().remove(section);
    }
}

/// The settings as they are now, kept in step with the file.
pub struct Live {
    file: PathBuf,
    current: RwLock<Arc<Settings>>,
    events: Arc<EventBus>,
    _watcher: Mutex<Option<Debouncer<RecommendedWatcher>>>,
}

impl Live {
    /// Read the file once and start watching it. A broken file at startup is
    /// reported and leaves the defaults in place.
    pub fn open(file: PathBuf, events: Arc<EventBus>) -> Arc<Self> {
        let (settings, problem) = match read(&file) {
            Ok(settings) => (settings, None),
            Err(problem) => (Settings::default(), Some(problem)),
        };
        let live = Arc::new(Self {
            file,
            current: RwLock::new(Arc::new(settings)),
            events,
            _watcher: Mutex::new(None),
        });
        if let Some(problem) = problem {
            live.complain(&problem);
        }
        live.watch();
        live
    }

    pub fn get(&self) -> Arc<Settings> {
        self.current.read().expect("settings lock").clone()
    }

    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Write these settings to the file. The watcher then applies them, so
    /// there is one path from a changed file to a changed app — whether the
    /// change came from the settings window or from an editor.
    pub fn save(&self, settings: &Settings) -> std::io::Result<()> {
        write(&self.file, settings)?;
        // Don't wait for the watcher: the window that saved should see it now.
        self.reload();
        Ok(())
    }

    /// Put the file back to the template: every setting at its default, with
    /// the comments that explain them.
    pub fn reset(&self) -> std::io::Result<()> {
        write_atomic(&self.file, TEMPLATE.as_bytes())?;
        self.reload();
        Ok(())
    }

    /// Re-read the file. Unchanged settings publish nothing; a broken file
    /// keeps what was working and says so.
    pub fn reload(&self) {
        let settings = match read(&self.file) {
            Ok(settings) => settings,
            Err(problem) => {
                self.complain(&problem);
                return;
            }
        };
        let settings = Arc::new(settings);
        {
            let mut current = self.current.write().expect("settings lock");
            if *current == settings {
                return;
            }
            *current = settings.clone();
        }
        debug!("settings changed");
        self.events
            .publish(Scope::App, CoreEvent::Settings((*settings).clone()));
    }

    fn complain(&self, problem: &Problem) {
        warn!(file = %self.file.display(), %problem, "settings not applied");
        self.events.publish(
            Scope::App,
            CoreEvent::Toast {
                level: Level::Warning,
                message: format!(
                    "{FILE_NAME} has a problem, so your last working settings are still in use.\n{problem}"
                ),
            },
        );
    }

    /// Watch the file for edits made anywhere else.
    ///
    /// The folder is watched rather than the file: an editor that saves by
    /// writing a new file and renaming it over the old one (most of them)
    /// leaves a watch on the file itself pointing at nothing.
    fn watch(self: &Arc<Self>) {
        let Some(folder) = self.file.parent().map(Path::to_path_buf) else {
            return;
        };
        let live = Arc::downgrade(self);
        let wanted = self.file.clone();
        let watcher = new_debouncer(DEBOUNCE, move |result: DebounceEventResult| {
            let Ok(events) = result else { return };
            let ours = events
                .iter()
                .any(|e| e.path == wanted || e.path.file_name() == wanted.file_name());
            if !ours {
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
            Ok(watcher) => {
                *self._watcher.lock().expect("watcher lock") = Some(watcher);
                debug!(folder = %folder.display(), "watching the settings file");
            }
            Err(error) => warn!(%error, "settings file will not be watched"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tinkerpad-settings-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_missing_file_is_the_defaults() {
        let dir = scratch("missing");
        assert_eq!(read(&path(&dir)).unwrap(), Settings::default());
    }

    #[test]
    fn the_template_is_written_once_and_left_alone() {
        let dir = scratch("template");
        ensure_exists(&dir);
        std::fs::write(path(&dir), "theme = \"dark\"\n").unwrap();
        ensure_exists(&dir);
        assert_eq!(
            std::fs::read_to_string(path(&dir)).unwrap(),
            "theme = \"dark\"\n",
            "an existing file is never overwritten",
        );
    }

    #[test]
    fn a_broken_file_says_which_line() {
        let problem = parse("[editor]\nfont_size = 13\ntab_size = \n").unwrap_err();
        assert_eq!(problem.line, Some(3), "{problem}");
        assert!(!problem.message.is_empty());
    }

    #[test]
    fn saving_keeps_the_comments_and_writes_only_what_changed() {
        let dir = scratch("save");
        ensure_exists(&dir);
        let file = path(&dir);

        let mut settings = Settings::default();
        settings.editor.font_size = 16;
        settings.lsp.prewarm = vec!["rust".to_owned()];
        settings
            .toolchains
            .entry("python".to_owned())
            .or_default()
            .insert("python3".to_owned(), "/usr/bin/python3".to_owned());
        write(&file, &settings).unwrap();

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("# TinkerPad settings."),
            "comments kept:\n{text}"
        );
        assert!(text.contains("font_size = 16"), "{text}");
        assert!(text.contains(r#"prewarm = ["rust"]"#), "{text}");
        assert!(text.contains(r#"python3 = "/usr/bin/python3""#), "{text}");
        assert!(
            !text.contains("\nclear_output"),
            "a default is left commented out:\n{text}"
        );
        assert_eq!(
            read(&file).unwrap(),
            settings,
            "what was written reads back"
        );

        // Putting one back to its default removes its line again.
        settings.editor.font_size = 13;
        settings.toolchains.clear();
        write(&file, &settings).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("font_size = 16"), "{text}");
        assert!(
            !text.contains("\npython3 = "),
            "an uncommented override is gone:\n{text}"
        );
        assert_eq!(read(&file).unwrap(), settings);
    }

    #[test]
    fn a_key_this_build_does_not_know_survives_a_save() {
        let dir = scratch("unknown");
        let file = path(&dir);
        std::fs::write(
            &file,
            "# mine\nwrap_columns = 100\n\n[editor]\nfont_size = 14\n",
        )
        .unwrap();
        let mut settings = read(&file).unwrap();
        assert_eq!(settings.font_size(), 14);
        settings.editor.font_size = 18;
        write(&file, &settings).unwrap();

        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.contains("wrap_columns = 100"), "{text}");
        assert!(text.contains("# mine"), "{text}");
        assert!(text.contains("font_size = 18"), "{text}");
    }
}
