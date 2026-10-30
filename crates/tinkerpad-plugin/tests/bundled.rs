//! The plugins shipped in the repository's `plugins/` folder: each must load
//! on every OS without problems or warnings, and every kind must scaffold.

use std::path::{Path, PathBuf};

use tinkerpad_core::KindId;
use tinkerpad_library::{Library, NewPlayground, Scaffold};
use tinkerpad_plugin::scaffold::{self, ScaffoldVars};
use tinkerpad_plugin::{Os, Plugin};

const SCRATCH: &str = "/tinkerpad-data/scratch";

/// Create a playground of `kind` the way the app does.
fn create(
    lib: &Library,
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    name: &str,
    scratch: &str,
) -> tinkerpad_core::Playground {
    lib.create_with(
        NewPlayground {
            name: name.into(),
            language: plugin.id.clone(),
            kind: KindId::new(kind.id.clone()),
            main_path: plugin.main_path.clone(),
            files: Vec::new(),
        },
        |claimed| {
            let package = scaffold::package_name(claimed.folder, claimed.id.as_str());
            let vars = ScaffoldVars {
                name,
                date: "2026-09-15",
                folder: claimed.folder,
                package: &package,
                plugin_dir: &plugin.dir,
                scratch: Path::new(scratch),
            };
            let out = scaffold::playground_files(plugin, kind, &vars).map_err(|e| e.to_string())?;
            Ok(Scaffold {
                files: out.files,
                plugin_meta: out.paths,
            })
        },
    )
    .unwrap()
}

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
        let p = create(&lib, &plugin, kind, &name, SCRATCH);

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
        let p = create(&lib, &plugin, kind, &name, SCRATCH);
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

#[test]
fn rust_kinds_scaffold_a_crate_and_record_their_paths() {
    let root = std::env::temp_dir().join(format!("tinkerpad-bundled-rs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let lib = Library::open(&root).unwrap();
    let plugin =
        Plugin::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/rust")).unwrap();

    assert_eq!(plugin.id, "rust");
    assert_eq!(plugin.main_path, Path::new("src/main.rs"));
    let kinds: Vec<&str> = plugin.kinds.iter().map(|k| k.id.as_str()).collect();
    assert_eq!(kinds, ["program", "expression"]);
    assert!(plugin.kinds[0].default);
    assert!(plugin.output.as_ref().is_some_and(|o| o.multiline));
    assert!(matches!(
        plugin.deps,
        Some(tinkerpad_plugin::DepsSpec::Command { .. })
    ));

    // Two playgrounds, one with a name that isn't a valid package name.
    for (name, kind_id) in [("Fizz Buzz", "program"), ("日本 expression", "expression")] {
        let kind = plugin.kind(kind_id).unwrap();
        let p = create(&lib, &plugin, kind, name, SCRATCH);
        assert_eq!(p.main_file, p.dir.join("src").join("main.rs"));
        let folder = p.dir.file_name().unwrap().to_string_lossy().into_owned();

        let cargo: toml::Table =
            toml::from_str(&std::fs::read_to_string(p.dir.join("Cargo.toml")).unwrap()).unwrap();
        let package = cargo["package"]["name"].as_str().unwrap();
        if kind_id == "program" {
            assert_eq!(package, "fizz-buzz");
        } else {
            assert_eq!(
                package,
                format!("playground-{}", p.id.as_str().to_ascii_lowercase())
            );
        }
        let dump = cargo["dependencies"]["tinkerpad"]["path"].as_str().unwrap();
        assert_eq!(
            dump,
            format!("{}/dump", scaffold::template_path(&plugin.dir))
        );
        let config: toml::Table = toml::from_str(
            &std::fs::read_to_string(p.dir.join(".cargo").join("config.toml")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            config["build"]["target-dir"].as_str().unwrap(),
            format!("{SCRATCH}/target")
        );

        let meta: toml::Table = toml::from_str(
            &std::fs::read_to_string(p.dir.join(tinkerpad_library::METADATA_FILE)).unwrap(),
        )
        .unwrap();
        let recorded = &meta["plugin"]["rust"];
        assert_eq!(
            recorded["plugin_dir"].as_str().unwrap(),
            scaffold::template_path(&plugin.dir)
        );
        assert_eq!(recorded["scratch"].as_str().unwrap(), SCRATCH, "{folder}");

        let main = std::fs::read_to_string(&p.main_file).unwrap();
        if kind_id == "expression" {
            assert_eq!(
                scaffold::locked(&main, kind),
                scaffold::Locked::Regions {
                    head: "fn main() {\n    tinkerpad::dump(&(\n".len(),
                    tail: "\n    ));\n}\n".len()
                }
            );
        } else {
            assert!(
                main.contains("fn main()") && main.contains("Fizz Buzz, created"),
                "{main}"
            );
        }
    }

    // The TypeScript and C# plugins use no path variables, so nothing is recorded.
    let ts = Plugin::load(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/typescript"))
        .unwrap();
    let p = create(&lib, &ts, ts.default_kind(), "No Paths", SCRATCH);
    let meta = std::fs::read_to_string(p.dir.join(tinkerpad_library::METADATA_FILE)).unwrap();
    assert!(!meta.contains("[plugin"), "{meta}");
    let _ = std::fs::remove_dir_all(&root);
}

/// C#'s language server is its helper, started with `--lsp`, so the helper's
/// build has to run first (work item 04-13).
#[test]
fn csharp_serves_its_language_from_the_helper() {
    let dir = bundled_dirs()
        .into_iter()
        .find(|d| d.ends_with("csharp"))
        .expect("a csharp plugin");
    for os in Os::ALL {
        let plugin = Plugin::load_for(&dir, os).unwrap();
        let lsp = plugin.lsp.as_ref().expect("C# has a language server");
        assert!(lsp.from_helper, "{os:?}");
        assert!(lsp.command.args.iter().any(|a| a == "--lsp"), "{os:?}");
        assert!(
            plugin.helper.as_ref().is_some_and(|h| h.build.is_some()),
            "{os:?}"
        );
    }
}
