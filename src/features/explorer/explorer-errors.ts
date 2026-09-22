/**
 * Pure helpers shared by the explorer's dialog flows and error surfaces.
 *
 * Extracted from `explorer-view.tsx` so the view only orchestrates: nothing
 * in here touches React state, the DOM, or the backend.
 */
import { i18n } from "@/i18n";

/** True when the backend reported an encrypted-archive password failure. */
export function isWrongPasswordError(error: unknown): boolean {
  return (
    typeof error === "object" &&
    error !== null &&
    "kind" in error &&
    (error as { kind?: unknown }).kind === "wrong_password"
  );
}

/** Final path segment used as the dialog's display name. */
export function displayNameOfPath(path: string): string {
  const segments = path.split(/[\\/]/);
  return segments[segments.length - 1] || path;
}

export function pathListsEqual(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((path, index) => path === b[index]);
}

export function getCreateEntryErrorMessage(message: string, rawError: unknown): string {
  const kind =
    typeof rawError === "object" &&
    rawError !== null &&
    "kind" in rawError &&
    typeof (rawError as { kind?: unknown }).kind === "string"
      ? (rawError as { kind: string }).kind
      : null;

  switch (kind) {
    case "already_exists":
      return i18n.t("explorer:createEntryError.alreadyExists");
    case "permission_denied":
      return i18n.t("explorer:createEntryError.permissionDenied");
    case "not_found":
      return i18n.t("explorer:createEntryError.notFound");
    case "not_directory":
      return i18n.t("explorer:createEntryError.notDirectory");
  }

  if (message.includes("must not contain a path separator")) {
    return i18n.t("explorer:createEntryError.pathSeparator");
  }

  return message;
}
