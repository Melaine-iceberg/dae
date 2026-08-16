import { useEffect, useRef, useState } from "react";

import { commands, type Thumbnail } from "@/bindings";

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

/**
 * Module-level in-flight/result cache. Keys include mtime and size so
 * replaced files refresh naturally; the Rust side keeps its own cache too.
 */
const thumbnailCache = new Map<string, Promise<Thumbnail | null>>();
const THUMBNAIL_CACHE_MAX = 300;

export function loadThumbnail(entry: DirectoryEntry, size: number): Promise<Thumbnail | null> {
  const cacheKey = `${entry.path}|${entry.modifiedAt ?? 0}|${entry.size ?? 0}|${size}`;
  const cached = thumbnailCache.get(cacheKey);
  if (cached) return cached;

  if (thumbnailCache.size >= THUMBNAIL_CACHE_MAX) {
    thumbnailCache.clear();
  }

  const pending: Promise<Thumbnail | null> = commands
    .getThumbnail(entry.path, size)
    .then((result) => result ?? null)
    .catch((error: unknown) => {
      console.warn(`Unable to render thumbnail for ${entry.path}`, error);
      thumbnailCache.delete(cacheKey);
      return null;
    });

  thumbnailCache.set(cacheKey, pending);
  return pending;
}

/**
 * Lazy thumbnail image: renders nothing but a subtle placeholder until the
 * element approaches the viewport (SKILL.md §49 lazy + cached thumbnails).
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
  const [thumbnail, setThumbnail] = useState<Thumbnail | null>(null);
  const [isVisible, setIsVisible] = useState(false);

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

  useEffect(() => {
    if (!isVisible) return;

    let cancelled = false;
    void loadThumbnail(entry, requestSize).then((result) => {
      if (!cancelled) setThumbnail(result);
    });
    return () => {
      cancelled = true;
    };
  }, [entry, isVisible, requestSize]);

  return (
    <div className={className} ref={containerRef}>
      {thumbnail ? (
        <img
          alt=""
          className="h-full w-full object-contain"
          draggable={false}
          src={thumbnail.dataUrl}
        />
      ) : (
        <div className="h-full w-full animate-pulse rounded-sm bg-muted" />
      )}
    </div>
  );
}
