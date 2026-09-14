// The two pipes to the core: typed commands out, one event stream in.

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import { type CommandError, type Envelope, EVENT_NAME } from "./protocol";
import { toast } from "./toast";

type Handler = (env: Envelope) => void;

const handlers = new Set<Handler>();
let lastSeq = 0;
let listening: Promise<unknown> | null = null;

/**
 * Receive every core event, in order. Call as many times as needed; there is
 * only ever one underlying Tauri listener. Returns an unsubscribe function.
 */
export function subscribe(handler: Handler): () => void {
  handlers.add(handler);
  listening ??= listen<Envelope>(EVENT_NAME, ({ payload }) => {
    if (payload.seq <= lastSeq) {
      console.warn(`event out of order: ${payload.seq} after ${lastSeq}`);
    }
    lastSeq = payload.seq;
    for (const h of handlers) h(payload);
  });
  return () => handlers.delete(handler);
}

/** Resolves once the event listener is registered. */
export function ready(): Promise<unknown> {
  return listening ?? Promise.resolve();
}

function isCommandError(e: unknown): e is CommandError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e;
}

/**
 * Invoke a core command. Failures are normalised to a CommandError, shown as
 * a toast, and rethrown so callers can still react.
 */
export async function invoke<T = void>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (e) {
    const error: CommandError = isCommandError(e)
      ? e
      : { code: "internal", message: String(e) };
    toast("error", error.message);
    throw error;
  }
}
