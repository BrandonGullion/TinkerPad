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
/// settings, else `~/TinkerPad`.
pub fn root(
    app: &AppHandle,
    data_dir: &Path,
    settings: &tinkerpad_core::Settings,
    notices: &mut Vec<(Level, String)>,
) -> PathBuf {
    let _ = notices;
    if let Some(dir) = std::env::var_os(LIBRARY_ENV).filter(|v| !v.is_empty()) {
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

/// Point the library at the folder the settings now name, and watch it there
/// (work item 04-02).
///
/// Returns the folder in use, which is the one asked for unless it couldn't be
/// opened — in which case the library stays where it was and the caller is
/// told what happened.
pub fn change_root(
    app: &AppHandle,
    state: &crate::AppState,
) -> Result<String, tinkerpad_core::CommandError> {
    let settings = state.settings.get();
    let mut notices = Vec::new();
    let root = root(app, &state.data_dir, &settings, &mut notices);
    // Stop watching the old folder before the library leaves it.
    *state._library_watcher.lock().expect("watcher lock") = None;
    state.library.change_root(&root).map_err(to_command_error)?;
    *state._library_watcher.lock().expect("watcher lock") =
        watch(&state.library, state.events.clone());
    state.events.publish(Scope::App, CoreEvent::LibraryChanged);
    Ok(root.to_string_lossy().into_owned())
}

/// The library folder as the panel's header shows it, and the folders in it
/// that can't be opened (work item 04-12).
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryInfo {
    pub root: PathBuf,
    /// `root`, with the home folder written `~`.
    pub display: String,
    pub broken: Vec<tinkerpad_core::BrokenPlayground>,
}

pub fn info(library: &Library, home: Option<&Path>) -> LibraryInfo {
    let root = library.root();
    LibraryInfo {
        display: shorten(&root, home),
        broken: library.broken(),
        root,
    }
}

/// `C:\Users\me\TinkerPad` as `~/TinkerPad`: short enough for a panel header,
/// and the same on every OS. Anywhere outside the home folder is left whole.
fn shorten(path: &Path, home: Option<&Path>) -> String {
    let rest = home
        .filter(|home| !home.as_os_str().is_empty())
        .and_then(|home| path.strip_prefix(home).ok());
    match rest {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.to_string_lossy().replace('\\', "/")),
        None => path.to_string_lossy().into_owned(),
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

/// Delete a playground, dealing with whatever has its folder open
/// (work item 04-09).
///
/// The order matters. The playground is detached from its language server
/// first (02-10), which is both the right thing to do and the thing most
/// likely to make the folder free. The library then retries the trash
/// operation for about a second, because a watcher letting go is a race.
///
/// If that isn't enough, the server watching it is stopped and the delete
/// tried once more. Measured on Windows: rust-analyzer holds a handle without
/// `FILE_SHARE_DELETE` for as long as it is running, and no amount of waiting
/// changes that — the retry alone refuses every time. Stopping the server is
/// the thing that always works, it costs nothing the user can't get back (the
/// next tab starts it again), and it is what the work item says to do when the
/// retry can't be made reliable.
///
/// What never happens is a permanent delete behind the user's back: the trash
/// is the undo they expect.
pub async fn delete(
    state: &crate::AppState,
    id: &tinkerpad_core::PlaygroundId,
) -> Result<(), CommandError> {
    let playground = state.library.get(id);
    if let Some(playground) = &playground {
        crate::lsp::detach(state, playground).await;
    }
    let Err(error) = state.library.delete(id) else {
        return Ok(());
    };
    if !is_in_use(&error) {
        return Err(to_command_error(error));
    }

    // Something has it. If that something is one of ours, it can be asked.
    let Some(playground) = playground else {
        return Err(to_command_error(error));
    };
    let Some(server) = crate::lsp::server_for(state, &playground) else {
        return Err(in_use_error(&playground.name, None));
    };
    let program = program_name(&server.program());
    warn!(
        playground = %playground.id,
        %program,
        "stopping the language server so the playground can be deleted"
    );
    server.stop().await;
    state.events.publish(
        Scope::App,
        CoreEvent::Toast {
            level: Level::Info,
            message: format!(
                "Stopped {program} so {} could go to the recycle bin. It starts again with the next tab.",
                playground.name
            ),
        },
    );

    match state.library.delete(id) {
        Ok(()) => Ok(()),
        Err(again) if is_in_use(&again) => Err(in_use_error(&playground.name, Some(&program))),
        Err(again) => Err(to_command_error(again)),
    }
}

/// Whether a failure is "someone else has this open" rather than a real
/// problem with the folder.
///
/// The trash crate flattens the reason into a message, so this reads it back.
/// A wrong guess here costs an extra attempt and a clearer sentence, not
/// correctness.
fn is_in_use(error: &LibraryError) -> bool {
    match error {
        LibraryError::Trash { .. } => true,
        LibraryError::Io { source, .. } => {
            matches!(
                source.kind(),
                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ResourceBusy
            ) || source.raw_os_error() == Some(32)
        } // ERROR_SHARING_VIOLATION
        _ => false,
    }
}

/// What to say when a folder still can't be moved. Never the `trash` crate's
/// own words: "Some operations were aborted" is not something anyone can act
/// on (work item 04-09).
fn in_use_error(name: &str, program: Option<&str>) -> CommandError {
    let message = match program {
        Some(program) => format!(
            "{name} couldn't be moved to the recycle bin: something still has its folder open,              even after {program} was stopped. Close anything looking at that folder — a terminal,              an editor, a file manager — and try again."
        ),
        None => format!(
            "{name} couldn't be moved to the recycle bin: something else has its folder open.              Close anything looking at it — a terminal, an editor, a file manager — and try again."
        ),
    };
    CommandError::new(ErrorCode::Io, message)
}

/// The name of a program, for a sentence: `rust-analyzer`, not its full path.
fn program_name(program: &str) -> String {
    Path::new(program)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.to_owned())
}

/// Rename a playground, dealing with whatever has its folder open. A rename
/// moves the folder, so it is refused for the same reasons a delete is
/// (work item 04-09).
pub async fn rename(
    state: &crate::AppState,
    id: &tinkerpad_core::PlaygroundId,
    name: &str,
) -> Result<tinkerpad_core::Playground, CommandError> {
    let playground = state.library.get(id);
    if let Some(playground) = &playground {
        crate::lsp::detach(state, playground).await;
    }
    let error = match state.library.rename(id, name) {
        Ok(renamed) => return Ok(renamed),
        Err(error) => error,
    };
    if !is_in_use(&error) {
        return Err(to_command_error(error));
    }
    let Some(playground) = playground else {
        return Err(to_command_error(error));
    };
    let Some(server) = crate::lsp::server_for(state, &playground) else {
        return Err(renaming_error(&playground.name, None));
    };
    let program = program_name(&server.program());
    warn!(
        playground = %playground.id,
        %program,
        "stopping the language server so the playground can be renamed"
    );
    server.stop().await;
    state.events.publish(
        Scope::App,
        CoreEvent::Toast {
            level: Level::Info,
            message: format!(
                "Stopped {program} so {} could be renamed. It starts again with the next tab.",
                playground.name
            ),
        },
    );
    match state.library.rename(id, name) {
        Ok(renamed) => Ok(renamed),
        Err(again) if is_in_use(&again) => Err(renaming_error(&playground.name, Some(&program))),
        Err(again) => Err(to_command_error(again)),
    }
}

fn renaming_error(name: &str, program: Option<&str>) -> CommandError {
    let message = match program {
        Some(program) => format!(
            "{name} couldn't be renamed: something still has its folder open, even after              {program} was stopped. Close anything looking at that folder and try again."
        ),
        None => format!(
            "{name} couldn't be renamed: something else has its folder open. Close anything              looking at it and try again."
        ),
    };
    CommandError::new(ErrorCode::Io, message)
}

pub fn to_command_error(error: LibraryError) -> CommandError {
    let code = match &error {
        LibraryError::NotFound(_) | LibraryError::NotBroken(_) => ErrorCode::NotFound,
        LibraryError::InvalidName(_) => ErrorCode::InvalidInput,
        LibraryError::Io { .. }
        | LibraryError::Trash { .. }
        | LibraryError::Watch(_)
        | LibraryError::Scaffold(_) => ErrorCode::Io,
    };
    CommandError::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_library_folder_is_shortened_under_home_only() {
        let home = std::env::temp_dir().join("someone");
        assert_eq!(shorten(&home.join("TinkerPad"), Some(&home)), "~/TinkerPad");
        assert_eq!(shorten(&home.join("a").join("b"), Some(&home)), "~/a/b");
        assert_eq!(shorten(&home, Some(&home)), "~");
        let elsewhere = std::env::temp_dir().join("elsewhere");
        assert_eq!(
            shorten(&elsewhere, Some(&home)),
            elsewhere.to_string_lossy()
        );
        assert_eq!(shorten(&elsewhere, None), elsewhere.to_string_lossy());
    }

    #[test]
    fn a_program_is_named_rather_than_pathed() {
        assert_eq!(program_name("C:/tools/rust-analyzer.exe"), "rust-analyzer");
        assert_eq!(program_name("/usr/local/bin/gopls"), "gopls");
        assert_eq!(program_name("rust-analyzer"), "rust-analyzer");
        assert_eq!(program_name(""), "");
    }

    #[test]
    fn a_refusal_is_told_apart_from_a_real_problem() {
        assert!(is_in_use(&LibraryError::Trash {
            path: PathBuf::from("/x"),
            message: "Some operations were aborted".to_owned(),
        }));
        assert!(is_in_use(&LibraryError::Io {
            path: PathBuf::from("/x"),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        }));
        assert!(!is_in_use(&LibraryError::Io {
            path: PathBuf::from("/x"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        }));
        assert!(!is_in_use(&LibraryError::InvalidName("no".into())));
    }

    /// The `trash` crate says "Some operations were aborted", which is not
    /// something anyone can act on. None of that may reach the window.
    #[test]
    fn the_message_says_what_to_do_and_never_quotes_the_filesystem() {
        for message in [
            in_use_error("My Playground", None).message,
            in_use_error("My Playground", Some("rust-analyzer")).message,
            renaming_error("My Playground", None).message,
            renaming_error("My Playground", Some("rust-analyzer")).message,
        ] {
            assert!(message.contains("My Playground"), "{message}");
            assert!(message.contains("try again"), "{message}");
            assert!(!message.contains("trash operation"), "{message}");
            assert!(!message.contains("aborted"), "{message}");
            assert!(!message.contains("Unknown"), "{message}");
        }
        assert!(
            in_use_error("P", Some("rust-analyzer"))
                .message
                .contains("rust-analyzer"),
            "it should name what was stopped"
        );
    }
}
