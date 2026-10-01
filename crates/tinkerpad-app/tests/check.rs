//! `tinkerpad plugin check` over plugins built for the occasion (work item
//! 05-01a).
//!
//! The fixtures use `tinkerpad-faketool` as their whole toolchain, so these
//! run the same on every machine and need no language installed: a plugin
//! whose probe and whose `[run]` are both the fake tool exercises every path
//! the checker has, deterministically.

use std::path::{Path, PathBuf};

use tinkerpad_app::check::{Options, Outcome, check};

const FAKETOOL: &str = env!("CARGO_BIN_EXE_tinkerpad-faketool");

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-checkfx-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("templates")).unwrap();
        Self(dir)
    }

    /// Write a plugin whose manifest is `manifest`, with one template.
    fn plugin(&self, manifest: &str) -> &Path {
        std::fs::write(self.0.join("templates/main.x"), "the template\n").unwrap();
        std::fs::write(self.0.join("plugin.toml"), manifest).unwrap();
        &self.0
    }

    /// Put a file in the plugin's `check/` folder.
    fn sample(&self, name: &str, text: &str) -> &Self {
        let dir = self.0.join("check");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(format!("{name}.x")), text).unwrap();
        self
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A TOML literal string, so Windows backslashes need no escaping.
fn lit(s: &str) -> String {
    format!("'{s}'")
}

/// A manifest whose toolchain and runner are both the fake tool. `run_steps`
/// are the fake tool's arguments, so a test picks what the run does.
fn manifest(run_steps: &str) -> String {
    format!(
        r#"api = 1

[language]
id = "fake"
name = "Fake"
extension = "x"

[toolchain]
probe = [{{ command = {tool}, args = ["print", "9.9.9"], min_version = "1.0" }}]
install_hint = "Install the fake tool"

[[kind]]
id = "statements"
name = "Statements"
template = "main.x"
default = true

[run]
command = {tool}
args = [{run_steps}]
"#,
        tool = lit(FAKETOOL),
    )
}

fn run_check(dir: &Path, only: Option<&str>) -> (tinkerpad_app::check::Report, Vec<String>) {
    let mut lines = Vec::new();
    let report = check(
        &Options {
            dir: dir.to_path_buf(),
            json: false,
            only: only.map(str::to_owned),
        },
        &mut |line| lines.push(line.to_owned()),
    );
    (report, lines)
}

fn outcome(report: &tinkerpad_app::check::Report, id: &str) -> Option<Outcome> {
    report
        .checks
        .iter()
        .find(|check| check.id == id)
        .map(|check| check.outcome)
}

/// A manifest whose run prints `run_steps`, with an `[output]` pattern that
/// finds `line N: message`.
fn manifest_with_output(run_steps: &str) -> String {
    format!(
        "{}\n[output]\nerror_regex = 'line (?<line>\\d+): (?<message>.+)'\n",
        manifest(run_steps)
    )
}

#[test]
fn a_plugin_that_works_passes_every_check() {
    let s = Scratch::new("good");
    let dir = s.plugin(&manifest(r#""print", "hello from the fake tool""#));
    let (report, lines) = run_check(dir, None);

    assert_eq!(report.failed(), 0, "{lines:#?}");
    assert_eq!(report.exit_code(), 0);
    assert_eq!(outcome(&report, "manifest"), Some(Outcome::Pass));
    assert_eq!(outcome(&report, "toolchain"), Some(Outcome::Pass));
    assert_eq!(outcome(&report, "templates"), Some(Outcome::Pass));
    assert_eq!(outcome(&report, "run:statements"), Some(Outcome::Pass));
    // The report says something about each, and ends with a count.
    assert!(
        lines.last().is_some_and(|last| last.contains("4 passed")),
        "{lines:#?}"
    );
}

#[test]
fn a_manifest_that_does_not_load_fails_and_says_why_verbatim() {
    let s = Scratch::new("bad-manifest");
    let dir = s.plugin("api = 1\n[language]\nid = \"fake\"\nnmae = \"typo\"\n");
    let (report, lines) = run_check(dir, None);

    assert_eq!(report.exit_code(), 1);
    assert_eq!(outcome(&report, "manifest"), Some(Outcome::Fail));
    let detail = &report.checks[0].detail;
    // The loader's own words, including its suggestion (01-01).
    assert!(detail.contains("nmae"), "{detail}");
    assert!(detail.contains("did you mean"), "{detail}");
    assert!(detail.contains("plugin.toml"), "{detail}");
    // And nothing after it is pretended to have been checked.
    for id in ["toolchain", "templates", "run"] {
        assert_eq!(outcome(&report, id), Some(Outcome::Skip), "{lines:#?}");
    }
}

#[test]
fn a_toolchain_that_is_not_here_skips_rather_than_fails() {
    let s = Scratch::new("no-toolchain");
    let dir = s.plugin(
        &manifest(r#""print", "x""#).replace(&lit(FAKETOOL), "'tp-definitely-not-installed'"),
    );
    let (report, _) = run_check(dir, None);

    // A laptop without Go can still say the Go plugin's manifest is sound.
    assert_eq!(report.exit_code(), 0);
    assert_eq!(outcome(&report, "manifest"), Some(Outcome::Pass));
    assert_eq!(outcome(&report, "toolchain"), Some(Outcome::Skip));
    let detail = &report.checks[1].detail;
    assert!(detail.contains("tp-definitely-not-installed"), "{detail}");
    // The plugin's own instructions, which are what the user needs next.
    assert!(detail.contains("Install the fake tool"), "{detail}");
}

#[test]
fn a_run_that_fails_is_reported_with_what_it_said() {
    let s = Scratch::new("bad-run");
    let dir = s.plugin(&manifest(r#""stderr", "it all went wrong", "exit", "3""#));
    let (report, _) = run_check(dir, None);

    assert_eq!(report.exit_code(), 1);
    assert_eq!(outcome(&report, "run:statements"), Some(Outcome::Fail));
    let detail = &report
        .checks
        .iter()
        .find(|c| c.id == "run:statements")
        .unwrap()
        .detail;
    assert!(detail.contains("exit 3"), "{detail}");
    assert!(detail.contains("it all went wrong"), "{detail}");
}

#[test]
fn a_run_that_prints_nothing_is_a_failure_even_at_exit_zero() {
    let s = Scratch::new("silent-run");
    // Exits cleanly and says nothing: a kind's template should show something,
    // or the first thing a user sees is an empty pane.
    let dir = s.plugin(&manifest(r#""exit", "0""#));
    let (report, _) = run_check(dir, None);

    assert_eq!(report.exit_code(), 1);
    let detail = &report
        .checks
        .iter()
        .find(|c| c.id == "run:statements")
        .unwrap()
        .detail;
    assert!(detail.contains("printed nothing"), "{detail}");
}

#[test]
fn a_template_that_is_not_there_fails_before_anything_runs() {
    let s = Scratch::new("no-template");
    let dir = s.plugin(&manifest(r#""print", "x""#));
    std::fs::remove_file(dir.join("templates/main.x")).unwrap();
    let (report, _) = run_check(dir, None);

    assert_eq!(report.exit_code(), 1);
    // The loader checks templates exist, so this is caught at the manifest.
    assert!(
        outcome(&report, "manifest") == Some(Outcome::Fail)
            || outcome(&report, "templates") == Some(Outcome::Fail),
        "{:#?}",
        report.checks
    );
}

#[test]
fn only_runs_one_check_and_does_not_do_the_others_work() {
    let s = Scratch::new("only");
    // The run would take a second; `--only manifest` must not wait for it.
    let dir = s.plugin(&manifest(r#""sleep", "2000", "print", "x""#));
    let started = std::time::Instant::now();
    let (report, _) = run_check(dir, Some("manifest"));

    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.checks[0].id, "manifest");
    assert!(
        started.elapsed() < std::time::Duration::from_millis(1500),
        "--only manifest should not probe or run: took {:?}",
        started.elapsed()
    );
}

#[test]
fn only_can_name_one_kinds_run() {
    let s = Scratch::new("only-run");
    let dir = s.plugin(&manifest(r#""print", "x""#));
    let (report, _) = run_check(dir, Some("run:statements"));

    // Earlier stages still have to happen — a run needs a scaffolded
    // playground — but only the asked-for check is reported.
    assert_eq!(report.checks.len(), 1);
    assert_eq!(report.checks[0].id, "run:statements");
    assert_eq!(report.checks[0].outcome, Outcome::Pass);
}

#[test]
fn nothing_is_left_behind_in_the_users_library() {
    let s = Scratch::new("tidy");
    // An id of its own, so the folders counted below belong to this check and
    // not to whichever other test is running beside it.
    let dir =
        s.plugin(&manifest(r#""print", "x""#).replace(r#"id = "fake""#, r#"id = "tidyfake""#));
    let left_behind = || {
        std::fs::read_dir(std::env::temp_dir())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("tinkerpad-check-tidyfake-")
            })
            .count()
    };
    assert_eq!(left_behind(), 0, "nothing of this check's should exist yet");
    let (report, _) = run_check(dir, None);
    assert_eq!(report.failed(), 0);
    assert_eq!(
        left_behind(),
        0,
        "the check's temporary library should be gone"
    );
}

// --- the checks that need a `check/` folder (work item 05-01b) -------------

#[test]
fn a_dump_that_matches_the_convention_passes() {
    let s = Scratch::new("dump-good");
    let dir = s.plugin(&manifest(r#""table", "good""#)).to_path_buf();
    s.sample("dump", "ignored: the fake tool prints the table itself");
    let (report, _) = run_check(&dir, Some("dump"));

    assert_eq!(
        outcome(&report, "dump"),
        Some(Outcome::Pass),
        "{:#?}",
        report.checks
    );
    assert!(
        report.checks[0].detail.contains("2 rows"),
        "{:?}",
        report.checks[0]
    );
}

#[test]
fn a_dump_with_the_wrong_columns_says_which() {
    let s = Scratch::new("dump-columns");
    let dir = s.plugin(&manifest(r#""table", "columns""#)).to_path_buf();
    s.sample("dump", "x");
    let (report, _) = run_check(&dir, Some("dump"));

    assert_eq!(outcome(&report, "dump"), Some(Outcome::Fail));
    let detail = &report.checks[0].detail;
    assert!(detail.contains("born"), "{detail}");
    assert!(detail.contains("expected"), "{detail}");
}

#[test]
fn a_run_that_dumps_nothing_is_a_failed_dump_check() {
    let s = Scratch::new("dump-none");
    let dir = s
        .plugin(&manifest(r#""print", "no dump here""#))
        .to_path_buf();
    s.sample("dump", "x");
    let (report, _) = run_check(&dir, Some("dump"));

    assert_eq!(outcome(&report, "dump"), Some(Outcome::Fail));
    assert!(
        report.checks[0].detail.contains("no table"),
        "{:?}",
        report.checks[0]
    );
}

#[test]
fn no_check_folder_skips_and_says_what_to_add() {
    let s = Scratch::new("dump-missing");
    let dir = s.plugin(&manifest(r#""print", "x""#)).to_path_buf();
    let (report, _) = run_check(&dir, Some("dump"));

    assert_eq!(outcome(&report, "dump"), Some(Outcome::Skip));
    // The reason names the file the author has to write.
    assert!(
        report.checks[0].detail.contains("check/dump.x"),
        "{:?}",
        report.checks[0]
    );
}

#[test]
fn an_error_sample_that_produces_a_position_passes() {
    let s = Scratch::new("error-good");
    let dir = s
        .plugin(&manifest_with_output(
            r#""stderr", "line 7: it went wrong", "exit", "1""#,
        ))
        .to_path_buf();
    s.sample("error", "x");
    let (report, _) = run_check(&dir, Some("errors"));

    assert_eq!(
        outcome(&report, "errors"),
        Some(Outcome::Pass),
        "{:#?}",
        report.checks
    );
    assert!(
        report.checks[0].detail.contains("line 7"),
        "{:?}",
        report.checks[0]
    );
}

#[test]
fn an_error_sample_the_pattern_misses_fails() {
    let s = Scratch::new("error-missed");
    let dir = s
        .plugin(&manifest_with_output(
            r#""stderr", "something went wrong somewhere", "exit", "1""#,
        ))
        .to_path_buf();
    s.sample("error", "x");
    let (report, _) = run_check(&dir, Some("errors"));

    assert_eq!(outcome(&report, "errors"), Some(Outcome::Fail));
    assert!(
        report.checks[0].detail.contains("matched nothing"),
        "{:?}",
        report.checks[0]
    );
}

#[test]
fn an_error_sample_that_succeeds_is_not_testing_anything() {
    let s = Scratch::new("error-passes");
    let dir = s
        .plugin(&manifest_with_output(r#""print", "all fine""#))
        .to_path_buf();
    s.sample("error", "x");
    let (report, _) = run_check(&dir, Some("errors"));

    assert_eq!(outcome(&report, "errors"), Some(Outcome::Fail));
    assert!(
        report.checks[0].detail.contains("should contain a mistake"),
        "{:?}",
        report.checks[0]
    );
}

#[test]
fn a_plugin_with_no_output_section_skips_the_error_check() {
    let s = Scratch::new("error-nopattern");
    let dir = s.plugin(&manifest(r#""print", "x""#)).to_path_buf();
    s.sample("error", "x");
    let (report, _) = run_check(&dir, Some("errors"));

    assert_eq!(outcome(&report, "errors"), Some(Outcome::Skip));
    assert!(
        report.checks[0].detail.contains("no [output].error_regex"),
        "{:?}",
        report.checks[0]
    );
}
