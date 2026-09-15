//! How much the dump parser costs on plain output.
//!
//! Pushes 100 MB of stdout with no sentinels through the UTF-8 decoder alone
//! (what runs did before the parser) and through `OutputDecoder` (parser plus
//! decoder), and prints both rates. Two inputs: plain log lines, and the same
//! with ANSI colour codes, whose ESC bytes make the parser look closer.
//!
//! ```text
//! cargo run --release -p tinkerpad-runner --example dump_throughput
//! ```

use std::hint::black_box;
use std::time::{Duration, Instant};

use tinkerpad_runner::{OutputDecoder, Utf8Stream};

const TOTAL: usize = 100 * 1024 * 1024;
const ROUNDS: usize = 5;

fn main() {
    let plain = "2026-09-15T10:00:00Z INFO request handled path=/api/users/42 status=200 ms=3 · héllo 日本\n";
    let ansi = "2026-09-15T10:00:00Z \x1b[32mINFO\x1b[0m request handled path=/api/users/42 status=200 · 日本\n";
    println!("100 MB of stdout with no dump sentinels, best of {ROUNDS} rounds");
    for (name, line) in [("plain", plain), ("ansi colours", ansi)] {
        let data = repeat(line);
        for chunk in [8 * 1024, 64 * 1024] {
            let baseline = best(|| {
                let mut utf8 = Utf8Stream::new();
                let mut bytes = 0;
                for piece in data.chunks(chunk) {
                    bytes += black_box(utf8.push(piece)).len();
                }
                bytes
            });
            let parsed = best(|| {
                let mut decoder = OutputDecoder::new();
                let mut events = Vec::new();
                let mut bytes = 0;
                for piece in data.chunks(chunk) {
                    decoder.stdout(piece, &mut events);
                    for event in events.drain(..) {
                        if let tinkerpad_core::RunEvent::Stdout { text } = black_box(event) {
                            bytes += text.len();
                        }
                    }
                }
                decoder.finish(&mut events);
                bytes
            });
            let rate = |d: Duration| data.len() as f64 / (1024.0 * 1024.0) / d.as_secs_f64();
            println!(
                "{name:>12}, {:>2} KiB chunks: decoder only {:>5.0} MB/s, with dump parser {:>5.0} MB/s ({:>3} ms for 100 MB), overhead {:+.0}%",
                chunk / 1024,
                rate(baseline),
                rate(parsed),
                parsed.as_millis(),
                (parsed.as_secs_f64() / baseline.as_secs_f64() - 1.0) * 100.0,
            );
        }
    }
}

fn repeat(line: &str) -> Vec<u8> {
    let mut data = Vec::with_capacity(TOTAL + line.len());
    while data.len() < TOTAL {
        data.extend_from_slice(line.as_bytes());
    }
    data.truncate(TOTAL);
    // Don't end mid-character.
    while std::str::from_utf8(&data).is_err() {
        data.pop();
    }
    data
}

fn best(mut f: impl FnMut() -> usize) -> Duration {
    let expected = f();
    (0..ROUNDS)
        .map(|_| {
            let start = Instant::now();
            assert_eq!(f(), expected);
            start.elapsed()
        })
        .min()
        .unwrap()
}
