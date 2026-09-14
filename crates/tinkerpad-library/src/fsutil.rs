//! Small filesystem helpers.

use std::io;
use std::path::{Path, PathBuf};

/// Suffix of temporary files written during an atomic write.
pub const TMP_SUFFIX: &str = ".tinkerpad-tmp";

/// Write `bytes` to `path` so readers see either the old or the new contents,
/// never a partial file: write a hidden sibling, then rename it over `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    std::fs::write(&tmp, bytes)?;
    // `rename` replaces an existing file on every platform std supports.
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

fn tmp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}{TMP_SUFFIX}"))
}

pub fn is_tmp(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(TMP_SUFFIX))
}
