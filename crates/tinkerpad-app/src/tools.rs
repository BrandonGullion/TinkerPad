//! Getting a language's tools, when the user asks (work item 04-08a).
//!
//! A plugin says how to get what it needs — run the command its maintainers
//! tell you to run, or fetch the release they publish — and this does it. What
//! lands here is the same tool you would have installed by hand: TinkerPad
//! neither vendors nor wraps it.
//!
//! Three rules shape the module.
//!
//! **Nothing happens unasked.** Every entry point is a request with a plugin
//! and a tool named in it. There is no timer, no first-run sweep, and
//! `[tools] acquire = "never"` refuses the lot.
//!
//! **A download is checked before it is believed.** The SHA-256 in the
//! manifest is computed as the bytes arrive; a mismatch deletes the file and
//! says so. Only then is anything unpacked, and only the member the manifest
//! names, into a folder of its own — never over anything already there.
//!
//! **What lands is removable.** Everything goes under
//! `<app-data>/tools/<plugin>/<version>/`, which the resolver searches after
//! the user's settings and before `PATH`. Deleting that folder undoes this
//! module entirely.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tinkerpad_core::{CoreEvent, LanguageId, Level, Scope, ToolchainState};
use tinkerpad_plugin::{Acquire, AcquireFile, AcquiredTools, Arch, Os, Plugin, Resolver};
use tokio::io::AsyncWriteExt;
use tracing::{debug, info, warn};

use crate::AppState;
use crate::events::EventBus;

/// How long a download may stall before it is given up on. The whole transfer
/// has no limit: a slow network is not an error.
const IDLE: Duration = Duration::from_secs(30);

/// Which of a plugin's tools is being asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Which {
    /// What `[toolchain]` probes for: the language itself.
    Toolchain,
    /// What `[lsp]` runs: the language server.
    Lsp,
    /// What `[lsp].workspace_needs` names: not a program but something the
    /// server's workspace must contain, such as the TypeScript the TypeScript
    /// server analyses with (work item 04-08b).
    Workspace,
}

impl Which {
    pub const ALL: [Which; 3] = [Which::Toolchain, Which::Lsp, Which::Workspace];

    fn of(self, plugin: &Plugin) -> Option<&Acquire> {
        match self {
            Which::Toolchain => plugin.toolchain_acquire.as_ref(),
            Which::Lsp => plugin.lsp.as_ref()?.acquire.as_ref(),
            Which::Workspace => plugin.lsp.as_ref()?.workspace_acquire.as_ref(),
        }
    }

    fn what(self, plugin: &Plugin) -> String {
        match self {
            Which::Toolchain => plugin.name.clone(),
            Which::Lsp => format!("{}'s language server", plugin.name),
            Which::Workspace => format!(
                "what {}'s language server needs in its workspace",
                plugin.name
            ),
        }
    }

    /// The short label the window puts in the list.
    pub fn label(self) -> &'static str {
        match self {
            Which::Toolchain => "Toolchain",
            Which::Lsp => "Language server",
            Which::Workspace => "Workspace",
        }
    }
}

/// What this module needs from the app: where tools go, where to say what is
/// happening, how to find a program, and what `[tools]` allows.
///
/// It is a handful of values rather than the whole `AppState` so the engine can
/// be driven by a test with no window, no plugins loaded and no Tauri runtime.
pub struct Env {
    /// `<app-data>`; tools land under `tools/` inside it.
    pub data_dir: PathBuf,
    pub events: Arc<EventBus>,
    /// Finds the programs an acquire command needs, the same way runs do.
    pub resolver: Arc<Resolver>,
    /// `[tools] mirror`, when the machine fetches releases from somewhere else.
    pub mirror: Option<String>,
    /// `[tools] acquire`: false means never, and nothing here will run.
    pub offers: bool,
    /// The library folder, which is some servers' workspace (work item 04-08b).
    pub library_root: PathBuf,
    /// Where the app keeps per-plugin scratch folders, the other kind of
    /// workspace a server can have.
    pub scratch_root: PathBuf,
}

impl Env {
    pub fn of(state: &AppState) -> Env {
        let settings = state.settings.get();
        Env {
            data_dir: state.data_dir.clone(),
            events: state.events.clone(),
            resolver: state.toolchains.resolver(),
            mirror: settings
                .tools
                .mirror
                .clone()
                .filter(|mirror| !mirror.is_empty()),
            offers: settings.tools.offers(),
            library_root: state.library.root(),
            scratch_root: state.data_dir.join("scratch"),
        }
    }

    /// Where a plugin's language server has its workspace, if it has one that
    /// exists without a playground open. `[lsp].workspace_needs` is a path
    /// inside this.
    fn workspace(&self, plugin: &Plugin) -> Option<PathBuf> {
        crate::lsp::root_for(plugin, None, &self.library_root, &self.scratch_root).ok()?
    }
}

/// What a plugin can be offered for a tool, and why it might be refused.
///
/// Answering this needs no network and changes nothing, so the window can ask
/// about every language at once.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    /// What will happen, in one line: the command, or the version to fetch.
    pub label: String,
    /// `command` or `download`.
    pub kind: &'static str,
    /// The URL a download would fetch, so the user can see where it comes from.
    pub url: Option<String>,
    /// Its SHA-256, for the same reason.
    pub sha256: Option<String>,
    /// Set when the offer can't be made, and says why in one sentence.
    pub blocked: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} doesn't say how to install {1}")]
    NoAcquire(String, String),
    #[error("{0}")]
    Blocked(String),
    #[error("{program} isn't installed, and it is what installs this")]
    Needs { program: String },
    #[error("{0}")]
    Command(String),
    #[error("the download failed: {0}")]
    Download(String),
    #[error(
        "the download didn't match its checksum, so it was thrown away\n  expected {expected}\n  got      {got}"
    )]
    Checksum { expected: String, got: String },
    #[error("the archive didn't contain `{0}`")]
    Missing(String),
    #[error("{0}")]
    Io(String),
}

/// One thing a language needs, whether it is there, and what could be done
/// about it (work item 04-08b).
///
/// Answering needs no network and changes nothing, so the window asks about
/// every language at once and shows a whole picture rather than a trickle of
/// toasts.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Need {
    pub which: Which,
    /// "Toolchain", "Language server", "Workspace".
    pub label: &'static str,
    /// What is being looked for: a program name, or a path in the workspace.
    pub what: String,
    /// Whether it is there.
    pub present: bool,
    /// Where it was found, when it was.
    pub found_at: Option<String>,
    /// What version, when the probe captured one.
    pub version: Option<String>,
    /// Why it isn't there, in one line.
    pub reason: Option<String>,
    /// The plugin's own `install_hint`, which is what a machine that can't or
    /// won't acquire is left with.
    pub hint: Option<String>,
    /// What TinkerPad could do about it, if anything.
    pub offer: Option<Offer>,
}

/// Everything one language needs.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageNeeds {
    pub language: LanguageId,
    /// The display name: "C#", not "csharp".
    pub name: String,
    pub needs: Vec<Need>,
}

impl LanguageNeeds {
    /// Whether everything is there. A language with nothing missing should say
    /// so as plainly as one without.
    pub fn ready(&self) -> bool {
        self.needs.iter().all(|need| need.present)
    }
}

/// What every loaded plugin needs, in the order the plugins were loaded.
pub fn status(state: &AppState, env: &Env) -> Vec<LanguageNeeds> {
    let plugins: Vec<Arc<Plugin>> = state
        .plugins
        .read()
        .expect("plugins lock")
        .all()
        .iter()
        .map(|entry| entry.plugin.clone())
        .collect();

    plugins
        .iter()
        .map(|plugin| LanguageNeeds {
            language: plugin.id.clone(),
            name: plugin.name.clone(),
            needs: needs(env, plugin, state.toolchains.state(plugin.id.as_str())),
        })
        .collect()
}

/// What one plugin needs. `toolchain` is what the probe last found, which is
/// the only thing here the app has to be running to know.
pub fn needs(env: &Env, plugin: &Plugin, toolchain: Option<ToolchainState>) -> Vec<Need> {
    Which::ALL
        .iter()
        .filter_map(|&which| need(env, plugin, which, toolchain.clone()))
        .collect()
}

/// One row, or nothing when the plugin has no such tool at all — a language
/// with no server has no server row.
fn need(
    env: &Env,
    plugin: &Plugin,
    which: Which,
    toolchain: Option<ToolchainState>,
) -> Option<Need> {
    let (what, present, found_at, version, reason) = match which {
        Which::Toolchain => {
            let program = plugin
                .probes
                .first()
                .map(|probe| probe.command.clone())
                .unwrap_or_else(|| plugin.name.clone());
            match toolchain {
                Some(ToolchainState::Ready {
                    program,
                    path,
                    version,
                }) => (program, true, Some(path), version, None),
                Some(ToolchainState::Missing { reason, .. }) => {
                    (program, false, None, None, Some(reason))
                }
                // Still checking, or never checked: not missing, just unknown.
                _ => (program, true, None, None, None),
            }
        }
        Which::Lsp => {
            let lsp = plugin.lsp.as_ref()?;
            let program = lsp.command.command.clone();
            match env.resolver.resolve(plugin.id.as_str(), &program) {
                Ok(path) => (
                    program,
                    true,
                    Some(path.to_string_lossy().into_owned()),
                    None,
                    None,
                ),
                Err(reason) => (program, false, None, None, Some(reason)),
            }
        }
        Which::Workspace => {
            let needs = plugin.lsp.as_ref()?.workspace_needs.as_ref()?;
            let shown = needs.to_string_lossy().into_owned();
            // A server rooted in each playground's folder has no workspace
            // until one is open; nothing to report yet.
            let root = env.workspace(plugin)?;
            let path = root.join(needs);
            if path.exists() {
                (
                    shown,
                    true,
                    Some(path.to_string_lossy().into_owned()),
                    None,
                    None,
                )
            } else {
                let reason = format!(
                    "`{}` isn't in {}, where the server looks for it",
                    shown,
                    root.display()
                );
                (shown, false, None, None, Some(reason))
            }
        }
    };

    Some(Need {
        which,
        label: which.label(),
        what,
        present,
        found_at,
        version,
        reason,
        hint: plugin.install_hint.clone(),
        // Nothing to offer for something already here.
        offer: if present {
            None
        } else {
            offer(env, plugin, which)
        },
    })
}

/// What a plugin would do for one of its tools, and whether it can.
pub fn offer(env: &Env, plugin: &Plugin, which: Which) -> Option<Offer> {
    let acquire = which.of(plugin)?;
    let mut offer = match acquire {
        // The label is the command itself unless the plugin wrote one, so it
        // is already what the confirmation should say.
        Acquire::Command { needs, label, .. } => Offer {
            label: label.clone(),
            kind: "command",
            url: None,
            sha256: None,
            blocked: needs.as_ref().and_then(|program| {
                have(env, plugin, program)
                    .is_none()
                    .then(|| format!("{program} isn't installed, and it is what installs this"))
            }),
        },
        Acquire::Download { version, .. } => {
            let file = this_machine(acquire);
            Offer {
                label: format!("download version {version}"),
                kind: "download",
                url: file.map(|f| f.url.clone()),
                sha256: file.map(|f| f.sha256.clone()),
                blocked: file.is_none().then(|| {
                    format!(
                        "there is no {} build of this for {}",
                        Arch::current().map_or("this machine".to_owned(), |a| a.to_string()),
                        std::env::consts::OS,
                    )
                }),
            }
        }
    };
    if !env.offers {
        offer.blocked = Some(
            "installing tools is switched off in settings ([tools] acquire = \"never\")".to_owned(),
        );
    }
    Some(offer)
}

/// Get a tool, having been asked to.
///
/// Returns where it ended up, for a download, or nothing for a command that
/// installed it wherever its package manager keeps things. The caller probes
/// the toolchain again afterwards, so the language becomes usable without a
/// restart.
pub async fn acquire(env: &Env, plugin: &Plugin, which: Which) -> Result<Option<PathBuf>, Error> {
    let what = which.what(plugin);
    let acquire = which
        .of(plugin)
        .ok_or_else(|| Error::NoAcquire(plugin.name.clone(), what.clone()))?
        .clone();

    if !env.offers {
        return Err(Error::Blocked(
            "installing tools is switched off in settings ([tools] acquire = \"never\")".to_owned(),
        ));
    }

    say(&env.events, Level::Info, format!("Getting {what}…"));
    let result = match &acquire {
        Acquire::Command {
            command,
            env: vars,
            needs,
            label,
        } => {
            // A workspace dependency is installed *in* the workspace; every
            // other command is a package manager that knows its own mind.
            let folder = match which {
                Which::Workspace => env.workspace(plugin),
                _ => None,
            };
            let filled = fill(env, plugin, command, vars, folder.as_deref())?;
            run_command(
                env,
                plugin,
                &filled.command,
                &filled.vars,
                needs.as_deref(),
                label,
                folder.as_deref(),
            )
            .await
            .map(|()| None)
        }
        Acquire::Download { version, .. } => {
            let file = this_machine(&acquire).ok_or_else(|| {
                Error::Blocked(format!("there is no build of {what} for this machine"))
            })?;
            download(env, plugin, version, file).await.map(Some)
        }
    };

    match &result {
        Ok(_) => {
            info!(plugin = %plugin.id, ?which, "tool acquired");
            say(&env.events, Level::Info, format!("{what} is ready."));
        }
        Err(error) => warn!(plugin = %plugin.id, ?which, %error, "could not get the tool"),
    }
    result
}

/// Run a package manager, streaming what it says.
async fn run_command(
    env: &Env,
    plugin: &Plugin,
    command: &[String],
    vars: &BTreeMap<String, String>,
    needs: Option<&str>,
    label: &str,
    folder: Option<&Path>,
) -> Result<(), Error> {
    if let Some(program) = needs
        && have(env, plugin, program).is_none()
    {
        return Err(Error::Needs {
            program: program.to_owned(),
        });
    }
    let (program, args) = command
        .split_first()
        .ok_or_else(|| Error::Command("there is no command to run".to_owned()))?;
    let resolved = have(env, plugin, program)
        .ok_or_else(|| Error::Command(format!("{program} isn't installed")))?;

    debug!(plugin = %plugin.id, %label, "running an acquire command");
    let mut spec = tinkerpad_process::SpawnSpec::new(resolved);
    for arg in args {
        spec = spec.arg(arg);
    }
    spec = spec.env("PATH", env.resolver.path());
    for (key, value) in vars {
        spec = spec.env(key, value);
    }
    if let Some(folder) = folder {
        std::fs::create_dir_all(folder).map_err(|e| Error::Io(e.to_string()))?;
        spec = spec.cwd(folder);
    }

    let mut child = tinkerpad_process::spawn(spec)
        .map_err(|e| Error::Command(format!("{label} could not start: {e}")))?;
    let mut last = String::new();
    while let Some(event) = child.recv().await {
        match event {
            tinkerpad_process::ProcessEvent::Stdout(bytes)
            | tinkerpad_process::ProcessEvent::Stderr(bytes) => {
                let text = String::from_utf8_lossy(&bytes);
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    last = line.to_owned();
                    tool_line(&env.events, plugin, line);
                }
            }
            tinkerpad_process::ProcessEvent::Exited(exit) => {
                return match exit.code {
                    Some(0) => Ok(()),
                    Some(code) => Err(Error::Command(format!(
                        "{label} exited {code}{}",
                        if last.is_empty() {
                            String::new()
                        } else {
                            format!(": {last}")
                        }
                    ))),
                    None => Err(Error::Command(format!("{label} was stopped"))),
                };
            }
        }
    }
    Err(Error::Command(format!("{label} said nothing and stopped")))
}

/// A command and its environment with `{tools_dir}`, `{plugin_dir}` and
/// `{workspace}` filled in.
struct Filled {
    command: Vec<String>,
    vars: BTreeMap<String, String>,
}

/// Substitute what an acquire command is allowed to name.
///
/// `{tools_dir}` is this plugin's own folder under `<app-data>/tools`, which
/// the resolver searches: a package manager pointed at it installs where the
/// app will find it, needs no elevation, and leaves one folder to delete.
fn fill(
    env: &Env,
    plugin: &Plugin,
    command: &[String],
    vars: &BTreeMap<String, String>,
    folder: Option<&Path>,
) -> Result<Filled, Error> {
    let tools = acquired_dir(&env.data_dir, plugin);
    std::fs::create_dir_all(&tools).map_err(|e| Error::Io(e.to_string()))?;
    let tools = tools.to_string_lossy().into_owned();
    let plugin_dir = plugin.dir.to_string_lossy().into_owned();
    let workspace = folder
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let values = [
        ("tools_dir", tools.as_str()),
        ("plugin_dir", plugin_dir.as_str()),
        ("workspace", workspace.as_str()),
    ];
    let one = |text: &str| {
        tinkerpad_plugin::placeholders::substitute(text, &values)
            .map_err(|e| Error::Command(e.to_string()))
    };
    Ok(Filled {
        command: command
            .iter()
            .map(|part| one(part))
            .collect::<Result<_, _>>()?,
        vars: vars
            .iter()
            .map(|(key, value)| one(value).map(|value| (key.clone(), value)))
            .collect::<Result<_, _>>()?,
    })
}

/// Where a command-installed tool is pointed at: one folder per plugin, under
/// the same root a download lands in. It is named rather than versioned
/// because a package manager decides its own versions.
fn acquired_dir(data_dir: &Path, plugin: &Plugin) -> PathBuf {
    tools_dir(data_dir)
        .join(plugin.id.as_str())
        .join("acquired")
}

/// Fetch a release, check it, and unpack it into a folder of its own.
async fn download(
    env: &Env,
    plugin: &Plugin,
    version: &str,
    file: &AcquireFile,
) -> Result<PathBuf, Error> {
    let url = mirrored(env, &file.url);
    let root = tools_dir(&env.data_dir)
        .join(plugin.id.as_str())
        .join(version);
    // Already here: the checksum was verified when it landed, and the folder
    // is named for the version, so there is nothing to redo.
    if let Some(existing) = executable_in(&root) {
        debug!(plugin = %plugin.id, path = %existing.display(), "already downloaded");
        return Ok(existing);
    }

    let staging = root.with_extension("partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| Error::Io(e.to_string()))?;
    let temp = staging.join("download");

    say(
        &env.events,
        Level::Info,
        format!("Downloading {} {version}…", plugin.name),
    );
    let bytes = fetch(&url, &temp, &env.events, plugin)
        .await
        .inspect_err(|_| {
            let _ = std::fs::remove_dir_all(&staging);
        })?;
    tool_line(
        &env.events,
        plugin,
        &format!("downloaded {url} ({bytes} bytes)"),
    );

    // Verified before anything is unpacked, and thrown away if it is wrong.
    let got = hash_of(&temp).map_err(|e| Error::Io(e.to_string()))?;
    if got != file.sha256 {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(Error::Checksum {
            expected: file.sha256.clone(),
            got,
        });
    }
    tool_line(&env.events, plugin, "checksum matches");

    let unpacked = unpack(&temp, &staging, file).inspect_err(|_| {
        let _ = std::fs::remove_dir_all(&staging);
    })?;
    let _ = std::fs::remove_file(&temp);

    // Into place as one step: a half-extracted tool must never be found by the
    // resolver.
    if let Some(parent) = root.parent() {
        std::fs::create_dir_all(parent).map_err(|e| Error::Io(e.to_string()))?;
    }
    let _ = std::fs::remove_dir_all(&root);
    std::fs::rename(&staging, &root).map_err(|e| Error::Io(e.to_string()))?;
    let landed = root.join(unpacked.file_name().unwrap_or_default());
    // The same bit a plugin's own helpers get; a no-op on Windows, where the
    // extension decides. Platform code lives in one place (ARCHITECTURE §10).
    tinkerpad_plugin::env::make_executable(&landed).map_err(|e| Error::Io(e.to_string()))?;
    info!(plugin = %plugin.id, path = %landed.display(), "tool downloaded");
    Ok(landed)
}

/// Stream a URL into a file, giving up if it stalls.
async fn fetch(url: &str, into: &Path, events: &EventBus, plugin: &Plugin) -> Result<u64, Error> {
    let client = reqwest::Client::builder()
        .read_timeout(IDLE)
        .user_agent(concat!("TinkerPad/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| Error::Download(e.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| Error::Download(e.to_string()))?;
    if !response.status().is_success() {
        return Err(Error::Download(format!(
            "{} answered {}",
            url,
            response.status()
        )));
    }
    let total = response.content_length();

    let mut file = tokio::fs::File::create(into)
        .await
        .map_err(|e| Error::Io(e.to_string()))?;
    let mut written = 0u64;
    let mut said = 0u64;
    let mut response = response;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Error::Download(e.to_string()))?
    {
        file.write_all(&chunk)
            .await
            .map_err(|e| Error::Io(e.to_string()))?;
        written += chunk.len() as u64;
        // A line every megabyte: enough to see it moving, not enough to spam.
        if written - said > 1_000_000 {
            said = written;
            let text = match total {
                Some(total) => format!("{} of {} bytes", written, total),
                None => format!("{written} bytes"),
            };
            tool_line(events, plugin, &text);
        }
    }
    file.flush().await.map_err(|e| Error::Io(e.to_string()))?;
    Ok(written)
}

/// The SHA-256 of a file, as lowercase hex.
fn hash_of(path: &Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Take the tool out of what was downloaded.
///
/// Only the member the manifest names, and only into `into`: an archive that
/// asks for anything else — a path climbing out, a symlink, an absolute name —
/// is not unpacked.
fn unpack(archive: &Path, into: &Path, file: &AcquireFile) -> Result<PathBuf, Error> {
    let Some(member) = &file.member else {
        // Not an archive: the download is the tool.
        let name = file
            .rename_to
            .clone()
            .unwrap_or_else(|| name_from_url(&file.url));
        let target = into.join(name);
        std::fs::copy(archive, &target).map_err(|e| Error::Io(e.to_string()))?;
        return Ok(target);
    };

    let name = file
        .rename_to
        .clone()
        .unwrap_or_else(|| last_segment(member).to_owned());
    let target = into.join(&name);

    let url = file.url.to_ascii_lowercase();
    if url.ends_with(".zip") {
        take_from_zip(archive, member, &target)?;
    } else if url.ends_with(".tar.gz") || url.ends_with(".tgz") {
        take_from_tar_gz(archive, member, &target)?;
    } else if url.ends_with(".gz") {
        let input = std::fs::File::open(archive).map_err(|e| Error::Io(e.to_string()))?;
        let mut decoder = flate2::read::GzDecoder::new(input);
        let mut out = std::fs::File::create(&target).map_err(|e| Error::Io(e.to_string()))?;
        std::io::copy(&mut decoder, &mut out).map_err(|e| Error::Io(e.to_string()))?;
    } else {
        return Err(Error::Download(format!(
            "`member` was given, but {url} isn't an archive this understands (.zip, .tar.gz, .gz)"
        )));
    }
    Ok(target)
}

fn take_from_zip(archive: &Path, member: &str, target: &Path) -> Result<(), Error> {
    let file = std::fs::File::open(archive).map_err(|e| Error::Io(e.to_string()))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| Error::Download(e.to_string()))?;
    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| Error::Download(e.to_string()))?;
        if !entry.is_file() || !same_path(entry.name(), member) {
            continue;
        }
        let mut out = std::fs::File::create(target).map_err(|e| Error::Io(e.to_string()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| Error::Io(e.to_string()))?;
        return Ok(());
    }
    Err(Error::Missing(member.to_owned()))
}

fn take_from_tar_gz(archive: &Path, member: &str, target: &Path) -> Result<(), Error> {
    let file = std::fs::File::open(archive).map_err(|e| Error::Io(e.to_string()))?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let entries = tar.entries().map_err(|e| Error::Download(e.to_string()))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| Error::Download(e.to_string()))?;
        // Only ordinary files: a symlink or a hard link in an archive is a way
        // out of the folder it is being unpacked into.
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let path = entry.path().map_err(|e| Error::Download(e.to_string()))?;
        if !same_path(&path.to_string_lossy(), member) {
            continue;
        }
        let mut out = std::fs::File::create(target).map_err(|e| Error::Io(e.to_string()))?;
        std::io::copy(&mut entry, &mut out).map_err(|e| Error::Io(e.to_string()))?;
        return Ok(());
    }
    Err(Error::Missing(member.to_owned()))
}

/// Whether an archive entry is the member asked for, either as the whole path
/// or as the file at the end of it — releases often wrap everything in a
/// top-level folder named after the version.
fn same_path(entry: &str, member: &str) -> bool {
    let entry = entry.replace('\\', "/");
    let member = member.replace('\\', "/");
    entry == member || last_segment(&entry) == last_segment(&member)
}

fn last_segment(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

fn name_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or("tool")
        .split(['?', '#'])
        .next()
        .unwrap_or("tool")
        .to_owned()
}

/// Where acquired tools live: `<app-data>/tools`.
pub fn tools_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("tools")
}

/// What has been acquired, per plugin, for the resolver to search after the
/// settings overrides and before `PATH` (work item 04-08a).
///
/// A plugin only ever sees its own folder: Go's `gopls` is not something the
/// Python plugin should find, however alike two programs are named.
pub fn search_paths(data_dir: &Path) -> AcquiredTools {
    let mut acquired = AcquiredTools::new();
    let Ok(plugins) = std::fs::read_dir(tools_dir(data_dir)) else {
        return acquired;
    };
    for plugin in plugins.filter_map(Result::ok) {
        let Some(id) = plugin.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let Ok(versions) = std::fs::read_dir(plugin.path()) else {
            continue;
        };
        let mut found: Vec<PathBuf> = versions
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect();
        // Sorted, so a machine with two versions of a tool picks the same one
        // every launch rather than whatever the file system said first.
        found.sort();
        // A package manager pointed at a prefix puts programs one level down,
        // and each has its own idea where: npm in `node_modules/.bin`, pip in
        // `Scripts` or `bin`, Go straight in. Looking in all of them is what
        // makes a `--prefix` install usable without elevation.
        for folder in found.clone() {
            for inside in [
                folder.join("node_modules").join(".bin"),
                folder.join("Scripts"),
                folder.join("bin"),
            ] {
                if inside.is_dir() {
                    found.push(inside);
                }
            }
        }
        if found.is_empty() {
            continue;
        }
        acquired.insert(id, found);
    }
    acquired
}

/// The first executable in a folder, which is what a download leaves.
fn executable_in(dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.is_file())
}

/// Where a program is, if it is anywhere this app would look.
fn have(env: &Env, plugin: &Plugin, program: &str) -> Option<PathBuf> {
    env.resolver.resolve(plugin.id.as_str(), program).ok()
}

/// The download for this machine.
fn this_machine(acquire: &Acquire) -> Option<&AcquireFile> {
    acquire.file_for(Os::current(), Arch::current()?)
}

/// A URL through the mirror, when settings name one.
fn mirrored(env: &Env, url: &str) -> String {
    match env.mirror.as_deref() {
        // The mirror replaces the origin, keeping the path: an internal copy of
        // the same layout is the common arrangement.
        Some(mirror) => match url
            .split_once("://")
            .and_then(|(_, rest)| rest.split_once('/'))
        {
            Some((_, path)) => format!("{}/{}", mirror.trim_end_matches('/'), path),
            None => url.to_owned(),
        },
        None => url.to_owned(),
    }
}

fn say(events: &EventBus, level: Level, message: String) {
    events.publish(Scope::App, CoreEvent::Toast { level, message });
}

/// A line of progress, for the window to show while this is happening.
fn tool_line(events: &EventBus, plugin: &Plugin, line: &str) {
    events.publish(
        Scope::App,
        CoreEvent::Tool {
            plugin: plugin.id.clone(),
            line: line.to_owned(),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_member_is_matched_by_name_or_by_path() {
        assert!(same_path("rust-analyzer.exe", "rust-analyzer.exe"));
        assert!(same_path("gopls-v1/gopls", "gopls"));
        assert!(same_path("a/b/tool", "x/y/tool"));
        assert!(!same_path("something-else", "tool"));
    }

    #[test]
    fn a_name_is_taken_from_the_url_when_there_is_no_member() {
        assert_eq!(name_from_url("https://x/y/tool-1.2.exe"), "tool-1.2.exe");
        assert_eq!(name_from_url("https://x/y/tool?token=1"), "tool");
        // A URL that names no file falls back rather than guessing.
        assert_eq!(name_from_url("https://x/"), "tool");
    }

    #[test]
    fn hashing_matches_what_a_checksum_says() {
        let dir = std::env::temp_dir().join("tinkerpad-tools-hash");
        let _ = std::fs::create_dir_all(&dir);
        let file = dir.join("bytes");
        std::fs::write(&file, b"abc").unwrap();
        assert_eq!(
            hash_of(&file).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        );
        let _ = std::fs::remove_dir_all(dir);
    }
}
