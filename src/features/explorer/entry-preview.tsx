import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  CheckCircleIcon,
  ClipboardListIcon,
  CloseIcon,
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
      className="inspector flex h-full w-preview shrink-0 flex-col overflow-hidden"
    >
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pt-4 pb-12">
        {/* The reference's `.insp-cap` — an 11px uppercase meta caption, with
            the close control sitting on its baseline. The panel used to open
            with a 54px title bar holding the file name, which spent the widest
            strip in the panel on chrome and named the file in two places. */}
        <div className="mb-3 flex items-center justify-between gap-2">
          <p className="text-nano font-semibold tracking-[0.08em] text-meta uppercase">
            {t("preview.infoCaption")}
          </p>
          <Button
            aria-label={t("preview.collapsePanel")}
            className="-mr-1.5"
            onClick={onClose}
            size="icon-xs"
            title={t("preview.collapsePanelShortcut")}
            type="button"
            variant="ghost"
          >
            <CloseIcon />
          </Button>
        </div>

        {isMultiSelection ? (
          <SelectionSummary entries={entries} onOpen={onOpen} />
        ) : entry === null ? (
          <>
            <PreviewHead
              glyph={<SquareDashedIcon size={26} />}
              name={t("preview.noFileSelected")}
              sub={t("preview.selectHint")}
            />
          </>
        ) : (
          <>
            <PreviewHead
              glyph={VisualIcon ? <VisualIcon size={26} /> : null}
              name={entry.name}
              sub={`${visual?.label ?? "—"} · ${formatFileSize(entry.size)}`}
            />

            {supportsThumbnail ? (
              <div className="preview-body flex h-64 shrink-0 items-center justify-center">
                <ThumbnailImage
                  className="flex size-full items-center justify-center overflow-hidden"
                  entry={entry}
                  fallback={VisualIcon ? <VisualIcon size={48} /> : null}
                  displaySize={384}
                />
              </div>
            ) : textPreview?.status === "ready" ? (
              <div className="preview-body mb-5 flex max-h-[220px] min-w-0 flex-col overflow-hidden">
                {supportsMarkdown ? (
                  <div
                    className="markdown-preview min-h-0 flex-1 overflow-auto p-[10px_12px] text-[11.5px] leading-[1.7]"
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
                    className="code-preview min-h-0 flex-1 overflow-auto p-[10px_12px] font-mono text-[11px] leading-[1.7] text-foreground/85"
                    // Highlighted markup is generated locally from file contents; TanStack Highlight escapes all text nodes (escapeHtml in its core) before tokenizing.
                    // pi-lens-ignore: dangerously-set-inner-html
                    dangerouslySetInnerHTML={{ __html: textPreview.html }}
                  />
                ) : (
                  <pre className="min-h-0 flex-1 overflow-auto p-[10px_12px] font-mono text-[11px] leading-[1.7] text-foreground/85 whitespace-pre-wrap [overflow-wrap:anywhere]">
                    {textPreview.content}
                  </pre>
                )}
                {textPreview.truncated && (
                  <p className="shrink-0 border-t border-border-soft px-3 py-1.5 text-[11px] text-meta">
                    {t("preview.truncatedHint", { size: PREVIEW_READ_BYTES / 1024 })}
                  </p>
                )}
              </div>
            ) : textPreview?.status === "loading" ? (
              <div className="preview-body mb-5 flex flex-col gap-1.5 p-3">
                <Skeleton className="h-[9px] w-3/4" />
                <Skeleton className="h-[9px] w-1/2" />
                <Skeleton className="h-[9px] w-2/3" />
              </div>
            ) : isTooLarge ? (
              <p className="note">
                {t("preview.tooLarge", { size: PREVIEW_MAX_SOURCE_BYTES / 1024 / 1024 })}
              </p>
            ) : textPreview?.status === "error" ? (
              <p className="note">{t("preview.readError")}</p>
            ) : (
              <div className="preview-body mb-5 flex h-32 shrink-0 items-center justify-center">
                {showNativeIcon ? (
                  <NativeIconImage
                    className="size-12"
                    entry={entry}
                    fallback={VisualIcon ? <VisualIcon size={48} /> : null}
                    pixelSize={48}
                  />
                ) : VisualIcon ? (
                  <VisualIcon size={48} />
                ) : null}
              </div>
            )}

            <dl className="mb-5">
              <MetaRow label={t("preview.type")}>{visual?.label ?? "—"}</MetaRow>
              <MetaRow label={t("preview.size")}>
                <span className="tabular-nums">{formatFileSize(entry.size)}</span>
              </MetaRow>
              <MetaRow label={t("preview.modified")}>
                <span className="tabular-nums">
                  {entry.modifiedAt === null
                    ? "—"
                    : localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(entry.modifiedAt)}
                </span>
              </MetaRow>
              <MetaRow label={t("preview.location")}>
                <span title={parentDirectory(entry.path)}>{parentDirectory(entry.path)}</span>
              </MetaRow>
              {mediaPreview?.status === "ready" &&
                (mediaPreview.data.tags.length > 0 ||
                  mediaPreview.data.durationMs !== null ||
                  mediaPreview.data.width !== null ||
                  mediaPreview.data.bitrateBps !== null) && (
                  <>
                    {mediaPreview.data.tags.map(([label, value]) => (
                      <MetaRow key={label} label={label}>
                        {value}
                      </MetaRow>
                    ))}
                    {mediaPreview.data.durationMs !== null && (
                      <MetaRow label={t("preview.duration")}>
                        <span className="tabular-nums">
                          {formatDuration(mediaPreview.data.durationMs)}
                        </span>
                      </MetaRow>
                    )}
                    {mediaPreview.data.width !== null && mediaPreview.data.height !== null && (
                      <MetaRow label={t("preview.dimensions")}>
                        <span className="tabular-nums">
                          {mediaPreview.data.width} × {mediaPreview.data.height}
                        </span>
                      </MetaRow>
                    )}
                    {mediaPreview.data.bitrateBps !== null && (
                      <MetaRow label={t("preview.bitrate")}>
                        <span className="tabular-nums">
                          {formatBitrate(mediaPreview.data.bitrateBps)}
                        </span>
                      </MetaRow>
                    )}
                  </>
                )}
            </dl>

            <PreviewActions entries={entries} onOpen={onOpen} />
          </>
        )}
      </div>
    </aside>
  );
}

/** The reference's `.meta-row`: a 74px label column, a 7px-padded row and a
 *  half-strength hairline under it, with no card behind it. These facts used to
 *  be a bare `dl` with no rules, so four label/value pairs read as one
 *  paragraph. */
function MetaRow({ children, label }: { children: React.ReactNode; label: string }) {
  return (
    <div className="grid grid-cols-[74px_minmax(0,1fr)] gap-3 border-b border-border-soft py-[7px] text-[11.5px] leading-[1.5]">
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="m-0 min-w-0 text-foreground/85 [overflow-wrap:anywhere]">{children}</dd>
    </div>
  );
}

/** `.pv-head`: the 26px glyph, the 14px name, and a mono sub line under it. */
function PreviewHead({ glyph, name, sub }: { glyph: React.ReactNode; name: string; sub: string }) {
  return (
    <div className="mb-3 flex items-start gap-2.5">
      <span className="mt-0.5 flex shrink-0 text-foreground/85">
        {glyph}
      </span>
      <div className="min-w-0">
        <p className="text-[14px] leading-[1.4] font-semibold [overflow-wrap:anywhere]">{name}</p>
        <p className="mt-[3px] font-mono text-[10.5px] text-muted-foreground [overflow-wrap:anywhere]">
          {sub}
        </p>
      </div>
    </div>
  );
}

/** `.insp-actions`: a vertical stack, not a row of half-width buttons — two
 *  buttons sharing 268px turn "Open" into a 128px control with an icon and a
 *  word in it, and the primary action loses its rank to its neighbour. */
function PreviewActions({ entries, onOpen }: { entries: DirectoryEntry[]; onOpen: () => void }) {
  const { t } = useTranslation("explorer");

  if (entries.length === 0) return null;

  return (
    <div className="grid gap-1.5">
      <Button onClick={onOpen} type="button">
        <FolderOpenIcon />
        {t("preview.open")}
      </Button>
      <Button
        onClick={() =>
          // One newline-joined string for the clipboard, the selection size for
          // the notification — the same convention the action bar's "copy
          // paths" uses, so the two read identically.
          void copyWithNotice(
            entries.map((item) => item.path).join("\n"),
            entries.length,
          )
        }
        title={t("preview.copyPathTitle")}
        type="button"
        variant="outline"
      >
        <ClipboardListIcon />
        {t("preview.copyPath")}
      </Button>
    </div>
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
function SelectionSummary({
  entries,
  onOpen,
}: {
  entries: DirectoryEntry[];
  onOpen: () => void;
}) {
  const { t } = useTranslation("explorer");
  const stats = useMemo(() => summarizeSelection(entries), [entries]);
  // A selection of folders only has nothing to total: the panel says so with a
  // dash rather than claiming 0 B.
  const hasTotal = stats.sizedFileCount > 0;

  return (
    <>
      <PreviewHead
        glyph={<CheckCircleIcon size={26} />}
        name={t("preview.multiSelection", { number: localeNumber(entries.length) })}
        sub={
          hasTotal
            ? t("preview.totalLine", { size: formatFileSize(stats.totalBytes) })
            : t("preview.totalSize")
        }
      />

      <dl className="mb-5">
        <MetaRow label={t("preview.contents")}>
          {[
            stats.folderCount > 0
              ? t("preview.folderCount", { count: stats.folderCount })
              : null,
            stats.fileCount > 0 ? t("preview.fileCount", { count: stats.fileCount }) : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </MetaRow>
        <MetaRow label={t("preview.latestModified")}>
          <span className="tabular-nums">
            {stats.latestModified === null
              ? "—"
              : localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(stats.latestModified)}
          </span>
        </MetaRow>
        <MetaRow label={t("preview.location")}>
          <span title={stats.sharedParent ?? undefined}>
            {stats.sharedParent ?? t("preview.multipleLocations")}
          </span>
        </MetaRow>
      </dl>

      {stats.folderCount > 0 && (
        <p className="note mb-5">{t("preview.foldersExcluded")}</p>
      )}

      <PreviewActions entries={entries} onOpen={onOpen} />
    </>
  );
}

function getEntryVisualExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) return "";
  return name.slice(dotIndex + 1).toLowerCase();
}
