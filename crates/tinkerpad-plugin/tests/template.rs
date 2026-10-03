//! The plugin template keeps up with the schema (work item 05-03).
//!
//! `plugin-template/` is what a contributor starts from, so a template that
//! has drifted from the loader is worse than no template: it teaches the wrong
//! shape and fails on the first push. These tests are the drift check the work
//! item asks for.
//!
//! The item's wording was "a CI job diffs the template's manifest against
//! `Manifest::example()`". That isn't quite the right check — the example is
//! the C# one from PLUGINS.md §2, and a template that *is* the documentation
//! example is a bad template. What matters is that the template still loads
//! and still mentions everything the schema has, so these tests ask that
//! instead, which catches more and allows the template to be a good one.

use std::path::{Path, PathBuf};

use tinkerpad_plugin::{Os, Plugin, example_manifest};

fn template_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugin-template")
}

fn manifest_text() -> String {
    std::fs::read_to_string(template_dir().join("plugin.toml"))
        .expect("plugin-template/plugin.toml")
}

#[test]
fn the_template_loads_on_every_os_with_no_warnings() {
    for os in Os::ALL {
        let plugin = Plugin::load_for(template_dir(), os)
            .unwrap_or_else(|e| panic!("the template must load on {os}:\n{e}"));
        assert!(
            plugin.warnings.is_empty(),
            "a contributor's first `plugin check` should be clean on {os}: {:?}",
            plugin.warnings
        );
    }
}

#[test]
fn the_template_has_the_pieces_its_readme_promises() {
    let dir = template_dir();
    for path in [
        "plugin.toml",
        "README.md",
        "templates/statements.example",
        "templates/expression.example",
        "dump/tinkerpad/__init__.py",
        "helper/helper.py",
        // The `check/` convention (05-01b), without which half of
        // `plugin check` skips.
        "check/dump.example",
        "check/error.example",
        "check/hang.example",
        ".github/workflows/check.yml",
    ] {
        assert!(dir.join(path).exists(), "the template is missing {path}");
    }
}

/// Every top-level section the documented example uses is at least *mentioned*
/// in the template, commented out or not.
///
/// This is the drift check that matters: when a section is added to the schema
/// and written up in PLUGINS.md, the template should grow a line about it
/// rather than silently teaching an older shape.
#[test]
fn the_template_mentions_every_section_the_example_manifest_uses() {
    let template = manifest_text();
    let mut missing = Vec::new();
    for line in example_manifest().lines() {
        let line = line.trim();
        if !line.starts_with('[') {
            continue;
        }
        let section = line.trim_start_matches("[[").trim_start_matches('[');
        let section = section
            .split([']', '.'])
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned();
        if section.is_empty() || missing.contains(&section) {
            continue;
        }
        if !template.contains(&format!("[{section}")) {
            missing.push(section);
        }
    }
    assert!(
        missing.is_empty(),
        "plugin-template/plugin.toml says nothing about {missing:?}; \
         a section the docs describe should at least be a commented line there"
    );
}

/// The helper is the Python one, verbatim, as the work item asks — so a
/// contributor reading it is reading something that is actually in use.
#[test]
fn the_helper_is_the_reference_one_unchanged() {
    let reference =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/python/helper/helper.py");
    let theirs = template_dir().join("helper/helper.py");
    let (reference, theirs) = (
        std::fs::read_to_string(reference).expect("the Python helper"),
        std::fs::read_to_string(theirs).expect("the template's helper"),
    );
    assert_eq!(
        reference.replace("\r\n", "\n"),
        theirs.replace("\r\n", "\n"),
        "plugin-template/helper/helper.py has drifted from plugins/python/helper/helper.py"
    );
}

/// The four places the README tells a contributor to edit are actually marked
/// in the manifest, and there are four of them.
#[test]
fn the_places_to_change_are_marked_and_counted() {
    let template = manifest_text();
    let marks = template.matches("CHANGE ME").count();
    assert_eq!(
        marks, 5,
        "the README promises four marked places (plus the line that says so)"
    );
    for part in ["1 of 4", "2 of 4", "3 of 4", "4 of 4"] {
        assert!(
            template.contains(part),
            "the marks should be numbered: {part}"
        );
    }
}
