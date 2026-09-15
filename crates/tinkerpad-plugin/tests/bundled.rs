//! The plugins shipped in the repository's `plugins/` folder: each must load
//! on every OS without problems or warnings, and every kind must scaffold.

use std::path::{Path, PathBuf};

use tinkerpad_core::KindId;
use tinkerpad_library::{Library, NewPlayground};
use tinkerpad_plugin::scaffold;
use tinkerpad_plugin::{Os, Plugin};

fn bundled_dirs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("plugin.toml").is_file())
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn bundled_plugins_load_cleanly_on_every_os() {
    let dirs = bundled_dirs();
    assert!(
        dirs.iter().any(|d| d.ends_with("typescript")),
        "expected the TypeScript plugin in {dirs:?}"
    );
    for dir in dirs {
        for os in Os::ALL {
            let plugin = Plugin::load_for(&dir, os)
                .unwrap_or_else(|e| panic!("{} on {os:?}: {e}", dir.display()));
            assert!(
                plugin.warnings.is_empty(),
                "{} on {os:?} warns: {:?}",
                dir.display(),
                plugin.warnings
            );
            assert!(plugin.run.is_some(), "{} has no [run]", dir.display());
            assert!(
                plugin.install_hint.is_some(),
                "{} has no install hint",
                dir.display()
            );
            assert!(
                dir.join("README.md").is_file(),
                "{} has no README.md",
                dir.display()
            );
        }
    }
}

#[test]
fn typescript_kinds_scaffold_an_es_module_playground() {
    let root = std::env::temp_dir().join(format!("tinkerpad-bundled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib = Library::open(&root).unwrap();
    let plugin =
        Plugin::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/typescript"))
            .unwrap();

    assert_eq!(plugin.id, "typescript");
    assert_eq!(plugin.main_path, Path::new("main.ts"));
    let kinds: Vec<&str> = plugin.kinds.iter().map(|k| k.id.as_str()).collect();
    assert_eq!(kinds, ["statements", "program"]);
    assert!(plugin.kinds[0].default);

    for kind in &plugin.kinds {
        let name = format!("TS {}", kind.id);
        let files = scaffold::playground_files(&plugin, kind, &name, "2026-09-15").unwrap();
        let p = lib
            .create(NewPlayground {
                name: name.clone(),
                language: plugin.id.clone(),
                kind: KindId::new(kind.id.clone()),
                main_path: plugin.main_path.clone(),
                files,
            })
            .unwrap();

        let main = std::fs::read_to_string(&p.main_file).unwrap();
        assert!(
            main.starts_with("import { dump } from \"tinkerpad\";\n"),
            "{}: {main}",
            kind.id
        );
        assert!(
            main.contains(&format!("{name}, created 2026-09-15")),
            "{main}"
        );
        assert!(!main.contains("{{"), "unrendered placeholder in {main}");
        if kind.id == "program" {
            assert!(main.contains("function main()") && main.trim_end().ends_with("main();"));
        }

        let package: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p.dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(package["type"], "module");
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn csharp_kinds_scaffold_with_the_dump_library_wired_in() {
    let root = std::env::temp_dir().join(format!("tinkerpad-bundled-cs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib = Library::open(&root).unwrap();
    let plugin =
        Plugin::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/csharp")).unwrap();

    assert_eq!(plugin.id, "csharp");
    assert_eq!(plugin.main_path, Path::new("main.cs"));
    let kinds: Vec<&str> = plugin.kinds.iter().map(|k| k.id.as_str()).collect();
    assert_eq!(kinds, ["statements", "program", "expression"]);
    assert!(plugin.kinds[0].default);
    assert!(plugin.output.is_some(), "compile errors need [output]");
    assert!(plugin.deps.is_some(), "#:package needs [deps]");
    let run = plugin.run.as_ref().unwrap();
    assert!(
        run.env
            .iter()
            .any(|(k, v)| k == "TINKERPAD_CSHARP_DUMP" && v.contains("{plugin_dir}")),
        "{:?}",
        run.env
    );

    for kind in &plugin.kinds {
        let name = format!("CS {}", kind.id);
        let files = scaffold::playground_files(&plugin, kind, &name, "2026-09-15").unwrap();
        let p = lib
            .create(NewPlayground {
                name: name.clone(),
                language: plugin.id.clone(),
                kind: KindId::new(kind.id.clone()),
                main_path: plugin.main_path.clone(),
                files,
            })
            .unwrap();
        let main = std::fs::read_to_string(&p.main_file).unwrap();
        let props = std::fs::read_to_string(p.dir.join("Directory.Build.props")).unwrap();
        assert!(props.contains("$(TINKERPAD_CSHARP_DUMP)"), "{props}");
        assert!(props.contains("<PublishAot>false</PublishAot>"), "{props}");

        match kind.id.as_str() {
            "expression" => {
                assert_eq!(
                    scaffold::locked(&main, kind),
                    scaffold::Locked::Regions {
                        head: "TinkerPad.Dump.Show(\n".len(),
                        tail: "\n);\n".len()
                    }
                );
            }
            other => {
                assert!(
                    main.contains(&format!("{name}, created 2026-09-15")),
                    "{main}"
                );
                assert!(main.contains("using TinkerPad;"), "{other}: {main}");
                assert_eq!(scaffold::locked(&main, kind), scaffold::Locked::None);
            }
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}
