import { error as logError, warn as logWarn } from "@tauri-apps/plugin-log";

/**
 * Forwards the webview's own failures into the backend log file.
 *
 * The release build has no console anyone reads — it is a GUI subsystem binary,
 * so its stdout goes nowhere (see the log targets and `install_panic_logger` in
 * `src-tauri/src/lib.rs`) — which makes a `console.error` in here a message into
 * the void. Patching the two levels the app reports through covers every
 * existing call site without touching any of them.
 *
 * `debug`, `info`, and `log` are left alone on purpose: React, the bundler, and
 * the dev server emit plenty of them, and none of it earns a line on disk.
 */
export function forwardConsoleToLogFile(): void {
  patchConsole("warn", logWarn);
  patchConsole("error", logError);

  // Failures that never reach a `console.error`: an exception thrown from an
  // event listener, a rejected promise nobody awaited. React reports uncaught
  // render errors through `reportError`, so those arrive here too.
  //
  // Neither listener uses the capture phase, which keeps resource load errors
  // (a thumbnail that 404s) out of the log — those do not bubble.
  window.addEventListener("error", (event) => {
    report(logError, `Uncaught error: ${describe(event.error ?? event.message)}`);
  });
  window.addEventListener("unhandledrejection", (event) => {
    report(logError, `Unhandled rejection: ${describe(event.reason)}`);
  });
}

type Level = "warn" | "error";
type Sink = (message: string) => Promise<void>;

/** Longest single rendered argument before the rest is dropped (see `truncate`). */
const MAX_ARGUMENT_CHARS = 4_000;

function patchConsole(level: Level, sink: Sink): void {
  const original = console[level].bind(console);

  console[level] = (...args: unknown[]) => {
    original(...args);
    report(sink, args.map(describe).join(" "));
  };
}

/**
 * Sends one line, swallowing a failure to send it.
 *
 * Without the `catch`, the logger would be the thing that breaks the app it
 * exists to diagnose: a rejected `invoke` would surface as an unhandled
 * rejection, which the listener above would try to log, and so on.
 */
function report(sink: Sink, message: string): void {
  sink(message).catch(() => {});
}

/** Renders one `console.*` argument as text, preferring the detailed form. */
function describe(argument: unknown): string {
  return truncate(render(argument));
}

function render(argument: unknown): string {
  if (argument instanceof Error) return argument.stack ?? `${argument.name}: ${argument.message}`;
  if (typeof argument === "string") return argument;
  if (typeof argument === "object" && argument !== null) {
    try {
      // `stringify` answers `undefined` for a function or a symbol value, so
      // the fallback below is not dead code.
      return JSON.stringify(argument) ?? String(argument);
    } catch {
      // Circular structures and objects whose `toJSON` throws land here; a
      // rough rendering beats losing the line.
      return String(argument);
    }
  }
  return String(argument);
}

/**
 * Caps one rendered argument.
 *
 * Without this a single oversized argument — a big array, a function's source —
 * fills a whole log file by itself and pushes out the entries around it, which
 * are the ones worth reading. The serialisation still runs to completion, so
 * this bounds the file rather than the cost of writing to it.
 */
function truncate(text: string): string {
  if (text.length <= MAX_ARGUMENT_CHARS) return text;
  const dropped = text.length - MAX_ARGUMENT_CHARS;
  return `${text.slice(0, MAX_ARGUMENT_CHARS)}… (${dropped} more chars)`;
}
