// Wire types shared with the Rust core.
//
// Hand-written mirror of `tinkerpad-core/src/events.rs` and `error.rs`.
// Keep them in step. If they start to drift, generate this file from the Rust
// types (ts-rs or specta) instead of editing by hand.

/** What an event is about: "app" or "run:<id>". */
export type Scope = "app" | `run:${number}`;

export type Level = "info" | "warning" | "error";

export type RunEvent =
  | { kind: "started" }
  | { kind: "stdout"; text: string }
  | { kind: "stderr"; text: string }
  | { kind: "exited"; code: number | null; durationMs: number }
  | { kind: "cancelled"; durationMs: number }
  | { kind: "failed"; message: string };

export type CoreEvent =
  | { type: "pong"; readyMs: number }
  | { type: "toast"; level: Level; message: string }
  | ({ type: "run" } & RunEvent);

export interface Envelope {
  /** Strictly increasing per app session. */
  seq: number;
  scope: Scope;
  event: CoreEvent;
}

export type ErrorCode = "notFound" | "invalidInput" | "io" | "internal";

/** What every command rejects with. */
export interface CommandError {
  code: ErrorCode;
  message: string;
}

/** Name of the single Tauri event carrying every Envelope. */
export const EVENT_NAME = "core-event";

export type PluginSource = "bundled" | "user";

/** Reply to the `list_plugins` command. */
export interface PluginsStatus {
  bundledDir: string | null;
  userDir: string;
  loadMs: number;
  plugins: {
    id: string;
    name: string;
    extension: string;
    source: PluginSource;
    dir: string;
  }[];
  failures: { dir: string; source: PluginSource; message: string }[];
}
