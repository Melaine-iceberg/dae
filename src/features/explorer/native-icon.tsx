import { useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";

import { isWindowsPlatform } from "@/lib/platform";
import { cn } from "@/lib/utils";

import { getFileExtension, hasKnownFileExtension } from "./file-icons";
import type { DirectoryEntry } from "./types";

/** Application-like types whose shell icon is always more informative than
 *  a generic glyph (shortcut targets, embedded executable icons, ...). */
const NATIVE_ICON_EXTENSIONS = new Set(["exe", "msi", "lnk", "url", "dll", "scr", "cpl"]);

/** Windows exposes Tauri custom schemes as `http://<scheme>.localhost`. */
const FILE_ICON_URL_ORIGIN = isWindowsPlatform
  ? "http://fileicon.localhost"
  : "fileicon://localhost";

/**
 * OS icons take over for app-like files and for extensions the built-in map
 * does not know — the shell usually has a registered handler icon there.
 * Known categories keep their toned Phosphor glyphs for a consistent design.
 */
export function isNativeIconSupported(entry: DirectoryEntry): boolean {
  if (!isWindowsPlatform || entry.kind !== "file") {
    return false;
  }

  const extension = getFileExtension(entry.name);
  if (!extension) {
    return false;
  }

  return NATIVE_ICON_EXTENSIONS.has(extension) || !hasKnownFileExtension(extension);
}

/**
 * Versioned URL for the `fileicon://` protocol handler. Same immutable-cache
 * trick as thumbnails: mtime + size in the URL lets the webview cache the
 * response and refresh automatically when the file is replaced.
 */
export function buildFileIconUrl(entry: DirectoryEntry, size: number): string {
  const version = `${entry.modifiedAt ?? 0}-${entry.size ?? 0}`;
  return `${FILE_ICON_URL_ORIGIN}/?path=${encodeURIComponent(entry.path)}&size=${size}&v=${version}`;
}

/**
 * Lazy OS-native icon: shows the Phosphor fallback until the shell icon
 * arrives and keeps it forever on any error (missing path, dead shortcut
 * target, non-Windows platform), so every slot always renders something.
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
  const containerRef = useRef<HTMLSpanElement>(null);
  const [isVisible, setIsVisible] = useState(false);
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

  useEffect(() => {
    const element = containerRef.current;
    if (!element) return;

    if (typeof IntersectionObserver === "undefined") {
      setIsVisible(true);
      return;
    }

    const observer = new IntersectionObserver(
      (observed) => {
        for (const item of observed) {
          if (item.isIntersecting) {
            setIsVisible(true);
            observer.disconnect();
          }
        }
      },
      { rootMargin: "200px" },
    );
    observer.observe(element);
    return () => observer.disconnect();
  }, []);

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
      ref={containerRef}
      style={dimension}
    >
      {fallback}
      {isVisible && !isFailed && (
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
