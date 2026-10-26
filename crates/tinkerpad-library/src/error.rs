use std::path::{Path, PathBuf};

use tinkerpad_core::PlaygroundId;

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error("no playground with id {0}")]
    NotFound(PlaygroundId),
    /// A folder asked for as a broken playground that the index doesn't have
    /// as one: never a reason to touch it.
    #[error("{} isn't a broken playground in the library", .0.display())]
    NotBroken(PathBuf),
    #[error("{0}")]
    InvalidName(String),
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not move {} to the trash: {message}", path.display())]
    Trash { path: PathBuf, message: String },
    #[error("could not watch the library: {0}")]
    Watch(String),
    /// The files for a new playground couldn't be produced.
    #[error("{0}")]
    Scaffold(String),
}

impl LibraryError {
    pub(crate) fn io(path: &Path, source: std::io::Error) -> Self {
        Self::Io {
            path: path.to_path_buf(),
            source,
        }
    }
}
