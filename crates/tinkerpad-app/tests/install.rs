//! Installing and removing plugins (work item 05-02).
//!
//! A folder is copied; a folder that isn't a plugin is refused before anything
//! is copied. Nothing here clones anything: `git` is exercised only as far as
//! "it says so clearly when it isn't there", which is the part that can be
//! tested without a network.

use std::path::PathBuf;

use tinkerpad_app::install::{self, Source};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-install-t-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugins")).unwrap();
        Self(dir)
    }

    /// Where plugins are installed to.
    fn plugins(&self) -> PathBuf {
        self.0.join("plugins")
    }

    /// A plugin folder to install from, with `extra` appended to `[language]`.
    fn source(&self, id: &str, extra: &str) -> PathBuf {
        let dir = self.0.join(format!("src-{id}"));
        std::fs::create_dir_all(dir.join("templates")).unwrap();
        std::fs::write(dir.join("templates/main.x"), "the template\n").unwrap();
        std::fs::write(
            dir.join("plugin.toml"),
            format!(
                r#"api = 1

[language]
id = "{id}"
name = "Test {id}"
extension = "x"
{extra}

[toolchain]
probe = [{{ command = "tp-nothing" }}]

[[kind]]
id = "k"
name = "K"
template = "main.x"
default = true

[run]
command = "tp-nothing"
"#
            ),
        )
        .unwrap();
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn quiet() -> impl FnMut(&str) {
    |_: &str| {}
}

#[test]
fn a_folder_is_copied_under_its_own_id() {
    let s = Scratch::new("copy");
    let from = s.source("demo", "version = \"1.2.3\"");
    let installed = install::install(&Source::Path(from), &s.plugins(), &mut quiet())
        .unwrap_or_else(|e| panic!("{e}"));

    assert_eq!(installed.id, "demo");
    assert_eq!(installed.name, "Test demo");
    assert_eq!(installed.version.as_deref(), Some("1.2.3"));
    assert!(!installed.replaced);
    // The id from the manifest names the folder, not whatever the source was
    // called: two plugins with the same id are the same plugin.
    assert_eq!(installed.dir, s.plugins().join("demo"));
    assert!(installed.dir.join("plugin.toml").is_file());
    assert!(installed.dir.join("templates/main.x").is_file());
}

#[test]
fn a_folder_that_is_not_a_plugin_is_refused_before_anything_is_copied() {
    let s = Scratch::new("notplugin");
    let empty = s.0.join("just-a-folder");
    std::fs::create_dir_all(&empty).unwrap();

    let error = install::install(&Source::Path(empty), &s.plugins(), &mut quiet())
        .expect_err("not a plugin");
    let text = error.to_string();
    assert!(text.contains("isn't a plugin"), "{text}");
    // And nothing was left behind in the plugins folder.
    assert_eq!(
        std::fs::read_dir(s.plugins()).unwrap().count(),
        0,
        "a refused install copies nothing"
    );
}

#[test]
fn a_manifest_that_does_not_load_takes_its_reason_with_it() {
    let s = Scratch::new("badmanifest");
    let dir = s.0.join("broken");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("plugin.toml"),
        "api = 1\n[language]\nid = \"x\"\nnmae = 1\n",
    )
    .unwrap();

    let error = install::install(&Source::Path(dir), &s.plugins(), &mut quiet())
        .expect_err("a typo is not a plugin");
    let text = error.to_string();
    // The loader's own words, including its suggestion (01-01).
    assert!(text.contains("nmae"), "{text}");
    assert!(text.contains("did you mean"), "{text}");
}

#[test]
fn installing_over_an_existing_one_replaces_it() {
    let s = Scratch::new("replace");
    let first = s.source("demo", "version = \"1.0.0\"");
    install::install(&Source::Path(first), &s.plugins(), &mut quiet()).unwrap();
    // A file the new copy doesn't have: it must not survive the replacement,
    // or an update would leave the old version's leftovers behind.
    std::fs::write(s.plugins().join("demo/stale.txt"), "old").unwrap();

    let again = s.0.join("src-demo2");
    std::fs::create_dir_all(again.join("templates")).unwrap();
    std::fs::write(again.join("templates/main.x"), "newer\n").unwrap();
    std::fs::copy(s.0.join("src-demo/plugin.toml"), again.join("plugin.toml")).unwrap();
    let text = std::fs::read_to_string(again.join("plugin.toml"))
        .unwrap()
        .replace("1.0.0", "2.0.0");
    std::fs::write(again.join("plugin.toml"), text).unwrap();

    let installed = install::install(&Source::Path(again), &s.plugins(), &mut quiet()).unwrap();
    assert!(installed.replaced);
    assert_eq!(installed.version.as_deref(), Some("2.0.0"));
    assert!(
        !s.plugins().join("demo/stale.txt").exists(),
        "the old copy's files should be gone"
    );
}

#[test]
fn a_repository_and_its_build_output_are_not_copied() {
    let s = Scratch::new("skip");
    let from = s.source("demo", "");
    std::fs::create_dir_all(from.join(".git")).unwrap();
    std::fs::write(from.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
    std::fs::create_dir_all(from.join("helper/obj")).unwrap();
    std::fs::write(from.join("helper/obj/build.o"), "binary").unwrap();
    std::fs::create_dir_all(from.join("node_modules/left-pad")).unwrap();

    let installed = install::install(&Source::Path(from), &s.plugins(), &mut quiet()).unwrap();
    assert!(
        !installed.dir.join(".git").exists(),
        "a repo is not a plugin"
    );
    assert!(
        !installed.dir.join("helper/obj").exists(),
        "build output is remade on first use"
    );
    assert!(!installed.dir.join("node_modules").exists());
    assert!(installed.dir.join("plugin.toml").is_file());
}

#[test]
fn a_path_install_records_no_origin_and_cannot_be_updated() {
    let s = Scratch::new("origin");
    let from = s.source("demo", "");
    let installed = install::install(&Source::Path(from), &s.plugins(), &mut quiet()).unwrap();
    assert!(
        !installed.dir.join(install::SOURCE_FILE).exists(),
        "a folder install has no URL to go back to"
    );

    let error = install::update("demo", &s.plugins(), &mut quiet()).expect_err("nothing to update");
    assert!(
        error.to_string().contains("wasn't installed from a URL"),
        "{error}"
    );
}

#[test]
fn removing_takes_the_folder_away() {
    let s = Scratch::new("remove");
    let from = s.source("demo", "");
    install::install(&Source::Path(from), &s.plugins(), &mut quiet()).unwrap();
    assert!(s.plugins().join("demo").is_dir());

    install::remove("demo", &s.plugins()).unwrap_or_else(|e| panic!("{e}"));
    assert!(
        !s.plugins().join("demo").exists(),
        "the folder should be gone from the plugins folder"
    );
}

#[test]
fn removing_something_that_is_not_installed_says_so() {
    let s = Scratch::new("remove-missing");
    let error = install::remove("nothing-here", &s.plugins()).expect_err("not installed");
    assert!(error.to_string().contains("no plugin called"), "{error}");
}

#[test]
fn updating_something_that_is_not_installed_says_so() {
    let s = Scratch::new("update-missing");
    let error =
        install::update("nothing-here", &s.plugins(), &mut quiet()).expect_err("not installed");
    assert!(error.to_string().contains("no plugin called"), "{error}");
}

#[test]
fn an_install_says_what_it_is_doing() {
    let s = Scratch::new("said");
    // A manifest with a warning in it, which the install should pass on rather
    // than swallow: an unknown highlighter still installs, but quietly losing
    // the warning is how a plugin ends up mysteriously plain.
    let from = s.source("demo", "highlighter = \"not-a-real-highlighter\"");
    let mut said = Vec::new();
    let installed = install::install(&Source::Path(from), &s.plugins(), &mut |line| {
        said.push(line.to_owned())
    })
    .unwrap();

    assert_eq!(installed.id, "demo");
    assert!(
        said.iter().any(|line| line.contains("highlighter")),
        "{said:#?}"
    );
}

/// `Source::read` decides between a copy and a clone, and a folder that
/// exists is never mistaken for a URL.
#[test]
fn an_existing_folder_is_a_path_even_with_an_at_sign_in_it() {
    let s = Scratch::new("atsign");
    let odd = s.0.join("user@host");
    std::fs::create_dir_all(&odd).unwrap();
    assert_eq!(
        Source::read(&odd.to_string_lossy()),
        Source::Path(odd.clone())
    );
    assert_eq!(
        Source::read(&odd.display().to_string()).describe(),
        odd.display().to_string()
    );
}

/// The one thing about cloning that can be checked without a network: it says
/// so clearly when `git` isn't there.
#[test]
fn a_clone_without_git_says_which_program_is_missing() {
    // Only meaningful where git genuinely isn't on PATH, which is not most
    // development machines; where it is, the error is about the URL instead.
    let s = Scratch::new("nogit");
    let error = install::install(
        &Source::Git("https://example.invalid/not/a/repo".to_owned()),
        &s.plugins(),
        &mut quiet(),
    )
    .expect_err("example.invalid is not a repository");
    let text = error.to_string();
    assert!(
        text.contains("git isn't installed") || text.contains("could not clone"),
        "{text}"
    );
}
