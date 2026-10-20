import { cn } from "@/lib/utils";

/**
 * A placeholder block that must match the geometry of the content it stands in
 * for (same tokens, same height) so nothing jumps when the real data arrives.
 *
 * The fill is the reference's `--fg 4%` wash, not a surface token and not a
 * shimmer sweep: a skeleton is a smudge of the row that is coming, so its ink
 * is the text's ink at 4%, and its radius is the row's radius (`--radius-md`,
 * the reference's `--radius-sm`). `--muted` and `--secondary` are opaque
 * greys — on the card plane they turned a loading list into a ladder of
 * filled boxes, which is the one thing a skeleton must not be.
 *
 * The pulse stays (the reference's `.skel i` is static): a still grey block
 * over a folder that is genuinely being read from a network share reads as a
 * rendering failure, not as progress.
 */
function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("animate-pulse rounded-md bg-foreground/4", className)}
      {...props}
    />
  );
}

export { Skeleton };
