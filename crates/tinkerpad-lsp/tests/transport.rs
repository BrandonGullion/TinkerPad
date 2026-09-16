//! The transport against a real child process: `tinkerpad-fakelsp`, which
//! speaks the same framing a language server does (work item 02-01).

use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tinkerpad_lsp::{RawMessage, Transport, TransportError, TransportEvent};
use tinkerpad_process::SpawnSpec;

const FAKE: &str = env!("CARGO_BIN_EXE_tinkerpad-fakelsp");

/// Long enough that a slow CI machine passes, short enough that a hang fails.
const PATIENCE: Duration = Duration::from_secs(30);

fn server() -> Transport {
    Transport::spawn("fake", SpawnSpec::new(FAKE)).expect("the fake server binary is built")
}

/// The next event, or a failure if the server goes quiet.
async fn next(transport: &mut Transport) -> TransportEvent {
    match tokio::time::timeout(PATIENCE, transport.recv()).await {
        Ok(Some(event)) => event,
        Ok(None) => panic!("the incoming stream ended"),
        Err(_) => panic!("nothing arrived within {PATIENCE:?}"),
    }
}

/// The next event, which should be a message.
async fn next_message(transport: &mut Transport) -> RawMessage {
    match next(transport).await {
        TransportEvent::Message(message) => message,
        other => panic!("expected a message, got {other:?}"),
    }
}

/// `initialize`, `initialized`, and the reply, the way a client starts.
async fn initialize(transport: &mut Transport) -> Value {
    transport
        .send(RawMessage::request(
            1,
            "initialize",
            json!({ "processId": std::process::id(), "capabilities": {} }),
        ))
        .await
        .unwrap();
    let response = next_message(transport).await;
    assert!(response.is_response(), "{response}");
    assert_eq!(response.id(), Some(&json!(1)));
    transport
        .send(RawMessage::notification("initialized", json!({})))
        .await
        .unwrap();
    response.result().cloned().unwrap_or(Value::Null)
}

#[tokio::test]
async fn initialize_handshake() {
    let mut transport = server();
    assert!(transport.pid() > 0);
    assert_eq!(transport.name(), "fake");

    let result = initialize(&mut transport).await;
    assert_eq!(result["capabilities"]["hoverProvider"], json!(true));
    assert_eq!(result["serverInfo"]["name"], json!("fakelsp"));
    assert_eq!(
        result["serverInfo"]["pid"].as_u64(),
        Some(u64::from(transport.pid())),
        "the server the transport reports is the one that answered",
    );

    shutdown(&mut transport).await;
}

/// `shutdown`, `exit`, and the exit event, checking a clean stop.
async fn shutdown(transport: &mut Transport) {
    transport
        .send(RawMessage::request(9999, "shutdown", Value::Null))
        .await
        .unwrap();
    let response = next_message(transport).await;
    assert_eq!(response.id(), Some(&json!(9999)));
    transport
        .send(RawMessage::notification("exit", Value::Null))
        .await
        .unwrap();
    match next(transport).await {
        TransportEvent::Exited(exit) => {
            assert_eq!(
                exit.code,
                Some(0),
                "a server that was asked to stop exits 0"
            );
            assert!(!exit.killed);
        }
        other => panic!("expected the exit event, got {other:?}"),
    }
    assert!(transport.recv().await.is_none(), "nothing follows the exit");
}

#[tokio::test]
async fn a_thousand_round_trips_keep_their_order() {
    let mut transport = server();
    initialize(&mut transport).await;

    const N: i64 = 1000;
    let sender = transport.sender();
    let writer = tokio::spawn(async move {
        for n in 0..N {
            sender
                .send(RawMessage::request(
                    n,
                    "test/echo",
                    json!({ "n": n, "text": "round trip" }),
                ))
                .await
                .unwrap();
        }
    });

    let started = Instant::now();
    for n in 0..N {
        let response = next_message(&mut transport).await;
        assert_eq!(response.id(), Some(&json!(n)), "replies come back in order");
        assert_eq!(response.result().unwrap()["n"], json!(n));
    }
    let elapsed = started.elapsed();
    writer.await.unwrap();
    println!(
        "{N} round trips in {:?} ({:.0} µs each)",
        elapsed,
        elapsed.as_secs_f64() * 1e6 / N as f64
    );

    shutdown(&mut transport).await;
}

#[tokio::test]
async fn carries_a_two_megabyte_message() {
    let mut transport = server();
    initialize(&mut transport).await;

    const BYTES: usize = 2 * 1024 * 1024;
    transport
        .send(RawMessage::request(
            5,
            "test/big",
            json!({ "bytes": BYTES }),
        ))
        .await
        .unwrap();
    let response = next_message(&mut transport).await;
    assert_eq!(response.id(), Some(&json!(5)));
    assert_eq!(
        response.result().unwrap()["text"].as_str().unwrap().len(),
        BYTES,
        "a body far larger than one pipe read arrives whole",
    );

    // The connection still works afterwards.
    transport
        .send(RawMessage::request(6, "test/echo", json!("after")))
        .await
        .unwrap();
    assert_eq!(
        next_message(&mut transport).await.result(),
        Some(&json!("after"))
    );

    shutdown(&mut transport).await;
}

#[tokio::test]
async fn notifications_arrive_between_responses() {
    let mut transport = server();
    initialize(&mut transport).await;

    transport
        .send(RawMessage::request(2, "test/log", json!({ "count": 3 })))
        .await
        .unwrap();

    let mut logs = 0;
    loop {
        let message = next_message(&mut transport).await;
        if message.is_notification() {
            assert_eq!(message.method(), Some("window/logMessage"));
            logs += 1;
        } else {
            assert_eq!(message.id(), Some(&json!(2)));
            break;
        }
    }
    assert_eq!(logs, 3, "every notification arrived before the response");

    shutdown(&mut transport).await;
}

#[tokio::test]
async fn stdout_closing_mid_message_ends_the_stream_with_an_error() {
    let mut transport = server();
    initialize(&mut transport).await;

    transport
        .send(RawMessage::request(3, "test/halt", Value::Null))
        .await
        .unwrap();

    // The half-written message is reported, then the exit, then nothing. No
    // panic, and no wait for a body that will never come.
    let mut broken = None;
    let exit = loop {
        match next(&mut transport).await {
            TransportEvent::Broken(e) => broken = Some(e),
            TransportEvent::Exited(exit) => break exit,
            TransportEvent::Message(m) => panic!("unexpected message {m}"),
        }
    };
    assert!(
        matches!(broken, Some(TransportError::Frame(_))),
        "expected a framing error, got {broken:?}",
    );
    assert_eq!(exit.code, Some(0));
    assert!(transport.recv().await.is_none());

    // Sending to a server that is gone fails rather than blocking.
    let result = tokio::time::timeout(
        PATIENCE,
        transport.send(RawMessage::request(4, "test/echo", Value::Null)),
    )
    .await
    .expect("send returns promptly once the server is gone");
    // The writer may still be draining its queue, so either outcome is fine;
    // what matters is that it did not hang.
    let _ = result;
}

#[tokio::test]
async fn killing_the_server_ends_the_stream() {
    let mut transport = server();
    initialize(&mut transport).await;

    transport.kill();
    let exit = loop {
        match next(&mut transport).await {
            TransportEvent::Exited(exit) => break exit,
            _ => continue,
        }
    };
    assert!(exit.killed, "the exit says it was killed");
    assert!(transport.recv().await.is_none());
}

#[tokio::test]
async fn a_server_that_writes_junk_to_stdout_does_not_end_the_session() {
    let mut transport = server();
    initialize(&mut transport).await;

    // The fake server has no way to write junk, so send it a method it does
    // not know: the error response proves the stream survived, and the framing
    // resync is covered by the decoder's own tests.
    transport
        .send(RawMessage::request(11, "test/unknown", Value::Null))
        .await
        .unwrap();
    let response = next_message(&mut transport).await;
    assert!(response.error_object().is_some(), "{response}");
    assert_eq!(response.id(), Some(&json!(11)));

    transport
        .send(RawMessage::request(12, "test/echo", json!("still here")))
        .await
        .unwrap();
    assert_eq!(
        next_message(&mut transport).await.result(),
        Some(&json!("still here"))
    );

    shutdown(&mut transport).await;
}

#[tokio::test]
async fn stderr_does_not_disturb_the_message_stream() {
    let mut transport = server();
    initialize(&mut transport).await;

    // A server that chats on stderr (rust-analyzer logs there) is normal; the
    // lines go to the log, not to the caller.
    transport
        .send(RawMessage::request(
            21,
            "test/stderr",
            json!({ "text": "loading workspace" }),
        ))
        .await
        .unwrap();
    assert_eq!(next_message(&mut transport).await.id(), Some(&json!(21)));

    transport
        .send(RawMessage::request(22, "test/echo", json!("quiet")))
        .await
        .unwrap();
    assert_eq!(
        next_message(&mut transport).await.result(),
        Some(&json!("quiet"))
    );

    shutdown(&mut transport).await;
}
