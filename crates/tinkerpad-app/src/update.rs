//! Updates, from the GitHub releases this project publishes (work item 04-06).
//!
//! The rules here are about not being in the way. TinkerPad checks once, a few
//! seconds after the window is up and never before it; says so in a toast the
//! user can ignore; downloads only when they ask; and restarts only when they
//! say. An editor that interrupts you to talk about itself is an editor you
//! close.
//!
//! A check that fails — no network, a rate limit, a release without a manifest
//! — is a debug line and nothing else. Nobody asked.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;
use tinkerpad_core::{CommandError, CoreEvent, ErrorCode, Level, Scope};
use tracing::{debug, info, warn};

use crate::AppState;

/// How long after the window appears the check happens. Long enough that it
/// never competes with the first paint, short enough to be this session's news.
const AFTER_LAUNCH: Duration = Duration::from_secs(5);

/// What the window knows about an update.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Available {
    pub version: String,
    pub current: String,
    /// What the release notes say, when the manifest carries them.
    pub notes: Option<String>,
}

/// Set once an update has been found, so the window can ask about it later
/// (the palette's "Install update") without checking again.
#[derive(Debug, Default)]
pub struct Found {
    pub update: std::sync::Mutex<Option<Available>>,
    /// Stops two installs at once if the user is quick with the toast.
    installing: AtomicBool,
}

/// Look for an update, once, a few seconds after launch.
pub fn check_later(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(AFTER_LAUNCH).await;
        match look(&app).await {
            Ok(Some(update)) => {
                info!(version = %update.version, "an update is available");
                let state = app.state::<AppState>();
                *state.updates.update.lock().expect("update lock") = Some(update.clone());
                state.events.publish(
                    Scope::App,
                    CoreEvent::UpdateAvailable {
                        version: update.version,
                        notes: update.notes,
                    },
                );
            }
            Ok(None) => debug!("no update"),
            // Offline, rate-limited, or no manifest yet: not the user's problem.
            Err(error) => debug!(%error, "could not check for updates"),
        }
    });
}

/// Ask the endpoint what the latest release is.
async fn look(app: &AppHandle) -> Result<Option<Available>, tauri_plugin_updater::Error> {
    let current = app.package_info().version.to_string();
    let Some(update) = app.updater()?.check().await? else {
        return Ok(None);
    };
    Ok(Some(Available {
        version: update.version.clone(),
        current,
        notes: update.body.clone(),
    }))
}

/// Download the update and restart into it. Returns once the app is on its
/// way out, so the caller has nothing to do afterwards.
pub async fn install(app: AppHandle) -> Result<(), CommandError> {
    let state = app.state::<AppState>();
    if state.updates.installing.swap(true, Ordering::SeqCst) {
        return Ok(()); // Already going.
    }
    let events = state.events.clone();
    let result = download_and_install(&app, &events).await;
    if let Err(error) = &result {
        state.updates.installing.store(false, Ordering::SeqCst);
        warn!(%error, "the update could not be installed");
        events.publish(
            Scope::App,
            CoreEvent::Toast {
                level: Level::Warning,
                message: format!("The update couldn't be installed: {error}"),
            },
        );
    }
    result.map_err(|error| CommandError::new(ErrorCode::Io, error.to_string()))
}

async fn download_and_install(
    app: &AppHandle,
    events: &Arc<crate::events::EventBus>,
) -> Result<(), tauri_plugin_updater::Error> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(());
    };
    events.publish(
        Scope::App,
        CoreEvent::Toast {
            level: Level::Info,
            message: format!("Downloading TinkerPad {}…", update.version),
        },
    );
    update
        .download_and_install(|_chunk, _total| {}, || {})
        .await?;
    info!(version = %update.version, "update installed; restarting");
    app.restart();
}
