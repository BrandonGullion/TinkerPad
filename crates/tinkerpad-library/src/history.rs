//! `runs.jsonl`: what each run of a playground did, kept beside it.
//!
//! One JSON object per line, oldest first, appended after a run ends:
//!
//! ```jsonl
//! {"run":7,"startedMs":1758400000000,"durationMs":64,"outcome":"exited","code":0,"runner":"helper","reportedMs":58}
//! {"run":8,"startedMs":1758400020000,"durationMs":1102,"outcome":"exited","code":1,"runner":"perRun","error":"CS1002: ; expected","firstOutputMs":980}
//! ```
//!
//! Lines, not a document, because a run appends and nothing else edits: a
//! crash halfway through a write costs the last line and no more. Unparseable
//! lines are skipped rather than failing the read, which also makes a field
//! added later harmless to an older build.
//!
//! The file is capped at [`MAX`] entries. It is not part of the playground —
//! the index ignores it, nothing lists it as a file, and deleting it loses
//! only the history (work item 03-08).

use std::io::Write;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// The file, inside the playground folder.
pub const HISTORY_FILE: &str = "runs.jsonl";

/// How many runs are kept. Enough to answer "how long did that take before?"
/// through a morning's work, small enough that reading it is free.
pub const MAX: usize = 200;

/// How a run ended.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Outcome {
    /// The program ran and finished, with `code`.
    Exited,
    /// The user stopped it.
    Cancelled,
    /// It never ran: a compile error, or a runner that couldn't start.
    Failed,
}

/// What ran it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Runner {
    /// A process started for this run, from the plugin's `[run]`.
    PerRun,
    /// A warm plugin helper (work item 03-02).
    Helper,
}

/// One run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    /// The run's id in the session that recorded it. Nothing outside that
    /// session can use it; while the app is running it is how the window finds
    /// the output it still has in memory for a run.
    #[serde(default)]
    pub run: u64,
    /// When it started, Unix milliseconds.
    pub started_ms: u64,
    /// How long it took, measured in the core: the whole round trip the user
    /// waited for.
    pub duration_ms: u64,
    pub outcome: Outcome,
    /// The exit code, when it exited with one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<i32>,
    pub runner: Runner,
    /// The first thing that went wrong: the first annotation's message, or
    /// what a failed run said.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// How long until the first byte of output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_output_ms: Option<u64>,
    /// How long the runner itself said it took: the process's own time, or a
    /// helper's `duration_ms`. The gap to `duration_ms` is the overhead of
    /// getting there and back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_ms: Option<u64>,
}

/// Add a run to a playground's history, trimming it to [`MAX`] entries.
///
/// Does the reading and writing on the calling thread: call it off the run's
/// hot path, once the run has ended.
pub fn append(dir: &Path, record: &RunRecord) -> std::io::Result<()> {
    let path = dir.join(HISTORY_FILE);
    let mut line = serde_json::to_string(record).map_err(std::io::Error::other)?;
    line.push('\n');

    // Under the cap this is one append; at it, the file is rewritten without
    // its oldest entries. Rewriting a 200-line file is cheaper than keeping an
    // index of where lines start.
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let lines = existing.lines().filter(|l| !l.trim().is_empty()).count();
    if lines < MAX {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        return file.write_all(line.as_bytes());
    }

    let mut kept: String = existing
        .lines()
        .filter(|l| !l.trim().is_empty())
        .skip(lines + 1 - MAX)
        .map(|l| format!("{l}\n"))
        .collect();
    kept.push_str(&line);
    crate::fsutil::write_atomic(&path, kept.as_bytes())
}

/// A playground's runs, oldest first. An unreadable file reads as no history:
/// this is a convenience, and losing it is not worth an error the user has to
/// deal with.
pub fn read(dir: &Path) -> Vec<RunRecord> {
    let Ok(text) = std::fs::read_to_string(dir.join(HISTORY_FILE)) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| match serde_json::from_str(line) {
            Ok(record) => Some(record),
            Err(error) => {
                tracing::debug!(%error, "skipping a line of runs.jsonl");
                None
            }
        })
        .collect()
}

/// A playground's last `n` runs, newest first.
pub fn recent(dir: &Path, n: usize) -> Vec<RunRecord> {
    let mut records = read(dir);
    records.reverse();
    records.truncate(n);
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(started_ms: u64) -> RunRecord {
        RunRecord {
            run: started_ms,
            started_ms,
            duration_ms: 12,
            outcome: Outcome::Exited,
            code: Some(0),
            runner: Runner::Helper,
            error: None,
            first_output_ms: Some(8),
            reported_ms: Some(10),
        }
    }

    fn dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tinkerpad-history-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn runs_come_back_in_the_order_they_happened() {
        let dir = dir("order");
        for n in 0..3 {
            append(&dir, &record(n)).unwrap();
        }
        let read = read(&dir);
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].started_ms, 0);
        assert_eq!(read[2].started_ms, 2);
        assert_eq!(read[0], record(0));

        let recent = recent(&dir, 2);
        assert_eq!(recent[0].started_ms, 2, "newest first");
        assert_eq!(recent[1].started_ms, 1);
    }

    #[test]
    fn the_file_stops_growing_at_the_cap() {
        let dir = dir("cap");
        for n in 0..(MAX as u64 + 20) {
            append(&dir, &record(n)).unwrap();
        }
        let read = read(&dir);
        assert_eq!(read.len(), MAX);
        assert_eq!(read[0].started_ms, 20, "the oldest went first");
        assert_eq!(read[MAX - 1].started_ms, MAX as u64 + 19);
    }

    #[test]
    fn a_line_that_makes_no_sense_costs_only_itself() {
        let dir = dir("garbage");
        append(&dir, &record(1)).unwrap();
        let path = dir.join(HISTORY_FILE);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{\"startedMs\": \n");
        text.push_str("not json at all\n");
        std::fs::write(&path, &text).unwrap();
        append(&dir, &record(2)).unwrap();

        let read = read(&dir);
        assert_eq!(read.len(), 2);
        assert_eq!(read[1].started_ms, 2);
    }

    #[test]
    fn a_field_this_build_does_not_know_is_ignored() {
        let dir = dir("forward");
        let line = "{\"startedMs\":1,\"durationMs\":2,\"outcome\":\"exited\",\"runner\":\"perRun\",\"flavour\":\"new\"}\n";
        std::fs::write(dir.join(HISTORY_FILE), line).unwrap();
        let read = read(&dir);
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].code, None);
        assert_eq!(read[0].runner, Runner::PerRun);
    }

    #[test]
    fn no_history_is_not_an_error() {
        assert!(read(&dir("empty")).is_empty());
    }
}
