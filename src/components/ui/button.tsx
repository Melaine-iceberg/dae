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
  "group/button inline-flex shrink-0 items-center justify-center rounded-[9px] border border-transparent bg-clip-padding text-body font-medium whitespace-nowrap transition-[background-color,border-color,color,box-shadow,transform] duration-fast ease-standard outline-none select-none active:scale-[0.98] focus-visible:border-ring focus-visible:ring-[3px] focus-visible:ring-ring/35 disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        // The primary fill is the foreground itself — a near-white capsule in
        // the dark scheme, a near-black one in daylight — not the accent. That
        // is the reference's `.btn-primary`: the accent is spent on selection
        // and focus, and a window whose primary action were the same blue as
        // its selection would have one state too many. Raycast's own action
        // panels do exactly this, and it is why their blue stays scarce.
        default:
          "rounded-full bg-foreground text-background hover:bg-[color-mix(in_oklab,var(--foreground)_90%,var(--background))] active:bg-[color-mix(in_oklab,var(--foreground)_82%,var(--background))]",
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
          "h-8 gap-1.5 px-3.5 has-data-[icon=inline-end]:pr-2.5 has-data-[icon=inline-start]:pl-2.5",
        xs: "h-6 gap-1 px-2 text-caption has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-7 gap-1 px-2.5 text-caption has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-9 gap-2 px-4 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        icon: "size-8",
        "icon-xs": "size-5 [&_svg:not([class*='size-'])]:size-3",
        "icon-sm": "size-7",
        "icon-lg": "size-9",
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
