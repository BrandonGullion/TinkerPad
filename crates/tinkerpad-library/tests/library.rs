//! The library against a real temp folder, including the file watcher.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use tinkerpad_core::{KindId, LanguageId, PlaygroundId};
use tinkerpad_library::{Library, LibraryError, LibraryEvent, METADATA_FILE, NewPlayground};

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

/// Only a folder the index holds as broken can be sent to the trash by folder,
/// whatever path the window asks with (work item 04-12). No trash needed: every
/// one of these is refused before anything is moved.
#[test]
fn only_a_broken_folder_is_deleted_by_folder() {
    let s = Scratch::new("broken-refused");
    std::fs::create_dir_all(s.0.join("notes")).unwrap();
    let lib = Library::open(&s.0).unwrap();
    let good = lib.create(new("Good")).unwrap();

    for dir in [
        s.0.join("notes"),
        good.dir.clone(),
        s.0.clone(),
        s.0.join("missing"),
    ] {
        assert!(
            matches!(lib.delete_broken(&dir), Err(LibraryError::NotBroken(_))),
            "{} was not refused",
            dir.display()
        );
        assert!(
            dir == s.0.join("missing") || dir.exists(),
            "{} was touched",
            dir.display()
        );
    }
    assert!(lib.get(&good.id).is_some());
}

/// A broken playground goes to the trash and out of the listing.
#[test]
fn a_broken_playground_can_be_deleted() {
    if std::env::var_os("TINKERPAD_TEST_TRASH").is_none() {
        eprintln!("skipping: set TINKERPAD_TEST_TRASH=1 to test the recycle bin");
        return;
    }
    let s = Scratch::new("broken-trash");
    let dir = s.0.join("broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(METADATA_FILE), "id = \"nope\"\n").unwrap();
    let lib = Library::open(&s.0).unwrap();
    assert_eq!(lib.broken().len(), 1);

    lib.delete_broken(&dir).unwrap();
    assert!(!dir.exists());
    assert!(lib.broken().is_empty());
    assert!(lib.list().broken.is_empty());
    assert!(
        matches!(lib.delete_broken(&dir), Err(LibraryError::NotBroken(_))),
        "already gone"
    );
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

#[test]
fn create_with_scaffolds_from_the_folder_and_keeps_plugin_metadata() {
    let s = Scratch::new("create-with");
    let lib = Library::open(&s.0).unwrap();
    lib.create(new("Taken")).unwrap();

    let mut seen = None;
    let p = lib
        .create_with(new("Taken"), |claimed| {
            seen = Some((claimed.folder.to_owned(), claimed.id.clone()));
            Ok(tinkerpad_library::Scaffold {
                files: vec![(
                    "Cargo.toml".into(),
                    format!("name = \"{}\"\n", claimed.folder).into_bytes(),
                )],
                plugin_meta: [("plugin_dir".to_owned(), "C:/plugins/rust".to_owned())].into(),
            })
        })
        .unwrap();

    let (folder_seen, id_seen) = seen.unwrap();
    assert_eq!(folder_seen, "taken-2");
    assert_eq!(folder(&p.dir), "taken-2");
    assert_eq!(id_seen, p.id, "the callback sees the final id");
    assert_eq!(
        std::fs::read_to_string(p.dir.join("Cargo.toml")).unwrap(),
        "name = \"taken-2\"\n"
    );
    let meta = std::fs::read_to_string(p.dir.join(METADATA_FILE)).unwrap();
    assert!(meta.contains("[plugin.csharp]"), "{meta}");
    assert!(meta.contains("plugin_dir = \"C:/plugins/rust\""), "{meta}");

    // A rename rewrites playground.toml; the plugin table survives it.
    let renamed = lib.rename(&p.id, "Renamed").unwrap();
    let meta = std::fs::read_to_string(renamed.dir.join(METADATA_FILE)).unwrap();
    assert!(meta.contains("plugin_dir = \"C:/plugins/rust\""), "{meta}");
    assert!(meta.contains("name = \"Renamed\""), "{meta}");

    // And the folder still reads back as a playground.
    lib.rescan();
    assert!(
        lib.list()
            .playgrounds
            .iter()
            .any(|x| x.playground.id == p.id)
    );
    assert!(lib.list().broken.is_empty(), "{:?}", lib.list().broken);
}

#[test]
fn create_with_failure_leaves_nothing_behind() {
    let s = Scratch::new("create-with-fail");
    let lib = Library::open(&s.0).unwrap();
    let err = lib
        .create_with(new("Nope"), |_| Err("template missing".into()))
        .unwrap_err();
    assert!(err.to_string().contains("template missing"));
    assert!(!s.0.join("nope").exists());
    assert!(lib.list().playgrounds.is_empty());
}

#[test]
fn writing_identical_source_leaves_the_file_alone() {
    let s = Scratch::new("identical");
    let lib = Library::open(&s.0).unwrap();
    let p = lib.create(new("Same")).unwrap();

    assert!(
        lib.write_source(&p.id, "one\n").unwrap(),
        "new text is written"
    );
    let before = std::fs::metadata(&p.main_file).unwrap().modified().unwrap();
    std::thread::sleep(Duration::from_millis(1100));
    assert!(
        !lib.write_source(&p.id, "one\n").unwrap(),
        "identical text isn't"
    );
    let after = std::fs::metadata(&p.main_file).unwrap().modified().unwrap();
    assert_eq!(before, after, "modification time unchanged");
    assert!(lib.write_source(&p.id, "two\n").unwrap());
    assert_eq!(std::fs::read_to_string(&p.main_file).unwrap(), "two\n");
}

// ---- search, tags, pinning, duplicating (work item 04-01) -----------------

/// A playground with source in it, so there is something to search.
fn with_source(
    lib: &Library,
    name: &str,
    tags: &[&str],
    source: &str,
) -> tinkerpad_core::Playground {
    let p = lib.create(new(name)).unwrap();
    lib.write_source(&p.id, source).unwrap();
    if !tags.is_empty() {
        let tags: Vec<String> = tags.iter().map(|t| (*t).to_owned()).collect();
        lib.set_tags(&p.id, &tags).unwrap();
    }
    lib.get(&p.id).unwrap()
}

fn found(lib: &Library, query: &str) -> Vec<String> {
    lib.search(query, 20)
        .into_iter()
        .map(|hit| hit.summary.playground.name)
        .collect()
}

#[test]
fn search_finds_a_playground_by_name_tag_or_source() {
    let s = Scratch::new("search");
    let lib = Library::open(&s.0).unwrap();
    with_source(
        &lib,
        "Fizz Buzz",
        &["katas"],
        "for (var i = 1; i < 100; i++) { }
",
    );
    with_source(
        &lib,
        "Shapes",
        &["json"],
        "var doc = Parse(fizzy);
",
    );
    with_source(
        &lib,
        "Empty",
        &[],
        "var x = 1;
",
    );

    assert_eq!(
        found(&lib, "fizz"),
        ["Fizz Buzz", "Shapes"],
        "name before source"
    );
    assert_eq!(found(&lib, "katas"), ["Fizz Buzz"]);
    assert_eq!(found(&lib, "json"), ["Shapes"]);
    assert_eq!(found(&lib, "var").len(), 3, "all three have `var` in them");
    assert!(found(&lib, "nothing at all").is_empty());

    let hit = lib.search("fizzy", 10).remove(0);
    assert_eq!(hit.field, Some(tinkerpad_library::MatchField::Source));
    assert_eq!(hit.snippet.as_deref(), Some("var doc = Parse(fizzy);"));
}

#[test]
fn a_saved_edit_is_searchable_at_once() {
    let s = Scratch::new("search-edit");
    let lib = Library::open(&s.0).unwrap();
    let p = with_source(
        &lib,
        "Notes",
        &[],
        "nothing here
",
    );
    assert!(found(&lib, "lighthouse").is_empty());
    lib.write_source(
        &p.id,
        "// lighthouse
",
    )
    .unwrap();
    assert_eq!(found(&lib, "lighthouse"), ["Notes"], "no rescan needed");
}

#[test]
fn tags_are_tidied_and_kept_in_the_file() {
    let s = Scratch::new("tags");
    let lib = Library::open(&s.0).unwrap();
    let p = lib.create(new("Tagged")).unwrap();
    let tags = [
        "json".to_owned(),
        "  ".to_owned(),
        "JSON".to_owned(),
        " api ".to_owned(),
    ];
    let tagged = lib.set_tags(&p.id, &tags).unwrap();
    assert_eq!(
        tagged.tags,
        ["json", "api"],
        "blank dropped, duplicate collapsed, trimmed"
    );

    let text = std::fs::read_to_string(p.dir.join(METADATA_FILE)).unwrap();
    assert!(text.contains("tags = [\"json\", \"api\"]"), "{text}");
    lib.rescan();
    assert_eq!(lib.get(&p.id).unwrap().tags, ["json", "api"]);
}

#[test]
fn pinning_survives_a_rescan_and_sorts_first() {
    let s = Scratch::new("pinned");
    let lib = Library::open(&s.0).unwrap();
    let first = lib.create(new("Aaa")).unwrap();
    let second = lib.create(new("Zzz")).unwrap();
    lib.set_pinned(&second.id, true).unwrap();
    lib.rescan();

    assert!(lib.get(&second.id).unwrap().pinned);
    assert!(!lib.get(&first.id).unwrap().pinned);
    assert_eq!(found(&lib, "")[0], "Zzz", "pinned first");

    let text = std::fs::read_to_string(first.dir.join(METADATA_FILE)).unwrap();
    assert!(
        !text.contains("pinned"),
        "unpinned stays out of the file: {text}"
    );

    lib.set_pinned(&second.id, false).unwrap();
    lib.rescan();
    assert!(!lib.get(&second.id).unwrap().pinned);
}

#[test]
fn duplicating_copies_the_code_and_not_the_mess() {
    let s = Scratch::new("duplicate");
    let lib = Library::open(&s.0).unwrap();
    let p = with_source(
        &lib,
        "Notes",
        &["keep"],
        "// the code
",
    );
    std::fs::write(
        p.dir.join("runs.jsonl"),
        "{}
",
    )
    .unwrap();
    std::fs::write(
        p.dir.join("helper.json"),
        "{}
",
    )
    .unwrap();
    std::fs::create_dir_all(p.dir.join("node_modules").join("left-pad")).unwrap();
    std::fs::write(
        p.dir.join("node_modules").join("left-pad").join("index.js"),
        "x",
    )
    .unwrap();

    let copy = lib.duplicate(&p.id).unwrap();
    assert_eq!(copy.name, "Notes copy");
    assert_ne!(copy.id, p.id);
    assert_ne!(copy.dir, p.dir);
    assert_eq!(copy.tags, ["keep"], "tags come along");
    assert_eq!(
        lib.read_source(&copy.id).unwrap(),
        "// the code
"
    );
    assert!(
        copy.dir.join("helper.json").is_file(),
        "other files come along"
    );
    assert!(
        !copy.dir.join("runs.jsonl").exists(),
        "the history does not"
    );
    assert!(
        !copy.dir.join("node_modules").exists(),
        "nor what a toolchain filled in"
    );

    // A copy of a copy counts up rather than nesting "copy copy".
    assert_eq!(lib.duplicate(&copy.id).unwrap().name, "Notes copy 2");
    assert_eq!(lib.search("", 20).len(), 3);
}

#[test]
fn search_of_a_big_library_is_quick() {
    let s = Scratch::new("search-speed");
    let lib = Library::open(&s.0).unwrap();
    // 2,000 playgrounds of a few lines each, as the work item asks for. Built
    // by hand: `create` for each would spend the test's time on the file
    // system rather than on what is being measured.
    let source = "using System;
var total = 0;
for (var i = 0; i < 100; i++) total += i;
Console.WriteLine(total);
";
    for n in 0..2_000u32 {
        let dir = s.0.join(format!("playground-{n}"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("main.cs"),
            format!(
                "// playground {n}
{source}"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join(METADATA_FILE),
            format!(
                "id = \"{}\"
name = \"Playground {n}\"
language = \"csharp\"
kind = \"statements\"
main = \"main.cs\"
tags = [\"generated\"]
",
                ulid_like(n)
            ),
        )
        .unwrap();
    }
    lib.rescan();
    assert_eq!(lib.search("", 5_000).len(), 2_000);

    for query in ["console", "playground 1999", "generated", "nothing"] {
        let started = Instant::now();
        let hits = lib.search(query, 50);
        let took = started.elapsed();
        println!("search {query:?}: {} hits in {:?}", hits.len(), took);
        // Twenty milliseconds is the budget; debug builds are a few times
        // slower than what ships, so the assertion is loose and the number
        // printed above is the measurement.
        assert!(took < Duration::from_millis(200), "{query:?} took {took:?}");
    }
}

/// A valid ULID per number, so the fixtures above are readable ids.
fn ulid_like(n: u32) -> String {
    format!("01M30D7HT6YP8ESCX8XR4{:05}", n)
}
