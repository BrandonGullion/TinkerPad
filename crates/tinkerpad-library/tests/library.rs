//! The library against a real temp folder, including the file watcher.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tinkerpad_core::{KindId, LanguageId, PlaygroundId};
use tinkerpad_library::{Library, LibraryEvent, METADATA_FILE, NewPlayground};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tinkerpad-lib-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn new(name: &str) -> NewPlayground {
    NewPlayground {
        name: name.into(),
        language: LanguageId::new("csharp"),
        kind: KindId::new("statements"),
        main_path: "main.cs".into(),
        files: Vec::new(),
    }
}

fn folder(p: &Path) -> String {
    p.file_name().unwrap().to_string_lossy().into_owned()
}

/// Wait until `f` is true, or fail after `timeout`.
fn eventually(timeout: Duration, what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("timed out waiting for: {what}");
}

/// Watchers report changes up to a second or so late on some platforms
/// (macOS FSEvents); CI machines are slow. Generous, but still bounded.
const WATCH_TIMEOUT: Duration = Duration::from_secs(10);

#[test]
fn open_creates_root_with_readme() {
    let s = Scratch::new("open");
    let lib = Library::open(&s.0).unwrap();
    assert!(s.0.join("README.md").is_file());
    assert!(lib.list().playgrounds.is_empty());
}

#[test]
fn create_writes_folder_metadata_and_main_file() {
    let s = Scratch::new("create");
    let lib = Library::open(&s.0).unwrap();
    let p = lib.create(new("Fizz Buzz")).unwrap();

    assert_eq!(p.name, "Fizz Buzz");
    assert_eq!(folder(&p.dir), "fizz-buzz");
    assert_eq!(p.main_file, p.dir.join("main.cs"));
    assert!(p.main_file.is_file());
    assert!(p.dir.join(METADATA_FILE).is_file());
    assert_eq!(p.id.as_str().len(), 26, "ULID");
    assert!(p.created_ms > 0);

    let text = std::fs::read_to_string(p.dir.join(METADATA_FILE)).unwrap();
    assert!(text.contains("name = \"Fizz Buzz\""), "{text}");
    assert!(text.contains("language = \"csharp\""), "{text}");
    assert!(text.contains("main = \"main.cs\""), "{text}");
}

#[test]
fn create_writes_given_files_including_nested_ones() {
    let s = Scratch::new("files");
    let lib = Library::open(&s.0).unwrap();
    let p = lib
        .create(NewPlayground {
            name: "Rusty".into(),
            language: LanguageId::new("rust"),
            kind: KindId::new("program"),
            main_path: "src/main.rs".into(),
            files: vec![
                ("src/main.rs".into(), b"fn main() {}\n".to_vec()),
                ("Cargo.toml".into(), b"[package]\n".to_vec()),
            ],
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&p.main_file).unwrap(),
        "fn main() {}\n"
    );
    assert!(p.dir.join("Cargo.toml").is_file());

    let err = lib
        .create(NewPlayground {
            files: vec![("../escape".into(), Vec::new())],
            ..new("Bad")
        })
        .unwrap_err();
    assert!(err.to_string().contains("relative"), "{err}");
    assert!(
        !s.0.join("bad").exists(),
        "a failed create leaves nothing behind"
    );
}

#[test]
fn name_collisions_get_numbered_folders() {
    let s = Scratch::new("collide");
    let lib = Library::open(&s.0).unwrap();
    let a = lib.create(new("Scratch")).unwrap();
    let b = lib.create(new("scratch")).unwrap();
    let c = lib.create(new("SCRATCH!")).unwrap();
    assert_eq!(folder(&a.dir), "scratch");
    assert_eq!(folder(&b.dir), "scratch-2");
    assert_eq!(folder(&c.dir), "scratch-3");
    assert_eq!(b.name, "scratch");
    assert_ne!(a.id, b.id);
    assert!(lib.create(new("   ")).is_err());
}

#[test]
fn list_is_newest_first_with_previews() {
    let s = Scratch::new("list");
    let lib = Library::open(&s.0).unwrap();
    let old = lib.create(new("Old")).unwrap();
    lib.write_source(&old.id, "\n\n   // the old one  \nmore\n")
        .unwrap();
    std::thread::sleep(Duration::from_millis(1100)); // mtime resolution on some filesystems
    let newer = lib.create(new("Newer")).unwrap();
    lib.write_source(&newer.id, &format!("{}\n", "x".repeat(300)))
        .unwrap();

    let listing = lib.list();
    let names: Vec<&str> = listing
        .playgrounds
        .iter()
        .map(|p| p.playground.name.as_str())
        .collect();
    assert_eq!(names, ["Newer", "Old"]);
    assert_eq!(listing.playgrounds[1].preview, "// the old one");
    assert_eq!(
        listing.playgrounds[0].preview.chars().count(),
        121,
        "120 chars and an ellipsis"
    );
    assert!(listing.playgrounds[0].modified_ms >= listing.playgrounds[1].modified_ms);
    assert_eq!(listing.root, s.0);
}

#[test]
fn rename_moves_folder_and_keeps_id() {
    let s = Scratch::new("rename");
    let lib = Library::open(&s.0).unwrap();
    let p = lib.create(new("First Name")).unwrap();
    lib.write_source(&p.id, "keep me").unwrap();
    let taken = lib.create(new("Taken")).unwrap();

    let r = lib.rename(&p.id, "Second Name").unwrap();
    assert_eq!(r.id, p.id);
    assert_eq!(folder(&r.dir), "second-name");
    assert!(!p.dir.exists());
    assert_eq!(lib.read_source(&p.id).unwrap(), "keep me");
    assert_eq!(lib.get(&p.id).unwrap().name, "Second Name");

    // Renaming onto an existing folder name gets a number.
    let r = lib.rename(&p.id, "taken").unwrap();
    assert_eq!(folder(&r.dir), "taken-2");
    assert!(taken.dir.exists());

    // A name that slugs to the same folder only changes the display name.
    let r = lib.rename(&p.id, "TAKEN 2").unwrap();
    assert_eq!(folder(&r.dir), "taken-2");
    assert_eq!(r.name, "TAKEN 2");

    // Survives a rescan from disk.
    lib.rescan();
    let again = lib.get(&p.id).unwrap();
    assert_eq!(again.name, "TAKEN 2");
    assert_eq!(again.dir, r.dir);
}

#[test]
fn write_source_is_atomic_and_leaves_no_temp_files() {
    let s = Scratch::new("write");
    let lib = Library::open(&s.0).unwrap();
    let p = lib.create(new("W")).unwrap();
    for i in 0..20 {
        lib.write_source(&p.id, &format!("version {i}\n")).unwrap();
    }
    assert_eq!(lib.read_source(&p.id).unwrap(), "version 19\n");
    let leftovers: Vec<_> = std::fs::read_dir(&p.dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().contains("tinkerpad-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert!(
        lib.read_source(&PlaygroundId::new("01J00000000000000000000000"))
            .is_err()
    );
}

#[test]
fn non_playground_folders_are_ignored_and_broken_ones_reported() {
    let s = Scratch::new("broken");
    std::fs::create_dir_all(s.0.join("notes")).unwrap();
    std::fs::create_dir_all(s.0.join("broken")).unwrap();
    std::fs::write(s.0.join("broken").join(METADATA_FILE), "id = \"nope\"\n").unwrap();

    let lib = Library::open(&s.0).unwrap();
    let good = lib.create(new("Good")).unwrap();
    // A copied folder duplicates the id.
    let copy = s.0.join("good-copy");
    std::fs::create_dir_all(&copy).unwrap();
    std::fs::copy(good.dir.join(METADATA_FILE), copy.join(METADATA_FILE)).unwrap();
    lib.rescan();

    let listing = lib.list();
    assert_eq!(listing.playgrounds.len(), 1);
    let broken: Vec<String> = listing.broken.iter().map(|b| folder(&b.dir)).collect();
    assert_eq!(broken, ["broken", "good-copy"]);
    assert!(
        listing.broken[0].message.contains(METADATA_FILE),
        "{}",
        listing.broken[0].message
    );
    assert!(
        listing.broken[1].message.contains("already used"),
        "{}",
        listing.broken[1].message
    );
}

#[test]
fn watcher_sees_external_create_delete_and_edit_but_not_our_writes() {
    let s = Scratch::new("watch");
    let lib = Library::open(&s.0).unwrap();
    let (tx, rx) = mpsc::channel();
    let _watcher = lib.watch(move |e| tx.send(e).unwrap()).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    let ours = lib.create(new("Ours")).unwrap();
    lib.rename(&ours.id, "Ours Renamed").unwrap();
    let ours = lib.get(&ours.id).unwrap();
    // Our own create must not be reported as a change once the watcher
    // catches up with it.
    std::thread::sleep(Duration::from_millis(1000));
    let early: Vec<LibraryEvent> = rx.try_iter().collect();
    // A create and rename made through the library itself are already in the
    // index, so the watcher has nothing to report.
    assert!(
        early.is_empty(),
        "our own create and rename were reported: {early:?}"
    );

    // An external create: another app writes a playground folder.
    let other = Library::open(
        std::env::temp_dir().join(format!("tinkerpad-lib-{}-watch-source", std::process::id())),
    )
    .unwrap();
    let made = other.create(new("Made Elsewhere")).unwrap();
    let dest = s.0.join("made-elsewhere");
    std::fs::rename(&made.dir, &dest).unwrap();
    let _ = std::fs::remove_dir_all(other.root());

    eventually(WATCH_TIMEOUT, "external playground indexed", || {
        lib.get(&made.id).is_some_and(|p| p.dir == dest)
    });
    assert!(rx.recv_timeout(WATCH_TIMEOUT).unwrap() == LibraryEvent::Changed);

    // Our own save doesn't count as an external change.
    while rx.try_recv().is_ok() {}
    lib.write_source(&ours.id, "saved by the app\n").unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let spurious: Vec<LibraryEvent> = rx.try_iter().collect();
    assert!(
        !spurious.contains(&LibraryEvent::SourceChangedOnDisk(ours.id.clone())),
        "{spurious:?}"
    );

    // An edit from another editor does.
    std::fs::write(&ours.main_file, "edited elsewhere\n").unwrap();
    let deadline = Instant::now() + WATCH_TIMEOUT;
    let mut saw = false;
    while Instant::now() < deadline && !saw {
        if let Ok(e) = rx.recv_timeout(Duration::from_millis(200)) {
            saw = e == LibraryEvent::SourceChangedOnDisk(ours.id.clone());
        }
    }
    assert!(saw, "no SourceChangedOnDisk for an external edit");

    // An external delete.
    std::fs::remove_dir_all(&dest).unwrap();
    eventually(WATCH_TIMEOUT, "deleted playground removed", || {
        lib.get(&made.id).is_none()
    });
}

/// Moves a folder to the real recycle bin, so it only runs when asked
/// (CI sets TINKERPAD_TEST_TRASH=1).
#[test]
fn delete_moves_to_trash() {
    if std::env::var_os("TINKERPAD_TEST_TRASH").is_none() {
        eprintln!("skipping: set TINKERPAD_TEST_TRASH=1 to test the recycle bin");
        return;
    }
    let s = Scratch::new("trash");
    let lib = Library::open(&s.0).unwrap();
    let p = lib.create(new("Throw Away")).unwrap();
    lib.delete(&p.id).unwrap();
    assert!(!p.dir.exists());
    assert!(lib.get(&p.id).is_none());
    assert!(lib.delete(&p.id).is_err(), "already gone");
}

#[test]
fn scaffolding_never_overwrites_and_first_entry_wins() {
    let s = Scratch::new("no-overwrite");
    let lib = Library::open(&s.0).unwrap();
    let p = lib
        .create(NewPlayground {
            files: vec![
                ("main.cs".into(), b"from the template\n".to_vec()),
                ("main.cs".into(), b"from a scaffold entry\n".to_vec()),
                ("sub/a.txt".into(), b"a\n".to_vec()),
            ],
            ..new("Twice")
        })
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&p.main_file).unwrap(),
        "from the template\n"
    );
    assert_eq!(
        std::fs::read_to_string(p.dir.join("sub/a.txt")).unwrap(),
        "a\n"
    );
}
