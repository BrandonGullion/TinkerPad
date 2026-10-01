//! `tinkerpad plugin check <dir>` (work item 05-01a).
//!
//! Loads a plugin folder the way the app loads it, probes its toolchain the way
//! the app probes it, scaffolds a playground of every kind the way the New
//! Playground button does, and runs each one the way Ctrl+Enter does. Exit code
//! 0 only if nothing failed.
//!
//! The point is the "the way the app does it" in every one of those sentences.
//! A checker with its own loader would pass plugins the app rejects, which is
//! worse than no checker at all, so this calls the same functions the window
//! calls and adds nothing but the report.
//!
//! Nothing here opens a window, touches the user's library, or needs the Tauri
//! runtime: a check is a terminal thing, and it scaffolds into a temporary
//! folder it removes afterwards.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tinkerpad_core::{KindId, ToolchainState};
use tinkerpad_library::{Library, NewPlayground};
use tinkerpad_plugin::scaffold::{self, ScaffoldVars};
use tinkerpad_plugin::{Os, PROBE_TIMEOUT, Plugin, Resolver, ToolchainOverrides, probe};
use tinkerpad_runner::spec::{RunContext, run_spec};

/// How long one kind's playground may take to run before it counts as failed.
const RUN_TIMEOUT: Duration = Duration::from_secs(60);

/// What was asked for.
#[derive(Clone, Debug)]
pub struct Options {
    pub dir: PathBuf,
    /// Machine-readable, for CI.
    pub json: bool,
    /// Run only the checks whose id starts with this, to iterate on one.
    pub only: Option<String>,
}

/// How one check went.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Outcome {
    Pass,
    Fail,
    /// Not run, and not a failure: a toolchain that isn't installed, a section
    /// the plugin doesn't have, or a check `--only` filtered out.
    Skip,
}

impl Outcome {
    fn mark(self) -> &'static str {
        match self {
            Outcome::Pass => "ok  ",
            Outcome::Fail => "FAIL",
            Outcome::Skip => "skip",
        }
    }
}

/// One line of the report.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    /// Stable, and what `--only` matches on: `manifest`, `run:statements`.
    pub id: String,
    pub outcome: Outcome,
    /// One line saying what happened, whichever way it went.
    pub detail: String,
    pub ms: u64,
}

/// Everything the check found.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// The plugin's id, once the manifest has loaded.
    pub plugin: Option<String>,
    pub name: Option<String>,
    pub checks: Vec<Check>,
}

impl Report {
    pub fn passed(&self) -> usize {
        self.count(Outcome::Pass)
    }

    pub fn failed(&self) -> usize {
        self.count(Outcome::Fail)
    }

    pub fn skipped(&self) -> usize {
        self.count(Outcome::Skip)
    }

    fn count(&self, outcome: Outcome) -> usize {
        self.checks.iter().filter(|c| c.outcome == outcome).count()
    }

    /// 0 when nothing failed. A skip is not a failure: a machine without Go
    /// installed can still say whether the Go plugin's manifest is sound.
    pub fn exit_code(&self) -> i32 {
        i32::from(self.failed() > 0)
    }
}

/// Collects checks and reports each as it finishes, so a sixty-second run says
/// something before it ends.
struct Run<'a> {
    report: Report,
    only: Option<String>,
    out: &'a mut dyn FnMut(&str),
    json: bool,
}

impl Run<'_> {
    /// Whether `--only` asked for this one.
    fn wanted(&self, id: &str) -> bool {
        match &self.only {
            Some(only) => id == only || id.starts_with(&format!("{only}:")),
            None => true,
        }
    }

    /// Record a check, unless `--only` filtered it out.
    fn add(&mut self, id: &str, outcome: Outcome, detail: impl Into<String>, started: Instant) {
        if !self.wanted(id) {
            return;
        }
        let check = Check {
            id: id.to_owned(),
            outcome,
            detail: detail.into(),
            ms: started.elapsed().as_millis() as u64,
        };
        if !self.json {
            (self.out)(&format!(
                "  {} {:<18} {}",
                check.outcome.mark(),
                check.id,
                check.detail
            ));
        }
        self.report.checks.push(check);
    }

    /// Whether `--only` named this stage and nothing after it, so the work the
    /// later checks would do can be skipped rather than done and discarded.
    /// `--only manifest` should be instant, not a toolchain probe with its
    /// result thrown away.
    fn done_after(&self, stage: &str) -> bool {
        self.only.as_deref() == Some(stage)
    }

    /// Record every remaining check as skipped, for the same reason.
    fn skip_rest(&mut self, ids: &[String], why: &str) {
        for id in ids {
            self.add(id, Outcome::Skip, why, Instant::now());
        }
    }
}

/// Check a plugin folder. `out` is given each line of the human-readable
/// report as it happens; the JSON form is returned by the caller instead.
pub fn check(options: &Options, out: &mut dyn FnMut(&str)) -> Report {
    let mut run = Run {
        report: Report::default(),
        only: options.only.clone(),
        out,
        json: options.json,
    };

    // 1. The manifest, with the loader the app uses and its errors verbatim.
    //
    // The folder is made absolute first. `{plugin_dir}` goes into commands and
    // into environment variables like PYTHONPATH, and a run's working
    // directory is the playground rather than wherever the check was typed —
    // so a relative path here becomes a plugin that can't find its own files.
    // The app never has this problem because it loads plugins from absolute
    // folders; the checker has to match it.
    let started = Instant::now();
    let dir = std::path::absolute(&options.dir).unwrap_or_else(|_| options.dir.clone());
    let plugin = match Plugin::load_for(&dir, Os::current()) {
        Ok(plugin) => plugin,
        Err(error) => {
            // Every problem at once, which is what the loader is built to do
            // (01-01); a checker that showed only the first would make a
            // contributor run it once per typo.
            run.add(
                "manifest",
                Outcome::Fail,
                one_line(&error.to_string()),
                started,
            );
            run.skip_rest(
                &["toolchain", "templates", "run"].map(String::from),
                "the manifest didn't load",
            );
            return finish(run, options);
        }
    };
    let warnings = plugin.warnings.len();
    run.report.plugin = Some(plugin.id.to_string());
    run.report.name = Some(plugin.name.clone());
    run.add(
        "manifest",
        Outcome::Pass,
        match warnings {
            0 => "loads, no warnings".to_owned(),
            1 => format!("loads, 1 warning: {}", one_line(&plugin.warnings[0])),
            n => format!("loads, {n} warnings: {}", one_line(&plugin.warnings[0])),
        },
        started,
    );
    if run.done_after("manifest") {
        return finish(run, options);
    }

    // The rest needs somewhere to work: a library and a scratch folder of this
    // check's own, both thrown away at the end.
    let Some(work) = Workspace::new(&plugin.id) else {
        run.add(
            "toolchain",
            Outcome::Fail,
            "could not make a temporary folder to check in",
            Instant::now(),
        );
        return finish(run, options);
    };

    // 2. The toolchain, through the same probe and the same resolver.
    let started = Instant::now();
    let resolver = Resolver::new(
        std::env::var_os("PATH").unwrap_or_default(),
        ToolchainOverrides::default(),
    );
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            run.add("toolchain", Outcome::Fail, error.to_string(), started);
            return finish(run, options);
        }
    };
    let state = runtime.block_on(probe(
        &plugin,
        &resolver,
        &work.scratch.join(plugin.id.as_str()),
        PROBE_TIMEOUT,
    ));
    let (program, version) = match &state {
        ToolchainState::Ready {
            program,
            path,
            version,
        } => {
            run.add(
                "toolchain",
                Outcome::Pass,
                format!(
                    "{program} {} at {path}",
                    version.as_deref().unwrap_or("(no version)")
                ),
                started,
            );
            (program.clone(), version.clone().unwrap_or_default())
        }
        ToolchainState::Missing { reason, hint } => {
            // Not a failure of the plugin: this machine hasn't got the
            // language. Everything after it needs the toolchain, so it is
            // skipped with the plugin's own instructions.
            let why = match hint {
                Some(hint) => format!("{reason} — {}", one_line(hint)),
                None => reason.clone(),
            };
            run.add("toolchain", Outcome::Skip, why.clone(), started);
            run.skip_rest(
                &["templates", "run"].map(String::from),
                "the toolchain isn't installed",
            );
            return finish(run, options);
        }
        ToolchainState::Checking => {
            run.add(
                "toolchain",
                Outcome::Fail,
                "the probe never finished",
                started,
            );
            return finish(run, options);
        }
    };

    if run.done_after("toolchain") {
        return finish(run, options);
    }

    // 3. Templates and scaffold files, for every kind.
    let started = Instant::now();
    let library = match Library::open(&work.library) {
        Ok(library) => library,
        Err(error) => {
            run.add("templates", Outcome::Fail, error.to_string(), started);
            return finish(run, options);
        }
    };
    let mut made = Vec::new();
    let mut template_problem = None;
    for kind in &plugin.kinds {
        match scaffold_one(&library, &plugin, &kind.id, &work.scratch) {
            Ok(playground) => made.push((kind.id.clone(), playground)),
            Err(error) => {
                template_problem = Some(format!("{}: {error}", kind.id));
                break;
            }
        }
    }
    match &template_problem {
        Some(problem) => {
            run.add("templates", Outcome::Fail, one_line(problem), started);
            let ids: Vec<String> = plugin
                .kinds
                .iter()
                .map(|kind| format!("run:{}", kind.id))
                .collect();
            run.skip_rest(&ids, "nothing was scaffolded");
            return finish(run, options);
        }
        None => {
            let files = plugin.scaffold.len();
            run.add(
                "templates",
                Outcome::Pass,
                format!(
                    "{} kind{}, {files} scaffold file{}",
                    made.len(),
                    plural(made.len()),
                    plural(files)
                ),
                started,
            );
        }
    }

    if run.done_after("templates") {
        return finish(run, options);
    }

    // 4. Each kind actually runs, through the same spec and the same spawn.
    for (kind, playground) in &made {
        let id = format!("run:{kind}");
        if !run.wanted(&id) {
            continue;
        }
        let started = Instant::now();
        let ctx = RunContext {
            resolver: &resolver,
            scratch_root: &work.scratch,
            toolchain_program: &program,
            toolchain_version: &version,
        };
        let spec = match run_spec(&plugin, playground, &ctx) {
            Ok(spec) => spec,
            Err(error) => {
                run.add(&id, Outcome::Fail, one_line(&error.to_string()), started);
                continue;
            }
        };
        let (outcome, detail) = runtime.block_on(run_once(spec));
        run.add(&id, outcome, detail, started);
    }

    finish(run, options)
}

/// Run one playground and say how it went.
async fn run_once(spec: tinkerpad_runner::RunSpec) -> (Outcome, String) {
    let mut handle = match tinkerpad_runner::run(spec) {
        Ok(handle) => handle,
        Err(error) => return (Outcome::Fail, one_line(&error.to_string())),
    };
    let canceller = handle.canceller();
    let mut stdout = 0usize;
    let mut stderr = Vec::new();
    let finished = tokio::time::timeout(RUN_TIMEOUT, async {
        while let Some(event) = handle.recv().await {
            match event {
                tinkerpad_runner::RunEvent::Stdout(bytes) => stdout += bytes.len(),
                tinkerpad_runner::RunEvent::Stderr(bytes) => stderr.extend_from_slice(&bytes),
                tinkerpad_runner::RunEvent::Exited { code, duration } => {
                    return Some((code, duration));
                }
                _ => {}
            }
        }
        None
    })
    .await;

    match finished {
        Err(_) => {
            // A run with no end is a failed check, not a hung checker.
            canceller.cancel();
            (
                Outcome::Fail,
                format!("still running after {} s", RUN_TIMEOUT.as_secs()),
            )
        }
        Ok(None) => (Outcome::Fail, "the run stopped without exiting".to_owned()),
        Ok(Some((code, duration))) => {
            let ms = duration.as_millis();
            match code {
                Some(0) if stdout > 0 => (
                    Outcome::Pass,
                    format!("exit 0 in {ms} ms, {stdout} bytes of stdout"),
                ),
                Some(0) => (
                    Outcome::Fail,
                    format!(
                        "exit 0 in {ms} ms but printed nothing; a kind's template should show something"
                    ),
                ),
                Some(code) => (
                    Outcome::Fail,
                    format!("exit {code} in {ms} ms{}", tail(&stderr)),
                ),
                None => (Outcome::Fail, format!("stopped by a signal after {ms} ms")),
            }
        }
    }
}

/// Scaffold one kind's playground, exactly as the New Playground button does.
fn scaffold_one(
    library: &Library,
    plugin: &Plugin,
    kind_id: &str,
    scratch_root: &Path,
) -> Result<tinkerpad_core::Playground, String> {
    let kind = plugin
        .kinds
        .iter()
        .find(|k| k.id == kind_id)
        .ok_or_else(|| format!("no kind `{kind_id}`"))?;
    let name = format!("Check {kind_id}");
    let date = scaffold::today_utc();
    let scratch = tinkerpad_runner::spec::scratch_dir(scratch_root, plugin);
    library
        .create_with(
            NewPlayground {
                name: name.clone(),
                language: plugin.id.clone(),
                kind: KindId::new(kind.id.clone()),
                main_path: plugin.main_path.clone(),
                files: Vec::new(),
            },
            |claimed| {
                let package = scaffold::package_name(claimed.folder, claimed.id.as_str());
                let vars = ScaffoldVars {
                    name: &name,
                    date: &date,
                    folder: claimed.folder,
                    package: &package,
                    plugin_dir: &plugin.dir,
                    scratch: &scratch,
                };
                let out =
                    scaffold::playground_files(plugin, kind, &vars).map_err(|e| e.to_string())?;
                Ok(tinkerpad_library::Scaffold {
                    files: out.files,
                    plugin_meta: out.paths,
                })
            },
        )
        .map_err(|e| e.to_string())
}

/// A library and a scratch folder of this check's own, removed when it ends.
struct Workspace {
    root: PathBuf,
    library: PathBuf,
    scratch: PathBuf,
}

impl Workspace {
    fn new(plugin: &tinkerpad_core::LanguageId) -> Option<Workspace> {
        // Unique per check, not per process: two checks of the same plugin at
        // once — a test suite, a CI matrix — must not share a folder and
        // delete each other's work.
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "tinkerpad-check-{}-{}-{n}",
            plugin.as_str(),
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let library = root.join("library");
        let scratch = root.join("scratch");
        std::fs::create_dir_all(&library).ok()?;
        std::fs::create_dir_all(&scratch).ok()?;
        Some(Workspace {
            root,
            library,
            scratch,
        })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        // A build folder a compiler is still finishing with can refuse; the
        // temp directory is the OS's to clean up either way.
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// Print the summary and hand back the report.
fn finish(run: Run<'_>, options: &Options) -> Report {
    let Run {
        report, out, json, ..
    } = run;
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(text) => out(&text),
            Err(error) => out(&format!("{{\"error\":\"{error}\"}}")),
        }
    } else {
        let _ = options;
        out(&format!(
            "{} passed, {} failed, {} skipped",
            report.passed(),
            report.failed(),
            report.skipped()
        ));
    }
    report
}

/// The last of what a failing run said, for the one line a report gets.
fn tail(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    match text.lines().rfind(|l| !l.trim().is_empty()) {
        Some(last) => format!(": {}", clip(last.trim(), 160)),
        None => String::new(),
    }
}

/// A multi-line message as one line, so a report stays a table.
fn one_line(text: &str) -> String {
    let joined = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    clip(&joined, 300)
}

fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}…")
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_message_of_several_lines_becomes_one() {
        assert_eq!(one_line("a\n\n  b  \nc"), "a · b · c");
        assert_eq!(one_line("just one"), "just one");
    }

    #[test]
    fn a_very_long_message_is_clipped_rather_than_wrapped() {
        let long = "x".repeat(400);
        let clipped = one_line(&long);
        assert_eq!(clipped.chars().count(), 300);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn only_matches_a_check_and_its_children() {
        let run = |only: Option<&str>, id: &str| {
            let mut sink = |_: &str| {};
            Run {
                report: Report::default(),
                only: only.map(String::from),
                out: &mut sink,
                json: false,
            }
            .wanted(id)
        };
        assert!(run(None, "manifest"));
        assert!(run(Some("manifest"), "manifest"));
        assert!(!run(Some("manifest"), "toolchain"));
        // `--only run` means every kind's run.
        assert!(run(Some("run"), "run:statements"));
        assert!(!run(Some("run:program"), "run:statements"));
        assert!(run(Some("run:program"), "run:program"));
    }

    #[test]
    fn a_skip_is_not_a_failure() {
        let mut report = Report::default();
        report.checks.push(Check {
            id: "toolchain".to_owned(),
            outcome: Outcome::Skip,
            detail: "not installed".to_owned(),
            ms: 0,
        });
        assert_eq!(report.exit_code(), 0, "a machine without Go is not a bug");
        report.checks.push(Check {
            id: "manifest".to_owned(),
            outcome: Outcome::Fail,
            detail: "no".to_owned(),
            ms: 0,
        });
        assert_eq!(report.exit_code(), 1);
    }
}
