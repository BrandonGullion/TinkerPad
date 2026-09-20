//! `playground.toml`: the metadata that makes a folder a playground.
//!
//! ```toml
//! id = "01J8Z3N6Q4X9W2V7T5R1M0K8HD"   # ULID; stable across renames
//! name = "Fizz Buzz"                   # display name
//! language = "csharp"
//! kind = "statements"
//! main = "main.cs"                     # relative to the folder
//! created = 2026-09-14T20:11:00Z
//! tags = ["experiments"]
//! pinned = true                        # optional; top of the library panel
//!
//! [plugin.rust]                        # optional, owned by that plugin
//! plugin_dir = "C:/Program Files/TinkerPad/plugins/rust"
//! ```

use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tinkerpad_core::{KindId, LanguageId, Playground, PlaygroundId};

pub const FILE_NAME: &str = "playground.toml";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    id: String,
    #[serde(default)]
    name: Option<String>,
    language: String,
    kind: String,
    main: String,
    #[serde(default)]
    created: Option<toml::value::Datetime>,
    #[serde(default)]
    tags: Vec<String>,
    /// Kept at the top of the library panel (work item 04-01). Absent when
    /// false, so an unpinned playground's file looks as it always did.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pinned: bool,
    /// Plugin-owned values, one table per plugin id: `[plugin.rust]`. The
    /// library keeps them but never interprets them.
    #[serde(default, skip_serializing_if = "toml::Table::is_empty")]
    plugin: toml::Table,
}

/// Read `dir/playground.toml` into a [`Playground`].
pub fn read(dir: &Path) -> Result<Playground, String> {
    let file = dir.join(FILE_NAME);
    let text =
        std::fs::read_to_string(&file).map_err(|e| format!("can't read {FILE_NAME}: {e}"))?;
    let meta: Meta = toml::from_str(&text).map_err(|e| format!("{FILE_NAME}: {}", e.message()))?;

    if ulid::Ulid::from_string(&meta.id).is_err() {
        return Err(format!(
            "{FILE_NAME}: `id` = \"{}\" is not a valid ULID",
            meta.id
        ));
    }
    let main = safe_relative(&meta.main).ok_or_else(|| {
        format!(
            "{FILE_NAME}: `main` = \"{}\" must be a relative path inside the folder",
            meta.main
        )
    })?;
    if meta.language.trim().is_empty() || meta.kind.trim().is_empty() {
        return Err(format!(
            "{FILE_NAME}: `language` and `kind` must not be empty"
        ));
    }

    let name = meta
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            dir.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let created_ms = meta
        .created
        .and_then(|d| humantime::parse_rfc3339_weak(&d.to_string()).ok())
        .map_or(0, to_ms);

    Ok(Playground {
        id: PlaygroundId::new(meta.id),
        name,
        language: LanguageId::new(meta.language),
        kind: KindId::new(meta.kind),
        dir: dir.to_path_buf(),
        main_file: dir.join(main),
        created_ms,
        tags: meta.tags,
        pinned: meta.pinned,
    })
}

/// Write `playground.toml` for `p` atomically, keeping any `[plugin.*]`
/// tables already in the file.
pub fn write(p: &Playground) -> std::io::Result<()> {
    write_with(p, plugin_tables(&p.dir))
}

/// The `[plugin.*]` tables of `dir/playground.toml`, or none if it can't be
/// read.
pub fn plugin_tables(dir: &Path) -> toml::Table {
    std::fs::read_to_string(dir.join(FILE_NAME))
        .ok()
        .and_then(|text| toml::from_str::<Meta>(&text).ok())
        .map(|m| m.plugin)
        .unwrap_or_default()
}

/// Write `playground.toml` for `p` atomically with exactly these `[plugin.*]`
/// tables.
pub fn write_with(p: &Playground, plugin: toml::Table) -> std::io::Result<()> {
    let main = p
        .main_file
        .strip_prefix(&p.dir)
        .unwrap_or(&p.main_file)
        .to_string_lossy()
        .replace('\\', "/");
    let created =
        humantime::format_rfc3339_seconds(UNIX_EPOCH + Duration::from_millis(p.created_ms))
            .to_string()
            .parse::<toml::value::Datetime>()
            .ok();
    let meta = Meta {
        id: p.id.to_string(),
        name: Some(p.name.clone()),
        language: p.language.to_string(),
        kind: p.kind.to_string(),
        main,
        created,
        tags: p.tags.clone(),
        pinned: p.pinned,
        plugin,
    };
    let text = toml::to_string(&meta).map_err(std::io::Error::other)?;
    crate::fsutil::write_atomic(&p.dir.join(FILE_NAME), text.as_bytes())
}

/// A new ULID.
pub fn new_id() -> PlaygroundId {
    PlaygroundId::new(ulid::Ulid::new().to_string())
}

pub fn to_ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `raw` as a relative path that stays inside its folder, or `None`.
pub fn safe_relative(raw: &str) -> Option<PathBuf> {
    let normalised = raw.replace('\\', "/");
    let path = PathBuf::from(&normalised);
    let ok = !normalised.is_empty()
        && !normalised.starts_with('/')
        && normalised.as_bytes().get(1) != Some(&b':')
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
    ok.then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let dir = std::env::temp_dir().join(format!("tinkerpad-meta-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = Playground {
            id: new_id(),
            name: "Fizz Buzz".into(),
            language: LanguageId::new("csharp"),
            kind: KindId::new("statements"),
            dir: dir.clone(),
            main_file: dir.join("src").join("main.rs"),
            created_ms: 1_789_000_000_000,
            tags: vec!["a".into(), "b".into()],
            pinned: true,
        };
        write(&p).unwrap();
        let text = std::fs::read_to_string(dir.join(FILE_NAME)).unwrap();
        assert!(text.contains("main = \"src/main.rs\""), "{text}");
        assert!(
            text.contains("created = 20"),
            "datetime, not a string: {text}"
        );
        let back = read(&dir).unwrap();
        assert_eq!(back, p);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn safe_paths() {
        assert_eq!(safe_relative("main.cs"), Some(PathBuf::from("main.cs")));
        assert_eq!(
            safe_relative("src/main.rs"),
            Some(PathBuf::from("src/main.rs"))
        );
        for bad in ["", "/etc/x", "../x", "a/../../x", "C:/x", "C:\\x"] {
            assert_eq!(safe_relative(bad), None, "{bad}");
        }
    }
}
