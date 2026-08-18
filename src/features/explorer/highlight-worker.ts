/**
 * Dedicated highlight worker: keeps shiki, its wasm engine and loaded
 * grammars off the main thread so highlighting never blocks UI work.
 *
 * The message loop coalesces requests with latest-wins semantics — while a
 * highlight is running, only the newest queued request survives. Older ones
 * were already settled with `null` by the client side.
 */
import {
  highlightText,
  type HighlightRequestMessage,
  type HighlightResponseMessage,
} from "./code-highlight";

// `self` is the DedicatedWorkerGlobalScope here; the DOM-lib `self`
// (Window) lacks the single-argument postMessage signature, so cast it.
const ctx = self as unknown as {
  postMessage(message: HighlightResponseMessage): void;
};

let queuedRequest: HighlightRequestMessage | null = null;
let isDraining = false;

self.onmessage = (event: MessageEvent<HighlightRequestMessage>) => {
  const message = event.data;
  if (message?.type !== "highlight") return;
  queuedRequest = message;
  void drainQueue();
};

async function drainQueue(): Promise<void> {
  if (isDraining) return;
  isDraining = true;
  try {
    while (queuedRequest !== null) {
      const request = queuedRequest;
      queuedRequest = null;
      try {
        const html = await highlightText(request.code, request.language);
        ctx.postMessage({ type: "result", id: request.id, html });
      } catch (error) {
        ctx.postMessage({
          type: "error",
          id: request.id,
          message: error instanceof Error ? error.message : String(error),
        });
      }
    }
  } finally {
    isDraining = false;
  }
}
