//! `tinkerpad tools list` and `install`, over a fake plugin (work item
//! 05-01c).
//!
//! The same fake package manager and the same local file server the
//! acquisition tests use (04-08a), so nothing here reaches the network and
//! nothing depends on which languages this machine happens to have.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use tinkerpad_app::cli::{Context, install_in, list_in};
use tinkerpad_app::events::EventBus;
use tinkerpad_app::tools::{Env, Which};
use tinkerpad_core::ToolchainState;
use tinkerpad_plugin::{Arch, Os, Plugin, Resolver};

const FAKETOOL: &str = env!("CARGO_BIN_EXE_tinkerpad-faketool");

/// What the fake release server hands out, and its SHA-256.
const RELEASE: &[u8] = b"#!/bin/sh\necho fake-tool\n";
const RELEASE_SHA: &str = "d9b434faf137810bbd0251baf68ba503cdb5e23e6feaf6b0c921b6eb71ed20ed";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("tinkerpad-cli-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugin/templates")).unwrap();
        std::fs::write(dir.join("plugin/templates/main.x"), "x\n").unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        std::fs::create_dir_all(dir.join("library")).unwrap();
        Self(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn lit(s: &str) -> String {
    format!("'{s}'")
}

/// A plugin whose `[toolchain]` carries `section`.
fn plugin(scratch: &Scratch, section: &str) -> Arc<Plugin> {
    let manifest = format!(
        r#"api = 1

[language]
id = "fake"
name = "Fake"
extension = "x"

[toolchain]
probe = [{{ command = "tp-nothing" }}]
install_hint = "Install Fake by hand"
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
    let dir = scratch.0.join("plugin");
    std::fs::write(dir.join("plugin.toml"), manifest).unwrap();
    Arc::new(Plugin::load_for(&dir, Os::current()).unwrap_or_else(|e| panic!("{e}")))
}

fn context(
    scratch: &Scratch,
    plugin: Arc<Plugin>,
    mirror: Option<String>,
    offers: bool,
) -> Context {
    let env = Env {
        data_dir: scratch.0.join("data"),
        events: Arc::new(EventBus::new()),
        resolver: Arc::new(Resolver::default()),
        mirror,
        offers,
        library_root: scratch.0.join("library"),
        scratch_root: scratch.0.join("data").join("scratch"),
    };
    Context::over(
        vec![plugin],
        env,
        vec![(
            "fake".to_owned(),
            ToolchainState::Missing {
                reason: "`tp-nothing` was not found".to_owned(),
                hint: Some("Install Fake by hand".to_owned()),
            },
        )],
    )
    .expect("a context")
}

fn lines(run: impl FnOnce(&mut dyn FnMut(&str)) -> i32) -> (i32, Vec<String>) {
    let mut said = Vec::new();
    let code = run(&mut |line| said.push(line.to_owned()));
    (code, said)
}

/// A `[toolchain.acquire]` download of `RELEASE`, for this machine.
fn download_section(sha: &str) -> String {
    format!(
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"1.0.0\"\n\n\
         [[toolchain.acquire.file]]\nos = \"{os}\"\narch = \"{arch}\"\n\
         url = \"https://releases.invalid/fake/1.0.0/{name}\"\nsha256 = \"{sha}\"\n",
        os = Os::current().as_str(),
        arch = Arch::current().expect("a known architecture").as_str(),
        name = program_name(),
    )
}

fn program_name() -> String {
    if cfg!(windows) {
        "tp-fake.exe".to_owned()
    } else {
        "tp-fake".to_owned()
    }
}

// --- list -----------------------------------------------------------------

#[test]
fn list_says_what_is_missing_and_whether_it_can_be_installed() {
    let s = Scratch::new("list");
    let p = plugin(
        &s,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'print', 'hi']\n\
             label = 'install Fake'\n",
            lit(FAKETOOL),
        ),
    );
    let (code, said) = lines(|out| list_in(&context(&s, p, None, true), false, out));

    assert_eq!(code, 0);
    let text = said.join("\n");
    assert!(text.contains("Fake"), "{text}");
    assert!(text.contains("missing"), "{text}");
    assert!(text.contains("can install"), "{text}");
    assert!(text.contains("1 thing missing"), "{text}");
}

#[test]
fn list_says_why_when_it_cannot_install() {
    let s = Scratch::new("list-blocked");
    let p = plugin(&s, "");
    let (code, said) = lines(|out| list_in(&context(&s, p, None, true), false, out));

    assert_eq!(code, 0, "nothing to install is not an error");
    let text = said.join("\n");
    // No `acquire` at all, so the plugin's own instructions are what is left.
    assert!(text.contains("Install Fake by hand"), "{text}");
    assert!(!text.contains("can install"), "{text}");
}

#[test]
fn list_as_json_is_the_same_answer_the_window_gets() {
    let s = Scratch::new("list-json");
    let p = plugin(
        &s,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'print', 'hi']\n",
            lit(FAKETOOL),
        ),
    );
    let (code, said) = lines(|out| list_in(&context(&s, p, None, true), true, out));

    assert_eq!(code, 0);
    let value: serde_json::Value = serde_json::from_str(&said.join("\n")).expect("json");
    let needs = value[0]["needs"].as_array().expect("needs");
    assert_eq!(value[0]["language"], "fake");
    assert_eq!(needs[0]["present"], false);
    assert_eq!(needs[0]["which"], "toolchain");
    assert!(needs[0]["offer"]["label"].is_string(), "{value}");
}

// --- install --------------------------------------------------------------

#[test]
fn install_asks_before_it_runs_anything() {
    let s = Scratch::new("install-asks");
    let marker = s.0.join("should-not-exist");
    let p = plugin(
        &s,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'install', {}]\n",
            lit(FAKETOOL),
            lit(&marker.to_string_lossy()),
        ),
    );
    // Not a terminal under `cargo test`, so there is nobody to ask: it says so
    // and does nothing rather than assuming yes.
    let (code, said) =
        lines(|out| install_in(&context(&s, p, None, true), "fake", None, false, out));

    assert_eq!(code, 1);
    let text = said.join("\n");
    assert!(text.contains("--yes"), "{text}");
    assert!(!marker.exists(), "nothing should have been run");
}

#[test]
fn install_with_yes_runs_it_and_says_what_it_said() {
    let s = Scratch::new("install-yes");
    let marker = s.0.join("installed-here").join("tp-fake");
    let p = plugin(
        &s,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'install', {}]\n",
            lit(FAKETOOL),
            lit(&marker.to_string_lossy()),
        ),
    );
    let (code, said) =
        lines(|out| install_in(&context(&s, p, None, true), "fake", None, true, out));

    assert_eq!(code, 0, "{said:#?}");
    assert!(marker.is_file(), "the command should have installed it");
    let text = said.join("\n");
    // What it was about to do, then what it said while doing it.
    assert!(text.contains("install"), "{text}");
    assert!(text.contains("installed "), "the tool's own output: {text}");
}

#[test]
fn install_downloads_through_the_mirror_and_says_where_it_landed() {
    let s = Scratch::new("install-download");
    let server = Server::start();
    let p = plugin(&s, &download_section(RELEASE_SHA));
    let (code, said) = lines(|out| {
        install_in(
            &context(&s, p, Some(server.base()), true),
            "fake",
            Some(Which::Toolchain),
            true,
            out,
        )
    });

    assert_eq!(code, 0, "{said:#?}");
    let text = said.join("\n");
    // The URL and checksum are shown before anything is fetched.
    assert!(text.contains("https://releases.invalid"), "{text}");
    assert!(text.contains(RELEASE_SHA), "{text}");
    assert!(text.contains("Installed at"), "{text}");
    let landed = s.0.join("data").join("tools").join("fake").join("1.0.0");
    assert!(landed.is_dir(), "it should land under <app-data>/tools");
}

#[test]
fn a_download_that_fails_its_checksum_is_reported_and_exits_non_zero() {
    let s = Scratch::new("install-badsum");
    let server = Server::start();
    let p = plugin(&s, &download_section(&"0".repeat(64)));
    let (code, said) = lines(|out| {
        install_in(
            &context(&s, p, Some(server.base()), true),
            "fake",
            None,
            true,
            out,
        )
    });

    assert_eq!(code, 1);
    assert!(
        said.join("\n").contains("didn't match its checksum"),
        "{said:#?}"
    );
}

#[test]
fn acquire_never_refuses_here_too() {
    let s = Scratch::new("install-never");
    let marker = s.0.join("should-not-exist");
    let p = plugin(
        &s,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'install', {}]\n",
            lit(FAKETOOL),
            lit(&marker.to_string_lossy()),
        ),
    );
    let (code, said) =
        lines(|out| install_in(&context(&s, p, None, false), "fake", None, true, out));

    assert_eq!(code, 1);
    let text = said.join("\n");
    assert!(text.contains("switched off"), "{text}");
    assert!(!marker.exists(), "nothing should have been run");
}

#[test]
fn a_language_that_is_not_here_says_so() {
    let s = Scratch::new("install-unknown");
    let p = plugin(&s, "");
    let (code, said) =
        lines(|out| install_in(&context(&s, p, None, true), "klingon", None, true, out));

    assert_eq!(code, 2);
    assert!(
        said.join("\n").contains("No plugin for `klingon`"),
        "{said:#?}"
    );
}

/// A one-file HTTP server on localhost, as in the acquisition tests.
struct Server {
    port: u16,
}

impl Server {
    fn start() -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = read_request(&mut stream);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    RELEASE.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(RELEASE);
                let _ = stream.flush();
            }
        });
        Server { port }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

fn read_request(stream: &mut std::net::TcpStream) -> std::io::Result<()> {
    use std::io::Read;
    let mut seen = Vec::new();
    let mut byte = [0u8; 1];
    while stream.read(&mut byte)? == 1 {
        seen.push(byte[0]);
        if seen.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    Ok(())
}
