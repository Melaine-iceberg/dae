import { useEffect, useRef, useState } from "react";

import { isWindowsPlatform } from "@/lib/platform";

import { getFileExtension } from "./file-icons";
import type { DirectoryEntry } from "./types";

/** Extensions decodable by the Rust `image` pipeline on every platform. */
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
]);

export function isThumbnailSupported(entry: DirectoryEntry): boolean {
  return entry.kind === "file" && THUMBNAIL_EXTENSIONS.has(getFileExtension(entry.name));
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
 * (parallel fetches + browser cache, no base64 IPC payload).
 */
export function ThumbnailImage({
  className,
  entry,
  requestSize,
}: {
  className?: string;
  entry: DirectoryEntry;
  requestSize: number;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [isVisible, setIsVisible] = useState(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const thumbnailUrl = buildThumbnailUrl(entry, requestSize);

  useEffect(() => {
    setIsLoaded(false);
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
      {isVisible && (
        <img
          alt=""
          className="h-full w-full object-contain"
          draggable={false}
          decoding="async"
          onLoad={() => setIsLoaded(true)}
          src={thumbnailUrl}
        />
      )}
      {!isLoaded && <div className="h-full w-full animate-pulse rounded-sm bg-muted" />}
    </div>
  );
}
