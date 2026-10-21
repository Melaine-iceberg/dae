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
      {/* The reference's launcher rhythm: one column, capped at a readable
          measure, left in the window's own gutter. The surface used to fill
          `max-w-6xl` with 9-gap sections and 36px of air, which is a marketing
          page's pacing — a file manager's home is a list of places. */}
      <div className="mx-auto flex w-full max-w-[52rem] flex-col gap-8 px-8 pt-7 pb-16">
        {children}
      </div>
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
          <h1 className="text-title">{title}</h1>
          {description && (
            <p className="mt-1 text-caption text-muted-foreground">{description}</p>
          )}
        </div>
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </header>
  );
}

export function SectionHeader({ action, title }: { action?: ReactNode; title: string }) {
  return (
    <div className="mb-2 flex h-6 items-center justify-between gap-4">
      {/* The reference's group caption: 11px, 600, uppercase, +0.08em, in the
          meta ink. It is the one place the window uppercases, which is what
          keeps the treatment from reading as decoration — a quiet label over a
          row list, not a heading over a card grid. */}
      <h2 className="text-nano font-semibold tracking-[0.08em] text-meta uppercase">{title}</h2>
      {action}
    </div>
  );
}

/**
 * A place row — the reference's `.rail-row` / `.row` anatomy rather than a
 * card: 36px tall, 6px corner, a 16px muted glyph, the name at 13px and the
 * path as mono meta on the trailing edge. The card it replaces was 76px tall
 * with a 40px tinted squircle and a two-line stack; four of them occupied the
 * same height twelve rows do, which is the opposite of what a launcher wants.
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
  /** Colour or weight for the glyph only; the row itself stays neutral. */
  iconClassName?: string;
  onClick: () => void;
  title: string;
}) {
  return (
    <button
      className={cn(
        "group flex h-9 w-full items-center gap-2.5 rounded-md px-2.5 text-left",
        "transition-colors duration-fast ease-standard hover:bg-accent focus-visible:ring-[3px] focus-visible:ring-ring/35 focus-visible:outline-none",
      )}
      onClick={onClick}
      title={description ? `${title} · ${description}` : title}
      type="button"
    >
      <Icon
        className={cn(
          "size-4 shrink-0 text-muted-foreground transition-colors group-hover:text-foreground",
          iconClassName,
        )}
      />
      <span className="min-w-0 flex-1 truncate text-caption text-foreground">{title}</span>
      {description && (
        <span className="hidden max-w-[52%] shrink-0 truncate font-mono text-[0.65625rem] text-meta sm:block">
          {description}
        </span>
      )}
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
