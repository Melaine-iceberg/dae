import { RotateCw, TriangleAlert } from "lucide-react";
import { useTranslation } from "react-i18next";

import { Button } from "@/components/ui/button";
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty";
import { cn } from "@/lib/utils";

/**
 * The failure counterpart of the empty state: a surface (or one section of a
 * surface) whose data could not be loaded.
 *
 * It exists because "we could not ask the backend" and "the backend said the
 * list is empty" used to render the same thing — every loader caught its
 * error, logged it, and substituted an empty array. The user then saw
 * "还没有收藏" for what was really a failed read, with nothing to act on.
 * An error has to look different from an empty result, and it has to carry
 * the one action that can resolve it.
 *
 * Keep this on the same primitives as `Empty` so the two read as siblings:
 * same media block, same title/description rhythm, same centering. Only the
 * glyph (warning instead of the surface's own icon) and the retry button
 * differ. `description` is the raw, already-localized backend detail and is
 * therefore not wrapped in a translation key.
 */
export function ErrorState({
  className,
  description,
  onRetry,
  title,
}: {
  /** Overrides the block's footprint; sections pass a small `min-h`. */
  className?: string;
  description?: string | null;
  onRetry?: () => void;
  title: string;
}) {
  const { t } = useTranslation("common");

  return (
    <Empty className={cn("min-h-40", className)} role="alert">
      <EmptyHeader>
        {/* A failed read is a warning, not a crash: the app is fine, the folder
            is not. `--destructive` is reserved for things the user must undo. */}
        <EmptyMedia className="text-warning" variant="icon">
          <TriangleAlert />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        {description && <EmptyDescription>{description}</EmptyDescription>}
      </EmptyHeader>
      {onRetry && (
        <Button onClick={onRetry} type="button">
          <RotateCw />
          {t("errors.retry")}
        </Button>
      )}
    </Empty>
  );
}
