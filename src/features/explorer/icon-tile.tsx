import type { CSSProperties } from "react";

import { cn } from "@/lib/utils";

import type { ExtensionPresentation } from "./file-icons";

/**
 * Entry icon frame: the type artwork seated in a fixed square cell so it
 * stays aligned with native shell icons and thumbnails in the same view.
 * Material Icon Theme artwork is full-bleed and self-colored, so there is
 * no tinted squircle behind it — the cell is pure layout.
 */

/** Inline style for a 13% type-color tint; shared by workspace place chips. */
export function tintStyle(varName: string): CSSProperties {
  return {
    backgroundColor: `color-mix(in oklab, var(${varName}) 13%, transparent)`,
    color: `var(${varName})`,
  };
}

export function TypeIconTile({
  className,
  iconClassName,
  iconSize,
  pop = false,
  presentation,
}: {
  /** Cell geometry: size + corner radius (e.g. "size-11 rounded-[13px]"). */
  className?: string;
  iconClassName?: string;
  iconSize: number;
  /** Spring up under row/cell hover (the house entry-icon-pop curve). */
  pop?: boolean;
  presentation: ExtensionPresentation;
}) {
  const Icon = presentation.icon;

  return (
    <span
      aria-hidden="true"
      className={cn("flex shrink-0 items-center justify-center", pop && "entry-icon-pop", className)}
    >
      <Icon className={iconClassName} size={iconSize} />
    </span>
  );
}
