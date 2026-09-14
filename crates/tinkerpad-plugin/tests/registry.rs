//! Discovering plugins from bundled and user folders. Each test builds its
//! own folders in a temp directory from the fixtures used by `load.rs`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tinkerpad_plugin::{Os, PluginRegistry, Source};

fn fixture(path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(path)
}

/// A scratch folder removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-registry-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn install(root: &Path, fixture_name: &str, folder: &str) {
    copy_dir(&fixture(fixture_name), &root.join(folder));
}

fn ids(registry: &PluginRegistry) -> Vec<String> {
    registry
        .all()
        .iter()
        .map(|e| e.plugin.id.to_string())
        .collect()
}

#[test]
fn loads_bundled_then_user_sorted_by_id() {
    let s = Scratch::new("both");
    let bundled = s.path("bundled");
    let user = s.path("user");
    install(&bundled, "valid/per-os", "python");
    install(&bundled, "valid/minimal", "mini");
    install(&user, "valid/full", "rust");

    let r = PluginRegistry::load_for(Some(bundled), user, Os::Linux);
    assert_eq!(ids(&r), ["mini", "py", "rust"]);
    assert!(r.failures().is_empty());
    assert!(r.overrides().is_empty());

    assert_eq!(r.get("rust").unwrap().name, "Rust");
    assert!(r.get("nope").is_none());
    let sources: Vec<Source> = r.all().iter().map(|e| e.source).collect();
    assert_eq!(sources, [Source::Bundled, Source::Bundled, Source::User]);
}

#[test]
fn by_extension_ignores_case_and_leading_dot() {
    let s = Scratch::new("ext");
    let bundled = s.path("bundled");
    install(&bundled, "valid/full", "rust");
    install(&bundled, "valid/per-os", "python");

    let r = PluginRegistry::load_for(Some(bundled), s.path("user"), Os::Linux);
    assert_eq!(r.by_extension("rs").unwrap().id, "rust");
    assert_eq!(r.by_extension(".RS").unwrap().id, "rust");
    assert_eq!(r.by_extension("py").unwrap().id, "py");
    assert!(r.by_extension("cs").is_none());
}

#[test]
fn user_plugin_overrides_bundled_plugin_with_same_id() {
    let s = Scratch::new("override");
    let bundled = s.path("bundled");
    let user = s.path("user");
    install(&bundled, "valid/minimal", "mini");
    install(&user, "valid/minimal", "my-mini");
    // Make the user copy distinguishable.
    let manifest = user.join("my-mini/plugin.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap()
        .replace("name = \"Mini\"", "name = \"My Mini\"");
    std::fs::write(&manifest, text).unwrap();

    let r = PluginRegistry::load_for(Some(bundled.clone()), user.clone(), Os::Linux);
    assert_eq!(ids(&r), ["mini"]);
    assert_eq!(r.get("mini").unwrap().name, "My Mini");
    assert_eq!(r.all()[0].source, Source::User);

    let o = &r.overrides()[0];
    assert_eq!(o.id, "mini");
    assert_eq!(o.bundled_dir, bundled.join("mini"));
    assert_eq!(o.user_dir, user.join("my-mini"));
}

#[test]
fn a_broken_plugin_is_reported_and_the_rest_still_load() {
    let s = Scratch::new("broken");
    let bundled = s.path("bundled");
    let user = s.path("user");
    install(&bundled, "invalid/bad-regex", "broken");
    install(&bundled, "valid/minimal", "mini");
    install(&user, "invalid/unknown-key", "typo");
    install(&user, "valid/per-os", "python");

    let r = PluginRegistry::load_for(Some(bundled.clone()), user.clone(), Os::Linux);
    assert_eq!(ids(&r), ["mini", "py"]);

    let failures = r.failures();
    assert_eq!(failures.len(), 2);
    assert_eq!(failures[0].dir, bundled.join("broken"));
    assert_eq!(failures[0].source, Source::Bundled);
    assert!(
        failures[0].message.contains("error_regex"),
        "{}",
        failures[0].message
    );
    assert_eq!(failures[1].dir, user.join("typo"));
    assert_eq!(failures[1].source, Source::User);
    assert!(
        failures[1]
            .message
            .contains("did you mean `idle_shutdown`?"),
        "{}",
        failures[1].message
    );
}

#[test]
fn two_folders_with_one_id_in_the_same_root() {
    let s = Scratch::new("dup");
    let bundled = s.path("bundled");
    install(&bundled, "valid/minimal", "a-mini");
    install(&bundled, "valid/minimal", "b-mini");

    let r = PluginRegistry::load_for(Some(bundled.clone()), s.path("user"), Os::Linux);
    assert_eq!(ids(&r), ["mini"]);
    assert_eq!(r.all()[0].plugin.dir, bundled.join("a-mini"));
    assert_eq!(r.failures().len(), 1);
    assert_eq!(r.failures()[0].dir, bundled.join("b-mini"));
    assert!(r.failures()[0].message.contains("already used"));
    assert!(
        r.overrides().is_empty(),
        "same-root duplicates aren't overrides"
    );
}

#[test]
fn folders_without_a_manifest_and_loose_files_are_ignored() {
    let s = Scratch::new("ignored");
    let bundled = s.path("bundled");
    install(&bundled, "valid/minimal", "mini");
    std::fs::create_dir_all(bundled.join("not-a-plugin/templates")).unwrap();
    std::fs::write(bundled.join("README.md"), "# plugins\n").unwrap();

    let r = PluginRegistry::load_for(Some(bundled), s.path("user"), Os::Linux);
    assert_eq!(ids(&r), ["mini"]);
    assert!(r.failures().is_empty());
}

#[test]
fn missing_folders_are_tolerated_and_user_folder_is_created() {
    let s = Scratch::new("missing");
    let user = s.path("does/not/exist/yet");
    let r = PluginRegistry::load_for(Some(s.path("no-bundled")), user.clone(), Os::Linux);
    assert!(r.all().is_empty());
    assert!(r.failures().is_empty());
    assert!(user.is_dir(), "user plugin folder should be created");

    let r = PluginRegistry::load_for(None, user, Os::Linux);
    assert!(r.all().is_empty());
}

#[test]
fn reload_picks_up_changes() {
    let s = Scratch::new("reload");
    let user = s.path("user");
    let mut r = PluginRegistry::load_for(None, user.clone(), Os::Linux);
    assert!(r.all().is_empty());

    install(&user, "valid/minimal", "mini");
    r.reload();
    assert_eq!(ids(&r), ["mini"]);

    std::fs::remove_dir_all(user.join("mini")).unwrap();
    r.reload();
    assert!(r.all().is_empty());
}

/// Work item 01-02: loading three plugins should add under 5 ms to startup.
/// The assertion is loose so shared CI machines don't flake; the printed
/// median is the number that matters.
#[test]
fn loading_three_plugins_is_fast() {
    let s = Scratch::new("timing");
    let bundled = s.path("bundled");
    install(&bundled, "valid/minimal", "mini");
    install(&bundled, "valid/per-os", "python");
    install(&bundled, "valid/full", "rust");
    let user = s.path("user");

    // Warm the filesystem cache once, as a second app launch would be.
    let _ = PluginRegistry::load_for(Some(bundled.clone()), user.clone(), Os::Linux);

    let mut samples: Vec<Duration> = (0..25)
        .map(|_| {
            let started = Instant::now();
            let r = PluginRegistry::load_for(Some(bundled.clone()), user.clone(), Os::Linux);
            let elapsed = started.elapsed();
            assert_eq!(r.all().len(), 3);
            elapsed
        })
        .collect();
    samples.sort();
    let median = samples[samples.len() / 2];
    eprintln!(
        "loading three plugins: median {:.2} ms, max {:.2} ms",
        median.as_secs_f64() * 1000.0,
        samples.last().unwrap().as_secs_f64() * 1000.0
    );
    assert!(median < Duration::from_millis(50), "median {median:?}");
}
