//! `tinkerpad tools list` and `tinkerpad tools install` (work item 05-01c).
//!
//! What every language needs, and the ability to get it, without opening a
//! window. A CI runner, a new machine or a Dockerfile wants exactly what the
//! Languages button offers, and has no display to offer it on.
//!
//! The work is [`crate::tools`]'s, unchanged: the same status, the same
//! offers, the same acquisition the window performs. What is here is the two
//! things a terminal needs that a window doesn't — somewhere to find the app's
//! folders without Tauri's path resolver, and somewhere to put the progress
//! that normally arrives as events.

use std::path::PathBuf;
use std::sync::Arc;

use tinkerpad_core::{CoreEvent, Settings, ToolchainState};
use tinkerpad_plugin::{PROBE_TIMEOUT, Plugin, Resolver, ToolchainOverrides, env::discover_path};

use crate::events::EventBus;
use crate::tools::{self, Env, Which};

/// The bundle identifier, which names the app's data folder.
///
/// It has to agree with `tauri.conf.json`; a test below reads that file and
/// says so if it ever stops agreeing. Without this the CLI would look in a
/// different folder from the window and report tools it cannot see.
const IDENTIFIER: &str = "io.github.brandongullion.tinkerpad";

/// `<app-data>`: the same folder Tauri's `app_data_dir()` resolves to.
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join(IDENTIFIER))
}

/// Where playgrounds live, by the same rules the window uses
/// (`crate::library::root`), minus the parts that need an app handle.
fn library_root(settings: &Settings) -> PathBuf {
    if let Some(dir) = std::env::var_os(crate::library::LIBRARY_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    if let Some(root) = settings
        .library
        .root
        .as_ref()
        .filter(|r| !r.as_os_str().is_empty())
    {
        return root.clone();
    }
    dirs::home_dir()
        .map(|home| home.join("TinkerPad"))
        .unwrap_or_default()
}

/// The bundled plugins folder, for a build with no resource resolver to ask.
///
/// Beside the executable in a packaged build, inside the repository in a
/// development one — the two places `plugins::bundled_plugins_dir` looks,
/// reached without an app handle.
fn bundled_plugins() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        for candidate in [
            dir.join("plugins"),
            // macOS: the binary is in `Contents/MacOS`, resources next door.
            dir.join("../Resources/plugins"),
        ] {
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../plugins");
    repo.is_dir().then(|| repo.canonicalize().unwrap_or(repo))
}

fn user_plugins(data: &std::path::Path) -> PathBuf {
    std::env::var_os(crate::plugins::USER_PLUGINS_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| data.join("plugins"))
}

/// Everything the two subcommands need, worked out once.
///
/// Public so a test can build one over a fake plugin and a temporary folder:
/// `open` reads the machine's own settings, plugins and `PATH`, which is right
/// for the command line and useless for a test.
pub struct Context {
    plugins: Vec<Arc<Plugin>>,
    env: Env,
    states: Vec<(String, ToolchainState)>,
    runtime: tokio::runtime::Runtime,
}

impl Context {
    /// A context over a given plugin set and environment, for tests.
    pub fn over(
        plugins: Vec<Arc<Plugin>>,
        env: Env,
        states: Vec<(String, ToolchainState)>,
    ) -> Result<Context, String> {
        Ok(Context {
            plugins,
            env,
            states,
            runtime: tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?,
        })
    }

    fn open(out: &mut dyn FnMut(&str)) -> Result<Context, String> {
        let Some(data) = data_dir() else {
            return Err("could not work out where TinkerPad keeps its data".to_owned());
        };
        let settings = crate::settings::read(&crate::settings::path(&data))
            .map_err(|problem| problem.message.clone())
            .unwrap_or_default();

        let registry =
            tinkerpad_plugin::PluginRegistry::load(bundled_plugins(), user_plugins(&data));
        for failure in registry.failures() {
            out(&format!(
                "warning: {} could not be loaded and was skipped",
                failure.dir.display()
            ));
        }
        let plugins: Vec<Arc<Plugin>> = registry
            .all()
            .iter()
            .map(|entry| entry.plugin.clone())
            .collect();
        if plugins.is_empty() {
            return Err("no language plugins were found".to_owned());
        }

        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;

        // The same resolver the window builds: settings overrides, then
        // whatever TinkerPad has acquired, then the discovered PATH (04-08a).
        let path = runtime.block_on(discover_path());
        let resolver = Arc::new(
            Resolver::new(path, ToolchainOverrides::of(&settings))
                .with_acquired(tools::search_paths(&data)),
        );

        let scratch_root = data.join("scratch");
        let states = runtime.block_on(async {
            let mut found = Vec::new();
            for plugin in &plugins {
                let scratch = scratch_root.join(plugin.id.as_str());
                let state =
                    tinkerpad_plugin::probe(plugin, &resolver, &scratch, PROBE_TIMEOUT).await;
                found.push((plugin.id.to_string(), state));
            }
            found
        });

        let env = Env {
            data_dir: data,
            events: Arc::new(EventBus::new()),
            resolver,
            mirror: settings
                .tools
                .mirror
                .clone()
                .filter(|mirror| !mirror.is_empty()),
            offers: settings.tools.offers(),
            library_root: library_root(&settings),
            scratch_root,
        };
        Ok(Context {
            plugins,
            env,
            states,
            runtime,
        })
    }

    fn state_of(&self, id: &str) -> Option<ToolchainState> {
        self.states
            .iter()
            .find(|(plugin, _)| plugin == id)
            .map(|(_, state)| state.clone())
    }

    fn needs(&self) -> Vec<tools::LanguageNeeds> {
        self.plugins
            .iter()
            .map(|plugin| tools::LanguageNeeds {
                language: plugin.id.clone(),
                name: plugin.name.clone(),
                needs: tools::needs(&self.env, plugin, self.state_of(plugin.id.as_str())),
            })
            .collect()
    }
}

/// `tinkerpad tools list [--json]`.
pub fn list(json: bool, out: &mut dyn FnMut(&str)) -> i32 {
    let context = match Context::open(out) {
        Ok(context) => context,
        Err(error) => {
            out(&error);
            return 2;
        }
    };
    list_in(&context, json, out)
}

/// The same, over a context already built.
pub fn list_in(context: &Context, json: bool, out: &mut dyn FnMut(&str)) -> i32 {
    let needs = context.needs();

    if json {
        match serde_json::to_string_pretty(&needs) {
            Ok(text) => out(&text),
            Err(error) => {
                out(&format!("{{\"error\":\"{error}\"}}"));
                return 2;
            }
        }
        return 0;
    }

    let mut missing = 0;
    for entry in &needs {
        out(&entry.name);
        for need in &entry.needs {
            if !need.present {
                missing += 1;
            }
            let state = if need.present { "ready" } else { "missing" };
            // What to do about it, in the same column as where it was found:
            // either this can install it, or here is why it can't.
            let where_ = if need.present {
                need.found_at.clone().unwrap_or_default()
            } else {
                match &need.offer {
                    Some(offer) => match &offer.blocked {
                        None => "can install".to_owned(),
                        Some(why) => why.clone(),
                    },
                    None => need
                        .hint
                        .clone()
                        .unwrap_or_else(|| "install it by hand".to_owned()),
                }
            };
            let version = need
                .version
                .as_ref()
                .map(|v| format!(" {v}"))
                .unwrap_or_default();
            out(format!(
                "  {:<16} {:<8} {}{}  {where_}",
                need.label, state, need.what, version
            )
            .trim_end());
        }
    }
    out(&match missing {
        0 => "Every language has what it needs.".to_owned(),
        1 => {
            "1 thing missing. `tinkerpad tools install <language>` can get what it can.".to_owned()
        }
        n => {
            format!("{n} things missing. `tinkerpad tools install <language>` can get what it can.")
        }
    });
    0
}

/// `tinkerpad tools install <language> [--which …] [--yes]`.
pub fn install(language: &str, which: Option<Which>, yes: bool, out: &mut dyn FnMut(&str)) -> i32 {
    let context = match Context::open(out) {
        Ok(context) => context,
        Err(error) => {
            out(&error);
            return 2;
        }
    };
    install_in(&context, language, which, yes, out)
}

/// The same, over a context already built.
pub fn install_in(
    context: &Context,
    language: &str,
    which: Option<Which>,
    yes: bool,
    out: &mut dyn FnMut(&str),
) -> i32 {
    let Some(plugin) = context
        .plugins
        .iter()
        .find(|plugin| plugin.id.as_str() == language)
        .cloned()
    else {
        out(&format!(
            "No plugin for `{language}`. `tinkerpad tools list` says what there is."
        ));
        return 2;
    };

    let needs = tools::needs(&context.env, &plugin, context.state_of(plugin.id.as_str()));
    // Everything missing that can be installed, or just the one asked for.
    let wanted: Vec<&tools::Need> = needs
        .iter()
        .filter(|need| which.is_none_or(|only| only == need.which))
        .filter(|need| !need.present)
        .collect();

    if wanted.is_empty() {
        out(&format!("{} has everything it needs.", plugin.name));
        return 0;
    }
    let blocked: Vec<&tools::Need> = wanted
        .iter()
        .copied()
        .filter(|need| need.offer.as_ref().is_none_or(|o| o.blocked.is_some()))
        .collect();
    let doable: Vec<Which> = wanted
        .iter()
        .filter(|need| need.offer.as_ref().is_some_and(|o| o.blocked.is_none()))
        .map(|need| need.which)
        .collect();

    for need in &blocked {
        let why = need
            .offer
            .as_ref()
            .and_then(|offer| offer.blocked.clone())
            .unwrap_or_else(|| format!("{} doesn't say how to install this", plugin.name));
        out(&format!("{}: {why}", need.label));
        if let Some(hint) = &need.hint {
            out(&format!("  {hint}"));
        }
    }
    if doable.is_empty() {
        return 1;
    }

    // What is about to happen, before it happens — the same thing the window
    // shows, in the place a terminal shows things.
    for need in wanted
        .iter()
        .filter(|need| need.offer.as_ref().is_some_and(|o| o.blocked.is_none()))
    {
        let offer = need.offer.as_ref().expect("checked just above");
        out(&format!("{}: {}", need.label, offer.label));
        if let Some(url) = &offer.url {
            out(&format!("  {url}"));
        }
        if let Some(sha) = &offer.sha256 {
            out(&format!("  sha256 {sha}"));
        }
    }
    if !yes && !confirm(out) {
        out("Nothing was installed.");
        return 1;
    }

    // Progress reaches a window as events; here it is lines on stdout.
    let mut failed = false;
    for which in doable {
        let mut listening = context.env.events.subscribe();
        let acquired = context.runtime.block_on(async {
            let work = tools::acquire(&context.env, &plugin, which);
            tokio::pin!(work);
            loop {
                tokio::select! {
                    event = listening.recv() => {
                        if let Ok(envelope) = event
                            && let CoreEvent::Tool { line, .. } = envelope.event
                        {
                            out(&format!("  {line}"));
                        }
                    }
                    result = &mut work => return result,
                }
            }
        });
        match acquired {
            Ok(Some(path)) => out(&format!("Installed at {}", path.display())),
            Ok(None) => out("Done."),
            Err(error) => {
                out(&format!("Failed: {error}"));
                failed = true;
            }
        }
    }
    i32::from(failed)
}

/// Ask before doing something that runs a program or reaches the network.
fn confirm(out: &mut dyn FnMut(&str)) -> bool {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
        out("Nothing here runs without being asked: add --yes to go ahead.");
        return false;
    }
    out("Go ahead? [y/N]");
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// --- plugins (work item 05-02) --------------------------------------------

/// Where user plugins live, for the install subcommands.
pub fn plugins_dir() -> Option<PathBuf> {
    data_dir().map(|data| user_plugins(&data))
}

/// `tinkerpad plugin install <path|url>`.
pub fn plugin_install(what: &str, out: &mut dyn FnMut(&str)) -> i32 {
    let Some(dir) = plugins_dir() else {
        out("could not work out where TinkerPad keeps its plugins");
        return 2;
    };
    let source = crate::install::Source::read(what);
    out(crate::install::TRUST_NOTICE);
    out("");
    match crate::install::install(&source, &dir, out) {
        Ok(installed) => {
            out(&format!(
                "{} {}{} at {}",
                if installed.replaced {
                    "Replaced"
                } else {
                    "Installed"
                },
                installed.name,
                installed
                    .version
                    .as_ref()
                    .map(|v| format!(" {v}"))
                    .unwrap_or_default(),
                installed.dir.display()
            ));
            // What it needs, now that the app knows it exists. A toolchain
            // that isn't here doesn't stop the install: the user may be
            // installing the plugin in order to go and get it.
            report_toolchain(&installed.id, out);
            0
        }
        Err(error) => {
            out(&error.to_string());
            1
        }
    }
}

/// `tinkerpad plugin update <id>`.
pub fn plugin_update(id: &str, out: &mut dyn FnMut(&str)) -> i32 {
    let Some(dir) = plugins_dir() else {
        out("could not work out where TinkerPad keeps its plugins");
        return 2;
    };
    match crate::install::update(id, &dir, out) {
        Ok(installed) => {
            out(&format!(
                "{} is up to date at {}",
                installed.name,
                installed.dir.display()
            ));
            0
        }
        Err(error) => {
            out(&error.to_string());
            1
        }
    }
}

/// `tinkerpad plugin remove <id>`.
pub fn plugin_remove(id: &str, out: &mut dyn FnMut(&str)) -> i32 {
    let Some(dir) = plugins_dir() else {
        out("could not work out where TinkerPad keeps its plugins");
        return 2;
    };
    match crate::install::remove(id, &dir) {
        Ok(gone) => {
            out(&format!(
                "{} moved to the recycle bin. Restart TinkerPad, or use the \
                 plugins panel, for the window to notice.",
                gone.display()
            ));
            0
        }
        Err(error) => {
            out(&error.to_string());
            1
        }
    }
}

/// `tinkerpad plugin list`.
pub fn plugin_list(json: bool, out: &mut dyn FnMut(&str)) -> i32 {
    let Some(data) = data_dir() else {
        out("could not work out where TinkerPad keeps its data");
        return 2;
    };
    let registry = tinkerpad_plugin::PluginRegistry::load(bundled_plugins(), user_plugins(&data));
    let rows: Vec<PluginRow> = registry
        .all()
        .iter()
        .map(|entry| PluginRow {
            id: entry.plugin.id.to_string(),
            name: entry.plugin.name.clone(),
            version: entry.plugin.version.clone(),
            source: match entry.source {
                tinkerpad_plugin::Source::Bundled => "bundled".to_owned(),
                tinkerpad_plugin::Source::User => {
                    std::fs::read_to_string(entry.plugin.dir.join(crate::install::SOURCE_FILE))
                        .map(|url| url.trim().to_owned())
                        .unwrap_or_else(|_| entry.plugin.dir.display().to_string())
                }
            },
            overridden: false,
            dir: entry.plugin.dir.display().to_string(),
        })
        // A bundled plugin a user plugin of the same id has replaced is
        // listed too, marked, so it is obvious why behaviour changed.
        .chain(registry.overrides().iter().map(|over| PluginRow {
            id: over.id.to_string(),
            name: over.id.to_string(),
            version: None,
            source: "bundled".to_owned(),
            overridden: true,
            dir: over.bundled_dir.display().to_string(),
        }))
        .collect();

    if json {
        match serde_json::to_string_pretty(&rows) {
            Ok(text) => out(&text),
            Err(error) => {
                out(&format!("{{\"error\":\"{error}\"}}"));
                return 2;
            }
        }
        return 0;
    }
    for row in &rows {
        out(&format!(
            "{:<14} {:<16} {:<10} {}{}",
            row.id,
            row.name,
            row.version.as_deref().unwrap_or("—"),
            row.source,
            if row.overridden {
                "  (overridden by a user plugin of the same id)"
            } else {
                ""
            }
        ));
    }
    for failure in registry.failures() {
        out(&format!(
            "{:<14} {}",
            failure
                .dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "failed to load"
        ));
    }
    0
}

/// One row of `plugin list --json`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PluginRow {
    id: String,
    name: String,
    version: Option<String>,
    /// `bundled`, the URL it came from, or the folder it was copied from.
    source: String,
    /// A bundled plugin a user plugin of the same id has replaced.
    overridden: bool,
    dir: String,
}

/// Say what a just-installed plugin needs, if anything.
fn report_toolchain(id: &str, out: &mut dyn FnMut(&str)) {
    let mut quiet = |_: &str| {};
    let Ok(context) = Context::open(&mut quiet) else {
        return;
    };
    let Some(plugin) = context
        .plugins
        .iter()
        .find(|plugin| plugin.id.as_str() == id)
    else {
        return;
    };
    let needs = tools::needs(&context.env, plugin, context.state_of(id));
    let missing: Vec<&tools::Need> = needs.iter().filter(|need| !need.present).collect();
    if missing.is_empty() {
        out("It has everything it needs.");
        return;
    }
    out("It still needs:");
    for need in missing {
        out(&format!("  {} {}", need.label, need.what));
        if let Some(hint) = &need.hint {
            out(&format!("    {hint}"));
        }
    }
    out(&format!(
        "`tinkerpad tools install {id}` can get what it can."
    ));
}

// --- the plugin index (work item 05-05) ------------------------------------

/// `tinkerpad registry check [path]`: everything wrong with an index.
///
/// The registry's own CI runs this on every pull request, so a contributor
/// adding an entry is told what is wrong with it rather than a maintainer
/// being told later.
pub fn registry_check(path: Option<&str>, out: &mut dyn FnMut(&str)) -> i32 {
    let path = path.map(str::to_owned).unwrap_or_else(|| {
        std::env::var(crate::registry::REGISTRY_ENV)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "registry/index.json".to_owned())
    });
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) => {
            out(&format!("could not read {path}: {error}"));
            return 2;
        }
    };
    let index = match crate::registry::Index::parse(&text) {
        Ok(index) => index,
        Err(error) => {
            out(&error.to_string());
            return 1;
        }
    };
    let problems = index.problems();
    if problems.is_empty() {
        out(&format!(
            "{}: {} plugin{} listed, nothing wrong",
            path,
            index.plugins.len(),
            if index.plugins.len() == 1 { "" } else { "s" }
        ));
        return 0;
    }
    for problem in &problems {
        out(&format!("  {problem}"));
    }
    out(&format!(
        "{}: {} problem{}",
        path,
        problems.len(),
        if problems.len() == 1 { "" } else { "s" }
    ));
    1
}

/// `tinkerpad registry schema [path]`: write the index's JSON Schema.
///
/// Generated from the types the app reads the index with, for the same reason
/// the manifest's is (05-04): a schema written beside a type is a schema that
/// drifts. `--check` fails instead of writing.
pub fn registry_schema(path: Option<&str>, check: bool, out: &mut dyn FnMut(&str)) -> i32 {
    let path = path.unwrap_or("registry/index.schema.json");
    let mut value = crate::registry::schema();
    if let Some(object) = value.as_object_mut() {
        object.insert(
            "title".into(),
            serde_json::Value::String("TinkerPad plugin index".into()),
        );
    }
    let mut text = match serde_json::to_string_pretty(&value) {
        Ok(text) => text,
        Err(error) => {
            out(&error.to_string());
            return 2;
        }
    };
    text.push('\n');

    let current = std::fs::read_to_string(path).unwrap_or_default();
    if current.replace("\r\n", "\n") == text {
        if check {
            out(&format!("{path} is up to date"));
        }
        return 0;
    }
    if check {
        out(&format!(
            "{path} is out of date. Run:\n\n    tinkerpad registry schema\n\nand commit it."
        ));
        return 1;
    }
    match std::fs::write(path, &text) {
        Ok(()) => {
            out(&format!("wrote {path}"));
            0
        }
        Err(error) => {
            out(&format!("could not write {path}: {error}"));
            2
        }
    }
}

/// What the Browse tab shows: the index, with what is installed marked.
///
/// Public so the window's command can call it; the fetch is
/// [`crate::registry::load`], which never runs on launch.
pub async fn registry_entries(
    data_dir: &std::path::Path,
    installed: &[String],
    force: bool,
) -> Result<Vec<BrowseEntry>, String> {
    let index = crate::registry::load(data_dir, force)
        .await
        .map_err(|e| e.to_string())?;
    Ok(index
        .plugins
        .into_iter()
        .map(|entry| BrowseEntry {
            installed: entry.bundled || installed.iter().any(|id| id == &entry.id),
            entry,
        })
        .collect())
}

/// Where the index comes from, and whether there is a copy already.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistrySource {
    /// The URL, or the path, the index would be read from.
    pub from: String,
    /// How old the cached copy is, if there is one. `None` means nothing has
    /// been fetched yet — which is what the window asks about.
    pub cached_age_ms: Option<u64>,
}

/// One row of the Browse tab.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseEntry {
    #[serde(flatten)]
    pub entry: crate::registry::Entry,
    /// Already here, so the button says so instead of offering to install it
    /// again.
    pub installed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The data folder is the window's, or the CLI reports on tools nobody
    /// else can see.
    #[test]
    fn the_identifier_is_the_one_tauri_is_configured_with() {
        let conf = include_str!("../tauri.conf.json");
        let value: serde_json::Value = serde_json::from_str(conf).expect("tauri.conf.json");
        assert_eq!(
            value.get("identifier").and_then(|v| v.as_str()),
            Some(IDENTIFIER),
            "src/cli.rs must use the identifier from tauri.conf.json"
        );
    }

    #[test]
    fn the_library_is_the_env_var_before_the_setting() {
        let mut settings = Settings::default();
        settings.library.root = Some(PathBuf::from("/from/settings"));
        // Safety: the test is single-threaded in its own process section and
        // puts the variable back.
        let before = std::env::var_os(crate::library::LIBRARY_ENV);
        unsafe { std::env::set_var(crate::library::LIBRARY_ENV, "/from/the/env") };
        assert_eq!(library_root(&settings), PathBuf::from("/from/the/env"));
        unsafe { std::env::remove_var(crate::library::LIBRARY_ENV) };
        assert_eq!(library_root(&settings), PathBuf::from("/from/settings"));
        if let Some(value) = before {
            unsafe { std::env::set_var(crate::library::LIBRARY_ENV, value) };
        }
    }
}
