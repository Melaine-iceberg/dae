import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * Keycap chip: a small, quiet key hint used in the command bar, menus and
 * footers. Sits inline with 13px UI text; the hairline face on a recessed
 * fill reads as a key rather than as body text — hence `text-nano` on the
 * micro 4px radius rather than a control radius. Sans, not mono: at 10px a
 * monospaced face reads heavier than the label beside it.
 */
function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-xs border border-border bg-muted px-1 font-sans text-nano font-medium text-muted-foreground tabular-nums select-none",
        className,
      )}
      {...props}
    />
  );
}

/**
 * One chip per chord step of an already-formatted binding ("Ctrl+Shift+P" →
 * three keys). Menus and the command bar both read their shortcuts this way:
 * Linear shows the key, not the key's name, and a chip survives a rebound
 * binding without re-measuring the row.
 */
function KbdShortcut({ className, keys }: { className?: string; keys: string }) {
  return (
    <span className={cn("flex shrink-0 items-center gap-0.5", className)}>
      {keys.split("+").map((token) => (
        <Kbd key={token}>{token}</Kbd>
      ))}
    </span>
  );
}

export { Kbd, KbdShortcut };
