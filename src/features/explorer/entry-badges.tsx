import type { ReactNode } from "react";
import { LockIcon } from "@solar-icons/react/line-duotone";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";

import type { DirectoryEntry } from "./types";

/**
 * Hidden entries stay listed but dimmed (Finder-style). Keep this before the
 * dragging class inside `cn()` so tailwind-merge lets dragging win.
 */
export const HIDDEN_ENTRY_CLASS = "opacity-60";

/**
 * The drag source: the row the pointer is carrying, half-faded so the
 * destination reads louder than the thing being moved. Named here beside
 * `HIDDEN_ENTRY_CLASS` rather than copied at each view — list, grid and column
 * all draw one drag treatment, and the opacity has to stay a utility so
 * tailwind-merge resolves it against the hidden dim above instead of a
 * component-layer class losing to it.
 */
export const DRAG_SOURCE_CLASS = "cursor-grabbing opacity-50";

function ReadOnlyBadge({ className, size }: { className?: string; size: "sm" | "md" }) {
  const { t } = useTranslation("explorer");
  const label = t("badges.readOnly");

  // Full-strength ink on the lock: the badge is a *state* (the file cannot be
  // written), and at 6–8px the muted step rendered it as dust on the plate.
  return (
    <span
      aria-label={label}
      className={cn(
        "flex items-center justify-center rounded-full bg-card shadow-ambient-xs ring-1 ring-border",
        size === "sm" ? "size-2.5" : "size-3",
        className,
      )}
      title={label}
    >
      <LockIcon
        className={cn("text-foreground", size === "sm" ? "size-1.5" : "size-2")}
        fill="currentColor"
      />
    </span>
  );
}

/**
 * Positioning layer around any icon variant (type glyph, native shell bitmap,
 * or thumbnail). Read-only files get a lock overlay in the bottom left corner
 * — the OS overlay convention, diagonal to the grid's top-right Git badge.
 * Directories and symlinks are excluded: the DOS READONLY bit on folders is
 * vestigial, and links report the target's attributes.
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
