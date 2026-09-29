//! What a plugin may declare about getting its tools (work item 04-08a).
//!
//! The manifests here are written to a temporary folder rather than kept as
//! fixtures, because each one differs from the last by a single bad value —
//! the point of every case is the one line that fails.

use std::path::PathBuf;

use tinkerpad_plugin::{Acquire, Arch, ManifestError, Os, Plugin};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-acquire-{}-{name}", std::process::id()));
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

const SHA: &str = "d9b434faf137810bbd0251baf68ba503cdb5e23e6feaf6b0c921b6eb71ed20ed";

/// A plugin whose `[toolchain]` carries `section`, loaded for `os`.
fn load_for(scratch: &Scratch, section: &str, os: Os) -> Result<Plugin, ManifestError> {
    let manifest = format!(
        r#"api = 1

[language]
id = "fake"
name = "Fake"
extension = "x"

[toolchain]
probe = [{{ command = "tp-nothing" }}]
{section}

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
    Plugin::load_for(&scratch.0, os)
}

fn load(scratch: &Scratch, section: &str) -> Plugin {
    load_for(scratch, section, Os::Linux).unwrap_or_else(|e| panic!("should load:\n{e}"))
}

fn fail(scratch: &Scratch, section: &str) -> String {
    match load_for(scratch, section, Os::Linux) {
        Ok(_) => panic!("should not load:\n{section}"),
        Err(e) => e.to_string(),
    }
}

/// A download entry for `os`/`arch`, with everything valid unless overridden.
fn file(os: &str, arch: &str, extra: &str) -> String {
    format!(
        "[[toolchain.acquire.file]]\nos = \"{os}\"\narch = \"{arch}\"\n\
         url = \"https://example.invalid/tool.zip\"\nsha256 = \"{SHA}\"\n{extra}"
    )
}

// ----------------------------------------------------------- what loads ----

#[test]
fn a_command_acquire_keeps_the_command_and_labels_itself() {
    let s = Scratch::new("command");
    let p = load(
        &s,
        "[toolchain.acquire]\nkind = \"command\"\n\
         command = [\"rustup\", \"component\", \"add\", \"rust-analyzer\"]\nneeds = \"rustup\"\n",
    );
    match p.toolchain_acquire.expect("an acquire") {
        Acquire::Command {
            command,
            needs,
            label,
        } => {
            assert_eq!(command, ["rustup", "component", "add", "rust-analyzer"]);
            assert_eq!(needs.as_deref(), Some("rustup"));
            // No label given, so the command is what the user is shown.
            assert_eq!(label, "rustup component add rust-analyzer");
        }
        other => panic!("expected a command, got {other:?}"),
    }
}

#[test]
fn a_label_replaces_the_command_when_one_is_given() {
    let s = Scratch::new("label");
    let p = load(
        &s,
        "[toolchain.acquire]\nkind = \"command\"\ncommand = [\"brew\", \"install\", \"go\"]\n\
         label = \"install Go with Homebrew\"\n",
    );
    assert_eq!(
        p.toolchain_acquire.expect("an acquire").label(),
        "install Go with Homebrew"
    );
}

#[test]
fn a_command_for_another_os_is_simply_absent() {
    let s = Scratch::new("os");
    let section = "[toolchain.acquire]\nkind = \"command\"\ncommand = [\"winget\", \"install\", \"Go\"]\nos = [\"windows\"]\n";
    // It loads everywhere; only Windows has anything to offer.
    assert!(
        load_for(&s, section, Os::Linux)
            .unwrap()
            .toolchain_acquire
            .is_none()
    );
    assert!(
        load_for(&s, section, Os::Macos)
            .unwrap()
            .toolchain_acquire
            .is_none()
    );
    assert!(
        load_for(&s, section, Os::Windows)
            .unwrap()
            .toolchain_acquire
            .is_some(),
        "the machine it is for gets it"
    );
}

#[test]
fn a_download_picks_the_file_for_this_machine() {
    let s = Scratch::new("download");
    let section = format!(
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"0.3.2000\"\n\n{}\n{}\n",
        file("windows", "x86_64", "member = \"rust-analyzer.exe\"\n"),
        file("linux", "aarch64", "member = \"rust-analyzer\"\n"),
    );
    let acquire = load(&s, &section).toolchain_acquire.expect("an acquire");

    assert_eq!(acquire.label(), "download version 0.3.2000");
    let picked = acquire
        .file_for(Os::Windows, Arch::X86_64)
        .expect("the windows build");
    assert_eq!(picked.member.as_deref(), Some("rust-analyzer.exe"));
    assert_eq!(picked.sha256, SHA);
    assert!(
        acquire.file_for(Os::Linux, Arch::X86_64).is_none(),
        "a platform with no file published has none"
    );
}

#[test]
fn a_checksum_is_kept_as_lowercase_hex() {
    let s = Scratch::new("case");
    let section = format!(
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"1\"\n\n{}",
        file("linux", "x86_64", "").replace(SHA, &SHA.to_ascii_uppercase()),
    );
    let acquire = load(&s, &section).toolchain_acquire.expect("an acquire");
    assert_eq!(
        acquire.file_for(Os::Linux, Arch::X86_64).unwrap().sha256,
        SHA
    );
}

// --------------------------------------------------------- what refuses ----

#[test]
fn a_download_without_a_version_does_not_load() {
    let s = Scratch::new("no-version");
    let section = format!(
        "[toolchain.acquire]\nkind = \"download\"\n\n{}",
        file("linux", "x86_64", "")
    );
    let text = fail(&s, &section);
    assert!(text.contains("version"), "{text}");
}

#[test]
fn a_url_that_is_not_https_does_not_load() {
    let s = Scratch::new("http");
    let section = format!(
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"1\"\n\n{}",
        file("linux", "x86_64", "").replace("https://", "http://"),
    );
    let text = fail(&s, &section);
    assert!(text.contains("https://"), "{text}");
}

#[test]
fn a_checksum_that_is_not_a_sha256_does_not_load() {
    let s = Scratch::new("badsha");
    for bad in ["deadbeef", &"z".repeat(64), ""] {
        let section = format!(
            "[toolchain.acquire]\nkind = \"download\"\nversion = \"1\"\n\n{}",
            file("linux", "x86_64", "").replace(SHA, bad),
        );
        let text = fail(&s, &section);
        assert!(text.contains("64 hex characters"), "{bad}: {text}");
    }
}

#[test]
fn a_member_that_climbs_out_of_the_archive_does_not_load() {
    let s = Scratch::new("traversal");
    for bad in [
        "../outside",
        "a/../../outside",
        "/etc/passwd",
        "C:/Windows/system32/cmd.exe",
        "..\\outside",
        "",
    ] {
        let section = format!(
            "[toolchain.acquire]\nkind = \"download\"\nversion = \"1\"\n\n{}",
            file("linux", "x86_64", &format!("member = '{bad}'\n")),
        );
        let text = fail(&s, &section);
        assert!(
            text.contains("relative path inside the archive"),
            "{bad}: {text}"
        );
    }
}

#[test]
fn the_same_platform_twice_does_not_load() {
    let s = Scratch::new("dupe");
    let section = format!(
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"1\"\n\n{}\n{}",
        file("linux", "x86_64", ""),
        file("linux", "amd64", ""), // the same thing under its other name
    );
    let text = fail(&s, &section);
    assert!(text.contains("named twice"), "{text}");
}

#[test]
fn a_download_with_no_files_does_not_load() {
    let s = Scratch::new("nofiles");
    let text = fail(
        &s,
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"1\"\n",
    );
    assert!(text.contains("at least one"), "{text}");
}

#[test]
fn a_command_with_nothing_to_run_does_not_load() {
    let s = Scratch::new("empty");
    let text = fail(
        &s,
        "[toolchain.acquire]\nkind = \"command\"\ncommand = [\"  \"]\n",
    );
    assert!(text.contains("needs a program to run"), "{text}");
}

#[test]
fn an_unknown_key_in_acquire_does_not_load() {
    let s = Scratch::new("unknown");
    let text = fail(
        &s,
        "[toolchain.acquire]\nkind = \"command\"\ncommand = [\"go\"]\nsudo = true\n",
    );
    assert!(text.contains("sudo"), "{text}");
}

#[test]
fn an_unknown_kind_does_not_load() {
    let s = Scratch::new("kind");
    let text = fail(&s, "[toolchain.acquire]\nkind = \"curl-pipe-sh\"\n");
    assert!(text.contains("curl-pipe-sh"), "{text}");
}
