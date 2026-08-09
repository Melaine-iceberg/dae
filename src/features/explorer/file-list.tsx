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

export function FileList({ entries, isLoading, onOpenDirectory }: FileListProps) {
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
        <div className="min-h-0 flex-1 overflow-auto">
          <Table className="table-fixed">
            <TableHeader>
              <TableRow>
                <TableHead>名称</TableHead>
                <TableHead className="hidden w-32 sm:table-cell">类型</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {entries.map((entry) => (
                <FileListRow
                  key={entry.path}
                  entry={entry}
                  isLoading={isLoading}
                  onOpenDirectory={onOpenDirectory}
                />
              ))}
            </TableBody>
          </Table>
        </div>
      )}
    </section>
  );
}

function FileListRow({
  entry,
  isLoading,
  onOpenDirectory,
}: {
  entry: DirectoryEntry;
  isLoading: boolean;
  onOpenDirectory: (path: string) => void;
}) {
  const presentation = ENTRY_PRESENTATION[entry.kind];
  const EntryIcon = presentation.icon;

  return (
    <TableRow>
      <TableCell>
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
      </TableCell>
      <TableCell className="hidden text-muted-foreground sm:table-cell">
        {presentation.label}
      </TableCell>
    </TableRow>
  );
}

export function FileListSkeleton() {
  return (
    <section aria-label="正在加载文件列表" className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-10 shrink-0 items-center justify-between border-b px-4">
        <Skeleton className="h-4 w-12" />
        <Skeleton className="h-3 w-16" />
      </div>
      <Table className="table-fixed">
        <TableHeader>
          <TableRow>
            <TableHead>名称</TableHead>
            <TableHead className="hidden w-32 sm:table-cell">类型</TableHead>
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
              <TableCell className="hidden sm:table-cell">
                <Skeleton className="h-4 w-14" />
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </section>
  );
}
