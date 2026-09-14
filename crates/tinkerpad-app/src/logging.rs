//! Logging: a daily rolling file in the app's log folder, plus pretty stderr in
//! debug builds.
//!
//! Level comes from `TINKERPAD_LOG` (for example `debug`, or a full filter
//! such as `info,tinkerpad_process=debug`), defaulting to `info`.
//!
//! Rule: never log the contents of a user's code at `info` or above. At
//! `trace` it's fine; `trace` is off by default.

use std::path::Path;
use std::sync::OnceLock;

use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer, fmt};

/// Days of logs to keep.
const KEEP_FILES: usize = 7;

/// Keeps the background log writer alive for the life of the process.
static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Install the global subscriber and the panic hook. Call once, early.
pub fn init(log_dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(log_dir)?;

    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("tinkerpad")
        .filename_suffix("log")
        .max_log_files(KEEP_FILES)
        .build(log_dir)
        .map_err(std::io::Error::other)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    let _ = GUARD.set(guard);

    let filter = std::env::var("TINKERPAD_LOG").unwrap_or_else(|_| "info".to_owned());
    let filter = EnvFilter::try_new(&filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let file_layer = fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_target(true);

    // Pretty stderr only in debug builds; release builds on Windows have no
    // console anyway.
    let stderr_layer = cfg!(debug_assertions).then(|| {
        fmt::layer()
            .with_writer(std::io::stderr)
            .with_target(false)
            .compact()
            .boxed()
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(stderr_layer)
        .try_init()
        .map_err(std::io::Error::other)?;

    install_panic_hook();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        platform = tinkerpad_process::platform::name(),
        log_dir = %log_dir.display(),
        "TinkerPad starting"
    );
    Ok(())
}

/// Log panics with a backtrace before the default hook runs (and the app dies).
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_default();
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic payload>".to_owned());
        tracing::error!(%location, %message, %backtrace, "panic");
        previous(info);
    }));
}
