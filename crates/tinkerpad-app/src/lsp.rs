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

use crate::AppState;
use crate::events::EventBus;
use crate::lsp_settings::LspSettings;

/// One language server, as the UI sees it. The status strip (02-08) renders
/// these; until then it is how the self-tests look at the pool.
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub language: String,
    /// The workspace root, or empty for a server that needs none.
    pub root: String,
    pub state: tinkerpad_core::ServerState,
}

/// Every server the pool holds, running or not.
pub fn status(state: &AppState) -> Vec<ServerStatus> {
    let mut servers: Vec<ServerStatus> = state
        .lsp
        .servers()
        .into_iter()
        .map(|server| ServerStatus {
            language: server.language().to_owned(),
            root: server
                .root()
                .map(|r| r.display().to_string())
                .unwrap_or_default(),
            state: server.state(),
        })
        .collect();
    servers.sort_by(|a, b| a.language.cmp(&b.language).then(a.root.cmp(&b.root)));
    servers
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
        state.library.root(),
        &state.lsp_settings,
    )?;
    // The editor's own client opens the document; the pool only has to have
    // the server running (02-04).
    let uri = tinkerpad_lsp::path_to_uri(&playground.main_file);
    let attachment = state.lsp.attach_editor(&config, &uri);

    Ok(state.proxy.open_tab(
        playground.id.as_str(),
        &playground.main_file,
        attachment,
        Arc::new(ChannelSink(channel)),
    ))
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
    settings: &LspSettings,
) -> Result<ServerConfig, CommandError> {
    let lsp = plugin.lsp.as_ref().ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidInput,
            format!("{} has no language server ([lsp] section)", plugin.name),
        )
    })?;

    // A pre-warmed server has no playground yet, and a strategy that wants
    // one can't be pre-warmed. 02-07 gives those languages a shared workspace,
    // which is exactly what makes them warmable.
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
    let root: Option<PathBuf> = match lsp.workspace {
        Workspace::None => None,
        Workspace::PlaygroundDir => Some(playground_dir()?),
        Workspace::LibraryRoot => Some(library_root.to_path_buf()),
        Workspace::ScratchWorkspace => {
            tracing::warn!(
                plugin = %plugin.id,
                "scratch_workspace is work item 02-07; using the playground folder"
            );
            Some(playground_dir()?)
        }
    };

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
    let root_text = root
        .as_deref()
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.clone());
    let values: [(&str, &str); 5] = [
        ("dir", &dir),
        ("main", &main),
        ("plugin_dir", &plugin_dir),
        ("root", &root_text),
        ("name", &name),
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
    config.root = root;
    config.init_options = lsp.init_options.clone();
    config.install_hint = plugin.install_hint.clone();
    Ok(config)
}

/// Start the servers the user pinned in `[lsp] prewarm`, so the first tab of
/// the day is as quick as the hundredth.
///
/// Called once the UI says it is up, never before: a language server starting
/// is not allowed to delay the window. A language that can't be pre-warmed
/// (one whose server lives in each playground's folder) or isn't installed is
/// logged and skipped.
pub fn prewarm(state: &AppState) {
    let languages = state.lsp_settings.prewarm.clone();
    if languages.is_empty() {
        return;
    }
    let resolver = state.toolchains.resolver();
    let library_root = state.library.root().to_path_buf();
    let plugins = state.plugins.read().expect("plugins lock");

    for language in languages {
        let Some(plugin) = plugins.get(&language).cloned() else {
            tracing::warn!(language, "[lsp] prewarm names a language with no plugin");
            continue;
        };
        match config(&plugin, None, &resolver, &library_root, &state.lsp_settings) {
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
