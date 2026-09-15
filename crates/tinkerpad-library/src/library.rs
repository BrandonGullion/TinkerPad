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
use crate::{fsutil, meta, platform, slug};

/// How long file events are collected before being handled, so one save
/// (which some platforms report several times) is handled once.
pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Longest preview line, in characters.
const PREVIEW_CHARS: usize = 120;
/// How much of a main file is read to find the preview line.
const PREVIEW_BYTES: usize = 16 * 1024;

const README: &str = "# TinkerPad library

Each folder here is a playground created by TinkerPad. A folder is a playground
if it contains a `playground.toml`; anything else is ignored, so this folder is
safe to keep under version control or to share.

    fizz-buzz/
      playground.toml   id, name, language, kind, main file, created, tags
      main.cs           the code

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
    /// Keyed by folder.
    broken: BTreeMap<PathBuf, BrokenPlayground>,
}

impl Index {
    fn remove_dir(&mut self, dir: &Path) {
        self.by_id.retain(|_, p| p.dir != dir);
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
    root: PathBuf,
    /// `root` canonicalised, because watchers may report paths in that form
    /// (macOS reports `/private/var/...` for `/var/...`).
    canonical_root: PathBuf,
    index: RwLock<Index>,
    /// Hash of each main file's contents as last written or seen by the app,
    /// to tell our own saves from edits made elsewhere.
    known_source: Mutex<HashMap<PathBuf, u64>>,
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
            root,
            canonical_root,
            index: RwLock::default(),
            known_source: Mutex::default(),
        });
        library.rescan();
        Ok(library)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Rebuild the index from disk.
    pub fn rescan(&self) {
        let mut dirs: Vec<PathBuf> = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect(),
            Err(e) => {
                warn!(root = %self.root.display(), error = %e, "could not read library");
                Vec::new()
            }
        };
        dirs.sort();
        let mut index = Index::default();
        for dir in dirs {
            load_folder(&mut index, &dir);
        }
        info!(
            root = %self.root.display(),
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
            let folder = slug::unique_folder(&self.root, &slug, None);
            let dir = self.root.join(&folder);
            match std::fs::create_dir(&dir) {
                Ok(()) => break dir,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(LibraryError::io(&dir, e)),
            }
        };

        let result = (|| {
            let mut wrote_main = false;
            for (rel, bytes) in &new.files {
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
                id: meta::new_id(),
                name: name.to_owned(),
                language: new.language.clone(),
                kind: new.kind.clone(),
                dir: dir.clone(),
                main_file: dir.join(&main_rel),
                // Whole seconds, as stored in playground.toml, so the
                // in-memory copy equals what a re-read produces.
                created_ms: meta::to_ms(std::time::SystemTime::now()) / 1000 * 1000,
                tags: Vec::new(),
            };
            // Metadata last, so a watcher never sees a half-made playground.
            meta::write(&playground).map_err(|e| LibraryError::io(&dir, e))?;
            Ok(playground)
        })();

        match result {
            Ok(playground) => {
                self.remember_source(&playground.main_file);
                index.remove_dir(&dir);
                index
                    .by_id
                    .insert(playground.id.clone(), playground.clone());
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
            let folder = slug::unique_folder(&self.root, &slug, Some(&current_folder));
            let new_dir = self.root.join(folder);
            std::fs::rename(&old_dir, &new_dir).map_err(|e| LibraryError::io(&old_dir, e))?;
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
        index.by_id.insert(p.id.clone(), p.clone());
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
        platform::move_to_trash(&p.dir).map_err(|message| LibraryError::Trash {
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

    /// Replace a playground's main file atomically.
    pub fn write_source(&self, id: &PlaygroundId, text: &str) -> Result<(), LibraryError> {
        let p = self.require(id)?;
        // Remember first: the watcher may see the rename before this returns.
        self.known_source
            .lock()
            .expect("known lock")
            .insert(p.main_file.clone(), hash_bytes(text.as_bytes()));
        fsutil::write_atomic(&p.main_file, text.as_bytes())
            .map_err(|e| LibraryError::io(&p.main_file, e))
    }

    /// Every playground, most recently modified first.
    pub fn list(&self) -> Listing {
        let index = self.index.read().expect("index lock");
        let mut playgrounds: Vec<PlaygroundSummary> = index
            .by_id
            .values()
            .map(|p| PlaygroundSummary {
                modified_ms: modified_ms(p),
                preview: preview(&p.main_file),
                playground: p.clone(),
            })
            .collect();
        playgrounds.sort_by(|a, b| {
            b.modified_ms
                .cmp(&a.modified_ms)
                .then_with(|| a.playground.name.cmp(&b.playground.name))
        });
        Listing {
            root: self.root.clone(),
            playgrounds,
            broken: index.broken.values().cloned().collect(),
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
            .watch(&self.root, RecursiveMode::Recursive)
            .map_err(|e| LibraryError::Watch(e.to_string()))?;
        info!(root = %self.root.display(), "watching library");
        Ok(Watcher {
            _debouncer: debouncer,
        })
    }

    /// Update the index for changed paths and say what changed.
    fn handle_paths(&self, paths: impl IntoIterator<Item = PathBuf>) -> Vec<LibraryEvent> {
        let mut folders = BTreeSet::new();
        let mut touched_files = BTreeSet::new();
        for path in paths {
            if fsutil::is_tmp(&path) {
                continue;
            }
            let rel = path
                .strip_prefix(&self.root)
                .or_else(|_| path.strip_prefix(&self.canonical_root));
            let Ok(rel) = rel else { continue };
            let mut components = rel.components();
            let Some(first) = components.next() else {
                continue;
            };
            let dir = self.root.join(first.as_os_str());
            if components.next().is_some() {
                touched_files.insert(self.root.join(rel));
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
                index.by_id.insert(p.id.clone(), p);
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

fn modified_ms(p: &Playground) -> u64 {
    std::fs::metadata(&p.main_file)
        .or_else(|_| std::fs::metadata(p.dir.join(meta::FILE_NAME)))
        .and_then(|m| m.modified())
        .map_or(p.created_ms, meta::to_ms)
}

fn preview(main_file: &Path) -> String {
    use std::io::Read;
    let mut buf = Vec::new();
    if let Ok(file) = std::fs::File::open(main_file) {
        let _ = file.take(PREVIEW_BYTES as u64).read_to_end(&mut buf);
    }
    String::from_utf8_lossy(&buf)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| {
            let mut s: String = l.chars().take(PREVIEW_CHARS).collect();
            if l.chars().count() > PREVIEW_CHARS {
                s.push('…');
            }
            s
        })
        .unwrap_or_default()
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}
