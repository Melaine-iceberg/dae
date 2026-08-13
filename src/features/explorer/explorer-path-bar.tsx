import { useEffect, useRef, useState, type ClipboardEvent, type FormEvent, type MouseEvent } from "react";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { FolderIcon } from "lucide-react";

import { Input } from "@/components/ui/input";

import { ExplorerBreadcrumbs } from "./explorer-breadcrumbs";
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

  // WebView2 的原生复制不会被 Windows 剪贴板历史(Win+V)记录，
  // 改走 clipboard-manager 插件的原生写入。
  const copySelection = (event: ClipboardEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const selected = input.value.slice(input.selectionStart ?? 0, input.selectionEnd ?? 0);
    if (!selected) return;

    event.preventDefault();
    void writeText(selected);
  };

  const cutSelection = (event: ClipboardEvent<HTMLInputElement>) => {
    const input = event.currentTarget;
    const start = input.selectionStart ?? 0;
    const end = input.selectionEnd ?? 0;
    const selected = input.value.slice(start, end);
    if (!selected) return;

    event.preventDefault();
    void writeText(selected);
    setValue(input.value.slice(0, start) + input.value.slice(end));
    setIsInvalid(false);
    requestAnimationFrame(() => {
      inputRef.current?.setSelectionRange(start, start);
    });
  };

  if (isEditing) {
    return (
      <form
        className="flex min-w-0 flex-1 items-center gap-2"
        onSubmit={(event) => void submitPath(event)}
      >
        <FolderIcon className="pointer-events-none size-4 shrink-0 text-muted-foreground" />
        <Input
          ref={inputRef}
          aria-invalid={isInvalid}
          aria-label="当前路径"
          className="h-7"
          onChange={(event) => {
            setValue(event.target.value);
            setIsInvalid(false);
          }}
          onBlur={() => {
            setIsEditing(false);
            setIsInvalid(false);
          }}
          onCopy={copySelection}
          onCut={cutSelection}
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
      className="-mx-1.5 -my-1 flex min-w-0 flex-1 cursor-default items-center rounded-md px-1.5 py-1 transition-colors hover:bg-accent/60"
      data-tauri-drag-region="false"
      onClick={startEditing}
      title="单击以编辑路径"
    >
      <ExplorerBreadcrumbs breadcrumbs={directory.breadcrumbs} onNavigate={onNavigate} />
    </div>
  );
}
