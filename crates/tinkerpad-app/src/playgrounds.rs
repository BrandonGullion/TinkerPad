//! Opening a playground for the editor.

use serde::Serialize;
use tinkerpad_core::{CommandError, Playground, PlaygroundId};
use tinkerpad_plugin::scaffold::{self, Locked};

use crate::AppState;

/// Everything the editor needs to show a playground.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenedPlayground {
    pub playground: Playground,
    /// The main file, exactly as on disk.
    pub text: String,
    /// The kind's prelude and postlude when they are locked in `text`. The
    /// editor locks the first `prelude.length` and last `postlude.length`
    /// characters; it measures them itself, since JavaScript string offsets
    /// aren't bytes.
    pub prelude: Option<String>,
    pub postlude: Option<String>,
    pub lock: LockState,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum LockState {
    /// The kind has no locked lines.
    None,
    /// `prelude` and `postlude` are locked.
    Locked,
    /// The kind has locked lines, but the file no longer starts and ends with
    /// them (edited outside the app), so nothing is locked.
    Mismatch,
    /// The playground's plugin or kind isn't loaded, so nothing is locked.
    UnknownKind,
}

pub fn open(state: &AppState, id: &PlaygroundId) -> Result<OpenedPlayground, CommandError> {
    let text = state
        .library
        .read_source(id)
        .map_err(crate::library::to_command_error)?;
    let playground = state.library.get(id).ok_or_else(|| {
        CommandError::new(
            tinkerpad_core::ErrorCode::NotFound,
            format!("no playground {id}"),
        )
    })?;

    let registry = state.plugins.read().expect("plugins lock");
    let kind = registry
        .get(playground.language.as_str())
        .and_then(|p| p.kind(playground.kind.as_str()).cloned());

    let (lock, prelude, postlude) = match &kind {
        None => (LockState::UnknownKind, None, None),
        Some(kind) => match scaffold::locked(&text, kind) {
            Locked::None => (LockState::None, None, None),
            Locked::Mismatch => (LockState::Mismatch, None, None),
            Locked::Regions { .. } => (
                LockState::Locked,
                kind.prelude.clone().filter(|s| !s.is_empty()),
                kind.postlude.clone().filter(|s| !s.is_empty()),
            ),
        },
    };

    Ok(OpenedPlayground {
        playground,
        text,
        prelude,
        postlude,
        lock,
    })
}
