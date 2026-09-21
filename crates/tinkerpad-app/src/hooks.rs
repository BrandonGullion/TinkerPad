//! Plugin hooks (work item 03-07, `docs/PLUGINS.md` §5).
//!
//! Four moments in a playground's life are offered to the plugin's helper:
//! after it is created, when it is opened, when it is closed, and when a
//! package is added. The helper answers with files to write and, for the two
//! that a language server cares about, settings to push.
//!
//! This is the escape hatch that keeps language-specific logic out of Rust. Go
//! is the canonical case: a `go.work` that has to list every open playground
//! is a few lines of Go in a helper, and would be a special case here.
//!
//! Everything in here is best-effort by design. A hook that is slow, absent,
//! broken, or answering nonsense costs the user nothing: the operation carries
//! on as if it had returned nothing. The single exception is `add_dependency`,
//! where the user asked for something specific and deserves to hear that it
//! didn't happen.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tinkerpad_core::{CoreEvent, Level, Playground, PlaygroundId, Scope};
use tinkerpad_plugin::Plugin;
use tinkerpad_plugin::helper::{HelperFile, HookRequest, HookResult};
use tracing::{debug, info, warn};

use crate::AppState;

/// A new playground, after its `[scaffold]` files are on disk.
///
/// A hook's files may overwrite what the scaffold wrote: the manifest is what
/// the plugin says in general, the hook is what it says about this one.
pub async fn created(state: &AppState, plugin: &Arc<Plugin>, playground: &Playground) {
    let result = call(
        state,
        plugin,
        HookRequest::PlaygroundCreated {
            playground: playground.clone(),
        },
    )
    .await;
    if let Some(result) = result {
        write_files(state, plugin, playground, &result.files);
    }
}

/// A playground was opened, with every playground of its language that is
/// open now — this one included.
pub async fn opened(state: &AppState, plugin: &Arc<Plugin>, playground: &Playground) {
    let open = open_playgrounds(state, plugin);
    let result = call(
        state,
        plugin,
        HookRequest::PlaygroundOpened {
            playground: playground.clone(),
            open,
        },
    )
    .await;
    let Some(result) = result else { return };
    write_files(state, plugin, playground, &result.files);
    if let Some(settings) = result.lsp_config {
        configure(state, plugin, playground, settings).await;
    }
}

/// A playground was closed, with the ones still open.
pub async fn closed(state: &AppState, plugin: &Arc<Plugin>, playground: &Playground) {
    let open = open_playgrounds(state, plugin);
    forget_configuration(state, plugin, playground);
    let result = call(
        state,
        plugin,
        HookRequest::PlaygroundClosed {
            playground: playground.clone(),
            open,
        },
    )
    .await;
    let Some(result) = result else { return };
    write_files(state, plugin, playground, &result.files);
}

/// The user asked for a package. `None` when the plugin has no such hook, in
/// which case the caller falls back to `[deps]`.
///
/// Unlike the others this one reports: the user asked for something, and a
/// silent nothing would leave them wondering.
pub async fn add_dependency(
    state: &AppState,
    plugin: &Arc<Plugin>,
    playground: &Playground,
    name: &str,
    version: Option<&str>,
) -> Option<Vec<PathBuf>> {
    if !has_hook(plugin, "add_dependency") {
        return None;
    }
    let result = call(
        state,
        plugin,
        HookRequest::AddDependency {
            playground: playground.clone(),
            name: name.to_owned(),
            version: version.map(str::to_owned),
        },
    )
    .await;

    let Some(result) = result else {
        state.events.publish(
            Scope::App,
            CoreEvent::Toast {
                level: Level::Warning,
                message: format!(
                    "{} couldn't add {name}: its helper didn't answer.",
                    plugin.name
                ),
            },
        );
        return Some(Vec::new());
    };
    Some(write_files(state, plugin, playground, &result.files))
}

/// A tab opened on a playground: remember it, and tell the plugin.
///
/// The telling happens in the background. A hook has five seconds to answer,
/// and none of what it does — writing a file, configuring a server — is
/// anything the editor needs before it can show the text.
pub fn tab_opened(state: &AppState, app: &tauri::AppHandle, id: &PlaygroundId) {
    let fresh = state.open.lock().expect("open lock").insert(id.clone());
    if !fresh {
        return; // Already open: the UI re-read it, which is not an opening.
    }
    in_background(app, id.clone(), Moment::Opened);
}

/// A tab closed. The playground is still on disk; it is just no longer one of
/// the open ones, which is what the hook is told.
pub fn tab_closed(state: &AppState, app: &tauri::AppHandle, id: &PlaygroundId) {
    let was_open = state.open.lock().expect("open lock").remove(id);
    if !was_open {
        return;
    }
    in_background(app, id.clone(), Moment::Closed);
}

/// Which of the two tab hooks to send.
#[derive(Clone, Copy)]
enum Moment {
    Opened,
    Closed,
}

/// Look up the playground and its plugin, then send the hook on the app's
/// runtime. Nothing waits for it, and nothing fails if the playground has gone
/// in the meantime.
fn in_background(app: &tauri::AppHandle, id: PlaygroundId, moment: Moment) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let state: &AppState = app.state::<AppState>().inner();
        let Some(playground) = state.library.get(&id) else {
            return;
        };
        let plugin = state
            .plugins
            .read()
            .expect("plugins lock")
            .get(playground.language.as_str())
            .cloned();
        let Some(plugin) = plugin else { return };
        match moment {
            Moment::Opened => opened(state, &plugin, &playground).await,
            Moment::Closed => closed(state, &plugin, &playground).await,
        }
    });
}

/// Whether the plugin's manifest says its helper implements a hook. One that
/// doesn't is never asked (`docs/PLUGINS.md` §5).
pub fn has_hook(plugin: &Plugin, hook: &str) -> bool {
    plugin
        .helper
        .as_ref()
        .is_some_and(|helper| helper.hooks.iter().any(|declared| declared == hook))
}

/// Ask the plugin's helper, and give up quietly if there is nothing to ask.
///
/// Starting a helper to answer a hook is fine even for a plugin whose helper
/// doesn't run playgrounds: it is what the plugin asked for by declaring the
/// hook, and the idle clock will reap it.
async fn call(state: &AppState, plugin: &Arc<Plugin>, request: HookRequest) -> Option<HookResult> {
    let hook = request.name();
    if !has_hook(plugin, hook) {
        return None;
    }
    let config = match crate::helpers::config_for(state, plugin.id.as_str()) {
        Ok(config) => config,
        Err(e) => {
            warn!(plugin = %plugin.id, hook, error = %e.message, "no helper to ask");
            return None;
        }
    };
    let handle = state.helpers.handle(&config);
    // A hook is work: it holds the helper's idle clock like a run does.
    let _using = handle.using();
    let client = match state.helpers.get(&config).await {
        Ok(client) => client,
        Err(e) => {
            warn!(plugin = %plugin.id, hook, error = %e, "the helper could not be started");
            return None;
        }
    };
    match client.hook(request).await {
        Ok(result) => {
            debug!(plugin = %plugin.id, hook, files = result.files.len(), "hook answered");
            Some(result)
        }
        Err(e) => {
            warn!(plugin = %plugin.id, hook, error = %e, "the hook was not answered");
            None
        }
    }
}

/// Write what a hook asked for, inside the playground folder.
///
/// A path that climbs out of it, or names a drive, is refused and logged: a
/// helper is someone else's program, and the folder it was told about is the
/// folder it may write to.
fn write_files(
    state: &AppState,
    plugin: &Arc<Plugin>,
    playground: &Playground,
    files: &[HelperFile],
) -> Vec<PathBuf> {
    let mut written = Vec::new();
    for file in files {
        let Some(target) = target(&playground.dir, &file.path) else {
            warn!(
                plugin = %plugin.id,
                path = %file.path.display(),
                "a hook asked to write outside the playground; refused",
            );
            continue;
        };
        if let Some(parent) = target.parent()
            && let Err(e) = std::fs::create_dir_all(parent)
        {
            warn!(plugin = %plugin.id, path = %target.display(), error = %e, "could not make the folder for a hook's file");
            continue;
        }
        match std::fs::write(&target, &file.content) {
            Ok(()) => {
                info!(plugin = %plugin.id, path = %target.display(), "wrote a hook's file");
                written.push(target);
            }
            Err(e) => {
                warn!(plugin = %plugin.id, path = %target.display(), error = %e, "could not write a hook's file");
            }
        }
    }
    if !written.is_empty() {
        // The folder changed under the library's feet; it lists again.
        state.events.publish(Scope::App, CoreEvent::LibraryChanged);
    }
    written
}

/// Where a hook's file lands, or `None` if its path doesn't stay inside the
/// folder the hook was given.
///
/// The rule is the library's own (`safe_relative`): relative, no `..`, no root,
/// no drive, nothing Windows treats as a device. A helper's scratch folder
/// needs no permission from here — it is the helper's own, and it can write
/// there itself.
fn target(dir: &Path, path: &Path) -> Option<PathBuf> {
    let relative = tinkerpad_library::safe_relative(&path.to_string_lossy())?;
    Some(dir.join(relative))
}

/// Push what a hook said to this language's server, alongside what
/// `[lsp.register]` says (02-07).
async fn configure(
    state: &AppState,
    plugin: &Arc<Plugin>,
    playground: &Playground,
    settings: serde_json::Value,
) {
    let Some(server) = awaited_server(state, plugin, playground).await else {
        debug!(plugin = %plugin.id, "a hook configured a server that is not running; dropped");
        return;
    };
    let uri = tinkerpad_lsp::path_to_uri(&playground.main_file);
    debug!(plugin = %plugin.id, %settings, "a hook configured the language server");
    server.configure(&uri, settings).await;
}

fn forget_configuration(state: &AppState, plugin: &Arc<Plugin>, playground: &Playground) {
    let Some(server) = server_for(state, plugin, playground) else {
        return;
    };
    let uri = tinkerpad_lsp::path_to_uri(&playground.main_file);
    if server.forget_configuration(&uri) {
        let server = server.clone();
        tokio::spawn(async move { server.push_registrations().await });
    }
}

/// The server, waiting a little for one that is on its way.
///
/// The tab that opened this playground is attaching its server as the hook is
/// being asked, and which of the two finishes first is a race between a helper
/// and a language server starting. Rather than lose the settings to it, wait
/// — in a background task nobody is blocked on — for the server to appear.
async fn awaited_server(
    state: &AppState,
    plugin: &Arc<Plugin>,
    playground: &Playground,
) -> Option<Arc<tinkerpad_lsp::ServerHandle>> {
    const PATIENCE: Duration = Duration::from_secs(10);
    const STEP: Duration = Duration::from_millis(100);
    let deadline = std::time::Instant::now() + PATIENCE;
    loop {
        if let Some(server) = server_for(state, plugin, playground) {
            return Some(server);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(STEP).await;
    }
}

/// The running server for this playground's language, if there is one.
fn server_for(
    state: &AppState,
    plugin: &Arc<Plugin>,
    playground: &Playground,
) -> Option<Arc<tinkerpad_lsp::ServerHandle>> {
    let root = crate::lsp::root_for(
        plugin,
        Some(playground),
        &state.library.root(),
        &state.scratch_root(),
    )
    .ok()?;
    state.lsp.get(plugin.id.as_str(), root.as_deref())
}

/// Every open playground of this plugin's language, this one included.
fn open_playgrounds(state: &AppState, plugin: &Arc<Plugin>) -> Vec<Playground> {
    let open = state.open.lock().expect("open lock").clone();
    open.iter()
        .filter_map(|id| state.library.get(id))
        .filter(|playground| playground.language == plugin.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hooks_file_lands_in_the_playground_folder() {
        let dir = Path::new("/library/notes");
        assert_eq!(target(dir, Path::new("go.work")), Some(dir.join("go.work")),);
        assert_eq!(
            target(dir, Path::new("sub/dir/file.txt")),
            Some(dir.join("sub").join("dir").join("file.txt")),
        );
    }

    #[test]
    fn a_path_that_leaves_the_playground_folder_is_refused() {
        let dir = Path::new("/library/notes");
        for bad in [
            "../escaped.txt",
            "sub/../../escaped.txt",
            "/etc/passwd",
            "C:\\Windows\\System32\\hosts",
            "\\\\server\\share\\file",
            "",
        ] {
            assert_eq!(target(dir, Path::new(bad)), None, "{bad}");
        }
    }
}
