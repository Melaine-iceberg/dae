import { useRef } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  FileIcon,
  FolderIcon,
  FolderOpenIcon,
  LinkIcon,
  ShapesIcon,
  type LucideIcon,
} from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { cn } from "@/lib/utils";

import type { DirectoryEntry, EntryKind } from "./types";

interface FileListProps {
  entries: DirectoryEntry[];
  isLoading: boolean;
  onOpenDirectory: (path: string) => void;
}

interface EntryPresentation {
  icon: LucideIcon;
  label: string;
}

const ENTRY_PRESENTATION: Record<EntryKind, EntryPresentation> = {
  directory: { icon: FolderIcon, label: "文件夹" },
  file: { icon: FileIcon, label: "文件" },
  symlink: { icon: LinkIcon, label: "符号链接" },
  other: { icon: ShapesIcon, label: "其他" },
};

const MODIFIED_DATE_FORMATTER = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const FILE_SIZE_FORMATTER = new Intl.NumberFormat("zh-CN", {
  maximumFractionDigits: 1,
});

const FILE_SIZE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

const ROW_HEIGHT = 48;

export function FileList({ entries, isLoading, onOpenDirectory }: FileListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const virtualizer = useVirtualizer({
    count: entries.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ROW_HEIGHT,
    overscan: 10,
  });

  return (
    <section aria-label="文件列表" className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
        <h1 className="text-sm font-medium">文件</h1>
        <p aria-live="polite" className="text-xs text-muted-foreground">
          {isLoading ? "正在读取…" : `${entries.length} 个项目`}
        </p>
      </div>

      {entries.length === 0 && !isLoading ? (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FolderOpenIcon />
            </EmptyMedia>
            <EmptyTitle>这个文件夹是空的</EmptyTitle>
            <EmptyDescription>此位置暂时没有文件或子文件夹。</EmptyDescription>
          </EmptyHeader>
        </Empty>
      ) : (
        <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto">
          <div className="min-w-160">
            <div className="flex h-10 items-center whitespace-nowrap border-b text-sm font-medium text-foreground">
              <div className="min-w-0 flex-1 px-2">名称</div>
              <div className="w-44 px-2">修改日期</div>
              <div className="w-28 px-2">类型</div>
              <div className="w-24 px-2 text-right">大小</div>
            </div>
            <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
              {virtualizer.getVirtualItems().map((virtualRow) => {
                const entry = entries[virtualRow.index];

                return (
                  <div
                    key={entry.path}
                    className="absolute inset-x-0 top-0"
                    style={{ transform: `translateY(${virtualRow.start}px)` }}
                  >
                    <FileListRow
                      entry={entry}
                      isLoading={isLoading}
                      isLast={virtualRow.index === entries.length - 1}
                      onOpenDirectory={onOpenDirectory}
                    />
                  </div>
                );
              })}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

function FileListRow({
  entry,
  isLoading,
  isLast,
  onOpenDirectory,
}: {
  entry: DirectoryEntry;
  isLoading: boolean;
  isLast: boolean;
  onOpenDirectory: (path: string) => void;
}) {
  const presentation = ENTRY_PRESENTATION[entry.kind];
  const EntryIcon = presentation.icon;

  return (
    <div
      className={cn(
        "flex items-center whitespace-nowrap text-sm transition-colors hover:bg-muted/50",
        !isLast && "border-b",
      )}
      style={{ height: ROW_HEIGHT }}
    >
      <div className="min-w-0 flex-1 p-2">
        {entry.kind === "directory" ? (
          <Button
            aria-label={`打开文件夹 ${entry.name}`}
            className="w-full justify-start"
            disabled={isLoading}
            onClick={() => onOpenDirectory(entry.path)}
            title={entry.path}
            type="button"
            variant="ghost"
          >
            <EntryIcon data-icon="inline-start" />
            <span className="truncate">{entry.name}</span>
          </Button>
        ) : (
          <div className="flex h-8 min-w-0 items-center gap-1.5 px-2">
            <EntryIcon className="size-4 shrink-0 text-muted-foreground" />
            <span className="truncate" title={entry.path}>
              {entry.name}
            </span>
          </div>
        )}
      </div>
      <div className="w-44 p-2 text-muted-foreground">{formatModifiedAt(entry.modifiedAt)}</div>
      <div className="w-28 p-2 text-muted-foreground">{presentation.label}</div>
      <div
        className="w-24 p-2 text-right text-muted-foreground"
        title={entry.size === null ? undefined : `${entry.size.toLocaleString("zh-CN")} 字节`}
      >
        {formatFileSize(entry.size)}
      </div>
    </div>
  );
}

function formatModifiedAt(modifiedAt: number | null): string {
  return modifiedAt === null ? "—" : MODIFIED_DATE_FORMATTER.format(modifiedAt);
}

function formatFileSize(size: number | null): string {
  if (size === null) {
    return "—";
  }

  if (size === 0) {
    return "0 B";
  }

  const unitIndex = Math.min(
    Math.floor(Math.log(size) / Math.log(1024)),
    FILE_SIZE_UNITS.length - 1,
  );
  const value = size / 1024 ** unitIndex;

  return `${FILE_SIZE_FORMATTER.format(value)} ${FILE_SIZE_UNITS[unitIndex]}`;
}

export function FileListSkeleton() {
  return (
    <section aria-label="正在加载文件列表" className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
        <Skeleton className="h-4 w-12" />
        <Skeleton className="h-3 w-16" />
      </div>
      <Table className="min-w-160 table-fixed">
        <TableHeader>
          <TableRow>
            <TableHead>名称</TableHead>
            <TableHead className="w-44">修改日期</TableHead>
            <TableHead className="w-28">类型</TableHead>
            <TableHead className="w-24 text-right">大小</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {Array.from({ length: 8 }, (_, index) => (
            <TableRow key={index}>
              <TableCell>
                <div className="flex h-8 items-center gap-2 px-2">
                  <Skeleton className="size-4" />
                  <Skeleton className={index % 3 === 0 ? "h-4 w-48" : "h-4 w-32"} />
                </div>
              </TableCell>
              <TableCell>
                <Skeleton className="h-4 w-30" />
              </TableCell>
              <TableCell>
                <Skeleton className="h-4 w-14" />
              </TableCell>
              <TableCell>
                <Skeleton className="ml-auto h-4 w-12" />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </section>
  );
}
