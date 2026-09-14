//! Probing toolchains with a fake tool (`tinkerpad-fakeprobe`), so every case
//! runs on every OS.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tinkerpad_core::ToolchainState;
use tinkerpad_plugin::{Os, PROBE_TIMEOUT, Plugin, Resolver, ToolchainOverrides, probe};

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakeprobe");

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-toolchain-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("templates")).unwrap();
        std::fs::write(dir.join("templates/main.x"), "x\n").unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A TOML literal string, so Windows backslashes need no escaping.
fn lit(s: &str) -> String {
    format!("'{s}'")
}

/// Write a plugin whose `[toolchain]` section is `toolchain` and load it.
fn plugin(scratch: &Scratch, toolchain: &str) -> Plugin {
    let manifest = format!(
        r#"api = 1

[language]
id = "fake"
name = "Fake"
extension = "x"

[toolchain]
{toolchain}

[[kind]]
id = "k"
name = "K"
template = "main.x"
default = true

[run]
command = "x"
"#
    );
    std::fs::write(scratch.0.join("plugin.toml"), manifest).unwrap();
    Plugin::load_for(&scratch.0, Os::current()).unwrap_or_else(|e| panic!("{e}"))
}

/// A probe entry running the fake tool with `steps`.
fn fake_probe(steps: &[&str], min_version: Option<&str>) -> String {
    let args = steps.iter().map(|s| lit(s)).collect::<Vec<_>>().join(", ");
    let min = min_version
        .map(|m| format!(", min_version = \"{m}\""))
        .unwrap_or_default();
    format!("{{ command = {}, args = [{args}]{min} }}", lit(FAKE))
}

fn resolver() -> Resolver {
    Resolver::new(
        std::env::var_os("PATH").unwrap_or_default(),
        ToolchainOverrides::default(),
    )
}

async fn run(p: &Plugin, r: &Resolver) -> ToolchainState {
    probe(p, r, &std::env::temp_dir(), PROBE_TIMEOUT).await
}

#[tokio::test]
async fn success_reports_version_and_path() {
    let s = Scratch::new("success");
    let p = plugin(
        &s,
        &format!(
            "probe = [{}]",
            fake_probe(&["print", "v24.14.0"], Some("22.6"))
        ),
    );
    match run(&p, &resolver()).await {
        ToolchainState::Ready {
            program,
            path,
            version,
        } => {
            assert_eq!(program, FAKE);
            assert_eq!(Path::new(&path), Path::new(FAKE));
            assert_eq!(version.as_deref(), Some("24.14.0"));
        }
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn too_old_is_missing_with_found_and_needed() {
    let s = Scratch::new("old");
    let p = plugin(
        &s,
        &format!(
            "probe = [{}]\ninstall_hint = \"Install the .NET 10 SDK: https://dot.net\"",
            fake_probe(&["print", "9.0.3"], Some("10.0"))
        ),
    );
    match run(&p, &resolver()).await {
        ToolchainState::Missing { reason, hint } => {
            assert!(reason.contains("found 9.0.3, need 10.0"), "{reason}");
            assert_eq!(
                hint.as_deref(),
                Some("Install the .NET 10 SDK: https://dot.net")
            );
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn prerelease_suffix_is_tolerated() {
    let s = Scratch::new("prerelease");
    let p = plugin(
        &s,
        &format!(
            "probe = [{}]",
            fake_probe(&["print", "10.0.100-rc.1.25451.107"], Some("10.0"))
        ),
    );
    assert!(
        matches!(run(&p, &resolver()).await, ToolchainState::Ready { version: Some(v), .. } if v == "10.0.100")
    );
}

#[tokio::test]
async fn version_on_stderr_counts() {
    let s = Scratch::new("stderr");
    let p = plugin(
        &s,
        &format!(
            "probe = [{}]",
            fake_probe(&["stderr", "openjdk 21.0.4 2024-07-16 LTS"], Some("17"))
        ),
    );
    assert!(matches!(
        run(&p, &resolver()).await,
        ToolchainState::Ready { .. }
    ));
}

#[tokio::test]
async fn not_found() {
    let s = Scratch::new("notfound");
    let p = plugin(
        &s,
        "probe = [{ command = \"tinkerpad-no-such-tool\", args = [\"--version\"] }]\ninstall_hint = \"get it\"",
    );
    match run(&p, &resolver()).await {
        ToolchainState::Missing { reason, hint } => {
            assert!(
                reason.contains("`tinkerpad-no-such-tool` was not found"),
                "{reason}"
            );
            assert_eq!(hint.as_deref(), Some("get it"));
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn non_zero_exit() {
    let s = Scratch::new("exit");
    let p = plugin(
        &s,
        &format!(
            "probe = [{}]",
            fake_probe(&["print", "1.0", "exit", "3"], None)
        ),
    );
    match run(&p, &resolver()).await {
        ToolchainState::Missing { reason, .. } => {
            assert!(reason.contains("exited with code 3"), "{reason}")
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn timeout_is_missing_and_quick() {
    let s = Scratch::new("timeout");
    let p = plugin(
        &s,
        &format!("probe = [{}]", fake_probe(&["sleep", "10000"], None)),
    );
    let started = Instant::now();
    let state = probe(
        &p,
        &resolver(),
        &std::env::temp_dir(),
        Duration::from_millis(300),
    )
    .await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
    match state {
        ToolchainState::Missing { reason, .. } => {
            assert!(reason.contains("didn't finish within"), "{reason}")
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn later_probe_wins_when_earlier_ones_fail() {
    let s = Scratch::new("fallback");
    let p = plugin(
        &s,
        &format!(
            "probe = [\n  {{ command = \"tinkerpad-no-such-tool\" }},\n  {},\n  {},\n]",
            fake_probe(&["print", "3.9.1"], Some("3.11")),
            fake_probe(&["print", "Python 3.12.4"], Some("3.11")),
        ),
    );
    match run(&p, &resolver()).await {
        ToolchainState::Ready { version, .. } => assert_eq!(version.as_deref(), Some("3.12.4")),
        other => panic!("expected Ready, got {other:?}"),
    }
}

#[tokio::test]
async fn all_reasons_are_reported_when_nothing_works() {
    let s = Scratch::new("reasons");
    let p = plugin(
        &s,
        &format!(
            "probe = [\n  {{ command = \"tinkerpad-no-such-tool\" }},\n  {},\n]",
            fake_probe(&["print", "3.9.1"], Some("3.11")),
        ),
    );
    match run(&p, &resolver()).await {
        ToolchainState::Missing { reason, .. } => {
            assert!(reason.contains("not found"), "{reason}");
            assert!(reason.contains("found 3.9.1, need 3.11"), "{reason}");
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}

#[tokio::test]
async fn settings_override_beats_path() {
    let s = Scratch::new("override");
    // `tp-dotnet` isn't on PATH; only the override can find it.
    let p = plugin(
        &s,
        "probe = [{ command = \"tp-dotnet\", args = [\"print\", \"10.0.201\"], min_version = \"10.0\" }]",
    );
    let settings = format!("[toolchains.fake]\ntp-dotnet = {}\n", lit(FAKE));
    let overrides = ToolchainOverrides::from_settings(&settings).unwrap();
    let r = Resolver::new(std::env::var_os("PATH").unwrap_or_default(), overrides);

    match run(&p, &r).await {
        ToolchainState::Ready { program, path, .. } => {
            assert_eq!(program, "tp-dotnet");
            assert_eq!(Path::new(&path), Path::new(FAKE));
        }
        other => panic!("expected Ready, got {other:?}"),
    }
    assert_eq!(r.resolve("fake", "tp-dotnet").unwrap(), PathBuf::from(FAKE));
    assert!(
        r.resolve("other-plugin", "tp-dotnet").is_err(),
        "overrides are per plugin"
    );
}

#[tokio::test]
async fn override_pointing_nowhere_says_so() {
    let s = Scratch::new("badoverride");
    let p = plugin(&s, "probe = [{ command = \"tp-dotnet\" }]");
    let overrides =
        ToolchainOverrides::from_settings("[toolchains.fake]\ntp-dotnet = '/no/such/dotnet'\n")
            .unwrap();
    let r = Resolver::new(std::env::var_os("PATH").unwrap_or_default(), overrides);
    match run(&p, &r).await {
        ToolchainState::Missing { reason, .. } => {
            assert!(reason.contains("settings override"), "{reason}");
            assert!(reason.contains("/no/such/dotnet"), "{reason}");
        }
        other => panic!("expected Missing, got {other:?}"),
    }
}
