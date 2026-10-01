//! Installing, updating and removing plugins (work item 05-02).
//!
//! A plugin is a folder under `<app-data>/plugins/<id>/`, so installing one is
//! copying a folder — with the checks that make that safe to do, and the
//! notice that makes it honest.
//!
//! Three rules shape it.
//!
//! **The manifest loads before anything is copied.** `tinkerpad plugin check`
//! already says whether a folder is a plugin (05-01a); this runs the same
//! check and refuses a folder that isn't one. A missing toolchain is not a
//! refusal: the user may be installing the plugin precisely so they can go and
//! install Go afterwards.
//!
//! **The user is told what they are agreeing to, every time.** A plugin runs
//! with their privileges, like an editor extension. That sentence appears on
//! every install, not once ever.
//!
//! **Removing is reversible.** The folder goes to the recycle bin, like a
//! playground does (04-09), not to `remove_dir_all`.

use std::path::{Path, PathBuf};

use tinkerpad_plugin::{Os, Plugin};

/// Written beside an installed plugin, recording where it came from, so
/// `plugin update` knows what to clone again.
pub const SOURCE_FILE: &str = ".tinkerpad-source";

/// What every install says, before it does anything.
pub const TRUST_NOTICE: &str = "\
A plugin is a program. It runs with your privileges, like an editor extension:
it can read and write your files and start other programs. Install plugins you
would run by hand.";

/// Where a plugin is being installed from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A folder on this machine.
    Path(PathBuf),
    /// Something to `git clone`.
    Git(String),
}

impl Source {
    /// What the user typed, read as one or the other.
    ///
    /// Anything that looks like a URL or an `scp`-style git address is a
    /// clone; everything else is a path, which is what a contributor working
    /// on a plugin in a folder wants.
    pub fn read(text: &str) -> Source {
        let looks_remote = text.starts_with("http://")
            || text.starts_with("https://")
            || text.starts_with("git://")
            || text.starts_with("ssh://")
            || (text.contains('@') && text.contains(':') && !Path::new(text).exists());
        if looks_remote {
            Source::Git(text.to_owned())
        } else {
            Source::Path(PathBuf::from(text))
        }
    }

    /// How it is shown in a list.
    pub fn describe(&self) -> String {
        match self {
            Source::Path(path) => path.display().to_string(),
            Source::Git(url) => url.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0} isn't a folder")]
    NotAFolder(String),
    #[error("{0} isn't a plugin:\n{1}")]
    NotAPlugin(String, String),
    #[error("git isn't installed, and it is what clones a plugin from a URL")]
    NoGit,
    #[error("could not clone {url}: {message}")]
    Clone { url: String, message: String },
    #[error("no plugin called `{0}` is installed")]
    NotInstalled(String),
    #[error("`{0}` is bundled with TinkerPad; it can't be removed")]
    Bundled(String),
    #[error("`{0}` wasn't installed from a URL, so there is nothing to update from")]
    NoSource(String),
    #[error("{0}")]
    Io(String),
}

/// What an install did.
#[derive(Debug)]
pub struct Installed {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub dir: PathBuf,
    /// Whether a plugin of this id was already there.
    pub replaced: bool,
    /// What the toolchain check said, if it was run.
    pub toolchain: Option<String>,
}

/// Install a plugin from a folder or a URL into `plugins_dir`.
///
/// `say` is given progress as it happens — cloning is slow enough to deserve
/// saying so.
pub fn install(
    source: &Source,
    plugins_dir: &Path,
    say: &mut dyn FnMut(&str),
) -> Result<Installed, Error> {
    // A clone lands in a folder of its own, which is thrown away whatever
    // happens next: a half-cloned repository is not an install.
    let temp = match source {
        Source::Git(url) => Some(clone(url, say)?),
        Source::Path(_) => None,
    };
    let from = match (source, &temp) {
        (Source::Path(path), _) => path.clone(),
        (Source::Git(_), Some(temp)) => temp.path.clone(),
        (Source::Git(_), None) => unreachable!("a clone always has a folder"),
    };

    if !from.is_dir() {
        return Err(Error::NotAFolder(from.display().to_string()));
    }

    // The same loader the app uses, so a plugin that installs is a plugin that
    // works (05-01a's rule, and the reason this isn't a copy command).
    let plugin = Plugin::load_for(&from, Os::current())
        .map_err(|e| Error::NotAPlugin(from.display().to_string(), e.to_string()))?;
    for warning in &plugin.warnings {
        say(&format!("warning: {warning}"));
    }

    let id = plugin.id.to_string();
    let target = plugins_dir.join(&id);
    let replaced = target.exists();
    if replaced {
        say(&format!("Replacing the `{id}` that is already installed."));
        // To the recycle bin rather than deleted: an install that goes wrong
        // should not take the old plugin with it.
        if let Err(message) = tinkerpad_library::platform::move_to_trash(&target) {
            // Falling back is fine here — what is being removed is a copy of
            // something that is about to be written again.
            tracing::debug!(%message, "could not trash the old plugin folder");
            std::fs::remove_dir_all(&target).map_err(|e| Error::Io(e.to_string()))?;
        }
    }
    std::fs::create_dir_all(plugins_dir).map_err(|e| Error::Io(e.to_string()))?;
    copy_dir(&from, &target).map_err(|e| Error::Io(e.to_string()))?;

    // Where it came from, so `plugin update` has something to go on.
    if let Source::Git(url) = source
        && let Err(error) = std::fs::write(target.join(SOURCE_FILE), format!("{url}\n"))
    {
        tracing::warn!(%error, "could not record where the plugin came from");
    }

    Ok(Installed {
        id,
        name: plugin.name.clone(),
        version: plugin.version.clone(),
        dir: target,
        replaced,
        toolchain: None,
    })
}

/// Clone a repository into a temporary folder that is removed afterwards.
fn clone(url: &str, say: &mut dyn FnMut(&str)) -> Result<TempDir, Error> {
    let git =
        tinkerpad_plugin::env::find_program("git", &std::env::var_os("PATH").unwrap_or_default())
            .ok_or(Error::NoGit)?;

    let temp = TempDir::new()?;
    say(&format!("Cloning {url}…"));
    // `--depth 1`: a plugin is a working folder, not a history.
    let output = std::process::Command::new(git)
        .arg("clone")
        .arg("--depth")
        .arg("1")
        .arg(url)
        .arg(&temp.path)
        .output()
        .map_err(|e| Error::Clone {
            url: url.to_owned(),
            message: e.to_string(),
        })?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr)
            .lines()
            .rfind(|line| !line.trim().is_empty())
            .unwrap_or("git said nothing")
            .to_owned();
        return Err(Error::Clone {
            url: url.to_owned(),
            message,
        });
    }
    Ok(temp)
}

/// Re-clone a plugin that was installed from a URL.
pub fn update(id: &str, plugins_dir: &Path, say: &mut dyn FnMut(&str)) -> Result<Installed, Error> {
    let dir = plugins_dir.join(id);
    if !dir.is_dir() {
        return Err(Error::NotInstalled(id.to_owned()));
    }
    let url = std::fs::read_to_string(dir.join(SOURCE_FILE))
        .map_err(|_| Error::NoSource(id.to_owned()))?
        .trim()
        .to_owned();
    if url.is_empty() {
        return Err(Error::NoSource(id.to_owned()));
    }
    install(&Source::Git(url), plugins_dir, say)
}

/// Move an installed plugin to the recycle bin.
pub fn remove(id: &str, plugins_dir: &Path) -> Result<PathBuf, Error> {
    let dir = plugins_dir.join(id);
    if !dir.is_dir() {
        return Err(Error::NotInstalled(id.to_owned()));
    }
    tinkerpad_library::platform::move_to_trash(&dir).map_err(Error::Io)?;
    Ok(dir)
}

/// Copy a folder, skipping what a plugin never needs and a repository always
/// has.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        // `.git` is a repository's own business and can be larger than the
        // plugin. Build output is remade on first use (PLUGINS.md §8).
        if matches!(
            name_text.as_ref(),
            ".git" | "node_modules" | "target" | "bin" | "obj"
        ) {
            continue;
        }
        let source = entry.path();
        let target = to.join(&name);
        if entry.file_type()?.is_dir() {
            copy_dir(&source, &target)?;
        } else {
            std::fs::copy(&source, &target)?;
        }
    }
    Ok(())
}

/// A folder that removes itself.
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Result<TempDir, Error> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("tinkerpad-install-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        // git wants to create the folder itself.
        Ok(TempDir { path })
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_a_clone_and_a_folder_is_a_copy() {
        assert_eq!(
            Source::read("https://github.com/someone/tinkerpad-elm"),
            Source::Git("https://github.com/someone/tinkerpad-elm".to_owned())
        );
        assert_eq!(
            Source::read("git@github.com:someone/plugin.git"),
            Source::Git("git@github.com:someone/plugin.git".to_owned())
        );
        assert_eq!(
            Source::read("./plugins/python"),
            Source::Path(PathBuf::from("./plugins/python"))
        );
        assert_eq!(
            Source::read("C:/code/my-plugin"),
            Source::Path(PathBuf::from("C:/code/my-plugin"))
        );
    }

    #[test]
    fn the_trust_notice_says_what_it_has_to() {
        assert!(TRUST_NOTICE.contains("your privileges"));
        assert!(TRUST_NOTICE.contains("start other programs"));
    }

    #[test]
    fn copying_leaves_out_what_a_plugin_does_not_need() {
        let root = std::env::temp_dir().join(format!("tinkerpad-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let from = root.join("from");
        std::fs::create_dir_all(from.join(".git")).unwrap();
        std::fs::create_dir_all(from.join("helper/bin")).unwrap();
        std::fs::create_dir_all(from.join("templates")).unwrap();
        std::fs::write(from.join(".git/HEAD"), "ref").unwrap();
        std::fs::write(from.join("helper/bin/helper.exe"), "binary").unwrap();
        std::fs::write(from.join("templates/main.x"), "x").unwrap();
        std::fs::write(from.join("plugin.toml"), "api = 1").unwrap();

        let to = root.join("to");
        copy_dir(&from, &to).unwrap();

        assert!(to.join("plugin.toml").is_file());
        assert!(to.join("templates/main.x").is_file());
        assert!(!to.join(".git").exists(), "a repository is not a plugin");
        assert!(
            !to.join("helper/bin").exists(),
            "build output is remade on first use"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
