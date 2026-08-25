import { useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import type { ConflictAction, EntryKind, TransferConflict } from "@/bindings";
import { localeDateTimeFormat, localeNumber, localeNumberFormat } from "@/i18n/format";
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

import type { TransferOperation } from "./drag-drop";
import {
  DIRECTORY_PRESENTATION,
  getFilePresentation,
  getPresentationIconClassName,
  OTHER_PRESENTATION,
  SYMLINK_PRESENTATION,
} from "./file-icons";

const MODIFIED_DATE_FORMAT_OPTIONS: Intl.DateTimeFormatOptions = {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
};

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB", "PB"] as const;

function formatConflictSize(bytes: number | null): string {
  if (bytes === null) return "—";
  if (bytes <= 0) return "0 B";

  const unitIndex = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), BYTE_UNITS.length - 1);
  const value = bytes / 1024 ** unitIndex;
  const formatter = localeNumberFormat({ maximumFractionDigits: 1 });
  return `${formatter.format(value)} ${BYTE_UNITS[unitIndex]}`;
}

function formatConflictDate(timestamp: number | null): string {
  if (timestamp === null) return "—";
  return localeDateTimeFormat(MODIFIED_DATE_FORMAT_OPTIONS).format(new Date(timestamp));
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
  operation: TransferOperation;
  onCancel: () => void;
  onResolve: (decisions: Record<string, ConflictAction>) => void;
}) {
  const { t } = useTranslation("explorer");
  const [index, setIndex] = useState(0);
  const [applyToAll, setApplyToAll] = useState(false);
  const [decisions, setDecisions] = useState<Record<string, ConflictAction>>({});

  const conflict = conflicts[index];
  if (!conflict) return null;

  const remaining = conflicts.length - index;
  const actionVerb = t(
    operation === "copy" ? "conflictDialog.actionCopy" : "conflictDialog.actionMove",
  );

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

  const sourcePresentation = kindPresentation(conflict.sourceKind, conflict.name);
  const targetPresentation = kindPresentation(conflict.targetKind, conflict.name);
  const SourceIcon = sourcePresentation.icon;
  const TargetIcon = targetPresentation.icon;
  const sourceLabel = sourcePresentation.label;
  const targetLabel = targetPresentation.label;

  return (
    <Dialog
      onOpenChange={(open) => {
        if (!open) onCancel();
      }}
      open
    >
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{t("conflictDialog.title")}</DialogTitle>
          <DialogDescription>
            {conflicts.length > 1 && (
              <span className="text-foreground">
                {t("conflictDialog.conflictCounter", {
                  current: localeNumber(index + 1),
                  total: localeNumber(conflicts.length),
                })}{" "}
              </span>
            )}
            {t("conflictDialog.description", { name: conflict.name, action: actionVerb })}
          </DialogDescription>
        </DialogHeader>

        <div className="grid grid-cols-2 gap-3">
          <ConflictSideCard
            icon={
              <SourceIcon
                className={cn("size-5", getPresentationIconClassName(sourcePresentation))}
              />
            }
            metadata={`${sourceLabel} · ${formatConflictSize(conflict.sourceSize)}`}
            modifiedAt={formatConflictDate(conflict.sourceModifiedAt)}
            title={conflict.name}
            subtitle={t("conflictDialog.sourceSubtitle", { action: actionVerb })}
          />
          <ConflictSideCard
            highlight
            icon={
              <TargetIcon
                className={cn("size-5", getPresentationIconClassName(targetPresentation))}
              />
            }
            metadata={`${targetLabel} · ${formatConflictSize(conflict.targetSize)}`}
            modifiedAt={formatConflictDate(conflict.targetModifiedAt)}
            title={conflict.name}
            subtitle={t("conflictDialog.targetSubtitle")}
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
            {t("conflictDialog.applyToAll", { count: localeNumber(remaining) })}
          </label>
        )}

        <DialogFooter className="gap-2 sm:justify-between">
          <Button onClick={onCancel} type="button" variant="ghost">
            {t("conflictDialog.cancelAction", { action: actionVerb })}
          </Button>
          <div className="flex gap-2">
            <Button onClick={() => choose("skip")} type="button" variant="outline">
              {t("conflictDialog.skip")}
            </Button>
            <Button onClick={() => choose("keep_both")} type="button" variant="outline">
              {t("conflictDialog.keepBoth")}
            </Button>
            <Button onClick={() => choose("replace")} type="button" variant="destructive">
              {t("conflictDialog.replace")}
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
  const { t } = useTranslation("explorer");

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
      <p className="text-xs text-muted-foreground">
        {t("conflictDialog.modifiedAt", { date: modifiedAt })}
      </p>
    </div>
  );
}
