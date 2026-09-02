import type { ReactNode } from "react";
import { LockIcon } from "@phosphor-icons/react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

import type { DirectoryEntry } from "./types";

/**
 * Hidden entries stay listed but dimmed (Finder-style). Keep this before the
 * dragging `opacity-50` inside `cn()` so tailwind-merge lets dragging win.
 */
export const HIDDEN_ENTRY_CLASS = "opacity-60";

function ReadOnlyBadge({ className, size }: { className?: string; size: "sm" | "md" }) {
  const { t } = useTranslation("explorer");
  const label = t("badges.readOnly");

  return (
    <span
      aria-label={label}
      className={cn(
        "flex items-center justify-center rounded-full bg-card shadow-ambient-xs ring-1 ring-border/60",
        size === "sm" ? "size-2.5" : "size-3",
        className,
      )}
      title={label}
    >
      <LockIcon
        className={cn("text-muted-foreground", size === "sm" ? "size-1.5" : "size-2")}
        weight="fill"
      />
    </span>
  );
}

/**
 * Positioning layer around any icon variant (Phosphor svg, native shell
 * bitmap, or thumbnail). Read-only files get a lock overlay in the bottom
 * left corner — the OS overlay convention, diagonal to the grid's top-right
 * Git badge. Directories and symlinks are excluded: the DOS READONLY bit on
 * folders is vestigial, and links report the target's attributes.
 */
export function EntryIconFrame({
  badgeSize = "sm",
  children,
  className,
  entry,
}: {
  badgeSize?: "md" | "sm";
  children: ReactNode;
  className?: string;
  entry: DirectoryEntry;
}) {
  return (
    <span className={cn("relative inline-flex shrink-0", className)}>
      {children}
      {entry.kind === "file" && entry.readOnly && (
        <ReadOnlyBadge className="absolute -bottom-0.5 -left-0.5" size={badgeSize} />
      )}
    </span>
  );
}
