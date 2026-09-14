//! The one pipe from the core to the UI.
//!
//! Anything in the core publishes a [`CoreEvent`] with a [`Scope`] on the
//! [`EventBus`]. Exactly one forwarding task drains the bus and hands each
//! [`Envelope`] to a sink, which in the app is Tauri's `emit`. Nothing else
//! calls `emit`, so the UI sees events in the order they were published.
//!
//! High-volume point-to-point streams (LSP traffic in Phase 2) should use a
//! per-call Tauri `Channel` instead of this bus.

use std::sync::Mutex;

use tauri::async_runtime::JoinHandle;
use tinkerpad_core::{CoreEvent, Envelope, Scope};
use tokio::sync::broadcast;
use tracing::{trace, warn};

/// Buffer size for the bus. A burst larger than this before the forwarder
/// catches up is logged as lag rather than silently dropped.
pub const CAPACITY: usize = 16_384;

pub struct EventBus {
    /// Sequence counter and sender behind one lock, so sequence numbers are
    /// assigned in the same order events enter the channel.
    inner: Mutex<(u64, broadcast::Sender<Envelope>)>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CAPACITY);
        Self {
            inner: Mutex::new((0, tx)),
        }
    }

    /// Publish an event. Never blocks; returns the sequence number assigned.
    pub fn publish(&self, scope: Scope, event: CoreEvent) -> u64 {
        let mut inner = self.inner.lock().expect("event bus lock poisoned");
        inner.0 += 1;
        let seq = inner.0;
        // An error only means no forwarder is subscribed yet (or anymore).
        let _ = inner.1.send(Envelope { seq, scope, event });
        seq
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Envelope> {
        self.inner
            .lock()
            .expect("event bus lock poisoned")
            .1
            .subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Start the single forwarding task. `sink` is called once per event, in
/// order. Returns when the bus is dropped.
pub fn spawn_forwarder<F>(mut rx: broadcast::Receiver<Envelope>, sink: F) -> JoinHandle<()>
where
    F: Fn(&Envelope) + Send + 'static,
{
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(env) => {
                    trace!(seq = env.seq, scope = %env.scope, "forward");
                    sink(&env);
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    warn!(missed, "UI event forwarder lagged; events were dropped");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use tinkerpad_core::{Level, RunEvent};

    use super::*;

    #[tokio::test]
    async fn burst_of_ten_thousand_arrives_in_order() {
        let bus = Arc::new(EventBus::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let forwarder = spawn_forwarder(bus.subscribe(), move |env| {
            sink_seen.lock().unwrap().push(env.seq);
        });

        // Publish from several tasks at once, as the core will.
        let mut publishers = Vec::new();
        for t in 0..4 {
            let bus = bus.clone();
            publishers.push(tokio::spawn(async move {
                for i in 0..2_500 {
                    bus.publish(
                        Scope::Run(t),
                        CoreEvent::Run(RunEvent::Stdout {
                            text: i.to_string(),
                        }),
                    );
                }
            }));
        }
        for p in publishers {
            p.await.unwrap();
        }
        drop(bus);
        forwarder.await.unwrap();

        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 10_000, "events were dropped");
        assert!(
            seen.windows(2).all(|w| w[0] + 1 == w[1]),
            "events arrived out of order"
        );
    }

    #[tokio::test]
    async fn publish_without_subscriber_does_not_panic() {
        let bus = EventBus::new();
        let seq = bus.publish(
            Scope::App,
            CoreEvent::Toast {
                level: Level::Info,
                message: "nobody listening".into(),
            },
        );
        assert_eq!(seq, 1);
    }
}
