import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";

/* THE EMPTY STATE — the reference `.empty` block.

   One recessed plate, one fact, one sentence, and at most one action. The
   reference draws it as `.empty` (`place-items:center`, `padding 64px 24px`),
   `.plate` (52×52, 12px radius, 5% foreground fill, muted glyph) and a 15px
   semibold `h2` over a 12.5px paragraph capped at 34ch — a measure, not a
   container width, so the sentence never runs wider than a comfortable line.

   What it replaced was a poster: a filled 40px tile over a 14.5px medium title
   and a full-width caption, with a filled button under them. An empty folder
   states a fact; the only thing louder than the fact is the action that
   changes it. */

function Empty({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty"
      className={cn(
        "flex w-full min-w-0 flex-1 flex-col items-center justify-center px-6 py-16 text-center",
        className,
      )}
      {...props}
    />
  );
}

function EmptyHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-header"
      className={cn("flex max-w-md flex-col items-center", className)}
      {...props}
    />
  );
}

const emptyMediaVariants = cva(
  "flex shrink-0 items-center justify-center [&_svg]:pointer-events-none [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        default: "bg-transparent",
        // The plate: 52px on a 12px radius (`--radius-xl`), a 5% foreground
        // wash rather than a surface token — it has to read as a recess in
        // whatever plane it lands on, and the sidebar, the list and a dialog
        // body are three different planes.
        icon: "mb-4 size-[52px] rounded-xl bg-foreground/5 text-muted-foreground [&_svg:not([class*='size-'])]:size-[22px]",
      },
    },
    defaultVariants: {
      variant: "default",
    },
  },
);

function EmptyMedia({
  className,
  variant = "default",
  ...props
}: React.ComponentProps<"div"> & VariantProps<typeof emptyMediaVariants>) {
  return (
    <div
      data-slot="empty-icon"
      data-variant={variant}
      className={cn(emptyMediaVariants({ variant, className }))}
      {...props}
    />
  );
}

function EmptyTitle({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-title"
      // 15px/600 — half a step above the list's 13px rows and a step below
      // `--text-lead`. The reference number, not a scale token: the empty
      // state's heading is the only 15px type in the window.
      className={cn("mb-1.5 text-[0.9375rem] font-semibold text-foreground", className)}
      {...props}
    />
  );
}

function EmptyDescription({ className, ...props }: React.ComponentProps<"p">) {
  return (
    <div
      data-slot="empty-description"
      // 12.5px on 1.6 with a 34ch measure. `--text-caption` would be 13px and
      // 1.46 — one line taller for the same sentence, and wider.
      className={cn(
        "mb-4 max-w-[34ch] text-[0.78125rem] leading-[1.6] text-muted-foreground [&>a]:underline [&>a]:underline-offset-4 [&>a:hover]:text-primary",
        className,
      )}
      {...props}
    />
  );
}

function EmptyContent({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="empty-content"
      className={cn("flex w-full max-w-sm min-w-0 flex-col items-center gap-2.5 text-body", className)}
      {...props}
    />
  );
}

export { Empty, EmptyHeader, EmptyTitle, EmptyDescription, EmptyContent, EmptyMedia };
