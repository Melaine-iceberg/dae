import { useEffect } from "react";
import { useAtomValue, useSetAtom } from "jotai";
import {
  ArrowsOutCardinalIcon,
  ClipboardTextIcon,
  CopyIcon,
  DotsThreeIcon,
  FileZipIcon,
  FilesIcon,
  FolderOpenIcon,
  PencilIcon,
  ScissorsIcon,
  SquaresFourIcon,
  TrashIcon,
  XIcon,
} from "@phosphor-icons/react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ensureSpacesLoadedAtom, spacesAtom } from "@/features/workspace/spaces-atoms";
import { MOD_KEY } from "@/lib/platform";

/**
 * Floating contextual action surface that follows the selection
 * (SKILL.md §19/§20). Appears only while items are selected; destructive
 * actions are visually differentiated.
 */
export function ContextualActionBar({
  hasDirectorySelection,
  isActionDisabled,
  isSingleSelection,
  onAddToSpace,
  onClearSelection,
  onCompress,
  onCopy,
  onCopyPaths,
  onCut,
  onDelete,
  onDuplicate,
  onMoveTo,
  onOpen,
  onRename,
  selectedCount,
}: {
  hasDirectorySelection: boolean;
  isActionDisabled: boolean;
  isSingleSelection: boolean;
  onAddToSpace: (spaceId: string) => void;
  onClearSelection: () => void;
  onCompress: () => void;
  onCopy: () => void;
  onCopyPaths: () => void;
  onCut: () => void;
  onDelete: () => void;
  onDuplicate: () => void;
  onMoveTo: () => void;
  onOpen: () => void;
  onRename: () => void;
  selectedCount: number;
}) {
  const spaces = useAtomValue(spacesAtom) ?? [];
  const ensureSpacesLoaded = useSetAtom(ensureSpacesLoadedAtom);

  useEffect(() => {
    void ensureSpacesLoaded();
  }, [ensureSpacesLoaded]);

  return (
    <div
      aria-label="选中项操作"
      className="animate-in absolute bottom-4 left-1/2 z-30 flex -translate-x-1/2 items-center gap-0.5 rounded-xl border bg-popover/95 p-1 shadow-lg backdrop-blur duration-150 fade-in slide-in-from-bottom-2"
      role="toolbar"
    >
      <span className="shrink-0 px-2.5 text-[13px] text-muted-foreground select-none tabular-nums">
        已选 {selectedCount.toLocaleString("zh-CN")} 项
      </span>
      <div aria-hidden="true" className="mr-0.5 h-5 w-px bg-border" />
      <Button
        aria-label="打开所选项目"
        disabled={isActionDisabled}
        onClick={onOpen}
        size="icon"
        title="打开 (Enter)"
        type="button"
        variant="ghost"
      >
        <FolderOpenIcon />
      </Button>
      <Button
        aria-label="复制"
        disabled={isActionDisabled}
        onClick={onCopy}
        size="icon"
        title={`复制 (${MOD_KEY}+C)`}
        type="button"
        variant="ghost"
      >
        <CopyIcon />
      </Button>
      <Button
        aria-label="剪切"
        disabled={isActionDisabled}
        onClick={onCut}
        size="icon"
        title={`剪切 (${MOD_KEY}+X)`}
        type="button"
        variant="ghost"
      >
        <ScissorsIcon />
      </Button>
      <Button
        aria-label="重命名"
        disabled={isActionDisabled || !isSingleSelection}
        onClick={onRename}
        size="icon"
        title="重命名 (F2)"
        type="button"
        variant="ghost"
      >
        <PencilIcon />
      </Button>
      <DropdownMenu>
        <DropdownMenuTrigger
          aria-label="更多操作"
          className="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-50"
          disabled={isActionDisabled}
          title="更多操作"
        >
          <DotsThreeIcon className="size-4" weight="bold" />
        </DropdownMenuTrigger>
        <DropdownMenuContent align="center" side="top">
          <DropdownMenuItem disabled={isActionDisabled} onClick={onDuplicate}>
            <FilesIcon />
            创建副本
          </DropdownMenuItem>
          <DropdownMenuItem disabled={isActionDisabled} onClick={onCompress}>
            <FileZipIcon />
            压缩为ZIP
          </DropdownMenuItem>
          <DropdownMenuItem disabled={isActionDisabled} onClick={onMoveTo}>
            <ArrowsOutCardinalIcon />
            移动到…
          </DropdownMenuItem>
          <DropdownMenuItem onClick={onCopyPaths}>
            <ClipboardTextIcon />
            复制文件地址
          </DropdownMenuItem>
          {hasDirectorySelection &&
            spaces.slice(0, 3).map((space) => (
              <DropdownMenuItem key={space.id} onClick={() => onAddToSpace(space.id)}>
                <SquaresFourIcon />
                添加到「{space.name}」
              </DropdownMenuItem>
            ))}
        </DropdownMenuContent>
      </DropdownMenu>
      <Button
        aria-label="删除"
        className="text-destructive hover:bg-destructive/10 hover:text-destructive"
        disabled={isActionDisabled}
        onClick={onDelete}
        size="icon"
        title="删除 (Delete)"
        type="button"
        variant="ghost"
      >
        <TrashIcon />
      </Button>
      <div aria-hidden="true" className="ml-0.5 h-5 w-px bg-border" />
      <Button
        aria-label="取消选择"
        onClick={onClearSelection}
        size="icon"
        title="取消选择 (Esc)"
        type="button"
        variant="ghost"
      >
        <XIcon />
      </Button>
    </div>
  );
}
