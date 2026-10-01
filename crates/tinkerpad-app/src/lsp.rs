//! The language server pool and proxy inside the app.
//!
//! `tinkerpad-lsp` owns the pool (02-02) and the proxy (02-03); this module is
//! the three things the app has to supply: a `ServerConfig` built from the
//! plugin's `[lsp]` section, a channel to the tab's editor, and the lifecycle
//! ends — state changes become `server` events, and quitting stops every
//! server.
//!
//! The editor side of the channel arrives in 02-04. Until then nothing calls
//! `open`, because none of the bundled plugins has an `[lsp]` section yet.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::ipc::Channel;
use tinkerpad_core::{CommandError, ErrorCode, Playground, Scope};
use tinkerpad_lsp::jsonrpc::RawMessage;
use tinkerpad_lsp::{EditorSink, LspPool, Proxy, ServerConfig};
use tinkerpad_plugin::{IdleShutdown, Plugin, Resolver, Workspace, placeholders};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::AppState;
use crate::events::EventBus;
use crate::service_settings::ServiceSettings;

/// One language server, as the status strip sees it (work item 02-08).
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub language: String,
    /// The plugin's display name — "Rust", not "rust".
    pub name: String,
    /// The workspace root, or empty for a server that needs none.
    pub root: String,
    pub state: tinkerpad_core::ServerState,
    /// How long the current process has been up.
    pub uptime_ms: u64,
    /// Crashes it has come back from in a row (02-05).
    pub restarts: u32,
    /// Documents it has open.
    pub documents: usize,
    /// Playgrounds in its registered setting, for a server that takes one
    /// (`[lsp.register]`, 02-07). This is the number that used to keep
    /// growing as playgrounds were deleted (work item 02-10).
    pub projects: usize,
    /// Messages it has sent since it started, and how quickly it answers.
    pub messages: u64,
    pub requests: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    /// The program that was started, as resolved.
    pub program: String,
    /// What to do about it when there is no program: the plugin's hint.
    pub install_hint: Option<String>,
}

/// Every server the pool holds, running or not.
pub fn status(state: &AppState) -> Vec<ServerStatus> {
    let names: std::collections::BTreeMap<String, String> = state
        .plugins
        .read()
        .expect("plugins lock")
        .all()
        .iter()
        .map(|entry| (entry.plugin.id.to_string(), entry.plugin.name.clone()))
        .collect();

    let mut servers: Vec<ServerStatus> = state
        .lsp
        .servers()
        .into_iter()
        .map(|server| {
            let stats = server.stats();
            ServerStatus {
                name: names
                    .get(server.language())
                    .cloned()
                    .unwrap_or_else(|| server.language().to_owned()),
                language: server.language().to_owned(),
                root: server
                    .root()
                    .map(|r| r.display().to_string())
                    .unwrap_or_default(),
                state: server.state(),
                uptime_ms: stats.uptime_ms,
                restarts: server.restarts(),
                documents: server.open_docs().len(),
                projects: server.registrations().len(),
                messages: stats.messages,
                requests: stats.requests,
                p50_ms: stats.p50_us as f64 / 1000.0,
                p95_ms: stats.p95_us as f64 / 1000.0,
                program: server.program(),
                install_hint: server.install_hint().map(str::to_owned),
            }
        })
        .collect();
    servers.sort_by(|a, b| a.name.cmp(&b.name).then(a.root.cmp(&b.root)));
    servers
}

/// The last few hundred lines a server wrote to stderr or sent as
/// `window/logMessage`, oldest first.
pub fn server_log(state: &AppState, language: &str, root: Option<&std::path::Path>) -> Vec<String> {
    state
        .lsp
        .get(language, root)
        .map(|server| server.log())
        .unwrap_or_default()
}

/// Build the pool and forward its state changes to the UI.
pub fn pool(events: Arc<EventBus>) -> Arc<LspPool> {
    Arc::new(LspPool::new(Some(forwarder(events))))
}

/// Build the proxy. It publishes what a server asks the user to be told
/// (`window/showMessage`) as toasts.
pub fn proxy(events: Arc<EventBus>) -> Arc<Proxy> {
    Proxy::new(Some(forwarder(events)))
}

/// A channel that turns core events into bus events.
fn forwarder(events: Arc<EventBus>) -> mpsc::UnboundedSender<tinkerpad_core::CoreEvent> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            events.publish(Scope::App, event);
        }
    });
    tx
}

/// One tab's editor, on the other end of a Tauri channel.
struct ChannelSink(Channel<serde_json::Value>);

impl EditorSink for ChannelSink {
    fn send(&self, message: RawMessage) {
        if let Err(error) = self.0.send(message.into_value()) {
            tracing::debug!(%error, "the editor's channel is gone");
        }
    }
}

/// Connect a playground's editor to its language server, opening the document.
/// Returns the URI the editor should use for it.
pub async fn open(
    state: &AppState,
    playground: &Playground,
    channel: Channel<serde_json::Value>,
) -> Result<String, CommandError> {
    let plugin = state
        .plugins
        .read()
        .expect("plugins lock")
        .get(playground.language.as_str())
        .cloned()
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidInput,
                format!("no plugin is loaded for `{}`", playground.language),
            )
        })?;

    let resolver = state.toolchains.resolver();
    let config = config(
        &plugin,
        Some(playground),
        &resolver,
        &state.library.root(),
        &state.scratch_root(),
        &state.lsp_settings(),
    )?;
    // The editor's own client opens the document; the pool only has to have
    // the server running (02-04). A shared server is told which project this
    // playground is, through the setting its plugin named (02-07).
    let uri = tinkerpad_lsp::path_to_uri(&playground.main_file);
    let attachment = state
        .lsp
        .attach_registered(&config, &uri, registration(&plugin, playground));

    Ok(state.proxy.open_tab(
        playground.id.as_str(),
        &playground.main_file,
        attachment,
        Arc::new(ChannelSink(channel)),
    ))
}

/// The running server that would have this playground's folder open, if there
/// is one. Used when something refuses to let go of it (work item 04-09).
pub fn server_for(
    state: &AppState,
    playground: &Playground,
) -> Option<Arc<tinkerpad_lsp::ServerHandle>> {
    let plugin = state
        .plugins
        .read()
        .expect("plugins lock")
        .get(playground.language.as_str())
        .cloned()?;
    let root = root_for(
        &plugin,
        Some(playground),
        &state.library.root(),
        &state.scratch_root(),
    )
    .ok()?;
    state.lsp.get(plugin.id.as_str(), root.as_deref())
}

/// A playground is going away — deleted, or renamed so its folder moves.
/// Tell whatever server has it, before the folder does (work item 02-10).
///
/// This runs from the library's delete and rename paths rather than from a
/// tab's close path, because the tab may have been closed an hour ago, or may
/// never have existed: a playground can be deleted from the library panel
/// without being opened. Doing it twice is harmless — a `didClose` for a
/// document the server no longer has is ignored — and doing it not at all is
/// what leaves a shared server holding a project that isn't there any more.
///
/// Nothing here fails: a server that has already stopped, or never started, is
/// exactly the case where there is nothing to do.
pub async fn detach(state: &AppState, playground: &Playground) {
    // A tab still showing this playground is talking to the server through the
    // proxy; that has to stop before the document does, or its next message
    // reopens what this is closing.
    state.proxy.close_tab(playground.id.as_str()).await;

    let Some(plugin) = state
        .plugins
        .read()
        .expect("plugins lock")
        .get(playground.language.as_str())
        .cloned()
    else {
        return;
    };
    let Ok(root) = root_for(
        &plugin,
        Some(playground),
        &state.library.root(),
        &state.scratch_root(),
    ) else {
        return;
    };
    let Some(server) = state.lsp.get(plugin.id.as_str(), root.as_deref()) else {
        return; // Never started, or already gone.
    };

    let uri = tinkerpad_lsp::path_to_uri(&playground.main_file);
    if let Err(error) = server.did_close(&uri).await {
        debug!(%error, uri, "didClose for a playground going away");
    }
    // Takes it out of `[lsp.register]`, tells the server, and sends the
    // plugin's reload request if it named one. For rust-analyzer that is
    // `rust-analyzer/reloadWorkspace`, which is what stops it holding a
    // linked project whose Cargo.toml is in the recycle bin.
    server.unregister(&uri).await;

    // A server with no reload request of its own — typescript-language-server
    // is the case — is told and left to notice. Restarting it here was tried
    // and measured: it does not bring completions about the new path back any
    // sooner, because what is stale is tsserver's own project cache rather
    // than anything TinkerPad holds, and a restart on every delete is a real
    // cost for no gain. The document is closed and unregistered either way,
    // which is what this is for.

    info!(
        playground = %playground.id,
        language = %plugin.id,
        "detached a playground from its language server"
    );
}

/// The workspace root this plugin's server would be given for a playground:
/// the playground's own folder, the library, a folder of the app's own, or
/// none.
///
/// Also the key half of the pool's [`ServerKey`](tinkerpad_lsp::ServerKey), so
/// anything that needs to find a running server — a hook pushing settings
/// (03-07), the status strip — asks here rather than working it out again.
pub fn root_for(
    plugin: &Plugin,
    playground: Option<&Playground>,
    library_root: &std::path::Path,
    scratch_root: &std::path::Path,
) -> Result<Option<PathBuf>, CommandError> {
    let Some(lsp) = plugin.lsp.as_ref() else {
        return Ok(None);
    };
    // A server whose workspace is one playground's folder can't be started
    // before there is a playground; every other strategy can.
    let playground_dir = || -> Result<PathBuf, CommandError> {
        playground.map(|p| p.dir.clone()).ok_or_else(|| {
            CommandError::new(
                ErrorCode::InvalidInput,
                format!(
                    "{} puts its server in each playground's folder, so there is nothing to start until one is open",
                    plugin.name
                ),
            )
        })
    };
    Ok(match lsp.workspace {
        Workspace::None => None,
        Workspace::PlaygroundDir => Some(playground_dir()?),
        Workspace::LibraryRoot => Some(library_root.to_path_buf()),
        // A folder of the app's own, shared by every playground of this
        // language, and empty except for whatever the plugin writes there. It
        // is deliberately *beside* the plugin's scratch folder rather than
        // inside it: Rust's shared cargo target directory lives there, and a
        // server rooted above it would index every build artefact in it.
        // The playgrounds themselves are named through `[lsp.register]`.
        Workspace::ScratchWorkspace => Some(scratch_root.join(plugin.id.as_str()).join("lsp")),
    })
}

/// What the plugin's `[lsp]` section says, as the pool wants it.
///
/// The workspace strategy decides the root: the playground's own folder, or
/// the library root shared by every playground of that language. A server that
/// needs no root gets none. `scratch_workspace` is work item 02-07; until then
/// it behaves as `playground_dir`, which is correct if wasteful.
fn config(
    plugin: &Plugin,
    playground: Option<&Playground>,
    resolver: &Resolver,
    library_root: &std::path::Path,
    scratch_root: &std::path::Path,
    settings: &ServiceSettings,
) -> Result<ServerConfig, CommandError> {
    let lsp = plugin.lsp.as_ref().ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidInput,
            format!("{} has no language server ([lsp] section)", plugin.name),
        )
    })?;

    let root = root_for(plugin, playground, library_root, scratch_root)?;
    if let Some(root) = &root
        && let Err(error) = std::fs::create_dir_all(root)
    {
        tracing::warn!(root = %root.display(), %error, "could not make the server's workspace folder");
    }

    // The same placeholders a run gets, minus the ones that only make sense
    // for one execution. A pre-warmed server has no playground, so the ones
    // that name it are empty rather than missing.
    let dir = playground
        .map(|p| p.dir.to_string_lossy().into_owned())
        .unwrap_or_default();
    let main = playground
        .map(|p| p.main_file.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = playground.map(|p| p.name.clone()).unwrap_or_default();
    let plugin_dir = plugin.dir.to_string_lossy();
    // A plugin's own scratch folder: where its helper keeps whatever the
    // server needs to see, such as Go's `go.work` (work item 04-07).
    let scratch = tinkerpad_runner::spec::scratch_dir(scratch_root, plugin)
        .to_string_lossy()
        .into_owned();
    let root_text = root
        .as_deref()
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    let values: [(&str, &str); 6] = [
        ("dir", &dir),
        ("main", &main),
        ("plugin_dir", &plugin_dir),
        ("root", &root_text),
        ("name", &name),
        ("scratch", &scratch),
    ];
    let fill = |field: &str, template: &str| {
        placeholders::substitute(template, &values)
            .map_err(|e| CommandError::new(ErrorCode::InvalidInput, format!("[lsp] {field}: {e}")))
    };

    let program = resolver
        .resolve(plugin.id.as_str(), &fill("command", &lsp.command.command)?)
        .map_err(|reason| {
            CommandError::new(
                ErrorCode::NotFound,
                match &plugin.install_hint {
                    Some(hint) => format!("{}'s language server: {reason}.\n{hint}", plugin.name),
                    None => format!("{}'s language server: {reason}.", plugin.name),
                },
            )
        })?;

    let mut spawn = tinkerpad_process::SpawnSpec::new(program);
    for (i, arg) in lsp.command.args.iter().enumerate() {
        spawn = spawn.arg(fill(&format!("args[{i}]"), arg)?);
    }
    spawn = spawn.env("PATH", resolver.path());
    for (key, value) in &lsp.command.env {
        spawn = spawn.env(key, fill(&format!("env.{key}"), value)?);
    }
    if let Some(cwd) = &lsp.command.cwd {
        spawn = spawn.cwd(fill("cwd", cwd)?);
    }

    // Idle: the plugin says how long, and the user may override it per
    // language in settings (02-06).
    let from_plugin = match lsp.idle_shutdown {
        IdleShutdown::Never => None,
        IdleShutdown::After(idle) => Some(idle),
    };

    let mut config = ServerConfig::new(plugin.id.as_str(), spawn)
        .with_idle_shutdown(settings.idle_for(plugin.id.as_str(), from_plugin));
    if let Some(register) = &lsp.register {
        config = config.with_register_setting(register.setting.clone());
        if let Some(reload) = &register.reload {
            config = config.with_register_reload(reload.clone());
        }
    }
    config.root = root;
    config.init_options = lsp.init_options.clone();
    config.install_hint = plugin.install_hint.clone();

    // Files the workspace itself needs — a `tsconfig.json` for a server that
    // wants one — written once and never overwritten, so anything the user
    // changes there stays changed.
    if let Some(root) = &config.root {
        write_workspace_files(plugin, root);
    }
    Ok(config)
}

/// What this playground contributes to its server's registered setting, if the
/// plugin asks for one: `[lsp.register].value` with the placeholders filled in.
fn registration(plugin: &Plugin, playground: &Playground) -> Option<serde_json::Value> {
    let register = plugin.lsp.as_ref()?.register.as_ref()?;
    let dir = playground.dir.to_string_lossy();
    let main = playground.main_file.to_string_lossy();
    let plugin_dir = plugin.dir.to_string_lossy();
    let values = [
        ("dir", &*dir),
        ("main", &*main),
        ("plugin_dir", &*plugin_dir),
        ("id", playground.id.as_str()),
        ("name", &*playground.name),
    ];
    match placeholders::substitute(&register.value, &values) {
        Ok(value) => Some(serde_json::Value::String(value)),
        Err(error) => {
            tracing::warn!(plugin = %plugin.id, %error, "[lsp.register] value");
            None
        }
    }
}

/// Write the plugin's `[lsp].workspace_files` into the server's workspace, if
/// they aren't there already.
fn write_workspace_files(plugin: &Plugin, root: &std::path::Path) {
    let Some(lsp) = &plugin.lsp else { return };
    for file in &lsp.workspace_files {
        let path = root.join(&file.path);
        if path.exists() {
            continue;
        }
        let template = plugin.dir.join("templates").join(&file.template);
        match std::fs::read_to_string(&template) {
            Ok(text) => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match std::fs::write(&path, text) {
                    Ok(()) => tracing::info!(
                        plugin = %plugin.id,
                        file = %path.display(),
                        "wrote a file the language server's workspace needs"
                    ),
                    Err(error) => {
                        tracing::warn!(file = %path.display(), %error, "could not write it")
                    }
                }
            }
            Err(error) => tracing::warn!(
                template = %template.display(),
                %error,
                "[lsp].workspace_files names a template that can't be read"
            ),
        }
    }
}

/// Start the servers the user pinned in `[lsp] prewarm`, so the first tab of
/// the day is as quick as the hundredth.
///
/// Called once the UI says it is up, never before: a language server starting
/// is not allowed to delay the window. A language that can't be pre-warmed
/// (one whose server lives in each playground's folder) or isn't installed is
/// logged and skipped.
pub fn prewarm(state: &AppState) {
    let languages = state.lsp_settings().prewarm;
    if languages.is_empty() {
        return;
    }
    let resolver = state.toolchains.resolver();
    let library_root = state.library.root();
    let scratch_root = state.scratch_root();
    let plugins = state.plugins.read().expect("plugins lock");

    for language in languages {
        let Some(plugin) = plugins.get(&language).cloned() else {
            tracing::warn!(language, "[lsp] prewarm names a language with no plugin");
            continue;
        };
        match config(
            &plugin,
            None,
            &resolver,
            &library_root,
            &scratch_root,
            &state.lsp_settings(),
        ) {
            Ok(config) => {
                tracing::info!(language, "pre-warming a language server");
                state.lsp.server_for(&config);
            }
            Err(error) => {
                tracing::warn!(language, %error, "cannot pre-warm this language");
            }
        }
    }
}
