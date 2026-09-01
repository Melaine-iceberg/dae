import { useEffect, useRef, useState, type ReactNode } from "react";

import { isWindowsPlatform } from "@/lib/platform";

import { getFileExtension } from "./file-icons";
import type { DirectoryEntry } from "./types";

/**
 * Extensions with a thumbnail producer on at least one platform: raster
 * formats decode in the Rust `image` pipeline everywhere, SVG streams
 * through as bytes for the webview to rasterize, and PDF/video/HEIC rely
 * on the Windows shell thumbnail handler (404 elsewhere falls back to the
 * type icon).
 */
const THUMBNAIL_EXTENSIONS = new Set([
  "jpg",
  "jpeg",
  "png",
  "gif",
  "webp",
  "bmp",
  "tif",
  "tiff",
  "ico",
  "svg",
  "pdf",
  "mp4",
  "m4v",
  "mov",
  "mkv",
  "webm",
  "avi",
  "wmv",
  "heic",
  "heif",
]);

/** Raster formats decoded by Rust everywhere; cheap regardless of size. */
const RASTER_EXTENSIONS = new Set([
  "jpg",
  "jpeg",
  "png",
  "gif",
  "webp",
  "bmp",
  "tif",
  "tiff",
  "ico",
]);

/**
 * Shell-produced thumbnails (PDF pages, video frames, HEIC) are capped
 * higher than the raster path but still bounded so a multi-gigabyte clip
 * never reaches the shell handler.
 */
const SHELL_THUMBNAIL_MAX_BYTES = 2 * 1024 * 1024 * 1024;

export function isThumbnailSupported(entry: DirectoryEntry): boolean {
  if (entry.kind !== "file") return false;
  const extension = getFileExtension(entry.name);
  if (!THUMBNAIL_EXTENSIONS.has(extension)) return false;
  if (RASTER_EXTENSIONS.has(extension) || extension === "svg") return true;
  return (entry.size ?? 0) <= SHELL_THUMBNAIL_MAX_BYTES;
}

/** Windows exposes Tauri custom schemes as `http://<scheme>.localhost`. */
const THUMBNAIL_URL_ORIGIN = isWindowsPlatform
  ? "http://thumbnail.localhost"
  : "thumbnail://localhost";

/**
 * Versioned URL for the `thumbnail://` protocol handler. Embedding mtime and
 * size lets the webview cache responses immutably and refresh automatically
 * when a file is replaced — no frontend promise cache needed.
 */
export function buildThumbnailUrl(entry: DirectoryEntry, size: number): string {
  const version = `${entry.modifiedAt ?? 0}-${entry.size ?? 0}`;
  return `${THUMBNAIL_URL_ORIGIN}/?path=${encodeURIComponent(entry.path)}&size=${size}&v=${version}`;
}

/**
 * Lazy thumbnail image: renders nothing but a subtle placeholder until the
 * element approaches the viewport, then loads through the custom protocol
 * (parallel fetches + browser cache, no base64 IPC payload). Formats whose
 * producer is platform-dependent (PDF/video/HEIC shell thumbnails) may 404;
 * the optional `fallback` node takes over in that case.
 */
export function ThumbnailImage({
  className,
  entry,
  fallback,
  requestSize,
}: {
  className?: string;
  entry: DirectoryEntry;
  fallback?: ReactNode;
  requestSize: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [isVisible, setIsVisible] = useState(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const [isFailed, setIsFailed] = useState(false);
  const thumbnailUrl = buildThumbnailUrl(entry, requestSize);

  useEffect(() => {
    setIsLoaded(false);
    setIsFailed(false);
  }, [thumbnailUrl]);

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

  return (
    <div className={className} ref={containerRef}>
      {isVisible && !isFailed && (
        <img
          alt=""
          className="h-full w-full object-contain"
          draggable={false}
          decoding="async"
          onError={() => setIsFailed(true)}
          onLoad={() => setIsLoaded(true)}
          src={thumbnailUrl}
        />
      )}
      {isFailed && fallback}
      {!isLoaded && !isFailed && (
        <div className="h-full w-full animate-pulse rounded-sm bg-muted" />
      )}
    </div>
  );
}
