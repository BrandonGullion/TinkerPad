//! Reading a file that isn't a playground (work item 02-09).
//!
//! Go-to-definition lands in other people's code: a dependency's source in
//! `~/.cargo/registry`, a `.d.ts` in `node_modules`, a generated file. The UI
//! shows those in a read-only "peek" tab, and this is where the bytes come
//! from.
//!
//! Nothing here writes, and nothing here is playground-shaped: a peek is a
//! path, some text, and a note when the file was too big to show. The debugger
//! will open stack frames the same way (`docs/DEBUGGING.md` D15), which is why
//! this is a general command rather than a part of go-to-definition.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tinkerpad_core::{CommandError, ErrorCode};

/// The most a peek tab will load. Decompiled assemblies and bundled JavaScript
/// run to tens of megabytes, and an editor that freezes is worse than a note
/// saying the file is too big.
pub const MAX_BYTES: u64 = 4 * 1024 * 1024;

/// A file opened for reading only.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Peek {
    /// The path as given, absolute.
    pub path: String,
    /// What to show in the tab: the file name, and enough of its folder to
    /// tell two `lib.rs`es apart.
    pub title: String,
    /// The file's extension, lowercased and without the dot, for the editor to
    /// pick a highlighter with.
    pub extension: String,
    pub text: String,
    /// Bytes on disk. Present whether or not the file was read.
    pub bytes: u64,
    /// True when the file was too large to load; `text` is then empty.
    pub too_large: bool,
}

/// Read a file for a peek tab.
pub fn read(path: &str) -> Result<Peek, CommandError> {
    let path = PathBuf::from(path);
    let io = |what: &str, e: std::io::Error| {
        CommandError::new(ErrorCode::Io, format!("{what} {}: {e}", path.display()))
    };

    let meta = std::fs::metadata(&path).map_err(|e| io("could not read", e))?;
    if !meta.is_file() {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            format!("{} is not a file", path.display()),
        ));
    }

    let bytes = meta.len();
    let too_large = bytes > MAX_BYTES;
    // Lossy: a peek is for reading, and a stray byte in someone else's
    // generated file shouldn't be the reason it can't be shown at all.
    let text = if too_large {
        String::new()
    } else {
        let raw = std::fs::read(&path).map_err(|e| io("could not read", e))?;
        String::from_utf8_lossy(&raw).into_owned()
    };

    Ok(Peek {
        title: title(&path),
        extension: path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default(),
        path: path.display().to_string(),
        text,
        bytes,
        too_large,
    })
}

/// A short name for the tab: the file, and its folder when that is what tells
/// it apart from the last one you opened.
fn title(path: &Path) -> String {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    match path.parent().and_then(Path::file_name) {
        Some(parent) => format!("{}/{file}", parent.to_string_lossy()),
        None => file,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("tinkerpad-peek-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn reads_a_file_with_its_name_and_extension() {
        let file = scratch("lib.rs");
        std::fs::write(&file, "pub fn one() -> u32 { 1 }\n").unwrap();

        let peek = read(&file.display().to_string()).unwrap();
        assert!(peek.text.contains("pub fn one"));
        assert_eq!(peek.extension, "rs");
        assert!(peek.title.ends_with("lib.rs"), "{}", peek.title);
        assert!(
            peek.title.contains('/'),
            "the folder is part of the title: {}",
            peek.title,
        );
        assert!(!peek.too_large);
        assert_eq!(peek.bytes, 26);
    }

    #[test]
    fn a_file_too_large_to_show_says_so_instead_of_loading() {
        let file = scratch("huge.js");
        let big = vec![b'x'; (MAX_BYTES + 1) as usize];
        std::fs::write(&file, &big).unwrap();

        let peek = read(&file.display().to_string()).unwrap();
        assert!(peek.too_large);
        assert!(peek.text.is_empty(), "nothing was loaded");
        assert_eq!(peek.bytes, MAX_BYTES + 1);
    }

    #[test]
    fn bytes_that_are_not_text_are_shown_anyway() {
        let file = scratch("odd.txt");
        std::fs::write(&file, [0xff, 0xfe, b'h', b'i']).unwrap();
        let peek = read(&file.display().to_string()).unwrap();
        assert!(peek.text.ends_with("hi"), "{:?}", peek.text);
    }

    #[test]
    fn a_missing_file_and_a_folder_are_both_errors() {
        let missing = scratch("nothing-here.rs");
        let _ = std::fs::remove_file(&missing);
        assert!(read(&missing.display().to_string()).is_err());

        let dir = scratch("a-folder");
        std::fs::create_dir_all(&dir).unwrap();
        let error = read(&dir.display().to_string()).unwrap_err();
        assert!(error.message.contains("not a file"), "{}", error.message);
    }
}
