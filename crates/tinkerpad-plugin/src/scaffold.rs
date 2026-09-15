//! What a new playground's files contain, and which of its lines are locked.
//!
//! A kind's main file is `prelude + template + postlude`. The template gets
//! `{{name}}` and `{{date}}` filled in; the prelude and postlude are written
//! exactly as the manifest has them, because locked lines are found later by
//! matching them against the file (docs/PLUGINS.md §3).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::plugin::{Kind, Plugin};

/// Fill `{{name}}` and `{{date}}`. Any other `{{…}}` is left as written.
pub fn render(template: &str, name: &str, date: &str) -> String {
    template.replace("{{name}}", name).replace("{{date}}", date)
}

/// Today's date in UTC as `YYYY-MM-DD`, for `{{date}}`.
pub fn today_utc() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() / 86_400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}")
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// algorithm), so no date library is needed.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[derive(Debug, thiserror::Error)]
pub enum ScaffoldError {
    #[error("could not read template {}: {source}", path.display())]
    Template {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

fn read_template(path: &std::path::Path) -> Result<String, ScaffoldError> {
    // Read at create time, never cached, so editing a template takes effect
    // on the next playground without restarting.
    std::fs::read_to_string(path).map_err(|source| ScaffoldError::Template {
        path: path.to_path_buf(),
        source,
    })
}

/// The main file for a new playground of `kind`.
pub fn main_file(kind: &Kind, name: &str, date: &str) -> Result<String, ScaffoldError> {
    let body = render(&read_template(&kind.template)?, name, date);
    Ok(format!(
        "{}{body}{}",
        kind.prelude.as_deref().unwrap_or(""),
        kind.postlude.as_deref().unwrap_or("")
    ))
}

/// Every file a new playground of `kind` starts with, relative to its folder:
/// the main file first, then `[scaffold].files`. A scaffold entry at the main
/// file's path is ignored; the kind's template wins.
pub fn playground_files(
    plugin: &Plugin,
    kind: &Kind,
    name: &str,
    date: &str,
) -> Result<Vec<(PathBuf, Vec<u8>)>, ScaffoldError> {
    let mut files = vec![(
        plugin.main_path.clone(),
        main_file(kind, name, date)?.into_bytes(),
    )];
    for entry in &plugin.scaffold {
        if entry.path == plugin.main_path {
            continue;
        }
        let text = render(&read_template(&entry.template)?, name, date);
        files.push((entry.path.clone(), text.into_bytes()));
    }
    Ok(files)
}

/// Which parts of a main file are locked, by byte length from each end.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Locked {
    /// The kind has no prelude or postlude.
    None,
    /// The file starts with the prelude and ends with the postlude; the first
    /// `head` and last `tail` bytes are locked.
    Regions { head: usize, tail: usize },
    /// The kind has a prelude or postlude but the file no longer matches it,
    /// for example after an edit outside the app. Nothing is locked.
    Mismatch,
}

/// Work out the locked regions of `text` for `kind`.
pub fn locked(text: &str, kind: &Kind) -> Locked {
    let prelude = kind.prelude.as_deref().unwrap_or("");
    let postlude = kind.postlude.as_deref().unwrap_or("");
    if prelude.is_empty() && postlude.is_empty() {
        return Locked::None;
    }
    let fits = text.len() >= prelude.len() + postlude.len();
    if fits && text.starts_with(prelude) && text.ends_with(postlude) {
        Locked::Regions {
            head: prelude.len(),
            tail: postlude.len(),
        }
    } else {
        Locked::Mismatch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(prelude: Option<&str>, postlude: Option<&str>) -> Kind {
        Kind {
            id: "k".into(),
            name: "K".into(),
            template: PathBuf::new(),
            default: true,
            prelude: prelude.map(Into::into),
            postlude: postlude.map(Into::into),
        }
    }

    #[test]
    fn renders_only_name_and_date() {
        assert_eq!(
            render(
                "// {{name}} ({{date}})\n{{other}} {{ name }}",
                "Fizz",
                "2026-09-14"
            ),
            "// Fizz (2026-09-14)\n{{other}} {{ name }}"
        );
        assert_eq!(render("{{name}}{{name}}", "a", ""), "aa");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(19_723), (2024, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29), "leap day");
        assert_eq!(civil_from_days(20_710), (2026, 9, 14));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        let today = today_utc();
        assert_eq!(today.len(), 10);
        assert!(today.starts_with("20"), "{today}");
    }

    #[test]
    fn locked_regions() {
        let k = kind(Some("dump(\n"), Some("\n);"));
        assert_eq!(
            locked("dump(\n1 + 1\n);", &k),
            Locked::Regions { head: 6, tail: 3 }
        );
        assert_eq!(
            locked("dump(\n\n);", &k),
            Locked::Regions { head: 6, tail: 3 }
        );
        assert_eq!(locked("1 + 1\n);", &k), Locked::Mismatch, "prelude removed");
        assert_eq!(
            locked("dump(\n1 + 1", &k),
            Locked::Mismatch,
            "postlude removed"
        );
        assert_eq!(
            locked("dump(\r\n1\r\n);", &k),
            Locked::Mismatch,
            "line endings changed"
        );
        assert_eq!(locked("dump(\n);", &k), Locked::Mismatch, "overlapping");

        assert_eq!(locked("anything", &kind(None, None)), Locked::None);
        assert_eq!(
            locked("fn main() {\nbody", &kind(Some("fn main() {\n"), None)),
            Locked::Regions { head: 12, tail: 0 }
        );
    }
}
