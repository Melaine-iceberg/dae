import type { ComponentType, ReactNode } from "react";

import { i18n } from "@/i18n";
import { cn } from "@/lib/utils";

/** Workspace surface glyphs stay on Solar Icons — that is UI chrome, not file type. */

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
    <main aria-label={ariaLabel} className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-9 px-8 pt-9 pb-14">{children}</div>
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
  /** A node rather than a string: the Trash pairs its summary with a keycap
   *  hint when a selection exists. */
  description?: ReactNode;
  icon?: ReactNode;
  title: string;
}) {
  return (
    <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
      <div className="flex min-w-0 items-center gap-3.5">
        {icon}
        <div className="min-w-0">
          {/* `title`, not a display step: this is a file manager's home
              surface. An 18px/650 heading over a slogan reads as a landing
              page, which is exactly the impression to avoid. */}
          <h1 className="text-[1.75rem] leading-9 font-semibold tracking-[-0.03em]">{title}</h1>
          {description && <p className="mt-1.5 text-caption text-muted-foreground">{description}</p>}
        </div>
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}

export function SectionHeader({ action, title }: { action?: ReactNode; title: string }) {
  return (
    <div className="mb-3.5 flex items-center justify-between gap-4">
      {/* Sentence case, no tracking: a group caption in a file manager is a
          quiet 12px label. Uppercasing and tracking it out does nothing for
          the hanzi this is usually set in and reads as decoration when it is
          set in Latin. */}
      <h2 className="text-[13px] font-semibold text-foreground/85">{title}</h2>
      {action}
    </div>
  );
}

/**
 * Card for a folder, location, or space — one of the few places the design
 * system uses cards. Hierarchy: icon → name → useful metadata.
 *
 * The chip is always `bg-secondary`. Colour, where there is any, lives on the
 * glyph and only when it carries meaning (a favourite's accent star) or
 * identity (a Space's hue). The gradient squircle that used to sit here was
 * the loudest thing on the surface and said nothing the name did not.
 */
export function LocationCard({
  description,
  icon: Icon,
  iconClassName,
  onClick,
  title,
}: {
  description?: string;
  icon: ComponentType<{ className?: string }>;
  /** Colour or weight for the glyph only; the chip plate stays neutral. */
  iconClassName?: string;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      className={cn(
        // A quick-access tile: a quiet plate that answers the pointer by
        // stepping *up* — the surface brightens, the border warms towards the
        // accent, the icon plate lifts a hair. Nothing scales.
        "group relative flex h-[4.75rem] w-full items-center gap-3.5 overflow-hidden rounded-2xl border border-border/60 bg-secondary/70 px-3.5 text-left",
        "transition-[background-color,border-color,box-shadow] duration-normal ease-standard",
        "hover:border-primary/30 hover:bg-secondary hover:shadow-ambient-xs",
        "focus-visible:ring-[3px] focus-visible:ring-ring/35 focus-visible:outline-none",
      )}
      onClick={onClick}
      title={description ? `${title} · ${description}` : title}
      type="button"
    >
      <span
        className={cn(
          // The plate takes a trace of the glyph's own hue, so a row of
          // quick-access tiles reads as a palette of places rather than as
          // identical grey boxes. `currentColor` is the tone class below.
          "flex size-10 shrink-0 items-center justify-center rounded-xl bg-[color-mix(in_oklab,currentColor_12%,var(--card))] transition-[background-color,box-shadow] duration-normal group-hover:shadow-ambient-xs",
          iconClassName,
        )}
      >
        <Icon className="size-[19px]" />
      </span>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-body font-medium tracking-[-0.006em]">{title}</span>
        {description && (
          <span className="mt-0.5 block truncate text-micro text-muted-foreground">
            {description}
          </span>
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
