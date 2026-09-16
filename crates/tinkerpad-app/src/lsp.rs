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
use tinkerpad_plugin::{Plugin, Resolver, Workspace, placeholders};
use tokio::sync::mpsc;

use crate::AppState;
use crate::events::EventBus;

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
    let config = config(&plugin, playground, &resolver, state.library.root())?;
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
    playground: &Playground,
    resolver: &Resolver,
    library_root: &std::path::Path,
) -> Result<ServerConfig, CommandError> {
    let lsp = plugin.lsp.as_ref().ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidInput,
            format!("{} has no language server ([lsp] section)", plugin.name),
        )
    })?;

    let root: Option<PathBuf> = match lsp.workspace {
        Workspace::None => None,
        Workspace::PlaygroundDir => Some(playground.dir.clone()),
        Workspace::LibraryRoot => Some(library_root.to_path_buf()),
        Workspace::ScratchWorkspace => {
            tracing::warn!(
                plugin = %plugin.id,
                "scratch_workspace is work item 02-07; using the playground folder"
            );
            Some(playground.dir.clone())
        }
    };

    // The same placeholders a run gets, minus the ones that only make sense
    // for one execution.
    let dir = playground.dir.to_string_lossy();
    let main = playground.main_file.to_string_lossy();
    let plugin_dir = plugin.dir.to_string_lossy();
    let root_text = root.as_deref().unwrap_or(&playground.dir).to_string_lossy();
    let values: [(&str, &str); 5] = [
        ("dir", &dir),
        ("main", &main),
        ("plugin_dir", &plugin_dir),
        ("root", &root_text),
        ("name", &playground.name),
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

    let mut config = ServerConfig::new(plugin.id.as_str(), spawn);
    config.root = root;
    config.init_options = lsp.init_options.clone();
    config.install_hint = plugin.install_hint.clone();
    Ok(config)
}
