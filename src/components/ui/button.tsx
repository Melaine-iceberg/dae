import { Button as ButtonPrimitive } from "@base-ui/react/button";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/* Linear/Raycast buttons: compact corners, tight paddings, quick state
   transitions. The filled variant stays restrained — a single accent tone,
   hover brightens slightly, no shape morphing.

   Nothing scales on press. A button that shrinks to 0.98 and springs back is
   the tell of a component library demoing its spring solver: real desktop
   chrome answers a click with a fill change and nothing else. */
const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-sm border border-transparent bg-clip-padding text-body font-medium whitespace-nowrap transition-[background-color,border-color,color,box-shadow] duration-fast ease-standard outline-none select-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40 disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        // The primary fill carries a hair of light from above — the same
        // aurora the frame roles get — so a filled button reads as a lit key
        // rather than as a flat swatch, and lifts on hover with a soft accent
        // glow under it. One gradient step, no gloss, no bevel.
        default:
          "bg-primary text-primary-foreground shadow-[0_1px_2px_-1px_color-mix(in_oklab,var(--shadow-color)_var(--shadow-a2),transparent)] hover:bg-[color-mix(in_oklab,var(--primary),var(--primary-foreground)_10%)] hover:shadow-[0_2px_6px_-2px_color-mix(in_oklab,var(--shadow-color)_var(--shadow-a2),transparent)]",
        outline:
          "border-input bg-transparent text-foreground hover:bg-accent hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-[color-mix(in_oklab,var(--secondary),var(--secondary-foreground)_10%)] aria-expanded:bg-secondary aria-expanded:text-secondary-foreground",
        ghost:
          "hover:bg-accent hover:text-foreground aria-expanded:bg-accent aria-expanded:text-foreground",
        destructive:
          "bg-destructive text-on-destructive hover:bg-[color-mix(in_oklab,var(--destructive),var(--on-destructive)_10%)] focus-visible:border-destructive focus-visible:ring-destructive/40",
        link: "text-primary underline-offset-4 hover:underline",
      },
      size: {
        default:
          "h-7 gap-1.5 px-3 has-data-[icon=inline-end]:pr-2.5 has-data-[icon=inline-start]:pl-2.5",
        xs: "h-5 gap-1 px-2 text-caption has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-6 gap-1 px-2.5 text-caption has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-8 gap-1.5 px-3.5 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        icon: "size-7",
        "icon-xs": "size-5 [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-6",
        "icon-lg": "size-8",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  },
);

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  return (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}

export { Button, buttonVariants };
