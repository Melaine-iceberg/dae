import * as React from "react";
import { Input as InputPrimitive } from "@base-ui/react/input";

import { cn } from "@/lib/utils";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <InputPrimitive
      type={type}
      data-slot="input"
      className={cn(
        // Linear input: 28px tall on the 6px control radius, a hairline edge
        // and no fill of its own — the field reads as a recess in whatever
        // plane it sits on, so one input token works on canvas, panel and
        // popover alike. Focus is the controls' 2px halo, never a third
        // opacity value.
        "h-7 w-full min-w-0 rounded-sm border border-input bg-transparent px-2.5 py-1 text-body transition-[background-color,border-color,box-shadow] duration-fast ease-standard outline-none file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-body file:font-medium file:text-foreground placeholder:text-muted-foreground hover:border-ring/60 focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20",
        className,
      )}
      {...props}
    />
  );
}

export { Input };
