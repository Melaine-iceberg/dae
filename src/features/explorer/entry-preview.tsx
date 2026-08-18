import { useEffect, useState } from "react";
import { ClipboardTextIcon, FolderOpenIcon, WarningIcon, XIcon } from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands, type TextPreview } from "@/bindings";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";

import { getPreviewLanguage, highlightCode } from "./code-highlight";
import { getEntryPresentation } from "./file-icons";
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

/** Files above this size skip content preview entirely for performance. */
const PREVIEW_MAX_SOURCE_BYTES = 512 * 1024;
const PREVIEW_READ_BYTES = 64 * 1024;
/**
 * Only this much text goes through the highlighter: the preview viewport
 * shows ~20 lines, so highlighting the full 64KB read would burn hundreds
 * of milliseconds on the main thread for invisible content.
 */
const HIGHLIGHT_MAX_BYTES = 16 * 1024;

function sliceForHighlight(content: string): string {
  if (content.length <= HIGHLIGHT_MAX_BYTES) return content;
  const slice = content.slice(0, HIGHLIGHT_MAX_BYTES);
  const newlineIndex = slice.lastIndexOf("\n");
  return newlineIndex > 0 ? slice.slice(0, newlineIndex) : slice;
}

const MODIFIED_DATE_FORMATTER = new Intl.DateTimeFormat("zh-CN", {
  dateStyle: "medium",
  timeStyle: "short",
});

const FILE_SIZE_FORMATTER = new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 });
const FILE_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

function formatFileSize(size: number | null): string {
  if (size === null) return "—";
  if (size === 0) return "0 B";

  const unitIndex = Math.min(
    Math.floor(Math.log(size) / Math.log(1024)),
    FILE_SIZE_UNITS.length - 1,
  );
  return `${FILE_SIZE_FORMATTER.format(size / 1024 ** unitIndex)} ${FILE_SIZE_UNITS[unitIndex]}`;
}

function parentDirectory(path: string): string {
  const separatorIndex = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  if (separatorIndex <= 0) return path;
  return path.slice(0, separatorIndex);
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

/**
 * Docked yazi-style preview panel for the current selection (SKILL.md §21):
 * shows content (thumbnail / highlighted code / text peek) plus metadata and
 * follow-up actions. Large files skip content preview for performance.
 */
export function EntryPreview({
  entry,
  onClose,
  onOpen,
}: {
  entry: DirectoryEntry | null;
  onClose: () => void;
  onOpen: () => void;
}) {
  const language = entry?.kind === "file" ? getPreviewLanguage(entry.name) : null;
  const supportsThumbnail = entry !== null && isThumbnailSupported(entry);
  const supportsText =
    entry?.kind === "file" &&
    (language !== null || TEXT_PREVIEW_EXTENSIONS.has(getEntryVisualExtension(entry.name)));
  const isTooLarge = (entry?.size ?? 0) > PREVIEW_MAX_SOURCE_BYTES;
  const [textPreview, setTextPreview] = useState<TextPreviewState | null>(null);

  useEffect(() => {
    if (!entry || entry.kind !== "file" || isTooLarge || !supportsText) {
      setTextPreview(null);
      return;
    }

    setTextPreview({ status: "loading" });
    let cancelled = false;
    void commands
      .readTextPreview(entry.path, PREVIEW_READ_BYTES)
      .then(async (result: TextPreview) => {
        let html: string | null = null;
        if (language !== null) {
          // Highlighting is best-effort in a worker: on failure or
          // supersede the raw text still renders as plain content.
          try {
            html = await highlightCode(sliceForHighlight(result.content), language);
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

  const visual = entry ? getEntryPresentation(entry) : null;
  const VisualIcon = visual?.icon;

  return (
    <aside
      aria-label="预览"
      className="animate-in flex h-full w-[26rem] shrink-0 flex-col overflow-hidden border-l bg-popover/95 duration-200 fade-in slide-in-from-right-2"
    >
      <header className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
        {VisualIcon ? <VisualIcon className="size-4 shrink-0 text-muted-foreground" /> : null}
        <p
          className="min-w-0 flex-1 truncate text-[13px] font-medium"
          title={entry?.name ?? undefined}
        >
          {entry?.name ?? "未选择文件"}
        </p>
        <Button
          aria-label="收起预览面板"
          onClick={onClose}
          size="icon-sm"
          title="收起预览面板 (Space)"
          type="button"
          variant="ghost"
        >
          <XIcon />
        </Button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
        {entry === null ? (
          <div className="flex flex-1 flex-col items-center justify-center gap-2 text-xs text-muted-foreground">
            <VisualPlaceholder name="preview" />
            <p>在列表中选择一个文件以预览</p>
          </div>
        ) : (
          <>
            {supportsThumbnail ? (
              <ThumbnailImage
                className="flex h-64 shrink-0 items-center justify-center overflow-hidden rounded-lg border bg-muted/40"
                entry={entry}
                requestSize={384}
              />
            ) : textPreview?.status === "ready" ? (
              <div className="flex min-h-48 min-w-0 flex-1 flex-col overflow-hidden rounded-lg border bg-muted/40">
                {textPreview.html !== null ? (
                  <div
                    className="code-preview min-h-0 flex-1 overflow-auto p-2.5 text-xs leading-relaxed"
                    // Shiki output is generated locally from file contents.
                    dangerouslySetInnerHTML={{ __html: textPreview.html }}
                  />
                ) : (
                  <pre className="min-h-0 flex-1 overflow-auto p-2.5 text-xs leading-relaxed whitespace-pre-wrap break-all">
                    {textPreview.content}
                  </pre>
                )}
                {textPreview.truncated && (
                  <p className="shrink-0 border-t px-2.5 py-1.5 text-xs text-muted-foreground">
                    内容较长，仅显示前 {PREVIEW_READ_BYTES / 1024} KB
                  </p>
                )}
              </div>
            ) : textPreview?.status === "loading" ? (
              <div className="flex shrink-0 flex-col gap-2 rounded-lg border bg-muted/40 p-2.5">
                <Skeleton className="h-3 w-3/4 rounded-full" />
                <Skeleton className="h-3 w-1/2 rounded-full" />
                <Skeleton className="h-3 w-2/3 rounded-full" />
              </div>
            ) : isTooLarge ? (
              <div className="flex h-32 shrink-0 flex-col items-center justify-center gap-1.5 rounded-lg border bg-muted/40 px-3 text-center text-xs text-muted-foreground">
                <WarningIcon className="size-5" />
                <p>文件超过 {PREVIEW_MAX_SOURCE_BYTES / 1024 / 1024} MB</p>
                <p>为保障性能，不预览内容</p>
              </div>
            ) : textPreview?.status === "error" ? (
              <div className="flex h-32 shrink-0 flex-col items-center justify-center gap-1.5 rounded-lg border bg-muted/40 text-xs text-muted-foreground">
                <WarningIcon className="size-5" />
                <p>无法读取文本内容</p>
              </div>
            ) : (
              <div className="flex h-32 shrink-0 items-center justify-center rounded-lg border bg-muted/40">
                {VisualIcon ? (
                  <VisualIcon className="size-12 text-muted-foreground/60" weight="duotone" />
                ) : null}
              </div>
            )}

            <dl className="grid shrink-0 grid-cols-[5rem_1fr] gap-x-3 gap-y-2 text-xs">
              <dt className="text-muted-foreground">类型</dt>
              <dd className="min-w-0 break-all">{visual?.label ?? "—"}</dd>
              <dt className="text-muted-foreground">大小</dt>
              <dd className="tabular-nums">{formatFileSize(entry.size)}</dd>
              <dt className="text-muted-foreground">修改日期</dt>
              <dd className="tabular-nums">
                {entry.modifiedAt === null ? "—" : MODIFIED_DATE_FORMATTER.format(entry.modifiedAt)}
              </dd>
              <dt className="text-muted-foreground">位置</dt>
              <dd className="min-w-0 break-all" title={parentDirectory(entry.path)}>
                {parentDirectory(entry.path)}
              </dd>
            </dl>
          </>
        )}
      </div>

      {entry !== null && (
        <footer className="flex shrink-0 items-center gap-2 border-t p-2.5">
          <Button onClick={onOpen} size="sm" type="button">
            <FolderOpenIcon />
            打开
          </Button>
          <Button
            onClick={() =>
              void writeText(entry.path).catch((error) => {
                console.warn("Unable to copy path to clipboard", error);
              })
            }
            size="sm"
            title="复制文件地址"
            type="button"
            variant="outline"
          >
            <ClipboardTextIcon />
            复制地址
          </Button>
        </footer>
      )}
    </aside>
  );
}

function VisualPlaceholder({ name }: { name: string }) {
  return (
    <svg
      aria-hidden="true"
      className="size-12 text-muted-foreground/40"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.5}
      viewBox="0 0 24 24"
    >
      <path
        d="M3 5.5A1.5 1.5 0 0 1 4.5 4h4l2 2.5h7A1.5 1.5 0 0 1 19 8v10a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 3 18V5.5Z"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <title>{name}</title>
    </svg>
  );
}

function getEntryVisualExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) return "";
  return name.slice(dotIndex + 1).toLowerCase();
}
