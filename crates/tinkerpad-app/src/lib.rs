//! The TinkerPad desktop application.
//!
//! Wires the core crates into a Tauri 2 shell: commands the UI can invoke, the
//! single event channel the UI subscribes to, and the `AppState` that owns the
//! pools and the library.
//!
//! Adding a feature is "add a command, add an event": commands live in
//! [`commands`], return `Result<T, CommandError>`, and report progress by
//! publishing on the [`EventBus`].

pub mod events;
mod library;
mod logging;
mod lsp;
mod packages;
mod playgrounds;
mod plugins;
mod runs;
mod session;
mod settings;
mod toolchains;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

use tauri::{Emitter, Manager};
use tinkerpad_core::{CoreEvent, Level, Scope};
use tinkerpad_library::Library;
use tinkerpad_plugin::PluginRegistry;

use events::EventBus;
use runs::Runs;
use toolchains::Toolchains;

/// Name of the single Tauri event the UI listens to.
pub const EVENT_NAME: &str = "core-event";

/// When `run` started, for the launch-time measurement.
static STARTED: OnceLock<Instant> = OnceLock::new();

/// Everything commands need, held in Tauri's managed state.
pub struct AppState {
    pub events: Arc<EventBus>,
    runs: Runs,
    /// `<app-data>`: settings, scratch folders, the user plugin folder.
    data_dir: PathBuf,
    /// Where the open tabs are remembered (`session.rs`).
    session_path: PathBuf,
    toolchains: Arc<Toolchains>,
    /// Language servers, kept alive across tabs (work item 02-02). Empty until
    /// the editor client attaches to one (02-04).
    lsp: Arc<tinkerpad_lsp::LspPool>,
    /// The editor's side of those servers: one channel per tab, URIs and
    /// request ids translated in both directions (02-03).
    proxy: Arc<tinkerpad_lsp::Proxy>,
    /// Read-only after startup; `reload` will take the write lock (05-02).
    plugins: RwLock<PluginRegistry>,
    library: Arc<Library>,
    /// Keeps the library watcher running; `None` if it couldn't start.
    _library_watcher: Mutex<Option<tinkerpad_library::Watcher>>,
    /// Notices raised before the UI was listening, delivered on its first `ping`.
    startup_notices: Mutex<Vec<(Level, String)>>,
}

mod commands {
    use tauri::State;
    use tinkerpad_core::CommandError;

    use super::*;

    /// The UI calls this once its script has loaded. The reply carries the
    /// time since process start, which is the launch-to-interactive number.
    #[tauri::command]
    pub fn ping(state: State<'_, AppState>) -> Result<(), CommandError> {
        let ready_ms = STARTED
            .get()
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);
        // Printed as well as logged so scripts/smoke-launch.sh can see it.
        println!("ui ready {ready_ms} ms after start");
        tracing::info!(ready_ms, "ui ready");
        state
            .events
            .publish(Scope::App, CoreEvent::Pong { ready_ms });
        let notices = std::mem::take(&mut *state.startup_notices.lock().expect("notices lock"));
        for (level, message) in notices {
            state
                .events
                .publish(Scope::App, CoreEvent::Toast { level, message });
        }
        Ok(())
    }

    /// The plugin folders, loaded plugins, and plugins that failed to load.
    #[tauri::command]
    pub fn list_plugins(state: State<'_, AppState>) -> crate::plugins::PluginsStatus {
        let registry = state.plugins.read().expect("plugins lock");
        crate::plugins::PluginsStatus::from_registry(&registry)
    }

    /// The latest toolchain state of every loaded plugin. States still being
    /// checked are `checking`; `toolchain` events report each result.
    #[tauri::command]
    pub fn toolchain_status(
        state: State<'_, AppState>,
    ) -> Vec<crate::toolchains::LanguageToolchain> {
        state.toolchains.snapshot()
    }

    /// Rediscover PATH, reread settings overrides, and probe every toolchain
    /// again. Returns immediately; results arrive as `toolchain` events.
    #[tauri::command]
    pub fn recheck_toolchains(app: tauri::AppHandle) {
        crate::toolchains::check_all(&app);
    }

    /// Every playground (newest first) and folders with broken metadata.
    #[tauri::command(async)]
    pub fn list_playgrounds(state: State<'_, AppState>) -> tinkerpad_library::Listing {
        state.library.list()
    }

    /// Create a playground for a loaded plugin. `kind` defaults to the
    /// plugin's default kind. The main file is the kind's prelude, template
    /// (with `{{name}}` and `{{date}}` filled in), and postlude; the plugin's
    /// scaffold files are written alongside it.
    #[tauri::command(async)]
    pub fn create_playground(
        state: State<'_, AppState>,
        name: String,
        language: String,
        kind: Option<String>,
    ) -> Result<tinkerpad_core::Playground, CommandError> {
        use tinkerpad_core::{ErrorCode, KindId};
        let plugin = state
            .plugins
            .read()
            .expect("plugins lock")
            .get(&language)
            .cloned()
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::InvalidInput,
                    format!("no plugin is loaded for language `{language}`"),
                )
            })?;
        let kind = match kind {
            Some(id) => plugin.kind(&id).ok_or_else(|| {
                CommandError::new(
                    ErrorCode::InvalidInput,
                    format!("{} has no kind `{id}`", plugin.name),
                )
            })?,
            None => plugin.default_kind(),
        };
        use tinkerpad_plugin::scaffold::{self, ScaffoldVars};
        let display_name = name.trim().to_owned();
        let date = scaffold::today_utc();
        let scratch = tinkerpad_runner::spec::scratch_dir(&state.data_dir.join("scratch"), &plugin);
        state
            .library
            .create_with(
                tinkerpad_library::NewPlayground {
                    name,
                    language: plugin.id.clone(),
                    kind: KindId::new(kind.id.clone()),
                    main_path: plugin.main_path.clone(),
                    files: Vec::new(),
                },
                |claimed| {
                    // Templates can name the folder, so they're rendered once
                    // the library has chosen it.
                    let package = scaffold::package_name(claimed.folder, claimed.id.as_str());
                    let vars = ScaffoldVars {
                        name: &display_name,
                        date: &date,
                        folder: claimed.folder,
                        package: &package,
                        plugin_dir: &plugin.dir,
                        scratch: &scratch,
                    };
                    let out = scaffold::playground_files(&plugin, kind, &vars)
                        .map_err(|e| e.to_string())?;
                    Ok(tinkerpad_library::Scaffold {
                        files: out.files,
                        plugin_meta: out.paths,
                    })
                },
            )
            .map_err(crate::library::to_command_error)
    }

    /// A playground's source and which of its lines are locked.
    #[tauri::command(async)]
    pub fn open_playground(
        state: State<'_, AppState>,
        id: tinkerpad_core::PlaygroundId,
    ) -> Result<crate::playgrounds::OpenedPlayground, CommandError> {
        crate::playgrounds::open(&state, &id)
    }

    /// Rename a playground; its folder follows the new name.
    #[tauri::command(async)]
    pub fn rename_playground(
        state: State<'_, AppState>,
        id: tinkerpad_core::PlaygroundId,
        name: String,
    ) -> Result<tinkerpad_core::Playground, CommandError> {
        state
            .library
            .rename(&id, &name)
            .map_err(crate::library::to_command_error)
    }

    /// Move a playground's folder to the OS recycle bin.
    #[tauri::command(async)]
    pub fn delete_playground(
        state: State<'_, AppState>,
        id: tinkerpad_core::PlaygroundId,
    ) -> Result<(), CommandError> {
        state
            .library
            .delete(&id)
            .map_err(crate::library::to_command_error)
    }

    #[tauri::command(async)]
    pub fn read_source(
        state: State<'_, AppState>,
        id: tinkerpad_core::PlaygroundId,
    ) -> Result<String, CommandError> {
        state
            .library
            .read_source(&id)
            .map_err(crate::library::to_command_error)
    }

    /// Replace a playground's main file atomically. Returns whether the file
    /// changed: identical text leaves it (and its modification time) alone.
    #[tauri::command(async)]
    pub fn write_source(
        state: State<'_, AppState>,
        id: tinkerpad_core::PlaygroundId,
        text: String,
    ) -> Result<bool, CommandError> {
        state
            .library
            .write_source(&id, &text)
            .map_err(crate::library::to_command_error)
    }

    /// Save `text` to a playground and run it with its plugin's `[run]`
    /// command. Returns the run id; output and the outcome arrive as `run`
    /// events scoped to that id.
    #[tauri::command]
    pub async fn run(
        state: State<'_, AppState>,
        playground: tinkerpad_core::PlaygroundId,
        text: String,
    ) -> Result<u64, CommandError> {
        use tinkerpad_core::ErrorCode;
        let language = state
            .library
            .get(&playground)
            .ok_or_else(|| {
                CommandError::new(
                    tinkerpad_core::ErrorCode::NotFound,
                    format!("no playground {playground}"),
                )
            })?
            .language;
        let plugin = state
            .plugins
            .read()
            .expect("plugins lock")
            .get(language.as_str())
            .cloned()
            .ok_or_else(|| {
                CommandError::new(
                    ErrorCode::NotFound,
                    format!("no plugin is loaded for `{language}`, so this playground can't run"),
                )
            })?;
        state
            .runs
            .start(&state.library, plugin, &playground, text)
            .await
    }

    /// Cancel a playground's run, if it has one.
    #[tauri::command]
    pub async fn cancel(
        state: State<'_, AppState>,
        playground: tinkerpad_core::PlaygroundId,
    ) -> Result<(), CommandError> {
        state.runs.cancel(&playground).await;
        Ok(())
    }

    /// Open the folder containing TinkerPad's log files.
    #[tauri::command]
    pub fn open_log_folder(app: tauri::AppHandle) -> Result<(), CommandError> {
        use tauri_plugin_opener::OpenerExt;
        let dir = app
            .path()
            .app_log_dir()
            .map_err(|e| CommandError::internal(format!("no log folder: {e}")))?;
        app.opener()
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| CommandError::new(tinkerpad_core::ErrorCode::Io, e.to_string()))
    }

    /// Connect this playground's editor to its language server (02-03).
    /// Messages for the editor arrive on `channel`; the reply is the URI the
    /// editor should use for the document.
    #[tauri::command(async)]
    pub async fn lsp_open(
        state: State<'_, AppState>,
        playground: tinkerpad_core::PlaygroundId,
        text: String,
        channel: tauri::ipc::Channel<serde_json::Value>,
    ) -> Result<String, CommandError> {
        let playground = state.library.get(&playground).ok_or_else(|| {
            CommandError::new(
                tinkerpad_core::ErrorCode::NotFound,
                format!("no playground {playground}"),
            )
        })?;
        crate::lsp::open(&state, &playground, text, channel).await
    }

    /// One message from a tab's editor, on its way to the server.
    #[tauri::command(async)]
    pub async fn lsp_send(
        state: State<'_, AppState>,
        playground: tinkerpad_core::PlaygroundId,
        message: serde_json::Value,
    ) -> Result<(), CommandError> {
        state
            .proxy
            .from_editor(
                playground.as_str(),
                tinkerpad_lsp::jsonrpc::RawMessage::new(message),
            )
            .await;
        Ok(())
    }

    /// The tab closed: cancel what it was waiting for and close its document.
    /// The server stays up for the next tab.
    #[tauri::command(async)]
    pub async fn lsp_close(
        state: State<'_, AppState>,
        playground: tinkerpad_core::PlaygroundId,
    ) -> Result<(), CommandError> {
        state.proxy.close_tab(playground.as_str()).await;
        Ok(())
    }

    /// Add a package to a playground the way its plugin's `[deps]` says:
    /// either a command runs (its output arrives as `run` events) or the UI
    /// gets a line to insert at the top of the main file.
    #[tauri::command(async)]
    pub async fn add_package(
        state: State<'_, AppState>,
        playground: tinkerpad_core::PlaygroundId,
        name: String,
        version: Option<String>,
    ) -> Result<crate::packages::AddPackage, CommandError> {
        crate::packages::add(
            &state,
            &playground,
            &name,
            version.as_deref().unwrap_or_default(),
        )
        .await
    }

    /// One sample of every dump kind, for the dev-only "render examples"
    /// command and the renderer self-test.
    #[tauri::command]
    pub fn dump_examples() -> Vec<tinkerpad_core::RichValue> {
        tinkerpad_core::dump::examples()
    }

    /// The tabs open when the app last ran. Empty if there's no usable file.
    #[tauri::command(async)]
    pub fn load_session(state: State<'_, AppState>) -> crate::session::Session {
        crate::session::load(&state.session_path)
    }

    /// Remember the open tabs for the next launch.
    #[tauri::command(async)]
    pub fn save_session(
        state: State<'_, AppState>,
        session: crate::session::Session,
    ) -> Result<(), CommandError> {
        crate::session::save(&state.session_path, &session).map_err(|e| {
            CommandError::new(
                tinkerpad_core::ErrorCode::Io,
                format!("Couldn't save the open tabs: {e}"),
            )
        })
    }

    /// Which in-app self-test scenarios to run, from `TINKERPAD_SELFTEST`
    /// (comma-separated). `None` in normal use. See `ui/src/selftest.ts`.
    #[tauri::command]
    pub fn selftest_scenario() -> Option<String> {
        std::env::var("TINKERPAD_SELFTEST")
            .ok()
            .filter(|s| !s.is_empty())
    }

    /// Self-tests only: change a playground's main file the way another program
    /// would, bypassing the library, so the file watcher reports it.
    #[tauri::command(async)]
    pub fn selftest_touch_source(
        state: State<'_, AppState>,
        id: tinkerpad_core::PlaygroundId,
        text: String,
    ) -> Result<(), CommandError> {
        use tinkerpad_core::ErrorCode;
        if selftest_scenario().is_none() {
            return Err(CommandError::new(
                ErrorCode::InvalidInput,
                "only available while self-testing",
            ));
        }
        let playground = state
            .library
            .get(&id)
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("no playground {id}")))?;
        std::fs::write(&playground.main_file, text)
            .map_err(|e| CommandError::new(ErrorCode::Io, e.to_string()))
    }

    /// One self-test result line, printed so scripts and CI can read it.
    #[tauri::command]
    pub fn selftest_report(line: String) {
        println!("selftest: {line}");
    }
}

pub fn run() {
    STARTED.get_or_init(Instant::now);

    tauri::Builder::default()
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            match app.path().app_log_dir() {
                Ok(dir) => {
                    if let Err(error) = logging::init(&dir) {
                        eprintln!("logging disabled: {error}");
                    }
                }
                Err(error) => eprintln!("logging disabled: no log folder: {error}"),
            }

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;

            // macOS has no parent-death signal; sweep children left behind by a
            // force-quit of the previous session. No-op elsewhere.
            tinkerpad_process::platform::set_registry(data_dir.join("children.pids"));
            let swept = tinkerpad_process::platform::sweep_orphans();
            if swept > 0 {
                tracing::warn!(
                    swept,
                    "terminated orphaned processes from a previous session"
                );
            }

            let bus = Arc::new(EventBus::new());
            let handle = app.handle().clone();
            events::spawn_forwarder(bus.subscribe(), move |env| {
                if let Err(error) = handle.emit(EVENT_NAME, env) {
                    tracing::warn!(%error, seq = env.seq, "failed to emit event to UI");
                }
            });

            let (registry, mut startup_notices) = plugins::load(app.handle())?;

            settings::ensure_exists(&data_dir);
            let toolchains = Arc::new(Toolchains::default());

            let library_root = library::root(app.handle(), &data_dir, &mut startup_notices);
            let playgrounds = library::open(library_root, &data_dir, &mut startup_notices);
            let library_watcher = library::watch(&playgrounds, bus.clone());

            let runs = Runs::new(bus.clone(), data_dir.join("scratch"), toolchains.clone());
            let lsp = lsp::pool(bus.clone());
            let proxy = lsp::proxy(bus.clone());
            app.manage(AppState {
                events: bus,
                runs,
                session_path: session::path(&data_dir),
                data_dir,
                toolchains,
                lsp,
                proxy,
                plugins: RwLock::new(registry),
                library: playgrounds,
                _library_watcher: Mutex::new(library_watcher),
                startup_notices: Mutex::new(startup_notices),
            });

            // Probes run in the background; the window never waits for them.
            toolchains::check_all(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::ping,
            commands::run,
            commands::cancel,
            commands::open_log_folder,
            commands::list_plugins,
            commands::toolchain_status,
            commands::recheck_toolchains,
            commands::list_playgrounds,
            commands::create_playground,
            commands::open_playground,
            commands::rename_playground,
            commands::delete_playground,
            commands::read_source,
            commands::write_source,
            commands::add_package,
            commands::lsp_open,
            commands::lsp_send,
            commands::lsp_close,
            commands::dump_examples,
            commands::load_session,
            commands::save_session,
            commands::selftest_scenario,
            commands::selftest_report,
            commands::selftest_touch_source,
        ])
        .build(tauri::generate_context!())
        .expect("error while building TinkerPad")
        .run(|app, event| {
            // Quitting: every language server gets `shutdown` then `exit`, all
            // at once, and is killed if it doesn't go. The pool caps the whole
            // thing at three seconds, so quitting is never held up (02-02).
            if matches!(event, tauri::RunEvent::Exit) {
                let pool = app.state::<AppState>().lsp.clone();
                tauri::async_runtime::block_on(pool.shutdown());
            }
        });
}
