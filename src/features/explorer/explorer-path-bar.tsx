import { useEffect, useRef, useState, type FormEvent, type MouseEvent } from "react";
import { FolderIcon } from "@phosphor-icons/react";

import { ExplorerBreadcrumbs } from "./explorer-breadcrumbs";
import { cn } from "@/lib/utils";
import type { Breadcrumb, DirectoryView } from "./types";

interface ExplorerPathBarProps {
  directory: DirectoryView;
  onNavigate: (breadcrumb: Breadcrumb) => void;
  onNavigatePath: (path: string) => Promise<boolean>;
}

export function ExplorerPathBar({ directory, onNavigate, onNavigatePath }: ExplorerPathBarProps) {
  const [isEditing, setIsEditing] = useState(false);
  const [value, setValue] = useState("");
  const [isInvalid, setIsInvalid] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setIsEditing(false);
    setIsInvalid(false);
  }, [directory.path]);

  useEffect(() => {
    if (isEditing) {
      const input = inputRef.current;
      input?.focus();
      input?.select();
    }
  }, [isEditing]);

  const startEditing = (event: MouseEvent) => {
    if (event.defaultPrevented) return;
    setValue(directory.path);
    setIsInvalid(false);
    setIsEditing(true);
  };

  const submitPath = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const path = value.trim();
    if (!path || path === directory.path) {
      setIsEditing(false);
      return;
    }

    const succeeded = await onNavigatePath(path);
    if (!succeeded) {
      setIsInvalid(true);
      inputRef.current?.focus();
    }
  };

  if (isEditing) {
    return (
      <form
        className={cn(
          "flex h-8 min-w-0 flex-1 items-center rounded-full border bg-muted/70 pr-3 pl-3.5 transition-[background-color,border-color,box-shadow] focus-within:bg-card focus-within:ring-2 focus-within:ring-ring/30",
          isInvalid ? "border-destructive" : "border-transparent focus-within:border-ring",
        )}
        onSubmit={(event) => void submitPath(event)}
      >
        <FolderIcon
          className="pointer-events-none mr-2 size-3.5 shrink-0 text-folder"
          weight="fill"
        />
        <input
          ref={inputRef}
          aria-invalid={isInvalid}
          aria-label="当前路径"
          className="h-full min-w-0 flex-1 bg-transparent text-[13px] outline-none"
          onChange={(event) => {
            setValue(event.target.value);
            setIsInvalid(false);
          }}
          onBlur={() => {
            setIsEditing(false);
            setIsInvalid(false);
          }}
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              setIsEditing(false);
            }
          }}
          spellCheck={false}
          value={value}
        />
      </form>
    );
  }

  return (
    <div
      className="flex h-8 min-w-0 flex-1 items-center rounded-full border border-transparent bg-muted/70 px-3.5 transition-colors hover:bg-muted dark:bg-muted/50 dark:hover:bg-muted/70"
      data-tauri-drag-region="false"
      onClick={startEditing}
      title="单击以编辑路径"
    >
      <FolderIcon
        className="pointer-events-none mr-2 size-3.5 shrink-0 text-folder"
        weight="fill"
      />
      <ExplorerBreadcrumbs breadcrumbs={directory.breadcrumbs} onNavigate={onNavigate} />
    </div>
  );
}
