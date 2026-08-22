import { i18n } from "./index";

/** Matches backend error codes such as `fs.trash_local_only` or `term.write_failed`. */
const ERROR_CODE_PATTERN = /^[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)+$/;

/**
 * Translates a backend user-facing message. The backend returns either a
 * stable code (`fs.trash_local_only`) or a code with a raw detail suffix
 * (`fs.terminal_launch_failed: Access is denied`); anything else (OS-level
 * io error text, for example) passes through untouched.
 */
export function translateBackendMessage(message: string): string {
  const separator = message.indexOf(": ");
  const code = separator === -1 ? message : message.slice(0, separator);
  if (!ERROR_CODE_PATTERN.test(code)) return message;

  const key = `errors:backend.${code.replaceAll(".", ".")}`;
  if (!i18n.exists(key)) return message;

  const base = i18n.t(key);
  if (separator === -1) return base;
  return i18n.t("errors:withDetail", {
    base,
    detail: message.slice(separator + 2),
  });
}

/** Friendly text for a `FileSystemError` kind (`not_found`, `io`, …). */
export function errorKindMessage(kind: string): string | null {
  const key = `errors:kind.${kind}`;
  return i18n.exists(key) ? i18n.t(key) : null;
}

/**
 * Renders any error thrown by a file-system command: known kinds get their
 * friendly text (plus the translated backend detail when present), anything
 * else falls back to the raw message.
 */
export function getFileOperationErrorMessage(error: unknown): string {
  const payload =
    typeof error === "object" && error !== null && !(error instanceof Error)
      ? (error as { kind?: unknown; message?: unknown })
      : null;
  const kind = payload && typeof payload.kind === "string" ? payload.kind : null;

  const friendly = kind ? errorKindMessage(kind) : null;
  if (friendly) {
    const detail =
      payload && typeof payload.message === "string" && payload.message
        ? translateBackendMessage(payload.message)
        : null;
    return detail ? i18n.t("errors:withDetail", { base: friendly, detail }) : friendly;
  }

  if (payload && typeof payload.message === "string" && payload.message) {
    return translateBackendMessage(payload.message);
  }
  if (error instanceof Error && error.message) return error.message;
  return String(error);
}
