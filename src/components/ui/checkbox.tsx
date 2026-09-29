import { Checkbox as CheckboxPrimitive } from "@base-ui/react/checkbox";
import { Check } from "lucide-react";

import { cn } from "@/lib/utils";

/**
 * The shell's checkbox: a hairline 16px rounded square that fills with the
 * accent and draws a bold tick when checked, replacing the native checkbox
 * glyph that every platform renders a little differently (and none of them
 * render like the rest of this shell).
 *
 * State is carried by the Base UI data attributes rather than a hidden
 * `:checked` selector, so the box and the tick stay in the same paint as the
 * surrounding text tokens.
 */
function Checkbox({ className, ...props }: CheckboxPrimitive.Root.Props) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        // Same recess as a text input: hairline edge, no fill of its own,
        // 2px halo on focus. The 5px "micro" radius reads as a rounded square
        // at 16px — the control radius would leave a circle.
        "inline-flex size-4 shrink-0 cursor-pointer items-center justify-center rounded-xs border border-input bg-transparent text-primary-foreground transition-[background-color,border-color,box-shadow] duration-fast ease-standard outline-none select-none",
        "hover:border-ring/60 focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40",
        "data-checked:border-primary data-checked:bg-primary data-indeterminate:border-primary data-indeterminate:bg-primary",
        "data-disabled:pointer-events-none data-disabled:cursor-not-allowed data-disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        className="flex size-full items-center justify-center text-current transition-transform duration-fast ease-standard data-[starting-style]:scale-75"
      >
        <Check className="size-3" strokeWidth={3} />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };
