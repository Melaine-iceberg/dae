import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * Raycast-style keyboard hint chip: a small, quiet keycap used in the
 * command palette, menus and footers. Sits inline with 13px UI text; the
 * bordered face + muted fill read as a key, not as body text.
 */
function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-xs border border-border/70 bg-muted/70 px-1 font-sans text-nano font-medium tracking-wide text-muted-foreground tabular-nums select-none dark:bg-muted/50",
        className,
      )}
      {...props}
    />
  );
}

export { Kbd };
