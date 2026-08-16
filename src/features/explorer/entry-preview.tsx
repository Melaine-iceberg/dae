import { useEffect, useState } from "react";
import {
  ClipboardTextIcon,
  FolderOpenIcon,
  WarningIcon,
  XIcon,
} from "@phosphor-icons/react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";

import { commands, type TextPreview } from "@/bindings";
import { Button } from "@/components/ui/button";

import { getEntryPresentation } from "./file-icons";
import { isThumbnailSupported, ThumbnailImage } from "./thumbnail";
import type { DirectoryEntry } from "./types";

/** Extensions offered an inline text peek (SKILL.md §21 preview surface). */
const TEXT_PREVIEW_EXTENSIONS = new Set([
  "txt", "log", "md", "markdown", "json", "yaml", "yml", "toml", "ini", "conf",
  "html", "htm", "css", "scss", "less", "js", "mjs", "cjs", "ts", "tsx", "jsx",
  "py", "rs", "c", "h", "cpp", "hpp", "cs", "vue", "sql", "sh", "bash", "zsh",
  "bat", "ps1", "csv", "xml",
]);

const TEXT_PREVIEW_MAX_SOURCE_BYTES = 512 * 1024;
const TEXT_PREVIEW_READ_BYTES = 8 * 1024;

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

/**
 * Floating preview surface for the current selection (SKILL.md §21): shows
 * content (thumbnail / text peek) plus metadata and follow-up actions.
 */
export function EntryPreview({
  entry,
  onClose,
  onOpen,
}: {
  entry: DirectoryEntry;
  onClose: () => void;
  onOpen: () => void;
}) {
  const supportsThumbnail = isThumbnailSupported(entry);
  const supportsText =
    entry.kind === "file" &&
    (entry.size ?? 0) <= TEXT_PREVIEW_MAX_SOURCE_BYTES &&
    TEXT_PREVIEW_EXTENSIONS.has(getEntryVisualExtension(entry.name));
  const [textPreview, setTextPreview] = useState<TextPreview | null>(null);

  useEffect(() => {
    if (!supportsText) {
      setTextPreview(null);
      return;
    }

    let cancelled = false;
    void commands
      .readTextPreview(entry.path, TEXT_PREVIEW_READ_BYTES)
      .then((result) => {
        if (!cancelled) setTextPreview(result);
      })
      .catch((error: unknown) => {
        console.warn(`Unable to read text preview for ${entry.path}`, error);
        if (!cancelled) setTextPreview(null);
      });
    return () => {
      cancelled = true;
    };
  }, [entry.path, supportsText]);

  const visual = getEntryPresentation(entry);
  const VisualIcon = visual.icon;

  return (
    <aside
      aria-label="预览"
      className="animate-in absolute top-4 right-4 bottom-4 z-30 flex w-80 flex-col overflow-hidden rounded-xl border bg-popover/95 shadow-lg backdrop-blur duration-150 fade-in slide-in-from-right-2"
    >
      <header className="flex shrink-0 items-center gap-2 border-b px-3 py-2">
        <VisualIcon className="size-4 shrink-0 text-muted-foreground" />
        <p className="min-w-0 flex-1 truncate text-[13px] font-medium" title={entry.name}>
          {entry.name}
        </p>
        <Button
          aria-label="关闭预览"
          onClick={onClose}
          size="icon-sm"
          title="关闭预览 (Space)"
          type="button"
          variant="ghost"
        >
          <XIcon />
        </Button>
      </header>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
        {supportsThumbnail ? (
          <ThumbnailImage
            className="flex h-48 shrink-0 items-center justify-center overflow-hidden rounded-lg border bg-muted/40"
            entry={entry}
            requestSize={256}
          />
        ) : textPreview ? (
          <div className="shrink-0 rounded-lg border bg-muted/40">
            <pre className="max-h-56 overflow-auto p-2.5 text-xs leading-relaxed whitespace-pre-wrap break-all">
              {textPreview.content}
            </pre>
            {textPreview.truncated && (
              <p className="border-t px-2.5 py-1.5 text-xs text-muted-foreground">
                内容较长，仅显示前 {TEXT_PREVIEW_READ_BYTES / 1024} KB
              </p>
            )}
          </div>
        ) : (
          <div className="flex h-32 shrink-0 items-center justify-center rounded-lg border bg-muted/40">
            <VisualIcon className="size-12 text-muted-foreground/60" weight="duotone" />
          </div>
        )}

        <dl className="grid shrink-0 grid-cols-[5rem_1fr] gap-x-3 gap-y-2 text-xs">
          <dt className="text-muted-foreground">类型</dt>
          <dd className="min-w-0 break-all">{visual.label}</dd>
          <dt className="text-muted-foreground">大小</dt>
          <dd className="tabular-nums">{formatFileSize(entry.size)}</dd>
          <dt className="text-muted-foreground">修改日期</dt>
          <dd className="tabular-nums">
            {entry.modifiedAt === null
              ? "—"
              : MODIFIED_DATE_FORMATTER.format(entry.modifiedAt)}
          </dd>
          <dt className="text-muted-foreground">位置</dt>
          <dd className="min-w-0 break-all" title={parentDirectory(entry.path)}>
            {parentDirectory(entry.path)}
          </dd>
        </dl>

        {textPreview === null && supportsText && (
          <p className="flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground">
            <WarningIcon className="size-3.5" />
            无法读取文本内容
          </p>
        )}
      </div>

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
    </aside>
  );
}

function getEntryVisualExtension(name: string): string {
  const dotIndex = name.lastIndexOf(".");
  if (dotIndex <= 0 || dotIndex === name.length - 1) return "";
  return name.slice(dotIndex + 1).toLowerCase();
}
