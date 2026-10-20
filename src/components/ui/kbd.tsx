import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * Keycap chip: a small, quiet key hint used in the command bar, menus and
 * footers.
 *
 * The cap is the reference's `.kbd` — 19px on a 4px corner, mono 11px at
 * medium weight, a two-tone face (a light step over a dark one) and a double
 * edge: an inset hairline of the text's own ink along the top, a 1.5px contact
 * shadow below, and a 1px ring for the key's outline. Three cues, all of them
 * at 7–9%: a keycap on a matte panel is a 1px-tall object, and anything
 * heavier turns a shortcut hint into a button. `.kbd-cap` in App.css carries
 * the values (both schemes), so a call site that needs a smaller key overrides
 * height and type and keeps the treatment.
 */
function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return (
    <kbd
      className={cn(
        "kbd-cap inline-flex shrink-0 items-center justify-center select-none",
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
