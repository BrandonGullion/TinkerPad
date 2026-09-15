//! What a new playground's files contain, and which of its lines are locked.
//!
//! A kind's main file is `prelude + template + postlude`. Templates get the
//! [`ScaffoldVars`] filled in; the prelude and postlude are written exactly as
//! the manifest has them, because locked lines are found later by matching
//! them against the file (docs/PLUGINS.md §3).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::plugin::{Kind, Plugin};

/// The values templates can use.
#[derive(Clone, Copy, Debug)]
pub struct ScaffoldVars<'a> {
    /// `{{name}}`: the display name.
    pub name: &'a str,
    /// `{{date}}`: today, `YYYY-MM-DD` (UTC).
    pub date: &'a str,
    /// `{{folder}}`: the playground's folder name, such as `fizz-buzz-2`.
    pub folder: &'a str,
    /// `{{package}}`: a name package managers accept; see [`package_name`].
    pub package: &'a str,
    /// `{{plugin_dir}}`: the plugin folder, absolute.
    pub plugin_dir: &'a Path,
    /// `{{scratch}}`: the plugin's scratch folder, absolute.
    pub scratch: &'a Path,
}

/// Names of the variables that are absolute paths. A playground whose files
/// use them records their values in `playground.toml`, so they can be
/// repaired if the plugin or scratch folder moves.
pub const PATH_VARS: &[&str] = &["plugin_dir", "scratch"];

/// Fill the `{{…}}` variables of [`ScaffoldVars`] in `template`. Any other
/// `{{…}}` is left as written. Paths are written with `/` separators, which
/// every tool that reads these files accepts on Windows too, and which need no
/// escaping inside TOML or JSON strings.
pub fn render(template: &str, vars: &ScaffoldVars<'_>) -> String {
    render_tracked(template, vars, &mut BTreeMap::new())
}

/// [`render`], adding each path variable used to `used`.
fn render_tracked(
    template: &str,
    vars: &ScaffoldVars<'_>,
    used: &mut BTreeMap<String, String>,
) -> String {
    let plugin_dir = template_path(vars.plugin_dir);
    let scratch = template_path(vars.scratch);
    for (name, value) in PATH_VARS.iter().zip([&plugin_dir, &scratch]) {
        if template.contains(&format!("{{{{{name}}}}}")) {
            used.insert((*name).to_owned(), value.clone());
        }
    }
    template
        .replace("{{name}}", vars.name)
        .replace("{{date}}", vars.date)
        .replace("{{folder}}", vars.folder)
        .replace("{{package}}", vars.package)
        .replace("{{plugin_dir}}", &plugin_dir)
        .replace("{{scratch}}", &scratch)
}

/// An absolute path as templates get it: `/` separators, no `\\?\` prefix.
pub fn template_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    let text = text.strip_prefix(r"\\?\").unwrap_or(&text);
    text.replace('\\', "/")
}

/// A package name for a playground in `folder`: the folder name when it is
/// already lowercase ASCII letters, digits, and hyphens starting with a letter
/// (as most are), otherwise `playground-` plus the lowercased id. Either way
/// it's unique in the library, which matters when playgrounds share a build
/// output folder.
pub fn package_name(folder: &str, id: &str) -> String {
    let valid = folder.starts_with(|c: char| c.is_ascii_lowercase())
        && folder
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    if valid {
        folder.to_owned()
    } else {
        format!("playground-{}", id.to_ascii_lowercase())
    }
}

/// Files for a new playground, and the path variables they used.
#[derive(Clone, Debug, Default)]
pub struct Scaffolded {
    /// Relative to the playground folder; the main file first.
    pub files: Vec<(PathBuf, Vec<u8>)>,
    /// Path variables used by any file, by name, with the value written.
    pub paths: BTreeMap<String, String>,
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
pub fn main_file(kind: &Kind, vars: &ScaffoldVars<'_>) -> Result<String, ScaffoldError> {
    main_file_tracked(kind, vars, &mut BTreeMap::new())
}

fn main_file_tracked(
    kind: &Kind,
    vars: &ScaffoldVars<'_>,
    used: &mut BTreeMap<String, String>,
) -> Result<String, ScaffoldError> {
    let body = render_tracked(&read_template(&kind.template)?, vars, used);
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
    vars: &ScaffoldVars<'_>,
) -> Result<Scaffolded, ScaffoldError> {
    let mut out = Scaffolded::default();
    let main = main_file_tracked(kind, vars, &mut out.paths)?;
    out.files
        .push((plugin.main_path.clone(), main.into_bytes()));
    for entry in &plugin.scaffold {
        if entry.path == plugin.main_path {
            continue;
        }
        let text = render_tracked(&read_template(&entry.template)?, vars, &mut out.paths);
        out.files.push((entry.path.clone(), text.into_bytes()));
    }
    Ok(out)
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

    fn vars<'a>(plugin_dir: &'a Path, scratch: &'a Path) -> ScaffoldVars<'a> {
        ScaffoldVars {
            name: "Fizz",
            date: "2026-09-14",
            folder: "fizz-2",
            package: "fizz-2",
            plugin_dir,
            scratch,
        }
    }

    #[test]
    fn renders_known_variables_only() {
        let (p, s) = (
            Path::new("/opt/tp/plugins/rust"),
            Path::new("/data/scratch/rust"),
        );
        let v = vars(p, s);
        assert_eq!(
            render(
                "// {{name}} ({{date}}) in {{folder}} as {{package}}\n{{other}} {{ name }}",
                &v
            ),
            "// Fizz (2026-09-14) in fizz-2 as fizz-2\n{{other}} {{ name }}"
        );
        assert_eq!(render("{{name}}{{name}}", &v), "FizzFizz");

        let mut used = BTreeMap::new();
        let text = render_tracked("path = \"{{plugin_dir}}/dump\"", &v, &mut used);
        assert_eq!(text, "path = \"/opt/tp/plugins/rust/dump\"");
        assert_eq!(used.keys().collect::<Vec<_>>(), ["plugin_dir"]);
    }

    #[test]
    fn template_paths_use_forward_slashes() {
        assert_eq!(
            template_path(Path::new(r"C:\Users\Ann O'Neil\plugins\rust")),
            "C:/Users/Ann O'Neil/plugins/rust"
        );
        assert_eq!(
            template_path(Path::new(r"\\?\C:\TinkerPad\plugins")),
            "C:/TinkerPad/plugins"
        );
    }

    #[test]
    fn package_names() {
        let id = "01K0ABCDEF0000000000000000";
        assert_eq!(package_name("fizz-buzz-2", id), "fizz-buzz-2");
        for folder in ["2024-notes", "日本", "café", "-x", "Upper"] {
            assert_eq!(
                package_name(folder, id),
                "playground-01k0abcdef0000000000000000",
                "{folder}"
            );
        }
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
