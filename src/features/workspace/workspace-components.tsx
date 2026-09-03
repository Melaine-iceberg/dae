import type { CSSProperties, ReactNode } from "react";

import { i18n } from "@/i18n";
import { cn } from "@/lib/utils";

import type { PhosphorIcon } from "@/features/explorer/file-icons";

/**
 * Shared building blocks for the workspace surfaces (Overview, Recents,
 * Favorites, Space). They encode the design system's surface hierarchy:
 * the page stays on the workspace surface while cards provide one
 * elevated level for grouped content.
 */

export function WorkspacePage({
  "aria-label": ariaLabel,
  children,
}: {
  "aria-label": string;
  children: ReactNode;
}) {
  return (
    <main aria-label={ariaLabel} className="min-h-0 flex-1 overflow-y-auto bg-card">
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-8 px-8 pt-8 pb-12">{children}</div>
    </main>
  );
}

export function WorkspacePageHeader({
  actions,
  description,
  icon,
  title,
}: {
  actions?: ReactNode;
  description?: string;
  icon?: ReactNode;
  title: string;
}) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
      <div className="flex min-w-0 items-center gap-3.5">
        {icon}
        <div className="min-w-0">
          <h1 className="text-display">{title}</h1>
          {description && <p className="mt-1 text-sm text-muted-foreground">{description}</p>}
        </div>
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}

export function SectionHeader({ action, title }: { action?: ReactNode; title: string }) {
  return (
    <div className="mb-3 flex items-center justify-between gap-4">
      <h2 className="text-label tracking-[0.05em] text-muted-foreground uppercase">{title}</h2>
      {action}
    </div>
  );
}

/**
 * Expressive card for a folder, location, or space — one of the few places
 * the design system uses cards. Hierarchy: icon → name → useful metadata.
 */
export function LocationCard({
  description,
  icon: Icon,
  iconClassName,
  iconWeight,
  onClick,
  tileClassName,
  tileStyle,
  title,
}: {
  description?: string;
  icon: PhosphorIcon;
  iconClassName?: string;
  iconWeight?: "regular" | "fill" | "bold" | "duotone" | "light" | "thin";
  onClick: () => void;
  /** Overrides the chip's neutral tone (e.g. "tile-folder", space accents). */
  tileClassName?: string;
  /** Inline tint for the chip (see tintStyle in icon-tile.tsx). */
  tileStyle?: CSSProperties;
  title: string;
}) {
  return (
    <button
      className={cn(
        "group flex w-full items-center gap-3 rounded-xl bg-muted/60 p-3 text-left",
        "transition-[background-color,box-shadow,translate,scale] duration-fast ease-standard",
        "hover:-translate-y-0.5 hover:bg-accent/70 hover:shadow-ambient-sm",
        "active:translate-y-0 active:scale-[0.98] active:shadow-none",
        "focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none",
      )}
      onClick={onClick}
      title={description ? `${title} · ${description}` : title}
      type="button"
    >
      <span
        className={cn(
          "flex size-9 shrink-0 items-center justify-center rounded-lg",
          !tileClassName && !tileStyle && "bg-secondary",
          tileClassName,
        )}
        style={tileStyle}
      >
        <Icon className={cn("size-4.5", iconClassName)} weight={iconWeight} />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px] font-medium">{title}</span>
        {description && (
          <span className="mt-0.5 block truncate text-xs text-muted-foreground">{description}</span>
        )}
      </span>
    </button>
  );
}

/** Computes the parent portion of a path for either separator style. */
export function parentPathOf(path: string): string | null {
  const trimmed = path.replace(/[\\/]+$/, "");
  const separatorIndex = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (separatorIndex < 0) return null;

  const parent = trimmed.slice(0, separatorIndex);
  if (/^[a-zA-Z]:$/.test(parent)) {
    return `${parent}${trimmed[separatorIndex]}`;
  }

  return parent || trimmed[separatorIndex] || null;
}

/** Computes the final path segment for either separator style. */
export function baseNameOf(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const separatorIndex = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const name = separatorIndex >= 0 ? trimmed.slice(separatorIndex + 1) : trimmed;

  return name || path;
}

/** Relative timestamp for recents lists: today → HH:mm, yesterday → label,
 *  older → M/d. */
export function formatRecentTime(accessedAt: number, yesterdayLabel: string): string {
  const date = new Date(accessedAt);
  const now = new Date();
  const startOfToday = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime();
  const startOfDate = new Date(date.getFullYear(), date.getMonth(), date.getDate()).getTime();

  if (startOfDate === startOfToday) {
    return date.toLocaleTimeString(i18n.language, {
      hour: "2-digit",
      minute: "2-digit",
      hour12: false,
    });
  }

  if (startOfToday - startOfDate === 86_400_000) {
    return yesterdayLabel;
  }

  return date.toLocaleDateString(i18n.language, { month: "numeric", day: "numeric" });
}
