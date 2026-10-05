import { useAtomValue } from "jotai";
import { useEffect, useState, type CSSProperties, type ReactNode } from "react";

import { isWindowsPlatform } from "@/lib/platform";
import { cn } from "@/lib/utils";

import { getFileExtension, hasKnownFileExtension } from "./file-icons";
import { iconStyleAtom } from "./preferences";
import type { DirectoryEntry } from "./types";

/** Extensions whose shell icon belongs to the individual file rather than to its
 *  type — an embedded icon resource, a shortcut's target, a `.url`'s site
 *  favicon. Windows is the only desktop that draws two files of the same
 *  extension differently, so it is the only one that has to ask for these by
 *  path; everywhere else an extension's icon is shared by every file with it. */
const FILE_SPECIFIC_ICON_EXTENSIONS = new Set(["exe", "msi", "lnk", "url", "dll", "scr", "cpl"]);

/** Windows exposes Tauri custom schemes as `http://<scheme>.localhost`. */
const FILE_ICON_URL_ORIGIN = isWindowsPlatform
  ? "http://fileicon.localhost"
  : "fileicon://localhost";

/**
 * Whether this row should draw the operating system's icon for its entry.
 *
 * A hook because the answer is a preference. It takes a nullable entry so a
 * caller with an optional one (`entry-preview.tsx`) can still call it
 * unconditionally.
 *
 * `"system"` has no platform gate to apply: every platform now answers
 * `fileicon://` for files and folders alike, and when the shell has nothing for
 * a path the protocol 404s and `NativeIconImage` keeps the drawn glyph.
 */
export function useNativeIconFor(entry: DirectoryEntry | null | undefined): boolean {
  const iconStyle = useAtomValue(iconStyleAtom);
  if (!entry) {
    return false;
  }
  if (iconStyle === "system") {
    return entry.kind === "file" || entry.kind === "directory";
  }
  return usesShellIconWhereNoGlyphExists(entry);
}

/**
 * The rule that predates the preference, and what `"themed"` still means:
 * OS icons take over for app-like files and for extensions the built-in type
 * table does not know — the Windows shell usually has a registered handler
 * icon there. Known categories keep their toned type glyphs, which is what
 * makes the listing read as one design.
 */
function usesShellIconWhereNoGlyphExists(entry: DirectoryEntry): boolean {
  if (!isWindowsPlatform || entry.kind !== "file") {
    return false;
  }

  const extension = getFileExtension(entry.name);
  if (!extension) {
    return false;
  }

  return FILE_SPECIFIC_ICON_EXTENSIONS.has(extension) || !hasKnownFileExtension(extension);
}

/**
 * The URL the webview fetches this row's icon from.
 *
 * The sharing rule is the whole reason the listing scrolls smoothly. An entry
 * whose icon is a property of its *type* asks for the type — `?ext=rs`, with no
 * path and no version in it — so a column of 800 `.rs` files is one URL, the
 * webview's own immutable cache answers 799 of the 800 without leaving the
 * process, and the shell is asked once. Everything that cannot share its answer
 * asks by path and versions the URL with the mtime and size the listing already
 * reported, so a replaced entry reads as a new URL and fetches again.
 *
 * The version travels in the URL rather than being read off the disk here for the
 * same reason: the row exists because something already stat'd this file.
 */
export function buildFileIconUrl(entry: DirectoryEntry, size: number): string {
  const extension = sharedIconExtension(entry);
  if (extension) {
    return `${FILE_ICON_URL_ORIGIN}/?ext=${encodeURIComponent(extension)}&size=${size}`;
  }

  const version = `${entry.modifiedAt ?? 0}-${entry.size ?? 0}`;
  const directory = entry.kind === "directory" ? "&dir=1" : "";
  return `${FILE_ICON_URL_ORIGIN}/?path=${encodeURIComponent(entry.path)}${directory}&size=${size}&v=${version}`;
}

/** The extension every row of this entry's type shares an icon with, or `""` when
 *  the icon is this entry's own and only a path can name it. */
function sharedIconExtension(entry: DirectoryEntry): string {
  if (entry.kind !== "file") {
    // A directory carries a per-folder icon wherever the desktop can hold one —
    // a dropped `.icon` on macOS, a `desktop.ini` on Windows — so even where the
    // answer happens to be the same glyph for every folder, only the path knows.
    return "";
  }

  const extension = getFileExtension(entry.name);
  // An extension-less Unix binary is named by its permissions rather than by a
  // type, and the backend reads the executable bit off the file itself.
  if (!extension) {
    return "";
  }

  return isWindowsPlatform && FILE_SPECIFIC_ICON_EXTENSIONS.has(extension) ? "" : extension;
}

/**
 * URL for the icon theme's own icon for a name — a `.desktop`'s `Icon=`, which may
 * equally be an absolute path — rather than for a file or a type. Same handler and
 * same render pool as {@link buildFileIconUrl}; on a platform whose shell keys
 * icons on a file the request 404s, so a caller needs the same fallback.
 */
export function buildNamedIconUrl(name: string, size: number): string {
  return `${FILE_ICON_URL_ORIGIN}/?name=${encodeURIComponent(name)}&size=${size}`;
}

/**
 * The OS-native icon for one entry: the Solar fallback draws until the shell icon
 * arrives, and stays for good on any error (a missing path, a dead shortcut
 * target, a platform whose shell has nothing for this file), so every slot always
 * renders something.
 *
 * Nothing waits for visibility here. Every view that draws a row of these is
 * virtualized, so the mounted rows already are the ones at or near the viewport,
 * and an `IntersectionObserver` per row was a second filter over a handful of
 * overscan cells — the same question the list had already answered, 800 times.
 */
export function NativeIconImage({
  className,
  entry,
  fallback,
  pixelSize,
}: {
  className?: string;
  entry: DirectoryEntry;
  fallback: ReactNode;
  pixelSize: number;
}) {
  const [isLoaded, setIsLoaded] = useState(false);
  const [isFailed, setIsFailed] = useState(false);
  // Ask for 2x so HiDPI displays get a crisp bitmap; the shell caps larger
  // requests at its biggest stock size anyway.
  const iconUrl = buildFileIconUrl(entry, Math.min(pixelSize * 2, 256));
  const dimension: CSSProperties = { width: pixelSize, height: pixelSize };

  useEffect(() => {
    setIsLoaded(false);
    setIsFailed(false);
  }, [iconUrl]);

  if (isLoaded && !isFailed) {
    return (
      <img
        alt=""
        className={className}
        decoding="async"
        draggable={false}
        src={iconUrl}
        style={dimension}
      />
    );
  }

  return (
    <span
      className={cn("inline-flex shrink-0 items-center justify-center", className)}
      style={dimension}
    >
      {fallback}
      {!isFailed && (
        <img
          alt=""
          className="hidden"
          decoding="async"
          draggable={false}
          onError={() => setIsFailed(true)}
          onLoad={() => setIsLoaded(true)}
          src={iconUrl}
        />
      )}
    </span>
  );
}
