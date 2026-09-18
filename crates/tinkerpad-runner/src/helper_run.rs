//! Running a playground on a warm helper (work item 03-03).
//!
//! The point of this module is that there is nothing to see. A helper run
//! produces the same [`RunEvent`] stream as a per-run command, through the same
//! [`OutputDecoder`](crate::OutputDecoder) and the same dump parser, so
//! everything above it — the output pane, annotations, cancel, the run slot per
//! playground — is written once and works for both.
//!
//! What is different is what happens before the first event: the helper may
//! have to be built, and it may be busy with another playground's run. Both are
//! waits the user should be told about rather than left in, so [`Waiting`]
//! reports them as they happen.

use std::sync::Arc;

use tinkerpad_plugin::helper::{HelperClient, HelperEvent, RunRequest};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::helpers::{HelperConfig, HelperHandle, HelperPool, Unavailable, Use};
use crate::run::{Cancel, RunEvent, RunHandle};

/// How many events may be queued towards the reader before the driver waits.
const QUEUE: usize = 256;

/// Something the run is waiting for before it can start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Waiting {
    /// The helper is being built (`[helper.build]`), and this is a line of the
    /// build's output. The first is the "Building the … helper" header.
    Building(String),
    /// Another playground's run has the helper. It is one run at a time, so
    /// this one goes when that one ends.
    Busy,
}

/// Start a run on the plugin's helper.
///
/// Waits for the helper to be built, started, and free, reporting each wait
/// through `waiting`, and then returns a handle over the run exactly as
/// [`run`](crate::run) does for a process.
///
/// The error is the caller's cue to fall back to the plugin's `[run]` command:
/// every [`Unavailable`] means "there is no helper to run this on", and none of
/// them means "the program failed".
pub async fn start(
    pool: &HelperPool,
    config: &HelperConfig,
    request: RunRequest,
    mut waiting: impl FnMut(Waiting) + Send + 'static,
) -> Result<RunHandle, Unavailable> {
    let handle = pool.handle(config);
    // Held from here so the helper can't idle out between being started and
    // being given the run.
    let hold = handle.using();

    let client = ask(&handle, &mut waiting).await?;

    // One run at a time per helper: another playground's run holds this until
    // it is done (`docs/PLUGINS.md` §5).
    if handle.is_running() {
        waiting(Waiting::Busy);
    }
    let slot = handle.take_run_slot().await;

    let run = client.run(request).await.map_err(|e| Unavailable::Start {
        plugin: handle.plugin().to_owned(),
        reason: e.to_string(),
    })?;

    let pid = client.pid();
    let (events, rx) = mpsc::channel(QUEUE);
    let (cancel_tx, cancel_rx) = mpsc::channel(1);
    // The one thing in the stream that says a helper ran this.
    let _ = events.send(RunEvent::Started { pid, warm: true }).await;
    tokio::spawn(drive(run, events, cancel_rx, hold, slot));
    Ok(RunHandle::over(pid, rx, Cancel::Helper(cancel_tx)))
}

/// Ask the pool for the helper, showing the build while it happens.
///
/// Subscribing before asking is what makes a build watchable: the pool sends
/// the header as its first line, and nothing is missed between the two.
async fn ask(
    handle: &Arc<HelperHandle>,
    waiting: &mut impl FnMut(Waiting),
) -> Result<Arc<HelperClient>, Unavailable> {
    let mut lines = handle.build_output();
    let asking = handle.client();
    tokio::pin!(asking);

    loop {
        tokio::select! {
            line = lines.recv() => {
                // A line, or a subscriber that fell behind: either way the
                // answer below is what the run is waiting for.
                if let Ok(line) = line {
                    waiting(Waiting::Building(line));
                }
            }
            answer = &mut asking => return answer,
        }
    }
}

/// Turn the helper's events into a run's events until the run ends.
///
/// Holds the use guard and the run slot for exactly as long as the run lasts:
/// dropping them at the end is what lets the next playground in and starts the
/// helper's idle clock.
async fn drive(
    mut run: tinkerpad_plugin::helper::HelperRun,
    events: mpsc::Sender<RunEvent>,
    mut cancel: mpsc::Receiver<()>,
    _hold: Use,
    _slot: tokio::sync::OwnedMutexGuard<()>,
) {
    loop {
        let event = tokio::select! {
            // A cancel doesn't end the run here: the helper reports the
            // ending, and that is what closes the stream (03-01).
            Some(()) = cancel.recv() => {
                run.cancel().await;
                continue;
            }
            event = run.recv() => event,
        };
        let Some(event) = event else { break };

        let (event, last) = match event {
            HelperEvent::Stdout(bytes) => (RunEvent::Stdout(bytes), false),
            HelperEvent::Stderr(bytes) => (RunEvent::Stderr(bytes), false),
            HelperEvent::Exited { code, duration } => (RunEvent::Exited { code, duration }, true),
            HelperEvent::Cancelled { duration } => (RunEvent::Cancelled { duration }, true),
            HelperEvent::Failed {
                message,
                diagnostics,
            } => {
                warn!(%message, "the helper could not run it");
                (
                    RunEvent::Failed {
                        message,
                        diagnostics,
                    },
                    true,
                )
            }
        };
        if events.send(event).await.is_err() {
            debug!("the run's reader has gone");
            break;
        }
        if last {
            break;
        }
    }
}
