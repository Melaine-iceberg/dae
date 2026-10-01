import type { MarkdownProps } from "@tanstack/markdown/react";

/**
 * Markdown ships destinations that are relative to the document, but the
 * preview renders inside the app's own webview document, so a relative href
 * resolves against the app instead of the file. TanStack Markdown's
 * `urlTransform` runs while parsing, so the rendered anchor already carries a
 * filesystem path and the click handler needs no path maths of its own.
 */

/**
 * Two-letter minimum: `[x](C:\notes.md)` is a drive-absolute path, not a
 * `c:` scheme. TanStack Markdown's own allowlist blanks every scheme it does
 * not recognise before this hook is consulted.
 */
const SCHEME = /^[a-z][a-z0-9+.-]+:/i;

/** Percent-decodes a destination segment, keeping the raw text if malformed. */
function decodeSegment(value: string): string {
  if (!value.includes("%")) return value;
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function isRootSegment(value: string | undefined): boolean {
  return value === undefined || value === "" || /^[a-z]:$/i.test(value);
}

/**
 * Resolves a document-relative destination against the source file, folding
 * `.` and `..` and dropping a `#fragment`. Returns null when nothing is left
 * to open, which makes the renderer emit plain text instead of a link.
 */
export function resolveLocalTarget(sourcePath: string, destination: string): string | null {
  // Angle-quoted destinations may contain raw newlines and tabs, which must
  // not survive into a path handed to the OS.
  // eslint-disable-next-line no-control-regex
  const withoutFragment = destination.replace(/[\u0000-\u001F\u007F]+/g, "").split("#")[0];
  if (!withoutFragment) return null;

  const separator = sourcePath.includes("\\") ? "\\" : "/";
  const parts = withoutFragment.replace(/\\/g, "/").split("/");
  const driveRooted = /^[a-z]:$/i.test(parts[0] ?? "");
  const rooted = driveRooted || withoutFragment.startsWith("/");
  // An absolute destination only means something in the document's own path
  // style; a `C:\…` link inside a POSIX document, or the reverse, has no
  // answer worth inventing.
  if (rooted && driveRooted !== (separator === "\\")) return null;
  const segments = rooted ? [] : sourcePath.split(/[\\/]/).slice(0, -1);

  for (const rawPart of parts) {
    const part = decodeSegment(rawPart);
    if (!part || part === ".") continue;
    if (part === "..") {
      if (!isRootSegment(segments[segments.length - 1]) && segments.length > 0) segments.pop();
      continue;
    }
    segments.push(part);
  }

  if (segments.length === 0) return null;
  const joined = segments.join(separator);
  return rooted && separator === "/" ? `/${joined}` : joined;
}

/** Builds the renderer's URL hook for one previewed document. */
export function createMarkdownUrlTransform(
  sourcePath: string,
): NonNullable<MarkdownProps["urlTransform"]> {
  return (url, kind, defaultUrl) => {
    // Images keep the sanitised destination: showing them needs the asset
    // protocol, which this app has not enabled.
    if (kind === "image" || url.startsWith("#") || SCHEME.test(url)) return defaultUrl;
    return resolveLocalTarget(sourcePath, url);
  };
}
