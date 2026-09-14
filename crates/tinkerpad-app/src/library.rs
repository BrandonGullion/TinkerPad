//! The playground library, as the app uses it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tauri::{AppHandle, Manager};
use tinkerpad_core::{CommandError, CoreEvent, ErrorCode, Level, Scope};
use tinkerpad_library::{Library, LibraryError, LibraryEvent, Watcher};
use tracing::warn;

use crate::events::EventBus;

/// Overrides the library folder. Meant for tests and portable setups.
pub const LIBRARY_ENV: &str = "TINKERPAD_LIBRARY";

/// Where playgrounds live: `TINKERPAD_LIBRARY`, else `[library].root` from
/// settings, else `~/TinkerPad`. Problems reading settings become notices.
pub fn root(app: &AppHandle, data_dir: &Path, notices: &mut Vec<(Level, String)>) -> PathBuf {
    if let Some(dir) = std::env::var_os(LIBRARY_ENV).filter(|v| !v.is_empty()) {
        return PathBuf::from(dir);
    }
    match crate::settings::library_root(data_dir) {
        Ok(Some(root)) => return root,
        Ok(None) => {}
        Err(message) => notices.push((
            Level::Warning,
            format!("Using the default library folder. {message}"),
        )),
    }
    app.path()
        .home_dir()
        .map(|home| home.join("TinkerPad"))
        .unwrap_or_else(|_| data_dir.join("library"))
}

/// Open the library at `root`, falling back to `<app-data>/library` if `root`
/// can't be created (a removed drive, a typo in settings).
pub fn open(root: PathBuf, data_dir: &Path, notices: &mut Vec<(Level, String)>) -> Arc<Library> {
    match Library::open(&root) {
        Ok(library) => library,
        Err(error) => {
            let fallback = data_dir.join("library");
            warn!(root = %root.display(), %error, fallback = %fallback.display(), "library folder unusable");
            notices.push((
                Level::Error,
                format!(
                    "Couldn't use the library folder {} ({error}). Playgrounds are being saved in {} for now.",
                    root.display(),
                    fallback.display()
                ),
            ));
            Library::open(&fallback).expect("app data folder is writable")
        }
    }
}

/// Start watching the library, forwarding changes to the UI.
pub fn watch(library: &Arc<Library>, events: Arc<EventBus>) -> Option<Watcher> {
    let result = library.watch(move |event| {
        let event = match event {
            LibraryEvent::Changed => CoreEvent::LibraryChanged,
            LibraryEvent::SourceChangedOnDisk(playground) => {
                CoreEvent::SourceChangedOnDisk { playground }
            }
        };
        events.publish(Scope::App, event);
    });
    match result {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            // The app still works; it just won't notice outside changes.
            warn!(%error, "library watcher unavailable");
            None
        }
    }
}

pub fn to_command_error(error: LibraryError) -> CommandError {
    let code = match &error {
        LibraryError::NotFound(_) => ErrorCode::NotFound,
        LibraryError::InvalidName(_) => ErrorCode::InvalidInput,
        LibraryError::Io { .. } | LibraryError::Trash { .. } | LibraryError::Watch(_) => {
            ErrorCode::Io
        }
    };
    CommandError::new(code, error.to_string())
}
