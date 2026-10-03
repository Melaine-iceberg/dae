import { Select as SelectPrimitive } from "@base-ui/react/select";
import { AltArrowDownIcon } from "@solar-icons/react/line-duotone";

import { cn } from "@/lib/utils";

function Select<Value, Multiple extends boolean | undefined = false>({
  ...props
}: SelectPrimitive.Root.Props<Value, Multiple>) {
  return <SelectPrimitive.Root data-slot="select" {...props} />;
}

function SelectTrigger({ className, children, ...props }: SelectPrimitive.Trigger.Props) {
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      className={cn(
        "flex h-7 w-full min-w-0 items-center justify-between gap-2 rounded-sm border border-input bg-transparent px-2.5 py-1 text-left text-body text-foreground transition-[background-color,border-color,box-shadow] duration-fast ease-standard outline-none hover:border-ring/60 focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-ring/40 disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 data-popup-open:border-ring data-popup-open:ring-2 data-popup-open:ring-ring/40 data-popup-open:[&_svg]:rotate-180 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5 [&_svg]:text-muted-foreground [&_svg]:transition-transform [&_svg]:duration-fast [&_svg]:ease-standard",
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon className="flex text-muted-foreground">
        <AltArrowDownIcon />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

function SelectValue({ className, ...props }: SelectPrimitive.Value.Props) {
  return (
    <SelectPrimitive.Value
      data-slot="select-value"
      className={cn("min-w-0 truncate", className)}
      {...props}
    />
  );
}

function SelectContent({
  align = "start",
  alignOffset = 0,
  side = "bottom",
  sideOffset = 4,
  className,
  ...props
}: SelectPrimitive.Popup.Props &
  Pick<SelectPrimitive.Positioner.Props, "align" | "alignOffset" | "side" | "sideOffset">) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Positioner
        className="isolate z-50 outline-none"
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
      >
        <SelectPrimitive.Popup
          data-slot="select-content"
          className={cn(
            // A select popup is a menu: same plane on the glass, same
            // hairline, same entrance. It keeps the anchor's width so the open
            // list lines up with the field it came from.
            "z-50 max-h-(--available-height) w-(--anchor-width) min-w-(--anchor-width) origin-(--transform-origin) overflow-x-hidden overflow-y-auto rounded-md border border-border floating-frost p-1 text-body text-popover-foreground shadow-ambient outline-none data-open:animate-float-in data-closed:animate-out data-closed:overflow-hidden data-closed:fade-out-0 data-closed:duration-instant data-closed:ease-standard-accelerate data-closed:fill-mode-forwards",
            className,
          )}
          {...props}
        />
      </SelectPrimitive.Positioner>
    </SelectPrimitive.Portal>
  );
}

function SelectItem({ className, children, ...props }: SelectPrimitive.Item.Props) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(
        "relative flex h-7 cursor-default items-center gap-2 rounded-sm pr-2 pl-7 text-body outline-hidden select-none focus:bg-accent focus:text-accent-foreground focus:**:text-accent-foreground data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-3.5",
        className,
      )}
      {...props}
    >
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      {/* A select list is single-choice, so it takes the same dot as the menu
          radio items rather than the tick reserved for booleans. Select rows
          carry no leading icon, so the dot stays in the leading gutter. The
          child is explicit because the indicator ships a default tick. */}
      <span className="pointer-events-none absolute left-2 flex size-3 items-center justify-center">
        <SelectPrimitive.ItemIndicator>
          <span className="block size-1.5 rounded-full bg-primary" />
        </SelectPrimitive.ItemIndicator>
      </span>
    </SelectPrimitive.Item>
  );
}

export { Select, SelectContent, SelectItem, SelectTrigger, SelectValue };
