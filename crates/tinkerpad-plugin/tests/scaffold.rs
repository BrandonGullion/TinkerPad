//! Creating playgrounds of every kind of the test plugins through the real
//! library, then checking the files on disk and the locked regions derived from
//! them. (No plugins are bundled yet; 01-09 to 01-11 add them, and their kinds
//! should be added here.)

use std::path::{Path, PathBuf};

use tinkerpad_core::KindId;
use tinkerpad_library::{Library, NewPlayground};
use tinkerpad_plugin::scaffold::{self, Locked};
use tinkerpad_plugin::{Os, Plugin};

fn fixture(name: &str) -> Plugin {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/valid")
        .join(name);
    Plugin::load_for(dir, Os::current()).unwrap()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-scaffold-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Create a playground of `kind` exactly as the app does.
fn create(lib: &Library, plugin: &Plugin, kind: &str, name: &str) -> tinkerpad_core::Playground {
    let kind = plugin.kind(kind).unwrap();
    let files = scaffold::playground_files(plugin, kind, name, "2026-09-14").unwrap();
    lib.create(NewPlayground {
        name: name.into(),
        language: plugin.id.clone(),
        kind: KindId::new(kind.id.clone()),
        main_path: plugin.main_path.clone(),
        files,
    })
    .unwrap()
}

#[test]
fn every_kind_of_every_test_plugin() {
    let s = Scratch::new("kinds");
    let lib = Library::open(&s.0).unwrap();

    for plugin_name in ["node-check", "full", "minimal", "per-os"] {
        let plugin = fixture(plugin_name);
        for kind in &plugin.kinds {
            let p = create(
                &lib,
                &plugin,
                &kind.id,
                &format!("{} {}", plugin.id, kind.id),
            );
            let text = std::fs::read_to_string(&p.main_file).unwrap();

            let template = std::fs::read_to_string(&kind.template).unwrap();
            let expected = format!(
                "{}{}{}",
                kind.prelude.as_deref().unwrap_or(""),
                scaffold::render(&template, &p.name, "2026-09-14"),
                kind.postlude.as_deref().unwrap_or("")
            );
            assert_eq!(text, expected, "{plugin_name}/{}", kind.id);
            assert_eq!(p.kind, kind.id.as_str());

            let expect_locked = if kind.prelude.is_some() || kind.postlude.is_some() {
                Locked::Regions {
                    head: kind.prelude.as_deref().map_or(0, str::len),
                    tail: kind.postlude.as_deref().map_or(0, str::len),
                }
            } else {
                Locked::None
            };
            assert_eq!(
                scaffold::locked(&text, kind),
                expect_locked,
                "{plugin_name}/{}",
                kind.id
            );
        }
    }
}

#[test]
fn node_check_expression_kind_on_disk() {
    let s = Scratch::new("expression");
    let lib = Library::open(&s.0).unwrap();
    let plugin = fixture("node-check");

    let p = create(&lib, &plugin, "expression", "Sum");
    let text = std::fs::read_to_string(&p.main_file).unwrap();
    assert_eq!(text, "console.log(\n1 + 1\n);\n");
    let kind = plugin.kind("expression").unwrap();
    assert_eq!(
        scaffold::locked(&text, kind),
        Locked::Regions { head: 13, tail: 4 }
    );

    let script = create(&lib, &plugin, "script", "Hello There");
    assert_eq!(
        std::fs::read_to_string(&script.main_file).unwrap(),
        "// Hello There, created 2026-09-14\nconsole.log(\"hi\");\n"
    );

    // Edited outside the app so the wrapper is gone: nothing is locked.
    std::fs::write(&p.main_file, "1 + 1\n").unwrap();
    let text = std::fs::read_to_string(&p.main_file).unwrap();
    assert_eq!(scaffold::locked(&text, kind), Locked::Mismatch);
}

#[test]
fn scaffold_files_are_rendered_and_nested() {
    let s = Scratch::new("files");
    let lib = Library::open(&s.0).unwrap();
    let plugin = fixture("full"); // Cargo.toml and .cargo/config.toml, main at src/main.rs

    let p = create(&lib, &plugin, "program", "Rusty");
    assert_eq!(p.main_file, p.dir.join("src").join("main.rs"));
    assert!(p.main_file.is_file());
    assert_eq!(
        std::fs::read_to_string(p.dir.join("Cargo.toml")).unwrap(),
        "[package]\nname = \"Rusty\"\n"
    );
    assert!(p.dir.join(".cargo").join("config.toml").is_file());
}

#[test]
fn templates_are_read_fresh_every_time() {
    let s = Scratch::new("fresh");
    let lib = Library::open(&s.0).unwrap();

    // A private copy of the plugin, so the template can be edited.
    let plugin_dir = s.0.with_file_name(format!(
        "tinkerpad-scaffold-{}-fresh-plugin",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&plugin_dir);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/valid/minimal");
    std::fs::create_dir_all(plugin_dir.join("templates")).unwrap();
    std::fs::copy(src.join("plugin.toml"), plugin_dir.join("plugin.toml")).unwrap();
    let template = plugin_dir.join("templates/statements.mini");
    std::fs::write(&template, "first\n").unwrap();

    let plugin = Plugin::load_for(&plugin_dir, Os::current()).unwrap();
    let a = create(&lib, &plugin, "statements", "A");
    std::fs::write(&template, "second\n").unwrap();
    let b = create(&lib, &plugin, "statements", "B");

    assert_eq!(std::fs::read_to_string(&a.main_file).unwrap(), "first\n");
    assert_eq!(std::fs::read_to_string(&b.main_file).unwrap(), "second\n");
    let _ = std::fs::remove_dir_all(plugin_dir);
}
