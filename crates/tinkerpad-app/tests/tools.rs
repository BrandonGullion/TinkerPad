//! Getting a language's tools (work item 04-08a).
//!
//! Nothing here reaches the network. The command case runs
//! `tinkerpad-faketool`, a stand-in package manager that writes a stub program,
//! so it behaves the same on every OS. The download case fetches from a server
//! this test starts on localhost, reached through the `[tools] mirror` setting,
//! which is the supported way to point TinkerPad at a copy of a release.

use std::io::Write;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use tinkerpad_app::events::EventBus;
use tinkerpad_app::tools::{self, Env, Which};
use tinkerpad_core::CoreEvent;
use tinkerpad_plugin::{Arch, Os, Plugin, Resolver};

const FAKETOOL: &str = env!("CARGO_BIN_EXE_tinkerpad-faketool");

/// The bytes the fake release server hands out, and their SHA-256.
const RELEASE: &[u8] = b"#!/bin/sh\necho fake-tool\n";
const RELEASE_SHA: &str = "d9b434faf137810bbd0251baf68ba503cdb5e23e6feaf6b0c921b6eb71ed20ed";

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("tinkerpad-tools-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("plugin/templates")).unwrap();
        std::fs::write(dir.join("plugin/templates/main.x"), "x\n").unwrap();
        std::fs::create_dir_all(dir.join("data")).unwrap();
        Self(dir)
    }

    fn plugin_dir(&self) -> PathBuf {
        self.0.join("plugin")
    }

    fn data_dir(&self) -> PathBuf {
        self.0.join("data")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A TOML literal string, so Windows backslashes need no escaping.
fn lit(s: &str) -> String {
    format!("'{}'", s.replace('\'', ""))
}

/// Write a plugin whose `[toolchain]` section is `toolchain`, and load it.
fn plugin(scratch: &Scratch, toolchain: &str) -> Plugin {
    let manifest = format!(
        r#"api = 1

[language]
id = "fake"
name = "Fake"
extension = "x"

[toolchain]
probe = [{{ command = "tp-nothing" }}]
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
    std::fs::write(scratch.plugin_dir().join("plugin.toml"), manifest).unwrap();
    Plugin::load_for(scratch.plugin_dir(), Os::current()).unwrap_or_else(|e| panic!("{e}"))
}

/// An `Env` with nothing acquired yet and offers allowed.
fn env(scratch: &Scratch, mirror: Option<String>) -> Env {
    Env {
        data_dir: scratch.data_dir(),
        events: Arc::new(EventBus::new()),
        // The faketool is given by its full path, so an empty PATH is enough.
        resolver: Arc::new(Resolver::default()),
        mirror,
        offers: true,
    }
}

/// Everything the engine said while it worked. The bus delivers only what is
/// published after a subscription, so the caller subscribes before it asks.
fn lines(receiver: &mut tokio::sync::broadcast::Receiver<tinkerpad_core::Envelope>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(envelope) = receiver.try_recv() {
        if let CoreEvent::Tool { line, .. } = envelope.event {
            out.push(line);
        }
    }
    out
}

// --- the command case ---------------------------------------------------

#[tokio::test]
async fn an_acquire_command_runs_and_its_output_is_reported() {
    let scratch = Scratch::new("command");
    let installed = scratch.0.join("installed-here").join("tp-fake");
    let p = plugin(
        &scratch,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'install', {}]\nlabel = 'install the fake toolchain'\n",
            lit(FAKETOOL),
            lit(&installed.to_string_lossy()),
        ),
    );

    let env = env(&scratch, None);
    let mut watching = env.events.subscribe();
    let result = tools::acquire(&env, &p, Which::Toolchain).await;

    assert!(result.is_ok(), "{:?}", result.err());
    // A command installs wherever its package manager keeps things, so the
    // engine has no path to report — only the side effect.
    assert_eq!(result.unwrap(), None);
    assert!(installed.is_file(), "the command should have installed it");

    let said = lines(&mut watching);
    assert!(
        said.iter().any(|line| line.starts_with("installed ")),
        "the command's output should reach the window: {said:?}"
    );
}

#[tokio::test]
async fn a_failing_acquire_command_says_what_it_printed() {
    let scratch = Scratch::new("command-fails");
    let p = plugin(
        &scratch,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'print', 'no such package', 'exit', '3']\n",
            lit(FAKETOOL),
        ),
    );
    let error = tools::acquire(&env(&scratch, None), &p, Which::Toolchain)
        .await
        .expect_err("exit 3 is a failure");
    let text = error.to_string();
    assert!(text.contains("exited 3"), "{text}");
    assert!(text.contains("no such package"), "{text}");
}

#[tokio::test]
async fn acquire_never_refuses_before_anything_runs() {
    let scratch = Scratch::new("never");
    let installed = scratch.0.join("should-not-exist");
    let p = plugin(
        &scratch,
        &format!(
            "[toolchain.acquire]\nkind = \"command\"\ncommand = [{}, 'install', {}]\n",
            lit(FAKETOOL),
            lit(&installed.to_string_lossy()),
        ),
    );
    let mut env = env(&scratch, None);
    env.offers = false;

    let error = tools::acquire(&env, &p, Which::Toolchain)
        .await
        .expect_err("[tools] acquire = never should refuse");
    assert!(error.to_string().contains("switched off"), "{error}");
    assert!(!installed.exists(), "nothing should have been run");
    assert!(
        tools::offer(&env, &p, Which::Toolchain)
            .expect("there is an offer")
            .blocked
            .is_some(),
        "and the offer should say so too"
    );
}

// --- the download case ---------------------------------------------------

#[tokio::test]
async fn a_download_is_verified_and_kept() {
    let scratch = Scratch::new("download");
    let server = Server::start();
    let p = plugin(&scratch, &download_manifest("2.1.0", RELEASE_SHA));

    let env = env(&scratch, Some(server.base()));
    let mut watching = env.events.subscribe();
    let landed = tools::acquire(&env, &p, Which::Toolchain)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("a download reports where it landed");

    assert_eq!(std::fs::read(&landed).unwrap(), RELEASE);
    assert!(
        landed.starts_with(
            tools::tools_dir(&scratch.data_dir())
                .join("fake")
                .join("2.1.0")
        ),
        "it should land under <app-data>/tools/<plugin>/<version>/: {}",
        landed.display()
    );
    assert!(
        lines(&mut watching).iter().any(|l| l == "checksum matches"),
        "the window should be told the checksum was checked"
    );

    // The resolver finds it, for this plugin and not for another.
    let resolver = Resolver::default().with_acquired(tools::search_paths(&scratch.data_dir()));
    assert_eq!(
        resolver.resolve("fake", &program_name()).unwrap(),
        landed,
        "an acquired tool should be found by runs, servers and helpers"
    );
    assert!(
        resolver.resolve("other-plugin", &program_name()).is_err(),
        "one plugin's tools are not another's"
    );

    // Asking again is free: the folder is named for the version, and what is
    // in it was verified when it landed.
    let again = tools::acquire(&env, &p, Which::Toolchain).await.unwrap();
    assert_eq!(again, Some(landed));
    assert_eq!(
        server.hits(),
        1,
        "the second ask should not have downloaded"
    );
}

#[tokio::test]
async fn a_download_that_fails_its_checksum_is_thrown_away() {
    let scratch = Scratch::new("checksum");
    let server = Server::start();
    let wrong = "0".repeat(64);
    let p = plugin(&scratch, &download_manifest("2.1.0", &wrong));

    let env = env(&scratch, Some(server.base()));
    let error = tools::acquire(&env, &p, Which::Toolchain)
        .await
        .expect_err("the checksum doesn't match");

    let text = error.to_string();
    assert!(text.contains("didn't match its checksum"), "{text}");
    assert!(
        text.contains(RELEASE_SHA),
        "it should say what it got: {text}"
    );

    let root = tools::tools_dir(&scratch.data_dir()).join("fake");
    assert!(
        tools::search_paths(&scratch.data_dir()).is_empty(),
        "nothing should be left for the resolver to find"
    );
    assert!(
        !root.join("2.1.0").exists() && !root.join("2.1.0.partial").exists(),
        "and nothing should be left on disk either"
    );
}

/// A `[toolchain.acquire]` download for this machine, fetching `RELEASE`.
///
/// The URL is `https`, as a manifest's must be; the test reaches its own server
/// through the mirror setting.
fn download_manifest(version: &str, sha256: &str) -> String {
    download_manifest_for(version, sha256, &program_name(), "")
}

/// The same, for a download whose URL ends in `name` and which needs `extra`
/// keys such as `member`.
fn download_manifest_for(version: &str, sha256: &str, name: &str, extra: &str) -> String {
    format!(
        "[toolchain.acquire]\nkind = \"download\"\nversion = \"{version}\"\n\n\
         [[toolchain.acquire.file]]\nos = \"{os}\"\narch = \"{arch}\"\n\
         url = \"https://releases.invalid/fake/{version}/{name}\"\nsha256 = \"{sha256}\"\n{extra}",
        os = Os::current().as_str(),
        arch = Arch::current().expect("a known architecture").as_str(),
    )
}

/// What the downloaded file is called. There is no `member`, so the name comes
/// from the URL; it needs a Windows extension to be runnable there.
fn program_name() -> String {
    if cfg!(windows) {
        "tp-fake.exe".to_owned()
    } else {
        "tp-fake".to_owned()
    }
}

// --- archives ------------------------------------------------------------

#[tokio::test]
async fn a_member_is_taken_out_of_a_zip_and_nothing_else_is() {
    let scratch = Scratch::new("zip");
    // A release archive as they usually come: the tool inside a folder named
    // for the version, alongside files nobody asked for.
    let archive = zip_of(&[
        ("rust-analyzer-1.2.3/README.md", b"not the tool"),
        ("rust-analyzer-1.2.3/bin/tp-fake", RELEASE),
    ]);
    let server = Server::serving(archive.clone());
    let p = plugin(
        &scratch,
        &download_manifest_for(
            "1.2.3",
            &sha256(&archive),
            "tool.zip",
            "member = 'rust-analyzer-1.2.3/bin/tp-fake'\n",
        ),
    );

    let env = env(&scratch, Some(server.base()));
    let landed = tools::acquire(&env, &p, Which::Toolchain)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("a download reports where it landed");

    assert_eq!(std::fs::read(&landed).unwrap(), RELEASE);
    assert_eq!(landed.file_name().unwrap(), "tp-fake");
    let landed_in: Vec<_> = std::fs::read_dir(landed.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        landed_in,
        ["tp-fake"],
        "only the member asked for should be unpacked"
    );
}

#[tokio::test]
async fn a_symlink_in_a_tarball_is_not_what_gets_unpacked() {
    let scratch = Scratch::new("tar");
    // The archive offers a symlink out of the folder first, under the very
    // name the manifest asks for, and the real file after it.
    let archive = tar_gz_with_symlink_first("tp-fake", RELEASE);
    let server = Server::serving(archive.clone());
    let p = plugin(
        &scratch,
        &download_manifest_for(
            "9.9.9",
            &sha256(&archive),
            "tool.tar.gz",
            "member = 'tp-fake'\n",
        ),
    );

    let env = env(&scratch, Some(server.base()));
    let landed = tools::acquire(&env, &p, Which::Toolchain)
        .await
        .unwrap_or_else(|e| panic!("{e}"))
        .expect("the ordinary file is still unpacked");

    assert_eq!(
        std::fs::read(&landed).unwrap(),
        RELEASE,
        "the symlink should have been skipped and the real file taken"
    );
    assert!(
        !std::fs::symlink_metadata(&landed).unwrap().is_symlink(),
        "nothing unpacked should be a link"
    );
}

/// A zip holding each named entry.
fn zip_of(entries: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write as _;
    let mut out = std::io::Cursor::new(Vec::new());
    {
        let mut zip = zip::ZipWriter::new(&mut out);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            zip.start_file(*name, options).unwrap();
            zip.write_all(bytes).unwrap();
        }
        zip.finish().unwrap();
    }
    out.into_inner()
}

/// A gzipped tarball whose first entry is a symlink called `name` pointing out
/// of the archive, and whose second is an ordinary file of the same name.
fn tar_gz_with_symlink_first(name: &str, bytes: &[u8]) -> Vec<u8> {
    let mut tar = tar::Builder::new(Vec::new());

    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    link.set_cksum();
    tar.append_link(&mut link.clone(), name, "../../../../../../etc/passwd")
        .unwrap();

    let mut file = tar::Header::new_gnu();
    file.set_size(bytes.len() as u64);
    file.set_mode(0o755);
    file.set_cksum();
    tar.append_data(&mut file, name, bytes).unwrap();

    let tarred = tar.into_inner().unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    std::io::Write::write_all(&mut encoder, &tarred).unwrap();
    encoder.finish().unwrap()
}

fn sha256(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A one-file HTTP server on localhost, so the download path is exercised
/// without the network.
struct Server {
    port: u16,
    hits: Arc<std::sync::atomic::AtomicUsize>,
}

impl Server {
    fn start() -> Server {
        Server::serving(RELEASE.to_vec())
    }

    fn serving(body: Vec<u8>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a local port");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                // The request is read only far enough to be polite about it.
                let _ = read_request(&mut stream);
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.flush();
            }
        });
        Server { port, hits }
    }

    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    fn hits(&self) -> usize {
        self.hits.load(std::sync::atomic::Ordering::SeqCst)
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
