import { cn } from "@/lib/utils";

import type { ExtensionPresentation } from "./file-icons";

/**
 * Entry icon frame: the type artwork seated in a fixed square cell so it
 * stays aligned with native shell icons and thumbnails in the same view.
 * Catppuccin artwork is full-bleed and self-colored, so there is
 * no tinted squircle behind it — the cell is pure layout.
 */

export function TypeIconTile({
  className,
  iconClassName,
  iconSize,
  presentation,
}: {
  /** Cell geometry: a square size plus `tile-radius` (e.g. "size-11 tile-radius"). */
  className?: string;
  iconClassName?: string;
  iconSize: number;
  presentation: ExtensionPresentation;
}) {
  const Icon = presentation.icon;

  return (
    <span aria-hidden="true" className={cn("flex shrink-0 items-center justify-center", className)}>
      <Icon className={iconClassName} size={iconSize} />
    </span>
  );
}
