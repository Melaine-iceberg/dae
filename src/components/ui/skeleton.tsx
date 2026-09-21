import { cn } from "@/lib/utils";

/**
 * A placeholder block that must match the geometry of the content it stands in
 * for (same tokens, same height) so nothing jumps when the real data arrives —
 * hence the control radius and `bg-muted` fill rather than a shimmer sweep.
 */
function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn("animate-pulse rounded-sm bg-muted", className)}
      {...props}
    />
  );
}

export { Skeleton };
