//! The language server pool inside the app.
//!
//! The pool itself is `tinkerpad-lsp` (work item 02-02); this is the two ends
//! of it the app owns: its state changes become `server` events for the UI,
//! and quitting stops every server before the process goes away.
//!
//! Nothing attaches to a server yet — that is the editor client, work item
//! 02-04 — so today the pool is empty unless a plugin's `[lsp]` section is
//! used by a later item.

use std::sync::Arc;

use tinkerpad_core::Scope;
use tinkerpad_lsp::LspPool;
use tokio::sync::mpsc;

use crate::events::EventBus;

/// Build the pool and forward its state changes to the UI.
pub fn pool(events: Arc<EventBus>) -> Arc<LspPool> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    tauri::async_runtime::spawn(async move {
        while let Some(event) = rx.recv().await {
            events.publish(Scope::App, event);
        }
    });
    Arc::new(LspPool::new(Some(tx)))
}
