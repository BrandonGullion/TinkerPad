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

/// The table every plugin's `check/dump.<ext>` must dump (work item 05-01b).
///
/// Fixed so the checker can assert rather than guess, and small enough to
/// write in any language in three lines. A plugin that produces exactly this
/// has a `Dump` the window can render; one that produces something else has a
/// bug the author would otherwise find out about from a user.
/// What the helper checks allow, from the work item: ready in 10 s, a hook
/// answered in 5 s, a cancel honoured in 3 s.
const READY_TIMEOUT: Duration = Duration::from_secs(10);
const HOOK_TIMEOUT: Duration = Duration::from_secs(5);
const CANCEL_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a server may take to start and have an opinion about a file
/// that is fine.
const SERVER_TIMEOUT: Duration = Duration::from_secs(30);

const DUMP_COLUMNS: [&str; 2] = ["name", "n"];
const DUMP_ROWS: [(&str, i64); 2] = [("ada", 1), ("grace", 2)];

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
        let ran = runtime.block_on(run_to_end(spec, plugin.output.clone(), RUN_TIMEOUT));
        let (outcome, detail) = run_outcome(&ran);
        run.add(&id, outcome, detail, started);
    }

    // 5 and 6. What the plugin provides in `check/`: a known table, and a
    // file that is meant to fail.
    let sample = |name: &str| work_sample(&plugin, name);
    // The default kind: what a new playground is, and so what a `check/`
    // sample is written against.
    let default_kind = plugin.default_kind().id.clone();
    let base = made
        .iter()
        .find(|(kind_id, _)| kind_id == &default_kind)
        .or_else(|| made.first())
        .and_then(|(kind_id, playground)| {
            plugin
                .kinds
                .iter()
                .find(|k| &k.id == kind_id)
                .map(|kind| (kind.clone(), playground.clone()))
        });

    if run.wanted("dump") {
        let started = Instant::now();
        let (outcome, detail) = match (&base, sample("dump")) {
            (Some((kind, playground)), Some(source)) => {
                let ctx = RunContext {
                    resolver: &resolver,
                    scratch_root: &work.scratch,
                    toolchain_program: &program,
                    toolchain_version: &version,
                };
                dump_check(&plugin, kind, playground, &source, &ctx, &runtime)
            }
            (_, None) => (
                Outcome::Skip,
                format!(
                    "no check/dump.{} — add one that dumps the table in PLUGINS.md §8.1",
                    plugin.extension
                ),
            ),
            (None, _) => (Outcome::Skip, "nothing was scaffolded".to_owned()),
        };
        run.add("dump", outcome, detail, started);
    }

    if run.wanted("errors") {
        let started = Instant::now();
        let (outcome, detail) = match (&base, sample("error"), &plugin.output) {
            (_, _, None) => (
                Outcome::Skip,
                "the plugin has no [output].error_regex".to_owned(),
            ),
            (Some((kind, playground)), Some(source), Some(_)) => {
                let ctx = RunContext {
                    resolver: &resolver,
                    scratch_root: &work.scratch,
                    toolchain_program: &program,
                    toolchain_version: &version,
                };
                error_check(&plugin, kind, playground, &source, &ctx, &runtime)
            }
            (_, None, _) => (
                Outcome::Skip,
                format!(
                    "no check/error.{} — add one with a mistake in it",
                    plugin.extension
                ),
            ),
            (None, _, _) => (Outcome::Skip, "nothing was scaffolded".to_owned()),
        };
        run.add("errors", outcome, detail, started);
    }

    // 7. The helper: the protocol, the hooks, and letting go when told.
    if plugin.helper.is_some() && run.wanted("helper") {
        helper_checks(
            &mut run,
            &plugin,
            base.as_ref(),
            &work,
            &resolver,
            &program,
            &version,
            &runtime,
        );
    }

    // 8. The language server starts and has nothing to complain about in a
    // file that is fine.
    if plugin.lsp.is_some() && run.wanted("server") {
        let started = Instant::now();
        let (outcome, detail) = match &base {
            Some((kind, playground)) => server_check(
                &plugin, kind, playground, &work, &resolver, &program, &version, &runtime,
            ),
            None => (Outcome::Skip, "nothing was scaffolded".to_owned()),
        };
        run.add("server", outcome, detail, started);
    }

    finish(run, options)
}

/// Check 8: `[lsp]` starts, initialises, and says the dump sample is fine.
///
/// A server that reports errors in a file the plugin itself provides as a good
/// one is a server that will underline everything a user writes.
#[allow(clippy::too_many_arguments)]
fn server_check(
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    work: &Workspace,
    resolver: &Resolver,
    program: &str,
    version: &str,
    runtime: &tokio::runtime::Runtime,
) -> (Outcome, String) {
    let Some(source) = work_sample(plugin, "dump") else {
        return (
            Outcome::Skip,
            format!("no check/dump.{} to open", plugin.extension),
        );
    };
    if let Err(error) = write_sample(kind, playground, &source) {
        return (Outcome::Fail, error);
    }

    // Not installed is a skip, exactly as it is for the toolchain: this
    // machine lacking rust-analyzer says nothing about the Rust plugin.
    //
    // Only `PATH` and the settings overrides are searched, not the folder the
    // window acquires tools into (04-08a): the checker has no app data
    // directory of its own. A server installed through the Languages button
    // and nowhere else therefore skips here. `tinkerpad tools` (05-01c) brings
    // that directory to the command line, and this can use it then.
    if let Some(lsp) = &plugin.lsp
        && let Err(reason) = resolver.resolve(plugin.id.as_str(), &lsp.command.command)
    {
        return (
            Outcome::Skip,
            match &plugin.install_hint {
                Some(hint) => format!("{reason} — {}", one_line(hint)),
                None => reason,
            },
        );
    }
    // The same goes for what the server needs in its workspace, such as the
    // TypeScript that typescript-language-server analyses with: the window
    // offers to install it, but this workspace is a fresh temporary one.
    if let Some(needs) = plugin.lsp.as_ref().and_then(|l| l.workspace_needs.as_ref())
        && let Ok(Some(root)) =
            crate::lsp::root_for(plugin, Some(playground), &work.library, &work.scratch)
        && !root.join(needs).exists()
    {
        return (
            Outcome::Skip,
            format!(
                "the server needs `{}` in its workspace, which a fresh check doesn't have",
                needs.display()
            ),
        );
    }

    let _guard = runtime.enter();
    let config = match crate::lsp::config_for_check(
        plugin,
        playground,
        resolver,
        &work.library,
        &work.scratch,
        program,
        version,
    ) {
        Ok(config) => config,
        Err(error) => return (Outcome::Fail, one_line(&error.message)),
    };

    let pool = tinkerpad_lsp::LspPool::new(None);
    let uri = tinkerpad_lsp::path_to_uri(&playground.main_file);
    let text = std::fs::read_to_string(&playground.main_file).unwrap_or_default();

    let result = runtime.block_on(async {
        let server = pool.server_for(&config);
        // Everything the server says that isn't a reply, from before the
        // document is opened: diagnostics for it must not be missed.
        let mut said = server.subscribe();
        let attached = tokio::time::timeout(SERVER_TIMEOUT, pool.attach(&config, &uri, &text))
            .await
            .map_err(|_| {
                format!(
                    "no `initialize` reply within {} s",
                    SERVER_TIMEOUT.as_secs()
                )
            })?
            // "Not running" alone is nothing an author can act on: why it
            // stopped, or the last thing it said, is.
            .map_err(|e| match (server.state(), server.log().last()) {
                (
                    tinkerpad_core::ServerState::Stopped {
                        reason: Some(why), ..
                    },
                    _,
                ) => {
                    format!("{e}: {}", clip(&why, 200))
                }
                (_, Some(last)) => format!("{e}; it last said: {}", clip(last, 200)),
                _ => e.to_string(),
            })?;

        // Wait for this file's diagnostics. Silence is a pass: plenty of
        // servers say nothing about a file with nothing wrong.
        let errors = tokio::time::timeout(SERVER_TIMEOUT, async {
            while let Ok(message) = said.recv().await {
                let value = message.into_value();
                if value.get("method").and_then(|m| m.as_str())
                    != Some("textDocument/publishDiagnostics")
                {
                    continue;
                }
                let params = value.get("params").cloned().unwrap_or_default();
                if params.get("uri").and_then(|u| u.as_str()) != Some(uri.as_str()) {
                    continue;
                }
                let found: Vec<String> = params
                    .get("diagnostics")
                    .and_then(|d| d.as_array())
                    .map(|list| {
                        list.iter()
                            .filter(|d| {
                                // 1 is Error; anything softer is the server's
                                // opinion, not a problem with the sample.
                                d.get("severity").and_then(|s| s.as_u64()).unwrap_or(1) == 1
                            })
                            .map(|d| {
                                d.get("message")
                                    .and_then(|m| m.as_str())
                                    .unwrap_or("")
                                    .to_owned()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                return found;
            }
            Vec::new()
        })
        .await
        .unwrap_or_default();
        drop(attached);
        Ok::<_, String>(errors)
    });
    runtime.block_on(pool.shutdown());

    match result {
        Ok(errors) if errors.is_empty() => (
            Outcome::Pass,
            "started, and found nothing wrong with check/dump".to_owned(),
        ),
        Ok(errors) => (
            Outcome::Fail,
            format!(
                "{} error{} in check/dump: {}",
                errors.len(),
                plural(errors.len()),
                clip(&errors.join("; "), 160)
            ),
        ),
        Err(error) => (Outcome::Fail, one_line(&error)),
    }
}

/// Check 7: the helper answers the protocol it says it speaks.
///
/// Reported as several lines rather than one, because "the helper is broken"
/// is not something an author can act on and "cancel took 11 s" is.
#[allow(clippy::too_many_arguments)]
fn helper_checks(
    run: &mut Run<'_>,
    plugin: &Plugin,
    base: Option<&(tinkerpad_plugin::Kind, tinkerpad_core::Playground)>,
    work: &Workspace,
    resolver: &Resolver,
    program: &str,
    version: &str,
    runtime: &tokio::runtime::Runtime,
) {
    use tinkerpad_plugin::helper::HookRequest;

    // The pool spawns supervision tasks when a helper is asked for, so the
    // runtime has to be the current one even outside `block_on`.
    let _guard = runtime.enter();

    let started = Instant::now();
    let toolchain = crate::helpers::Toolchain {
        program: program.to_owned(),
        version: version.to_owned(),
    };
    let config = match crate::helpers::config(
        plugin,
        resolver,
        &toolchain,
        &work.scratch,
        &work.root,
        &crate::service_settings::ServiceSettings::default(),
    ) {
        Ok(config) => config,
        Err(error) => {
            run.add(
                "helper:start",
                Outcome::Fail,
                one_line(&error.message),
                started,
            );
            return;
        }
    };

    let pool = tinkerpad_runner::HelperPool::new(None);
    // `get` covers the build (if the plugin declares one), the handshake, and
    // `ready` — everything between asking for a helper and having one.
    let client = match runtime.block_on(tokio::time::timeout(
        READY_TIMEOUT + BUILD_GRACE,
        pool.get(&config),
    )) {
        Ok(Ok(client)) => client,
        Ok(Err(error)) => {
            run.add(
                "helper:start",
                Outcome::Fail,
                one_line(&error.to_string()),
                started,
            );
            return;
        }
        Err(_) => {
            run.add(
                "helper:start",
                Outcome::Fail,
                format!(
                    "no `ready` within {} s",
                    (READY_TIMEOUT + BUILD_GRACE).as_secs()
                ),
                started,
            );
            return;
        }
    };
    run.add(
        "helper:start",
        Outcome::Pass,
        format!("built and ready in {} ms", started.elapsed().as_millis()),
        started,
    );
    drop(client);

    // A run through the helper, which is the whole point of having one — for
    // the plugins that use it for runs. A helper declared only for hooks
    // (Go's) says so by not listing `run`.
    let runs_things = plugin
        .helper
        .as_ref()
        .is_some_and(|helper| helper.hooks.iter().any(|hook| hook == "run"));
    if let (true, Some((kind, playground))) = (runs_things, base) {
        let started = Instant::now();
        let source = work_sample(plugin, "dump").unwrap_or_default();
        let (outcome, detail) = if source.is_empty() {
            (
                Outcome::Skip,
                format!("no check/dump.{} to run through it", plugin.extension),
            )
        } else {
            helper_run_check(&pool, &config, plugin, kind, playground, &source, runtime)
        };
        run.add("helper:run", outcome, detail, started);

        // Cancel, on something that would otherwise never stop.
        let started = Instant::now();
        let (outcome, detail) = match work_sample(plugin, "hang") {
            Some(source) => {
                helper_cancel_check(&pool, &config, plugin, kind, playground, &source, runtime)
            }
            None => (
                Outcome::Skip,
                format!("no check/hang.{} to cancel", plugin.extension),
            ),
        };
        run.add("helper:cancel", outcome, detail, started);
    }

    // Every hook the plugin says it answers, answered.
    if let Some((_, playground)) = base {
        for hook in plugin
            .helper
            .as_ref()
            .map(|h| h.hooks.clone())
            .unwrap_or_default()
        {
            let request = match hook.as_str() {
                "playground_created" => HookRequest::PlaygroundCreated {
                    playground: playground.clone(),
                },
                "playground_opened" => HookRequest::PlaygroundOpened {
                    playground: playground.clone(),
                    open: vec![playground.clone()],
                },
                "playground_closed" => HookRequest::PlaygroundClosed {
                    playground: playground.clone(),
                    open: Vec::new(),
                },
                // `run` and `stdin` are not request/result hooks; they are
                // covered by helper:run above.
                _ => continue,
            };
            let started = Instant::now();
            let id = format!("helper:{hook}");
            if !run.wanted(&id) {
                continue;
            }
            let answered = runtime.block_on(async {
                let client = pool.get(&config).await.map_err(|e| e.to_string())?;
                tokio::time::timeout(HOOK_TIMEOUT, client.hook(request))
                    .await
                    .map_err(|_| format!("no answer within {} s", HOOK_TIMEOUT.as_secs()))?
                    .map_err(|e| e.to_string())
            });
            let (outcome, detail) = match answered {
                Ok(result) => (
                    Outcome::Pass,
                    format!(
                        "answered in {} ms, {} file{}",
                        started.elapsed().as_millis(),
                        result.files.len(),
                        plural(result.files.len())
                    ),
                ),
                Err(error) => (Outcome::Fail, one_line(&error)),
            };
            run.add(&id, outcome, detail, started);
        }
    }

    // And it goes away when asked.
    let started = Instant::now();
    let stopped = runtime.block_on(tokio::time::timeout(READY_TIMEOUT, pool.shutdown()));
    let (outcome, detail) = match stopped {
        Ok(()) => (
            Outcome::Pass,
            format!("stopped in {} ms", started.elapsed().as_millis()),
        ),
        Err(_) => (
            Outcome::Fail,
            format!("still running {} s after shutdown", READY_TIMEOUT.as_secs()),
        ),
    };
    run.add("helper:shutdown", outcome, detail, started);
}

/// A build can be slow the first time — a C# helper is a compile — so the
/// handshake's own ten seconds get company rather than being spent waiting.
const BUILD_GRACE: Duration = Duration::from_secs(180);

/// Run the dump sample through the helper rather than a process of its own,
/// and ask the same question of the answer.
#[allow(clippy::too_many_arguments)]
fn helper_run_check(
    pool: &tinkerpad_runner::HelperPool,
    config: &tinkerpad_runner::HelperConfig,
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    source: &str,
    runtime: &tokio::runtime::Runtime,
) -> (Outcome, String) {
    if let Err(error) = write_sample(kind, playground, source) {
        return (Outcome::Fail, error);
    }
    let request = tinkerpad_plugin::helper::RunRequest {
        main: playground.main_file.clone(),
        dir: playground.dir.clone(),
        kind: playground.kind.clone(),
    };
    let ran = runtime.block_on(async {
        let handle = tinkerpad_runner::helper_run::start(pool, config, request, |_| {})
            .await
            .map_err(|e| e.to_string())?;
        Ok::<Ran, String>(collect(handle, plugin.output.clone(), RUN_TIMEOUT).await)
    });
    let ran = match ran {
        Ok(ran) => ran,
        Err(error) => return (Outcome::Fail, one_line(&error)),
    };
    if let Some(failed) = &ran.failed {
        return (Outcome::Fail, failed.clone());
    }
    let tables = ran
        .dumps
        .iter()
        .filter(|v| matches!(v.node, tinkerpad_core::RichNode::Table { .. }))
        .count();
    if tables == 0 {
        return (
            Outcome::Fail,
            format!(
                "the helper ran it but no table arrived{}",
                tail(&ran.stderr)
            ),
        );
    }
    (
        Outcome::Pass,
        format!("ran warm in {} ms, the table arrived", ran.ms),
    )
}

/// Cancel a run that would otherwise never end, and time how long it takes.
#[allow(clippy::too_many_arguments)]
fn helper_cancel_check(
    pool: &tinkerpad_runner::HelperPool,
    config: &tinkerpad_runner::HelperConfig,
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    source: &str,
    runtime: &tokio::runtime::Runtime,
) -> (Outcome, String) {
    if let Err(error) = write_sample(kind, playground, source) {
        return (Outcome::Fail, error);
    }
    let request = tinkerpad_plugin::helper::RunRequest {
        main: playground.main_file.clone(),
        dir: playground.dir.clone(),
        kind: playground.kind.clone(),
    };
    let result = runtime.block_on(async {
        let mut handle = tinkerpad_runner::helper_run::start(pool, config, request, |_| {})
            .await
            .map_err(|e| e.to_string())?;
        // Give it a moment to be properly going, so the cancel is a cancel
        // rather than a race with the start.
        tokio::time::sleep(Duration::from_millis(400)).await;
        let asked = Instant::now();
        handle.cancel();
        let ended = tokio::time::timeout(CANCEL_TIMEOUT, async {
            while let Some(event) = handle.recv().await {
                if matches!(
                    event,
                    tinkerpad_runner::RunEvent::Cancelled { .. }
                        | tinkerpad_runner::RunEvent::Exited { .. }
                ) {
                    return;
                }
            }
        })
        .await;
        Ok::<_, String>((ended.is_ok(), asked.elapsed()))
    });
    match result {
        Ok((true, took)) => (
            Outcome::Pass,
            format!("stopped {} ms after cancel", took.as_millis()),
        ),
        Ok((false, _)) => (
            Outcome::Fail,
            format!("still running {} s after cancel", CANCEL_TIMEOUT.as_secs()),
        ),
        Err(error) => {
            let _ = plugin;
            (Outcome::Fail, one_line(&error))
        }
    }
}

/// Write a `check/` sample into a playground, between its kind's locked lines.
fn write_sample(
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    source: &str,
) -> Result<(), String> {
    let mut text = String::new();
    if let Some(prelude) = kind.prelude.as_ref().filter(|p| !p.is_empty()) {
        text.push_str(prelude);
    }
    text.push_str(source);
    if let Some(postlude) = kind.postlude.as_ref().filter(|p| !p.is_empty()) {
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(postlude);
    }
    std::fs::write(&playground.main_file, &text).map_err(|e| e.to_string())
}

/// A file from the plugin's `check/` folder, if it has one.
fn work_sample(plugin: &Plugin, name: &str) -> Option<String> {
    let path = plugin
        .dir
        .join("check")
        .join(format!("{name}.{}", plugin.extension));
    std::fs::read_to_string(path).ok()
}

/// Run a playground with `source` as its main file, and give back what it
/// produced. The playground is one of the scaffolded ones, borrowed for the
/// purpose: its kind's locked lines and scaffold files are exactly what a real
/// one has, which is the point.
fn run_source(
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    source: &str,
    ctx: &RunContext<'_>,
    runtime: &tokio::runtime::Runtime,
) -> Result<Ran, String> {
    // A `check/` file is the part a person would type, so it goes where a
    // person types: between the kind's locked lines. PowerShell's `Dump` is
    // defined in its prelude, and a sample written over the top of that would
    // be testing a language feature that isn't there.
    write_sample(kind, playground, source)?;
    let spec = run_spec(plugin, playground, ctx).map_err(|e| one_line(&e.to_string()))?;
    Ok(runtime.block_on(run_to_end(spec, plugin.output.clone(), RUN_TIMEOUT)))
}

/// Check 5: the plugin's `Dump` produces the table it is supposed to.
fn dump_check(
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    source: &str,
    ctx: &RunContext<'_>,
    runtime: &tokio::runtime::Runtime,
) -> (Outcome, String) {
    let ran = match run_source(plugin, kind, playground, source, ctx, runtime) {
        Ok(ran) => ran,
        Err(error) => return (Outcome::Fail, error),
    };
    if let Some(failed) = &ran.failed {
        return (Outcome::Fail, failed.clone());
    }
    if ran.timed_out {
        return (Outcome::Fail, "check/dump never finished".to_owned());
    }
    if ran.code != Some(0) {
        return (
            Outcome::Fail,
            format!("check/dump exited {:?}{}", ran.code, tail(&ran.stderr)),
        );
    }
    if let Some(error) = ran.malformed.first() {
        // A payload that arrived and didn't parse is worse than none: the
        // window would show nothing and say nothing.
        return (
            Outcome::Fail,
            format!("a dump payload didn't parse: {}", one_line(error)),
        );
    }
    let Some(table) = ran.dumps.iter().find_map(|value| match &value.node {
        tinkerpad_core::RichNode::Table { columns, rows, .. } => Some((columns, rows)),
        _ => None,
    }) else {
        return (
            Outcome::Fail,
            format!(
                "no table arrived ({} other dump node{} did)",
                ran.dumps.len(),
                plural(ran.dumps.len())
            ),
        );
    };
    let (columns, rows) = table;

    // Compared without case, so C#, Go and PowerShell can name their fields
    // the way those languages name fields.
    let names: Vec<String> = columns.iter().map(|c| c.name.to_lowercase()).collect();
    if names != DUMP_COLUMNS {
        return (
            Outcome::Fail,
            format!("columns are {names:?}, expected {DUMP_COLUMNS:?}"),
        );
    }
    if rows.len() != DUMP_ROWS.len() {
        return (
            Outcome::Fail,
            format!("{} rows, expected {}", rows.len(), DUMP_ROWS.len()),
        );
    }
    for (row, (name, n)) in rows.iter().zip(DUMP_ROWS) {
        let got_name = row.first().and_then(cell_text).map(|t| t.to_lowercase());
        let got_n = row.get(1).and_then(cell_number);
        if got_name.as_deref() != Some(name) || got_n != Some(n) {
            return (
                Outcome::Fail,
                format!("row is {got_name:?}/{got_n:?}, expected {name:?}/{n}"),
            );
        }
    }
    (
        Outcome::Pass,
        format!(
            "a table of {} rows, columns {}",
            rows.len(),
            DUMP_COLUMNS.join(", ")
        ),
    )
}

/// Check 6: `[output].error_regex` finds a position in a real failure.
fn error_check(
    plugin: &Plugin,
    kind: &tinkerpad_plugin::Kind,
    playground: &tinkerpad_core::Playground,
    source: &str,
    ctx: &RunContext<'_>,
    runtime: &tokio::runtime::Runtime,
) -> (Outcome, String) {
    let ran = match run_source(plugin, kind, playground, source, ctx, runtime) {
        Ok(ran) => ran,
        Err(error) => return (Outcome::Fail, error),
    };
    if ran.timed_out {
        return (Outcome::Fail, "check/error never finished".to_owned());
    }
    // It is supposed to fail. One that succeeds isn't testing the pattern.
    if ran.code == Some(0) && ran.failed.is_none() {
        return (
            Outcome::Fail,
            "check/error ran cleanly; it should contain a mistake".to_owned(),
        );
    }
    match ran.annotations.first() {
        Some(found) => (
            Outcome::Pass,
            format!(
                "line {}{}: {}",
                found.line,
                found.col.map(|c| format!(":{c}")).unwrap_or_default(),
                clip(&found.message, 80)
            ),
        ),
        None => (
            Outcome::Fail,
            format!(
                "[output].error_regex matched nothing in the failure{}",
                tail(&ran.stderr)
            ),
        ),
    }
}

/// A table cell as text, whether the plugin sent a bare string or a rich value.
fn cell_text(cell: &serde_json::Value) -> Option<String> {
    match cell {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(map) => map.get("value").and_then(cell_text),
        other => Some(other.to_string()),
    }
}

fn cell_number(cell: &serde_json::Value) -> Option<i64> {
    match cell {
        serde_json::Value::Number(n) => n.as_i64(),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        serde_json::Value::Object(map) => map.get("value").and_then(cell_number),
        _ => None,
    }
}

/// What one run produced: enough for every check that watches a run, parsed
/// the way the window parses it.
#[derive(Debug, Default)]
struct Ran {
    /// `None` if it never exited (timed out) or was stopped by a signal.
    code: Option<i32>,
    timed_out: bool,
    ms: u128,
    /// Plain stdout, with the dump payloads taken out of it.
    text_bytes: usize,
    stderr: Vec<u8>,
    /// Dump values, through the parser the app uses (01-07).
    dumps: Vec<tinkerpad_core::RichValue>,
    /// Dump payloads that arrived but didn't parse: a plugin bug worth saying.
    malformed: Vec<String>,
    /// Positions found by `[output].error_regex` (01-13).
    annotations: Vec<tinkerpad_core::Annotation>,
    /// Why it couldn't start at all.
    failed: Option<String>,
}

/// Run one playground to the end, decoding its output the way the window does.
///
/// `OutputDecoder` is the app's own: the same dump parser, the same annotator,
/// the same line boundaries. A checker that decoded output its own way would
/// report positions the window never finds, which is the one thing a
/// conformance tool must not do.
async fn run_to_end(
    spec: tinkerpad_runner::RunSpec,
    output: Option<tinkerpad_plugin::OutputSpec>,
    timeout: Duration,
) -> Ran {
    let handle = match tinkerpad_runner::run(spec) {
        Ok(handle) => handle,
        Err(error) => {
            return Ran {
                failed: Some(one_line(&error.to_string())),
                ..Ran::default()
            };
        }
    };
    collect(handle, output, timeout).await
}

/// Drain a run that has already started, decoding as it goes.
async fn collect(
    mut handle: tinkerpad_runner::RunHandle,
    output: Option<tinkerpad_plugin::OutputSpec>,
    timeout: Duration,
) -> Ran {
    use tinkerpad_core::RunEvent as Decoded;

    let mut ran = Ran::default();
    let canceller = handle.canceller();
    let mut decoder = tinkerpad_runner::OutputDecoder::with_output(output);
    let mut decoded = Vec::new();

    let finished = tokio::time::timeout(timeout, async {
        while let Some(event) = handle.recv().await {
            match event {
                tinkerpad_runner::RunEvent::Stdout(bytes) => decoder.stdout(&bytes, &mut decoded),
                tinkerpad_runner::RunEvent::Stderr(bytes) => {
                    ran.stderr.extend_from_slice(&bytes);
                    decoder.stderr(&bytes, &mut decoded);
                }
                tinkerpad_runner::RunEvent::Failed { message, .. } => {
                    ran.failed = Some(one_line(&message));
                }
                tinkerpad_runner::RunEvent::Exited { code, duration } => {
                    return Some((code, duration));
                }
                tinkerpad_runner::RunEvent::Cancelled { duration } => {
                    return Some((None, duration));
                }
                tinkerpad_runner::RunEvent::Started { .. } => {}
            }
        }
        None
    })
    .await;
    decoder.finish(&mut decoded);

    for event in decoded {
        match event {
            Decoded::Stdout { text } => ran.text_bytes += text.len(),
            Decoded::Rich { value } => ran.dumps.push(value),
            Decoded::Warning { message } => ran.malformed.push(message),
            Decoded::Annotation(annotation) => ran.annotations.push(annotation),
            _ => {}
        }
    }

    match finished {
        Err(_) => {
            // A run with no end is a failed check, not a hung checker.
            canceller.cancel();
            ran.timed_out = true;
        }
        Ok(Some((code, duration))) => {
            ran.code = code;
            ran.ms = duration.as_millis();
        }
        Ok(None) => {}
    }
    ran
}

/// How a kind's run went, as check 4 reports it.
fn run_outcome(ran: &Ran) -> (Outcome, String) {
    if let Some(failed) = &ran.failed {
        return (Outcome::Fail, failed.clone());
    }
    if ran.timed_out {
        return (
            Outcome::Fail,
            format!("still running after {} s", RUN_TIMEOUT.as_secs()),
        );
    }
    let ms = ran.ms;
    match ran.code {
        Some(0) if ran.text_bytes > 0 || !ran.dumps.is_empty() => (
            Outcome::Pass,
            format!("exit 0 in {ms} ms, {} bytes of stdout", ran.text_bytes),
        ),
        Some(0) => (
            Outcome::Fail,
            format!(
                "exit 0 in {ms} ms but printed nothing; a kind's template should show something"
            ),
        ),
        Some(code) => (
            Outcome::Fail,
            format!("exit {code} in {ms} ms{}", tail(&ran.stderr)),
        ),
        None => (Outcome::Fail, "the run stopped without exiting".to_owned()),
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
