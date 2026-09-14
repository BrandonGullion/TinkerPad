//! Is a language's toolchain installed, and is it new enough?
//!
//! A plugin's `[toolchain].probe` entries run in order until one succeeds.
//! Programs are found through a [`Resolver`]: a settings override first, then
//! the discovered `PATH` (see [`crate::env::discover_path`]). The same resolver
//! is meant for every later spawn of a plugin's programs, so a user's override
//! applies everywhere.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tinkerpad_core::ToolchainState;
use tinkerpad_process::{ProcessEvent, SpawnSpec, spawn};
use tracing::{debug, info};

use crate::env::find_program;
use crate::placeholders;
use crate::plugin::{Plugin, Version};

/// How long one probe may take before it counts as failed.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Per-plugin program paths from `settings.toml`:
///
/// ```toml
/// [toolchains.csharp]
/// dotnet = "C:/Program Files/dotnet/dotnet.exe"
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolchainOverrides {
    by_plugin: BTreeMap<String, BTreeMap<String, PathBuf>>,
}

impl ToolchainOverrides {
    /// Read the `[toolchains]` table from the text of `settings.toml`. Other
    /// sections are ignored; they belong to other parts of the app.
    pub fn from_settings(text: &str) -> Result<Self, String> {
        let table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut by_plugin = BTreeMap::new();
        let Some(toolchains) = table.get("toolchains") else {
            return Ok(Self::default());
        };
        let toolchains = toolchains
            .as_table()
            .ok_or("`toolchains` should be a table, like [toolchains.csharp]")?;
        for (plugin, programs) in toolchains {
            let programs = programs.as_table().ok_or_else(|| {
                format!("`toolchains.{plugin}` should be a table of program = \"path\"")
            })?;
            let mut map = BTreeMap::new();
            for (program, path) in programs {
                let path = path.as_str().ok_or_else(|| {
                    format!("`toolchains.{plugin}.{program}` should be a path in quotes")
                })?;
                map.insert(program.clone(), PathBuf::from(path));
            }
            by_plugin.insert(plugin.clone(), map);
        }
        Ok(Self { by_plugin })
    }

    pub fn get(&self, plugin: &str, program: &str) -> Option<&Path> {
        self.by_plugin
            .get(plugin)
            .and_then(|m| m.get(program))
            .map(PathBuf::as_path)
    }

    pub fn is_empty(&self) -> bool {
        self.by_plugin.values().all(BTreeMap::is_empty)
    }
}

/// Finds a plugin's programs: settings override first, then `PATH`.
#[derive(Clone, Debug, Default)]
pub struct Resolver {
    path: OsString,
    overrides: ToolchainOverrides,
}

impl Resolver {
    pub fn new(path: OsString, overrides: ToolchainOverrides) -> Self {
        Self { path, overrides }
    }

    /// The `PATH` lookups use. Pass it to children so their own lookups match.
    pub fn path(&self) -> &OsStr {
        &self.path
    }

    pub fn overrides(&self) -> &ToolchainOverrides {
        &self.overrides
    }

    /// Where `program`, as used by plugin `plugin_id`, lives.
    pub fn resolve(&self, plugin_id: &str, program: &str) -> Result<PathBuf, String> {
        if let Some(path) = self.overrides.get(plugin_id, program) {
            return find_program(&path.to_string_lossy(), OsStr::new("")).ok_or_else(|| {
                format!(
                    "settings override for `{program}` points at {}, which isn't a program",
                    path.display()
                )
            });
        }
        find_program(program, &self.path).ok_or_else(|| format!("`{program}` was not found"))
    }
}

/// Run `plugin`'s probes in order and report the first that succeeds.
///
/// `scratch` fills the `{scratch}` placeholder. Never fails: problems become
/// [`ToolchainState::Missing`] with a reason per probe.
pub async fn probe(
    plugin: &Plugin,
    resolver: &Resolver,
    scratch: &Path,
    timeout: Duration,
) -> ToolchainState {
    let plugin_dir = plugin.dir.to_string_lossy();
    let scratch = scratch.to_string_lossy();
    let values = [("plugin_dir", &*plugin_dir), ("scratch", &*scratch)];
    let mut reasons = Vec::new();

    for probe in &plugin.probes {
        let command = match placeholders::substitute(&probe.command, &values) {
            Ok(c) => c,
            Err(e) => {
                reasons.push(e.to_string());
                continue;
            }
        };
        let path = match resolver.resolve(plugin.id.as_str(), &command) {
            Ok(path) => path,
            Err(reason) => {
                reasons.push(reason);
                continue;
            }
        };
        let args: Result<Vec<String>, _> = probe
            .args
            .iter()
            .map(|a| placeholders::substitute(a, &values))
            .collect();
        let args = match args {
            Ok(args) => args,
            Err(e) => {
                reasons.push(e.to_string());
                continue;
            }
        };

        let shown = std::iter::once(command.as_str())
            .chain(args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ");
        let output = match run_probe(&path, &args, resolver.path(), timeout).await {
            Ok(output) => output,
            Err(reason) => {
                reasons.push(format!("`{shown}` {reason}"));
                continue;
            }
        };

        let version = parse_version(&output);
        if let Some(min) = &probe.min_version {
            match &version {
                Some(found) if meets(found, min) => {}
                Some(found) => {
                    reasons.push(format!("`{command}`: found {found}, need {min}"));
                    continue;
                }
                None => {
                    reasons.push(format!(
                        "`{shown}` didn't print a version, and {min} or newer is needed"
                    ));
                    continue;
                }
            }
        }

        info!(plugin = %plugin.id, program = %command, path = %path.display(), version = ?version, "toolchain ready");
        return ToolchainState::Ready {
            program: command,
            path: path.display().to_string(),
            version,
        };
    }

    if plugin.probes.is_empty() {
        reasons.push("the plugin declares no probes for this OS".into());
    }
    let reason = reasons.join("; ");
    info!(plugin = %plugin.id, %reason, "toolchain missing");
    ToolchainState::Missing {
        reason,
        hint: plugin.install_hint.clone(),
    }
}

/// Run one probe, returning its combined output if it exits 0 in time.
async fn run_probe(
    program: &Path,
    args: &[String],
    path: &OsStr,
    timeout: Duration,
) -> Result<String, String> {
    let spec = SpawnSpec::new(program).args(args).env("PATH", path);
    let mut child = spawn(spec).map_err(|e| format!("couldn't start: {e}"))?;
    let kill = child.kill_handle();

    let collect = async {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        loop {
            match child.recv().await {
                Some(ProcessEvent::Stdout(b)) => out.extend_from_slice(&b),
                Some(ProcessEvent::Stderr(b)) => err.extend_from_slice(&b),
                Some(ProcessEvent::Exited(exit)) => break (out, err, exit.code),
                None => break (out, err, None),
            }
        }
    };

    let (out, err, code) = match tokio::time::timeout(timeout, collect).await {
        Ok(result) => result,
        Err(_) => {
            kill.kill();
            return Err(format!("didn't finish within {}s", timeout.as_secs_f32()));
        }
    };
    if code != Some(0) {
        let code = code.map_or_else(|| "a signal".to_owned(), |c| format!("code {c}"));
        return Err(format!("exited with {code}"));
    }
    // Some tools (java, older pythons) print their version to stderr.
    let mut text = String::from_utf8_lossy(&out).into_owned();
    text.push('\n');
    text.push_str(&String::from_utf8_lossy(&err));
    debug!(program = %program.display(), output = %text.trim(), "probe output");
    Ok(text)
}

fn version_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+)+").expect("valid regex"))
}

/// The first dotted version in a tool's output: `v24.14.0` gives `24.14.0`,
/// `cargo 1.98.1 (48a2 2026-09-01)` gives `1.98.1`, `10.0.100-rc.1` gives
/// `10.0.100`. A lone number (`zig 13`) is used if nothing dotted appears.
pub fn parse_version(output: &str) -> Option<String> {
    if let Some(m) = version_regex().find(output) {
        return Some(m.as_str().to_owned());
    }
    output
        .split(|c: char| !c.is_ascii_digit())
        .find(|s| !s.is_empty())
        .map(str::to_owned)
}

/// Whether version text `found` is at least `min`, comparing numerically with
/// missing parts as zero (`10` meets `10.0`).
pub fn meets(found: &str, min: &Version) -> bool {
    let found: Vec<u64> = found.split('.').map_while(|p| p.parse().ok()).collect();
    let len = found.len().max(min.0.len());
    let pad = |v: &[u64]| {
        (0..len)
            .map(|i| v.get(i).copied().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    pad(&found) >= pad(&min.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_real_tool_output() {
        let cases = [
            ("v24.14.0\n", "24.14.0"),
            ("10.0.201\n", "10.0.201"),
            ("cargo 1.98.1 (48a229cea 2026-09-01)\n", "1.98.1"),
            ("Python 3.12.4\n", "3.12.4"),
            ("go version go1.23.4 linux/amd64\n", "1.23.4"),
            ("10.0.100-rc.1.25451.107\n", "10.0.100"),
            ("PowerShell 7.5.0\n", "7.5.0"),
            ("openjdk 21.0.4 2024-07-16 LTS\n", "21.0.4"),
        ];
        for (output, expected) in cases {
            assert_eq!(
                parse_version(output).as_deref(),
                Some(expected),
                "{output:?}"
            );
        }
        assert_eq!(parse_version("zig 13").as_deref(), Some("13"));
        assert_eq!(parse_version("no version here"), None);
    }

    #[test]
    fn minimum_versions() {
        let v = |s: &str| Version::parse(s).unwrap();
        assert!(meets("24.14.0", &v("22.6")));
        assert!(meets("10.0.201", &v("10.0")));
        assert!(meets("10", &v("10.0")));
        assert!(meets("3.11", &v("3.11")));
        assert!(!meets("9.0.3", &v("10.0")));
        assert!(!meets("3.9.18", &v("3.11")));
        assert!(!meets("1.87.9", &v("1.88")));
    }

    #[test]
    fn overrides_from_settings() {
        let o = ToolchainOverrides::from_settings(
            r#"
            theme = "dark"

            [toolchains.csharp]
            dotnet = "C:/Program Files/dotnet/dotnet.exe"

            [toolchains.python]
            python3 = "/opt/python/bin/python3"

            [editor]
            font_size = 14
            "#,
        )
        .unwrap();
        assert_eq!(
            o.get("csharp", "dotnet"),
            Some(Path::new("C:/Program Files/dotnet/dotnet.exe"))
        );
        assert_eq!(
            o.get("python", "python3"),
            Some(Path::new("/opt/python/bin/python3"))
        );
        assert_eq!(o.get("csharp", "python3"), None);
        assert!(!o.is_empty());

        assert!(ToolchainOverrides::from_settings("").unwrap().is_empty());
        let err =
            ToolchainOverrides::from_settings("[toolchains.csharp]\ndotnet = 3\n").unwrap_err();
        assert!(err.contains("toolchains.csharp.dotnet"), "{err}");
    }
}
