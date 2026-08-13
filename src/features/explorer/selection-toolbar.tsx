import {
  CopyIcon,
  FolderOpenIcon,
  PencilIcon,
  ScissorsIcon,
  TrashIcon,
  XIcon,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import { Separator } from "@/components/ui/separator";

import type { DirectoryEntry } from "./types";

interface SelectionToolbarProps {
  actionsDisabled: boolean;
  isSingleSelection: boolean;
  onClear: () => void;
  onCopy: () => void;
  onCut: () => void;
  onDelete: () => void;
  onOpenEntry: (entry: DirectoryEntry) => void;
  onRename: () => void;
  selectedEntries: DirectoryEntry[];
}

export function SelectionToolbar({
  actionsDisabled,
  isSingleSelection,
  onClear,
  onCopy,
  onCut,
  onDelete,
  onOpenEntry,
  onRename,
  selectedEntries,
}: SelectionToolbarProps) {
  if (selectedEntries.length === 0) {
    return null;
  }

  return (
    <div
      aria-label="已选择项目的操作"
      className="absolute bottom-3 left-1/2 z-40 flex -translate-x-1/2 items-center gap-0.5 rounded-lg border bg-popover px-1.5 py-1 text-popover-foreground shadow-md"
      role="toolbar"
    >
      <span className="px-2 text-sm whitespace-nowrap tabular-nums">
        已选择 {selectedEntries.length} 项
      </span>
      <Separator orientation="vertical" className="h-4" />
      <Button
        aria-label="打开"
        disabled={actionsDisabled}
        onClick={() => selectedEntries.forEach((entry) => onOpenEntry(entry))}
        size="icon-sm"
        title="打开"
        type="button"
        variant="ghost"
      >
        <FolderOpenIcon />
      </Button>
      <Button
        aria-label="复制"
        disabled={actionsDisabled}
        onClick={onCopy}
        size="icon-sm"
        title="复制（Ctrl+C）"
        type="button"
        variant="ghost"
      >
        <CopyIcon />
      </Button>
      <Button
        aria-label="剪切"
        disabled={actionsDisabled}
        onClick={onCut}
        size="icon-sm"
        title="剪切（Ctrl+X）"
        type="button"
        variant="ghost"
      >
        <ScissorsIcon />
      </Button>
      <Button
        aria-label="重命名"
        disabled={actionsDisabled || !isSingleSelection}
        onClick={onRename}
        size="icon-sm"
        title="重命名（F2）"
        type="button"
        variant="ghost"
      >
        <PencilIcon />
      </Button>
      <Button
        aria-label="删除"
        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
        disabled={actionsDisabled}
        onClick={onDelete}
        size="icon-sm"
        title="删除（Delete）"
        type="button"
        variant="ghost"
      >
        <TrashIcon />
      </Button>
      <Separator orientation="vertical" className="h-4" />
      <Button
        aria-label="取消选择"
        onClick={onClear}
        size="icon-sm"
        title="取消选择（Esc）"
        type="button"
        variant="ghost"
      >
        <XIcon />
      </Button>
    </div>
  );
}
