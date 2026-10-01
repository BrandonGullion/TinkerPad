//! The playground library: folders on disk, an index in memory.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode};
use notify_debouncer_mini::{DebounceEventResult, Debouncer, new_debouncer};
use serde::Serialize;
use tinkerpad_core::{
    BrokenPlayground, KindId, LanguageId, Playground, PlaygroundId, PlaygroundSummary,
};
use tracing::{debug, info, warn};

use crate::error::LibraryError;
use crate::search::{Indexed, SearchHit};
use crate::{fsutil, history, meta, platform, search, slug};

/// How long file events are collected before being handled, so one save
/// (which some platforms report several times) is handled once.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

const README: &str = "# TinkerPad library

Each folder here is a playground created by TinkerPad. A folder is a playground
if it contains a `playground.toml`; anything else is ignored, so this folder is
safe to keep under version control or to share.

    fizz-buzz/
      playground.toml   id, name, language, kind, main file, created, tags
      main.cs           the code
      runs.jsonl        what each run did; delete it and you lose only that

You can rename, copy, or delete folders here while TinkerPad is running; it
notices. Copying a folder duplicates its `id`, so give the copy a new one (or
create a new playground in the app and paste the code in).
";

/// What to create.
#[derive(Clone, Debug)]
pub struct NewPlayground {
    /// Display name. The folder name is a slug of it.
    pub name: String,
    pub language: LanguageId,
    pub kind: KindId,
    /// The main file, relative to the playground folder (from the plugin).
    pub main_path: PathBuf,
    /// Files to write, relative to the folder. The main file is created empty
    /// if it isn't among them.
    pub files: Vec<(PathBuf, Vec<u8>)>,
}

/// Where a new playground is being made, for [`Library::create_with`].
#[derive(Debug)]
pub struct Claimed<'a> {
    /// The folder name chosen for it, such as `fizz-buzz-2`.
    pub folder: &'a str,
    pub dir: &'a Path,
    pub id: &'a PlaygroundId,
}

/// Files and plugin metadata from [`Library::create_with`]'s callback.
#[derive(Clone, Debug, Default)]
pub struct Scaffold {
    /// Relative to the playground folder.
    pub files: Vec<(PathBuf, Vec<u8>)>,
    /// Recorded in `playground.toml` under `[plugin.<language>]`.
    pub plugin_meta: BTreeMap<String, String>,
}

/// Something that changed on disk.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryEvent {
    /// Playgrounds were added, removed, renamed, or their metadata changed.
    Changed,
    /// A playground's main file was changed by something other than
    /// [`Library::write_source`].
    SourceChangedOnDisk(PlaygroundId),
}

/// Every playground, and folders that look like playgrounds but are broken.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub root: PathBuf,
    /// Most recently modified first.
    pub playgrounds: Vec<PlaygroundSummary>,
    pub broken: Vec<BrokenPlayground>,
}

#[derive(Default)]
struct Index {
    by_id: BTreeMap<PlaygroundId, Playground>,
    /// What each playground can be found by: its name, tags, and the text of
    /// its main file (work item 04-01). Read once, when the folder is
    /// indexed, and kept in step by the watcher.
    searchable: BTreeMap<PlaygroundId, Indexed>,
    /// Keyed by folder.
    broken: BTreeMap<PathBuf, BrokenPlayground>,
}

impl Index {
    /// Add or replace a playground, reading what it can be searched by.
    fn insert(&mut self, p: Playground) {
        self.searchable.insert(p.id.clone(), Indexed::of(&p));
        self.by_id.insert(p.id.clone(), p);
    }

    fn remove_dir(&mut self, dir: &Path) {
        let gone: Vec<PlaygroundId> = self
            .by_id
            .iter()
            .filter(|(_, p)| p.dir == dir)
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            self.by_id.remove(&id);
            self.searchable.remove(&id);
        }
        self.broken.remove(dir);
    }

    fn folder_state(&self, dir: &Path) -> (Option<Playground>, Option<BrokenPlayground>) {
        (
            self.by_id.values().find(|p| p.dir == dir).cloned(),
            self.broken.get(dir).cloned(),
        )
    }
}

/// Keeps a library's watcher running. Dropping it stops watching.
pub struct Watcher {
    _debouncer: Debouncer<RecommendedWatcher>,
}

pub struct Library {
    /// Where playgrounds are. It can move: the settings window can point the
    /// library at another folder (work item 04-02).
    root: RwLock<PathBuf>,
    /// `root` canonicalised, because watchers may report paths in that form
    /// (macOS reports `/private/var/...` for `/var/...`).
    canonical_root: RwLock<PathBuf>,
    index: RwLock<Index>,
    /// Hash of each main file's contents as last written or seen by the app,
    /// to tell our own saves from edits made elsewhere.
    known_source: Mutex<HashMap<PathBuf, u64>>,
}

/// How long to keep trying an operation another process is refusing, and how
/// often. A file watcher letting go of a handle is a race, not a permanent
/// state, so the first refusal is not the answer (work item 04-09).
///
/// Six attempts over about a second: long enough for a watcher that is on its
/// way out, short enough that a folder something really is holding says so
/// while the user is still looking at it.
const RETRY_DELAYS_MS: [u64; 5] = [30, 70, 150, 300, 450];

/// Run a file operation that another process may briefly refuse, retrying on
/// the kinds of failure that pass.
///
/// `Err` from the last attempt is what comes back, so the message is about the
/// state the folder was actually in rather than the first thing that went
/// wrong.
fn retrying<T, E>(mut attempt: impl FnMut() -> Result<T, E>) -> Result<T, E> {
    if let Ok(value) = attempt() {
        return Ok(value);
    }
    let mut last = None;
    for delay in RETRY_DELAYS_MS {
        std::thread::sleep(std::time::Duration::from_millis(delay));
        match attempt() {
            Ok(value) => return Ok(value),
            Err(error) => last = Some(error),
        }
    }
    Err(last.expect("the loop runs at least once"))
}

impl Library {
    /// Open the library at `root`, creating it (with a README) if needed, and
    /// index it.
    pub fn open(root: impl Into<PathBuf>) -> Result<Arc<Library>, LibraryError> {
        let root = root.into();
        let created = !root.exists();
        std::fs::create_dir_all(&root).map_err(|e| LibraryError::io(&root, e))?;
        let readme = root.join("README.md");
        if (created || !readme.exists())
            && let Err(e) = std::fs::write(&readme, README)
        {
            warn!(file = %readme.display(), error = %e, "could not write library README");
        }
        let canonical_root = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        let library = Arc::new(Library {
            root: RwLock::new(root),
            canonical_root: RwLock::new(canonical_root),
            index: RwLock::default(),
            known_source: Mutex::default(),
        });
        library.rescan();
        Ok(library)
    }

    /// Where playgrounds are kept now.
    pub fn root(&self) -> PathBuf {
        self.root.read().expect("root lock").clone()
    }

    /// Point the library at another folder and index it (work item 04-02).
    ///
    /// The caller is expected to have closed what was open: every playground
    /// the window was showing belongs to the old folder.
    pub fn change_root(&self, root: impl Into<PathBuf>) -> Result<(), LibraryError> {
        let root = root.into();
        std::fs::create_dir_all(&root).map_err(|e| LibraryError::io(&root, e))?;
        let readme = root.join("README.md");
        if !readme.exists()
            && let Err(e) = std::fs::write(&readme, README)
        {
            warn!(file = %readme.display(), error = %e, "could not write library README");
        }
        let canonical = std::fs::canonicalize(&root).unwrap_or_else(|_| root.clone());
        *self.root.write().expect("root lock") = root.clone();
        *self.canonical_root.write().expect("root lock") = canonical;
        self.known_source.lock().expect("known lock").clear();
        self.rescan();
        info!(root = %root.display(), "library moved");
        Ok(())
    }

    /// Rebuild the index from disk.
    pub fn rescan(&self) {
        let mut dirs: Vec<PathBuf> = match std::fs::read_dir(self.root()) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(e) => {
                warn!(root = %self.root().display(), error = %e, "could not read library");
                Vec::new()
            }
        };
        dirs.sort();
        let mut index = Index::default();
        for dir in dirs {
            load_folder(&mut index, &dir);
        }
        info!(
            root = %self.root().display(),
            playgrounds = index.by_id.len(),
            broken = index.broken.len(),
            "library indexed"
        );
        *self.index.write().expect("index lock") = index;
    }

    pub fn get(&self, id: &PlaygroundId) -> Option<Playground> {
        self.index
            .read()
            .expect("index lock")
            .by_id
            .get(id)
            .cloned()
    }

    fn require(&self, id: &PlaygroundId) -> Result<Playground, LibraryError> {
        self.get(id)
            .ok_or_else(|| LibraryError::NotFound(id.clone()))
    }

    /// Create a playground folder with its metadata and files.
    pub fn create(&self, new: NewPlayground) -> Result<Playground, LibraryError> {
        self.create_with(new, |_| Ok(Scaffold::default()))
    }

    /// Like [`Library::create`], with more files (and plugin metadata) made
    /// by `scaffold` once the folder name and id are known, for templates that
    /// need them. `new.files` are written first; `scaffold`'s files after, so
    /// they win on the same path.
    pub fn create_with(
        &self,
        new: NewPlayground,
        scaffold: impl FnOnce(&Claimed<'_>) -> Result<Scaffold, String>,
    ) -> Result<Playground, LibraryError> {
        let name = new.name.trim();
        if name.is_empty() {
            return Err(LibraryError::InvalidName(
                "a playground needs a name".into(),
            ));
        }
        let main_rel = meta::safe_relative(&new.main_path.to_string_lossy()).ok_or_else(|| {
            LibraryError::InvalidName(format!(
                "main file `{}` must be a relative path",
                new.main_path.display()
            ))
        })?;

        // Hold the index for the whole operation. The watcher takes the same
        // lock before comparing a folder with the index, so it only ever
        // compares against the finished result and never reports the app's
        // own changes. Lock order everywhere: index, then known_source.
        let mut index = self.index.write().expect("index lock");

        // Claim a folder. `create_dir` fails if something appeared since the
        // name was chosen, in which case try the next name.
        let slug = slug::slugify(name);
        let dir = loop {
            let folder = slug::unique_folder(&self.root(), &slug, None);
            let dir = self.root().join(&folder);
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(LibraryError::io(&dir, e)),
            }
        };

        let id = meta::new_id();
        let result = (|| {
            let folder = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let extra = scaffold(&Claimed {
                folder: &folder,
                dir: &dir,
                id: &id,
            })
            .map_err(LibraryError::Scaffold)?;
            let mut files = new.files.clone();
            files.retain(|(path, _)| !extra.files.iter().any(|(p, _)| p == path));
            files.extend(extra.files);

            let mut wrote_main = false;
            for (rel, bytes) in &files {
                let rel_str = rel.to_string_lossy();
                let rel = meta::safe_relative(&rel_str).ok_or_else(|| {
                    LibraryError::InvalidName(format!("file `{rel_str}` must be a relative path"))
                })?;
                wrote_main |= rel == main_rel;
                write_new_file(&dir.join(&rel), bytes)?;
            }
            if !wrote_main {
                write_new_file(&dir.join(&main_rel), b"")?;
            }

            let playground = Playground {
                id: id.clone(),
                name: name.to_owned(),
                language: new.language.clone(),
                kind: new.kind.clone(),
                dir: dir.clone(),
                main_file: dir.join(&main_rel),
                // Whole seconds, as stored in playground.toml, so the
                // in-memory copy equals what a re-read produces.
                created_ms: meta::to_ms(std::time::SystemTime::now()) / 1000 * 1000,
                tags: Vec::new(),
                pinned: false,
            };
            // Metadata last, so a watcher never sees a half-made playground.
            let mut plugin = toml::Table::new();
            if !extra.plugin_meta.is_empty() {
                let table: toml::Table = extra
                    .plugin_meta
                    .into_iter()
                    .map(|(k, v)| (k, toml::Value::String(v)))
                    .collect();
                plugin.insert(new.language.to_string(), toml::Value::Table(table));
            }
            meta::write_with(&playground, plugin).map_err(|e| LibraryError::io(&dir, e))?;
            Ok(playground)
        })();

        match result {
            Ok(playground) => {
                self.remember_source(&playground.main_file);
                index.remove_dir(&dir);
                index.insert(playground.clone());
                info!(id = %playground.id, dir = %dir.display(), "playground created");
                Ok(playground)
            }
            Err(e) => {
                // Only our own brand-new folder is removed.
                let _ = std::fs::remove_dir_all(&dir);
                Err(e)
            }
        }
    }

    /// Give a playground a new display name, renaming its folder to match.
    pub fn rename(&self, id: &PlaygroundId, new_name: &str) -> Result<Playground, LibraryError> {
        let name = new_name.trim();
        if name.is_empty() {
            return Err(LibraryError::InvalidName(
                "a playground needs a name".into(),
            ));
        }
        // Held throughout; see `create`.
        let mut index = self.index.write().expect("index lock");
        let mut p = index
            .by_id
            .get(id)
            .cloned()
            .ok_or_else(|| LibraryError::NotFound(id.clone()))?;
        let old_dir = p.dir.clone();
        let current_folder = old_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        let slug = slug::slugify(name);
        if !slug.eq_ignore_ascii_case(&current_folder) {
            let folder = slug::unique_folder(&self.root(), &slug, Some(&current_folder));
            let new_dir = self.root().join(folder);
            // A rename moves the folder, so a watcher can refuse it exactly
            // as it refuses a delete (work item 04-09).
            retrying(|| std::fs::rename(&old_dir, &new_dir))
                .map_err(|e| LibraryError::io(&old_dir, e))?;
            let old_main = p.main_file.clone();
            let rel = old_main.strip_prefix(&old_dir).unwrap_or(&old_main);
            p.main_file = new_dir.join(rel);
            p.dir = new_dir;

            let mut known = self.known_source.lock().expect("known lock");
            if let Some(hash) = known.remove(&old_main) {
                known.insert(p.main_file.clone(), hash);
            }
        }
        p.name = name.to_owned();
        meta::write(&p).map_err(|e| LibraryError::io(&p.dir, e))?;

        index.remove_dir(&old_dir);
        index.remove_dir(&p.dir);
        index.insert(p.clone());
        info!(id = %p.id, from = %old_dir.display(), to = %p.dir.display(), "playground renamed");
        Ok(p)
    }

    /// Move a playground's folder to the OS recycle bin.
    pub fn delete(&self, id: &PlaygroundId) -> Result<(), LibraryError> {
        // Held throughout; see `create`.
        let mut index = self.index.write().expect("index lock");
        let p = index
            .by_id
            .get(id)
            .cloned()
            .ok_or_else(|| LibraryError::NotFound(id.clone()))?;
        // A language server watching the folder can refuse this, and on
        // Windows always does while it holds a handle without
        // FILE_SHARE_DELETE. Retried rather than failed on: the app has just
        // told that server the playground is going (02-10), and letting go
        // takes it a moment (work item 04-09).
        retrying(|| platform::move_to_trash(&p.dir)).map_err(|message| LibraryError::Trash {
            path: p.dir.clone(),
            message,
        })?;
        index.remove_dir(&p.dir);
        self.known_source
            .lock()
            .expect("known lock")
            .remove(&p.main_file);
        info!(id = %p.id, dir = %p.dir.display(), "playground moved to trash");
        Ok(())
    }

    pub fn read_source(&self, id: &PlaygroundId) -> Result<String, LibraryError> {
        let p = self.require(id)?;
        std::fs::read_to_string(&p.main_file).map_err(|e| LibraryError::io(&p.main_file, e))
    }

    /// Replace a playground's main file atomically. Returns whether it was
    /// written: a file that already holds exactly `text` is left alone, so its
    /// modification time only changes when its content does (builds that key
    /// off it, such as Cargo's and `dotnet run`'s, then skip their work).
    pub fn write_source(&self, id: &PlaygroundId, text: &str) -> Result<bool, LibraryError> {
        let p = self.require(id)?;
        if std::fs::read(&p.main_file).is_ok_and(|current| current == text.as_bytes()) {
            return Ok(false);
        }
        // Remember first: the watcher may see the rename before this returns.
        self.known_source
            .lock()
            .expect("known lock")
            .insert(p.main_file.clone(), hash_bytes(text.as_bytes()));
        fsutil::write_atomic(&p.main_file, text.as_bytes())
            .map_err(|e| LibraryError::io(&p.main_file, e))?;
        // The text the user just saved is what a search should find, without
        // waiting for the watcher to say so (work item 04-01).
        self.index
            .write()
            .expect("index lock")
            .searchable
            .insert(p.id.clone(), Indexed::from_text(&p, text.to_owned()));
        Ok(true)
    }

    /// Every playground, most recently modified first.
    pub fn list(&self) -> Listing {
        let index = self.index.read().expect("index lock");
        let mut playgrounds: Vec<PlaygroundSummary> = index
            .by_id
            .values()
            .map(|p| {
                // Both from the index, which the watcher and our own writes
                // keep in step; a listing that stats every folder is a listing
                // that costs a disk (work item 04-01).
                let indexed = index.searchable.get(&p.id);
                PlaygroundSummary {
                    modified_ms: indexed.map_or_else(|| modified_ms(p), |i| i.modified_ms),
                    preview: indexed.map(|i| i.preview.clone()).unwrap_or_default(),
                    playground: p.clone(),
                }
            })
            .collect();
        playgrounds.sort_by(|a, b| {
            b.modified_ms
                .cmp(&a.modified_ms)
                .then_with(|| a.playground.name.cmp(&b.playground.name))
        });
        Listing {
            root: self.root(),
            playgrounds,
            broken: index.broken.values().cloned().collect(),
        }
    }

    /// The playgrounds matching `query`, best first (work item 04-01).
    ///
    /// An empty query is every playground, pinned first and then by when it
    /// was last touched — which is the library panel with nothing typed in it.
    pub fn search(&self, query: &str, limit: usize) -> Vec<SearchHit> {
        let index = self.index.read().expect("index lock");
        let entries = index.by_id.values().filter_map(|p| {
            let indexed = index.searchable.get(&p.id)?;
            let summary = PlaygroundSummary {
                modified_ms: indexed.modified_ms,
                preview: indexed.preview.clone(),
                playground: p.clone(),
            };
            Some((summary, indexed.clone()))
        });
        search::rank(entries, query, limit)
    }

    /// Replace a playground's tags. Blank tags are dropped, duplicates (case
    /// insensitively) collapse, and the rest are kept in the order given.
    pub fn set_tags(&self, id: &PlaygroundId, tags: &[String]) -> Result<Playground, LibraryError> {
        let mut index = self.index.write().expect("index lock");
        let mut p = index
            .by_id
            .get(id)
            .cloned()
            .ok_or_else(|| LibraryError::NotFound(id.clone()))?;
        let mut kept: Vec<String> = Vec::new();
        for tag in tags {
            let tag = tag.trim();
            if tag.is_empty() || kept.iter().any(|k| k.eq_ignore_ascii_case(tag)) {
                continue;
            }
            kept.push(tag.to_owned());
        }
        if kept == p.tags {
            return Ok(p);
        }
        p.tags = kept;
        meta::write(&p).map_err(|e| LibraryError::io(&p.dir, e))?;
        index.insert(p.clone());
        info!(id = %p.id, tags = ?p.tags, "tags changed");
        Ok(p)
    }

    /// Pin or unpin a playground.
    pub fn set_pinned(&self, id: &PlaygroundId, pinned: bool) -> Result<Playground, LibraryError> {
        let mut index = self.index.write().expect("index lock");
        let mut p = index
            .by_id
            .get(id)
            .cloned()
            .ok_or_else(|| LibraryError::NotFound(id.clone()))?;
        if p.pinned == pinned {
            return Ok(p);
        }
        p.pinned = pinned;
        meta::write(&p).map_err(|e| LibraryError::io(&p.dir, e))?;
        index.insert(p.clone());
        info!(id = %p.id, pinned, "playground pinned");
        Ok(p)
    }

    /// Copy a playground into a new one: same files, new id, new folder.
    ///
    /// What a playground made is not what it is: `runs.jsonl` and the folders
    /// a toolchain fills (`node_modules`, `target`, `bin`, `obj`) are left
    /// behind, so a duplicate starts as clean as a new playground with the
    /// same code in it. Its tags and its `[plugin.*]` tables come along,
    /// because those are what it is.
    pub fn duplicate(&self, id: &PlaygroundId) -> Result<Playground, LibraryError> {
        let source = self.require(id)?;
        let main_path = source
            .main_file
            .strip_prefix(&source.dir)
            .unwrap_or(Path::new("main"))
            .to_path_buf();
        let mut files = Vec::new();
        copy_into(&source.dir, Path::new(""), &mut files)?;
        let copy = self.create(NewPlayground {
            name: copy_name(&source.name),
            language: source.language.clone(),
            kind: source.kind.clone(),
            main_path,
            files,
        })?;

        let plugin = meta::plugin_tables(&source.dir);
        if !plugin.is_empty()
            && let Err(e) = meta::write_with(&copy, plugin)
        {
            warn!(id = %copy.id, error = %e, "could not carry the plugin tables over");
        }
        info!(from = %source.id, to = %copy.id, "playground duplicated");
        if source.tags.is_empty() {
            Ok(copy)
        } else {
            self.set_tags(&copy.id, &source.tags)
        }
    }

    /// Watch the library for changes made outside the app. `on_event` is
    /// called on the watcher's thread.
    pub fn watch<F>(self: &Arc<Self>, on_event: F) -> Result<Watcher, LibraryError>
    where
        F: Fn(LibraryEvent) + Send + 'static,
    {
        let library = Arc::clone(self);
        let mut debouncer =
            new_debouncer(DEBOUNCE, move |result: DebounceEventResult| match result {
                Ok(events) => {
                    for event in library.handle_paths(events.into_iter().map(|e| e.path)) {
                        on_event(event);
                    }
                }
                Err(error) => {
                    warn!(%error, "library watcher error; rescanning");
                    library.rescan();
                    on_event(LibraryEvent::Changed);
                }
            })
            .map_err(|e| LibraryError::Watch(e.to_string()))?;
        debouncer
            .watcher()
            .watch(&self.root(), RecursiveMode::Recursive)
            .map_err(|e| LibraryError::Watch(e.to_string()))?;
        info!(root = %self.root().display(), "watching library");
        Ok(Watcher {
            _debouncer: debouncer,
        })
    }

    /// Update the index for changed paths and say what changed.
    fn handle_paths(&self, paths: impl IntoIterator<Item = PathBuf>) -> Vec<LibraryEvent> {
        let root = self.root();
        let canonical = self.canonical_root.read().expect("root lock").clone();
        let mut folders = BTreeSet::new();
        let mut touched_files = BTreeSet::new();
        for path in paths {
            if fsutil::is_tmp(&path) || path.ends_with(history::HISTORY_FILE) {
                // A run recording itself is not a change to the library
                // (work item 03-08).
                continue;
            }
            let rel = path
                .strip_prefix(&root)
                .or_else(|_| path.strip_prefix(&canonical));
            let Ok(rel) = rel else { continue };
            let mut components = rel.components();
            let Some(first) = components.next() else {
                continue;
            };
            let dir = root.join(first.as_os_str());
            if components.next().is_some() {
                touched_files.insert(root.join(rel));
            }
            folders.insert(dir);
        }

        let mut events = Vec::new();
        let mut changed = false;
        for dir in &folders {
            let mut index = self.index.write().expect("index lock");
            let before = index.folder_state(dir);
            index.remove_dir(dir);
            load_folder(&mut index, dir);
            if index.folder_state(dir) != before {
                changed = true;
            }
        }
        if changed {
            debug!(folders = folders.len(), "library changed on disk");
            events.push(LibraryEvent::Changed);
        }

        let index = self.index.read().expect("index lock");
        for file in touched_files {
            let Some(p) = index.by_id.values().find(|p| p.main_file == file) else {
                continue;
            };
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            let hash = hash_bytes(&bytes);
            let mut known = self.known_source.lock().expect("known lock");
            if known.get(&file) != Some(&hash) {
                known.insert(file.clone(), hash);
                debug!(id = %p.id, "main file changed outside the app");
                events.push(LibraryEvent::SourceChangedOnDisk(p.id.clone()));
            }
        }
        events
    }

    fn remember_source(&self, main_file: &Path) {
        if let Ok(bytes) = std::fs::read(main_file) {
            self.known_source
                .lock()
                .expect("known lock")
                .insert(main_file.to_path_buf(), hash_bytes(&bytes));
        }
    }
}

/// Index one folder: a playground, a broken one, or nothing.
fn load_folder(index: &mut Index, dir: &Path) {
    if !dir.join(meta::FILE_NAME).is_file() {
        return;
    }
    match meta::read(dir) {
        Ok(p) => {
            if let Some(other) = index.by_id.get(&p.id).filter(|o| o.dir != p.dir) {
                let message = format!(
                    "id {} is already used by {}; give one of them a new id",
                    p.id,
                    other.dir.display()
                );
                warn!(dir = %dir.display(), "{message}");
                index.broken.insert(
                    dir.to_path_buf(),
                    BrokenPlayground {
                        dir: dir.to_path_buf(),
                        message,
                    },
                );
            } else {
                index.insert(p);
            }
        }
        Err(message) => {
            warn!(dir = %dir.display(), "{message}");
            index.broken.insert(
                dir.to_path_buf(),
                BrokenPlayground {
                    dir: dir.to_path_buf(),
                    message,
                },
            );
        }
    }
}

/// Write a file only if nothing is there yet. An existing file is left alone,
/// so scaffolding never clobbers a user's work (and the first of two entries
/// for the same path wins).
fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), LibraryError> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| LibraryError::io(parent, e))?;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => file.write_all(bytes).map_err(|e| LibraryError::io(path, e)),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            debug!(file = %path.display(), "exists; not overwritten");
            Ok(())
        }
        Err(e) => Err(LibraryError::io(path, e)),
    }
}

/// What to call a copy: `Notes` becomes `Notes copy`, and a copy of that
/// `Notes copy 2`. The user renames it in a second if they care.
fn copy_name(name: &str) -> String {
    let base = name
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .trim_end();
    match base.strip_suffix("copy") {
        Some(before) => {
            let n: u32 = name[base.len()..].trim().parse().unwrap_or(1);
            format!("{}copy {}", before, n + 1)
        }
        None => format!("{name} copy"),
    }
}

/// Every file of a playground folder worth copying, as (relative path, bytes).
fn copy_into(
    dir: &Path,
    prefix: &Path,
    files: &mut Vec<(PathBuf, Vec<u8>)>,
) -> Result<(), LibraryError> {
    let entries = std::fs::read_dir(dir).map_err(|e| LibraryError::io(dir, e))?;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == meta::FILE_NAME || name == history::HISTORY_FILE {
            continue; // Written fresh for the copy, or not the copy's.
        }
        if path.is_dir() {
            if matches!(&*name, "node_modules" | "target" | "bin" | "obj" | ".git") {
                continue;
            }
            copy_into(&path, &prefix.join(&*name), files)?;
        } else if let Ok(bytes) = std::fs::read(&path) {
            files.push((prefix.join(&*name), bytes));
        }
    }
    Ok(())
}

fn modified_ms(p: &Playground) -> u64 {
    std::fs::metadata(&p.main_file)
        .or_else(|_| std::fs::metadata(p.dir.join(meta::FILE_NAME)))
        .and_then(|m| m.modified())
        .map_or(p.created_ms, meta::to_ms)
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod retry_tests {
    use super::retrying;
    use std::cell::Cell;

    #[test]
    fn something_that_works_first_time_is_not_slept_on() {
        let started = std::time::Instant::now();
        let calls = Cell::new(0);
        let result: Result<u32, ()> = retrying(|| {
            calls.set(calls.get() + 1);
            Ok(7)
        });
        assert_eq!(result, Ok(7));
        assert_eq!(calls.get(), 1);
        assert!(started.elapsed() < std::time::Duration::from_millis(20));
    }

    #[test]
    fn a_refusal_that_passes_is_waited_out() {
        let calls = Cell::new(0);
        // Refused twice, as a watcher on its way out would.
        let result: Result<&str, &str> = retrying(|| {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                Err("busy")
            } else {
                Ok("moved")
            }
        });
        assert_eq!(result, Ok("moved"));
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn a_refusal_that_stays_gives_back_the_last_words() {
        let calls = Cell::new(0);
        let result: Result<(), String> = retrying(|| {
            calls.set(calls.get() + 1);
            Err(format!("still busy ({})", calls.get()))
        });
        // Every delay is tried, and the message is from the final attempt
        // rather than the first.
        assert_eq!(calls.get(), super::RETRY_DELAYS_MS.len() + 1);
        assert_eq!(result, Err("still busy (6)".to_owned()));
    }

    #[test]
    fn it_gives_up_in_about_a_second() {
        let total: u64 = super::RETRY_DELAYS_MS.iter().sum();
        assert!(
            (700..=1500).contains(&total),
            "long enough for a watcher, short enough to answer while the user is looking: {total} ms"
        );
    }
}
