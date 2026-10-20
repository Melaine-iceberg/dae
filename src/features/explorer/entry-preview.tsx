import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  ClipboardListIcon,
  CloseIcon,
  DangerTriangleIcon,
  FolderOpenIcon,
  SquareDashedIcon,
} from "@solar-icons/react/linear";
import { openPath } from "@tauri-apps/plugin-opener";
import { Markdown } from "@tanstack/markdown/react";

import { commands, type MediaPreview, type TextPreview } from "@/bindings";
import { localeDateTimeFormat, localeNumber, localeNumberFormat } from "@/i18n/format";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { copyWithNotice } from "@/lib/notifications";

import { getPreviewLanguage, highlightCode, highlightMarkdownCode } from "./code-highlight";
import { getEntryPresentation } from "./file-icons";
import { createMarkdownUrlTransform } from "./markdown-links";
import { NativeIconImage, useNativeIconFor } from "./native-icon";
import { isThumbnailSupported, ThumbnailImage } from "./thumbnail";
import type { DirectoryEntry } from "./types";

/** Extensions offered a plain-text peek (no grammar, e.g. logs and CSV). */
const TEXT_PREVIEW_EXTENSIONS = new Set([
  "txt",
  "log",
  "csv",
  "bat",
  "cmd",
  "env",
  "gitignore",
  "editorconfig",
  "properties",
  "diff",
  "patch",
  "lock",
]);

/** Extensions rendered as Markdown instead of a plain-text peek. */
const MARKDOWN_EXTENSIONS = new Set(["md", "markdown"]);

/**
 * Audio/video containers probed for metadata. The backend parses container
 * headers only (a few KB of reads), so this stays cheap even for huge
 * clips — no decoding or frame extraction happens in Rust.
 */
const MEDIA_EXTENSIONS = new Set([
  "mp3",
  "flac",
  "wav",
  "m4a",
  "aac",
  "ogg",
  "opus",
  "wma",
  "aiff",
  "aif",
  "mp4",
  "m4v",
  "mov",
  "mkv",
  "webm",
  "avi",
  "wmv",
  "flv",
]);

/** Files above this size skip content preview entirely for performance. */
const PREVIEW_MAX_SOURCE_BYTES = 512 * 1024;
const PREVIEW_READ_BYTES = 64 * 1024;
/**
 * Only this much text goes through the highlighter: the preview viewport
 * shows ~20 lines, so highlighting the full 64KB read would build a large
 * DOM tree for invisible content.
 */
const HIGHLIGHT_MAX_BYTES = 16 * 1024;

function sliceForHighlight(content: string): string {
  if (content.length <= HIGHLIGHT_MAX_BYTES) return content;
  const slice = content.slice(0, HIGHLIGHT_MAX_BYTES);
  const newlineIndex = slice.lastIndexOf("\n");
  return newlineIndex > 0 ? slice.slice(0, newlineIndex) : slice;
}

const MODIFIED_DATE_FORMAT_OPTIONS: Intl.DateTimeFormatOptions = {
  dateStyle: "medium",
  timeStyle: "short",
};

const FILE_SIZE_FORMAT_OPTIONS: Intl.NumberFormatOptions = { maximumFractionDigits: 1 };
const FILE_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

function formatFileSize(size: number | null): string {
  if (size === null) return "—";
  if (size === 0) return "0 B";

  const unitIndex = Math.min(
    Math.floor(Math.log(size) / Math.log(1024)),
    FILE_SIZE_UNITS.length - 1,
  );
  const formatter = localeNumberFormat(FILE_SIZE_FORMAT_OPTIONS);
  return `${formatter.format(size / 1024 ** unitIndex)} ${FILE_SIZE_UNITS[unitIndex]}`;
}

/** `formatFileSize` split at its unit, so a total can set the number at the
 *  panel's title step and the unit at caption step on the same baseline. */
function splitFileSize(size: number): { amount: string; unit: string } {
  const formatted = formatFileSize(size);
  const separator = formatted.lastIndexOf(" ");
  return { amount: formatted.slice(0, separator), unit: formatted.slice(separator + 1) };
}

function parentDirectory(path: string): string {
  const separatorIndex = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (separatorIndex <= 0) return path;
  return path.slice(0, separatorIndex);
}

function formatDuration(durationMs: number): string {
  const totalSeconds = Math.floor(durationMs / 1000);
  const hours = Math.floor(totalSeconds / 3600);
  const minutes = Math.floor((totalSeconds % 3600) / 60);
  const seconds = totalSeconds % 60;
  const pad = (value: number) => value.toString().padStart(2, "0");
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds)}` : `${minutes}:${pad(seconds)}`;
}

function formatBitrate(bitrateBps: number): string {
  if (bitrateBps >= 1_000_000) {
    return `${localeNumberFormat(FILE_SIZE_FORMAT_OPTIONS).format(bitrateBps / 1_000_000)} Mbps`;
  }
  return `${Math.round(bitrateBps / 1000)} kbps`;
}

type TextPreviewState =
  | { status: "loading" }
  | { status: "error" }
  | {
      status: "ready";
      content: string;
      html: string | null;
      truncated: boolean;
    };

type MediaPreviewState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; data: MediaPreview };

/** The preview panel's props, imported by the lazy boundary in `explorer-view`. */
export interface EntryPreviewProps {
  /** The selection, in listing order. One entry previews its contents; several
   *  are summarized instead of the first one standing in for the rest. */
  entries: DirectoryEntry[];
  onClose: () => void;
  onOpen: () => void;
}

/**
 * Docked yazi-style preview panel for the current selection (SKILL.md §21):
 * one selection shows content (thumbnail / highlighted code / text peek) plus
 * metadata and follow-up actions, while a larger one shows what it adds up to
 * — the panel used to describe only its first entry even though its own open
 * button acted on every selected one. Large files skip content preview.
 */
export function EntryPreview({ entries, onClose, onOpen }: EntryPreviewProps) {
  const { t } = useTranslation("explorer");
  const isMultiSelection = entries.length > 1;
  // Everything below is written for exactly one entry: the multi-selection
  // summary takes the panel over before any of the preview pipeline runs, and
  // a null entry is what keeps it from fetching on top of the summary.
  const entry = entries.length === 1 ? entries[0] : null;
  const visualExtension = entry?.kind === "file" ? getEntryVisualExtension(entry.name) : "";
  // Markdown gets rendered styling instead of a code grammar.
  const supportsMarkdown = MARKDOWN_EXTENSIONS.has(visualExtension);
  const language =
    entry?.kind === "file" && !supportsMarkdown ? getPreviewLanguage(entry.name) : null;
  const supportsThumbnail = entry !== null && isThumbnailSupported(entry);
  const showNativeIcon = useNativeIconFor(entry);
  // Markdown rides the text-read path even though it skips code highlighting.
  const supportsText =
    entry?.kind === "file" &&
    (supportsMarkdown || language !== null || TEXT_PREVIEW_EXTENSIONS.has(visualExtension));
  const supportsMedia = entry?.kind === "file" && MEDIA_EXTENSIONS.has(visualExtension);
  const isTooLarge = (entry?.size ?? 0) > PREVIEW_MAX_SOURCE_BYTES;
  const [textPreview, setTextPreview] = useState<TextPreviewState | null>(null);
  const [mediaPreview, setMediaPreview] = useState<MediaPreviewState | null>(null);
  // Links in a rendered document are relative to that document, not the app.
  const markdownUrlTransform = useMemo(
    () => (entry ? createMarkdownUrlTransform(entry.path) : undefined),
    [entry],
  );

  useEffect(() => {
    if (!entry || entry.kind !== "file" || isTooLarge || !supportsText) {
      setTextPreview(null);
      return;
    }

    setTextPreview({ status: "loading" });
    let cancelled = false;
    void commands
      .readTextPreview(entry.path, PREVIEW_READ_BYTES)
      .then((result: TextPreview) => {
        let html: string | null = null;
        if (language !== null) {
          // Highlighting is best-effort: on failure the raw text still
          // renders as plain content.
          try {
            html = highlightCode(sliceForHighlight(result.content), language);
          } catch {
            html = null;
          }
        }
        if (cancelled) return;
        setTextPreview({
          status: "ready",
          content: result.content,
          html,
          truncated: result.truncated,
        });
      })
      .catch((error: unknown) => {
        console.warn(`Unable to read text preview for ${entry.path}`, error);
        if (!cancelled) setTextPreview({ status: "error" });
      });
    return () => {
      cancelled = true;
    };
  }, [entry, isTooLarge, language, supportsText]);

  useEffect(() => {
    if (!entry || entry.kind !== "file" || !supportsMedia) {
      setMediaPreview(null);
      return;
    }

    setMediaPreview({ status: "loading" });
    let cancelled = false;
    void commands
      .readMediaPreview(entry.path)
      .then((result: MediaPreview) => {
        if (!cancelled) setMediaPreview({ status: "ready", data: result });
      })
      .catch((error: unknown) => {
        console.warn(`Unable to read media metadata for ${entry.path}`, error);
        if (!cancelled) setMediaPreview({ status: "error" });
      });
    return () => {
      cancelled = true;
    };
  }, [entry, supportsMedia]);

  const visual = entry ? getEntryPresentation(entry) : null;
  const VisualIcon = visual?.icon;

  function handleMarkdownLinkClick(event: MouseEvent<HTMLDivElement>) {
    const anchor = (event.target as Element).closest("a");
    if (!anchor) return;
    const href = anchor.getAttribute("href");
    // Fragments scroll within the preview, and scheme links never reach here:
    // the app-wide guard hands those to the system and stops propagating.
    if (!href || href.startsWith("#")) return;
    // Every relative destination was already resolved against this document by
    // the renderer, and the guard keeps the webview from navigating anywhere.
    void openPath(href).catch((error) => {
      console.warn(`Unable to open link ${href}`, error);
    });
  }

  return (
    <aside
      aria-label={t("preview.ariaLabel")}
      className="flex h-full w-preview shrink-0 flex-col overflow-hidden border-l border-border bg-card"
    >
      <header className="flex h-toolbar shrink-0 items-center gap-2.5 border-b border-border px-4">
        {visual && VisualIcon ? (
          <span className="flex shrink-0">
            <VisualIcon className="size-[17px] shrink-0 text-muted-foreground" />
          </span>
        ) : null}
        <p
          className="min-w-0 flex-1 truncate text-body font-medium"
          title={entry?.name ?? undefined}
        >
          {isMultiSelection
            ? t("preview.multiSelection", { number: localeNumber(entries.length) })
            : (entry?.name ?? t("preview.noFileSelected"))}
        </p>
        <Button
          aria-label={t("preview.collapsePanel")}
          onClick={onClose}
          size="icon-sm"
          title={t("preview.collapsePanelShortcut")}
          type="button"
          variant="ghost"
        >
          <CloseIcon />
        </Button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3.5 overflow-y-auto p-4">
        {isMultiSelection ? (
          <SelectionSummary entries={entries} />
        ) : entry === null ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2.5 px-4 text-center text-caption text-muted-foreground">
            <SquareDashedIcon className="size-8 shrink-0 text-muted-foreground" />
            <p>{t("preview.selectHint")}</p>
          </div>
        ) : (
          <>
            {supportsThumbnail ? (
              <ThumbnailImage
                className="flex h-64 shrink-0 items-center justify-center overflow-hidden rounded-xl bg-muted ring-1 ring-border/60"
                entry={entry}
                fallback={VisualIcon ? <VisualIcon className="size-12" /> : null}
                displaySize={384}
              />
            ) : textPreview?.status === "ready" ? (
              <div className="flex min-h-48 min-w-0 flex-1 flex-col overflow-hidden rounded-xl bg-muted">
                {supportsMarkdown ? (
                  <div
                    className="markdown-preview min-h-0 flex-1 overflow-auto p-2.5 text-caption leading-relaxed"
                    onClickCapture={handleMarkdownLinkClick}
                  >
                    {/* Rendered locally from file contents via TanStack Markdown. */}
                    <Markdown
                      highlighter={highlightMarkdownCode}
                      urlTransform={markdownUrlTransform}
                    >
                      {textPreview.content}
                    </Markdown>
                  </div>
                ) : textPreview.html !== null ? (
                  <div
                    className="code-preview min-h-0 flex-1 overflow-auto p-2.5 text-caption leading-relaxed"
                    // Highlighted markup is generated locally from file contents; TanStack Highlight escapes all text nodes (escapeHtml in its core) before tokenizing.
                    // pi-lens-ignore: dangerously-set-inner-html
                    dangerouslySetInnerHTML={{ __html: textPreview.html }}
                  />
                ) : (
                  <pre className="min-h-0 flex-1 overflow-auto p-2.5 text-caption leading-relaxed whitespace-pre-wrap [overflow-wrap:anywhere]">
                    {textPreview.content}
                  </pre>
                )}
                {textPreview.truncated && (
                  <p className="shrink-0 border-t px-2.5 py-1.5 text-caption text-muted-foreground">
                    {t("preview.truncatedHint", { size: PREVIEW_READ_BYTES / 1024 })}
                  </p>
                )}
              </div>
            ) : textPreview?.status === "loading" ? (
              <div className="flex shrink-0 flex-col gap-2 rounded-xl bg-muted p-3">
                <Skeleton className="h-3 w-3/4" />
                <Skeleton className="h-3 w-1/2" />
                <Skeleton className="h-3 w-2/3" />
              </div>
            ) : isTooLarge ? (
              <div className="flex h-32 shrink-0 flex-col items-center justify-center gap-1.5 rounded-xl bg-muted px-3 text-center text-caption text-muted-foreground">
                <DangerTriangleIcon className="size-5" />
                <p>{t("preview.tooLarge", { size: PREVIEW_MAX_SOURCE_BYTES / 1024 / 1024 })}</p>
                <p>{t("preview.tooLargeHint")}</p>
              </div>
            ) : textPreview?.status === "error" ? (
              <div className="flex h-32 shrink-0 flex-col items-center justify-center gap-1.5 rounded-xl bg-muted text-caption text-muted-foreground">
                <DangerTriangleIcon className="size-5" />
                <p>{t("preview.readError")}</p>
              </div>
            ) : (
              <div className="flex h-32 shrink-0 items-center justify-center rounded-xl bg-muted">
                {showNativeIcon ? (
                  <NativeIconImage
                    className="size-12"
                    entry={entry}
                    fallback={VisualIcon ? <VisualIcon className="size-12" /> : null}
                    pixelSize={48}
                  />
                ) : VisualIcon ? (
                  <VisualIcon className="size-12" />
                ) : null}
              </div>
            )}

            {mediaPreview?.status === "ready" &&
              (mediaPreview.data.tags.length > 0 ||
                mediaPreview.data.durationMs !== null ||
                mediaPreview.data.width !== null ||
                mediaPreview.data.bitrateBps !== null) && (
                <div className="shrink-0 rounded-xl bg-muted p-3">
                  <dl className="grid grid-cols-[5rem_1fr] gap-x-3 gap-y-1.5 text-caption">
                    {mediaPreview.data.tags.map(([label, value]) => (
                      <div key={label} className="contents">
                        <dt className="text-muted-foreground">{label}</dt>
                        <dd className="min-w-0 [overflow-wrap:anywhere]">{value}</dd>
                      </div>
                    ))}
                    {mediaPreview.data.durationMs !== null && (
                      <>
                        <dt className="text-muted-foreground">{t("preview.duration")}</dt>
                        <dd className="tabular-nums">
                          {formatDuration(mediaPreview.data.durationMs)}
                        </dd>
                      </>
                    )}
                    {mediaPreview.data.width !== null && mediaPreview.data.height !== null && (
                      <>
                        <dt className="text-muted-foreground">{t("preview.dimensions")}</dt>
                        <dd className="tabular-nums">
                          {mediaPreview.data.width} × {mediaPreview.data.height}
                        </dd>
                      </>
                    )}
                    {mediaPreview.data.bitrateBps !== null && (
                      <>
                        <dt className="text-muted-foreground">{t("preview.bitrate")}</dt>
                        <dd className="tabular-nums">
                          {formatBitrate(mediaPreview.data.bitrateBps)}
                        </dd>
                      </>
                    )}
                  </dl>
                </div>
              )}

            <dl className="grid shrink-0 grid-cols-[5rem_1fr] gap-x-3 gap-y-2 text-caption">
              <dt className="text-muted-foreground">{t("preview.type")}</dt>
              <dd className="min-w-0 [overflow-wrap:anywhere]">{visual?.label ?? "—"}</dd>
              <dt className="text-muted-foreground">{t("preview.size")}</dt>
              <dd className="tabular-nums">{formatFileSize(entry.size)}</dd>
              <dt className="text-muted-foreground">{t("preview.modified")}</dt>
              <dd className="tabular-nums">
                {entry.modifiedAt === null
                  ? "—"
                  : localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(entry.modifiedAt)}
              </dd>
              <dt className="text-muted-foreground">{t("preview.location")}</dt>
              <dd className="min-w-0 [overflow-wrap:anywhere]" title={parentDirectory(entry.path)}>
                {parentDirectory(entry.path)}
              </dd>
            </dl>
          </>
        )}
      </div>

      {entries.length > 0 && (
        <footer className="flex shrink-0 items-center gap-2 border-t p-2.5">
          <Button onClick={onOpen} size="sm" type="button">
            <FolderOpenIcon />
            {t("preview.open")}
          </Button>
          <Button
            onClick={() =>
              // One newline-joined string for the clipboard, the selection size
              // for the notification — the same convention the action bar's
              // "copy paths" uses, so the two read identically.
              void copyWithNotice(
                entries.map((item) => item.path).join("\n"),
                entries.length,
              )
            }
            size="sm"
            title={t("preview.copyPathTitle")}
            type="button"
            variant="outline"
          >
            <ClipboardListIcon />
            {t("preview.copyPath")}
          </Button>
        </footer>
      )}
    </aside>
  );
}

type SelectionStats = {
  /** Bytes of the selected files only: a directory entry carries no size. */
  totalBytes: number;
  fileCount: number;
  /** Files that reported a size at all — a symlink does not. */
  sizedFileCount: number;
  folderCount: number;
  /** Newest modification in the selection, in epoch ms. */
  latestModified: number | null;
  /** The directory every entry shares, or null when they came from different
   *  places (a content-search result can span the whole tree). */
  sharedParent: string | null;
};

function summarizeSelection(entries: DirectoryEntry[]): SelectionStats {
  let totalBytes = 0;
  let fileCount = 0;
  let sizedFileCount = 0;
  let folderCount = 0;
  let latestModified: number | null = null;
  let sharedParent = entries.length > 0 ? parentDirectory(entries[0].path) : null;

  for (const item of entries) {
    if (item.kind === "directory") {
      folderCount += 1;
    } else {
      fileCount += 1;
      if (item.size !== null) {
        sizedFileCount += 1;
        totalBytes += item.size;
      }
    }

    if (item.modifiedAt !== null && (latestModified === null || item.modifiedAt > latestModified)) {
      latestModified = item.modifiedAt;
    }

    if (sharedParent !== null && parentDirectory(item.path) !== sharedParent) {
      sharedParent = null;
    }
  }

  return { totalBytes, fileCount, sizedFileCount, folderCount, latestModified, sharedParent };
}

/**
 * What a multi-selection adds up to: count by kind, the size of the files, the
 * newest change and the common location.
 *
 * The total is files only, and it says so. The listing carries no directory
 * size, and measuring one means walking it — a disk scan behind every click on
 * a folder. The alternative would be a number that silently means something
 * else depending on what happened to be selected, which in a file manager is
 * worse than a total with a stated boundary.
 */
function SelectionSummary({ entries }: { entries: DirectoryEntry[] }) {
  const { t } = useTranslation("explorer");
  const stats = useMemo(() => summarizeSelection(entries), [entries]);
  // A selection of folders only has nothing to total: the panel says so with a
  // dash rather than claiming 0 B.
  const totalSize = stats.sizedFileCount === 0 ? null : splitFileSize(stats.totalBytes);
  const composition = [
    stats.folderCount > 0 ? t("preview.folderCount", { count: stats.folderCount }) : null,
    stats.fileCount > 0 ? t("preview.fileCount", { count: stats.fileCount }) : null,
  ].filter((part): part is string => part !== null);

  return (
    <>
      <div className="shrink-0 rounded-xl bg-muted p-3.5">
        <p className="text-micro text-muted-foreground">{t("preview.totalSize")}</p>
        {totalSize === null ? (
          <p className="mt-1 text-title tabular-nums">—</p>
        ) : (
          // Value and unit are separate runs on one baseline: a 22px "GB"
          // shouts as loudly as the number it qualifies.
          <p className="mt-1 flex items-baseline gap-1.5">
            <span className="text-title tabular-nums">{totalSize.amount}</span>
            <span className="text-caption text-muted-foreground">{totalSize.unit}</span>
          </p>
        )}
        <p className="mt-1 text-caption text-muted-foreground">{composition.join(" · ")}</p>
        {stats.folderCount > 0 && (
          <p className="mt-1.5 text-micro text-muted-foreground">
            {t("preview.foldersExcluded")}
          </p>
        )}
      </div>

      <dl className="grid shrink-0 grid-cols-[5rem_1fr] gap-x-3 gap-y-2 text-caption">
        <dt className="text-muted-foreground">{t("preview.latestModified")}</dt>
        <dd className="tabular-nums">
          {stats.latestModified === null
            ? "—"
            : localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(stats.latestModified)}
        </dd>
        <dt className="text-muted-foreground">{t("preview.location")}</dt>
        <dd className="min-w-0 [overflow-wrap:anywhere]" title={stats.sharedParent ?? undefined}>
          {stats.sharedParent ?? t("preview.multipleLocations")}
        </dd>
      </dl>
    </>
  );
}

function getEntryVisualExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) return "";
  return name.slice(dotIndex + 1).toLowerCase();
}
