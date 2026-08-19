import { useState, type ReactNode } from "react";

import type { ConflictAction, EntryKind, TransferConflict } from "@/bindings";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { cn } from "@/lib/utils";

import type { FileTransferOperation } from "./drag-drop";
import {
  DIRECTORY_PRESENTATION,
  getFilePresentation,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
} from "./file-icons";

const MODIFIED_DATE_FORMATTER = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

function formatConflictSize(bytes: number | null): string {
  if (bytes === null) return "—";
  if (bytes <= 0) return "0 B";

  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  const value = bytes / 1024 ** unitIndex;
  return `${new Intl.NumberFormat("zh-CN", { maximumFractionDigits: 1 }).format(value)} ${
    BYTE_UNITS[unitIndex]
  }`;
}

function formatConflictDate(timestamp: number | null): string {
  if (timestamp === null) return "—";
  return MODIFIED_DATE_FORMATTER.format(new Date(timestamp));
}

function kindPresentation(kind: EntryKind, name: string) {
  switch (kind) {
    case "directory":
      return DIRECTORY_PRESENTATION;
    case "symlink":
      return SYMLINK_PRESENTATION;
    case "other":
      return OTHER_PRESENTATION;
    default:
      return getFilePresentation(name);
  }
}

/**
 * Standard transfer-conflict resolution dialog (replace / skip / keep both),
 * shown one conflict at a time with an "apply to all remaining" shortcut.
 */
export function TransferConflictDialog({
  conflicts,
  operation,
  onCancel,
  onResolve,
}: {
  conflicts: TransferConflict[];
  operation: FileTransferOperation;
  onCancel: () => void;
  onResolve: (decisions: Record<string, ConflictAction>) => void;
}) {
  const [index, setIndex] = useState(0);
  const [applyToAll, setApplyToAll] = useState(false);
  const [decisions, setDecisions] = useState<Record<string, ConflictAction>>({});

  const conflict = conflicts[index];
  if (!conflict) return null;

  const remaining = conflicts.length - index;
  const actionVerb = operation === "copy" ? "复制" : "移动";

  const choose = (action: ConflictAction) => {
    const next = { ...decisions };
    next[conflict.sourcePath] = action;

    if (applyToAll) {
      for (let i = index + 1; i < conflicts.length; i += 1) {
        next[conflicts[i].sourcePath] = action;
      }
      onResolve(next);
      return;
    }

    if (index + 1 >= conflicts.length) {
      onResolve(next);
      return;
    }

    setDecisions(next);
    setIndex(index + 1);
  };

  const SourceIcon = kindPresentation(conflict.sourceKind, conflict.name).icon;
  const TargetIcon = kindPresentation(conflict.targetKind, conflict.name).icon;
  const sourceLabel = kindPresentation(conflict.sourceKind, conflict.name).label;
  const targetLabel = kindPresentation(conflict.targetKind, conflict.name).label;

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      open
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>替换或跳过文件</DialogTitle>
          <DialogDescription>
            {conflicts.length > 1 && (
              <span className="text-foreground">
                第 {(index + 1).toLocaleString("zh-CN")} / {conflicts.length.toLocaleString("zh-CN")}{" "}
                个冲突 ·{" "}
              </span>
            )}
            目标文件夹中已存在“{conflict.name}”，请选择{actionVerb}方式。
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-3">
          <ConflictSideCard
            icon={<SourceIcon className="size-5 text-muted-foreground" />}
            metadata={`${sourceLabel} · ${formatConflictSize(conflict.sourceSize)}`}
            modifiedAt={formatConflictDate(conflict.sourceModifiedAt)}
            title={conflict.name}
            subtitle={`准备${actionVerb}的项目`}
          />
          <ConflictSideCard
            highlight
            icon={<TargetIcon className="size-5 text-muted-foreground" />}
            metadata={`${targetLabel} · ${formatConflictSize(conflict.targetSize)}`}
            modifiedAt={formatConflictDate(conflict.targetModifiedAt)}
            title={conflict.name}
            subtitle="目标文件夹中的现有项目"
          />
        </div>

        {remaining > 1 && (
          <label className="flex items-center gap-2 text-[13px] text-muted-foreground">
            <input
              checked={applyToAll}
              className="size-4 accent-[var(--primary)]"
              onChange={(event) => setApplyToAll(event.target.checked)}
              type="checkbox"
            />
            应用到剩余全部（{remaining.toLocaleString("zh-CN")} 个冲突）
          </label>
        )}

        <DialogFooter className="gap-2 sm:justify-between">
          <Button onClick={onCancel} type="button" variant="ghost">
            取消{actionVerb}
          </Button>
          <div className="flex gap-2">
            <Button onClick={() => choose("skip")} type="button" variant="outline">
              跳过
            </Button>
            <Button onClick={() => choose("keep_both")} type="button" variant="outline">
              两者保留
            </Button>
            <Button onClick={() => choose("replace")} type="button" variant="destructive">
              替换
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

function ConflictSideCard({
  highlight,
  icon,
  metadata,
  modifiedAt,
  subtitle,
  title,
}: {
  highlight?: boolean;
  icon: ReactNode;
  metadata: string;
  modifiedAt: string;
  subtitle: string;
  title: string;
}) {
  return (
    <div
      className={cn(
        "flex flex-col gap-1 rounded-xl border p-3",
        highlight ? "border-primary/40 bg-primary/5" : "border-border/60 bg-muted/40",
      )}
    >
      <p className="text-xs text-muted-foreground">{subtitle}</p>
      <div className="flex min-w-0 items-center gap-2">
        <span className="shrink-0">{icon}</span>
        <p className="truncate text-[13px] font-medium" title={title}>
          {title}
        </p>
      </div>
      <p className="truncate text-xs text-muted-foreground">{metadata}</p>
      <p className="text-xs text-muted-foreground">修改于 {modifiedAt}</p>
    </div>
  );
}
