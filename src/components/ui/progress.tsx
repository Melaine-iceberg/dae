import { Progress as ProgressPrimitive } from "@base-ui/react/progress";

import { cn } from "@/lib/utils";

/* Island progress: a thin track with a rounded fill. No decorative waves — a
   progress bar's job is to communicate state, fast.

   Two sizes, because the strip progress sits in a 32px status bar built out of
   `text-micro` text, where a 4px track out-weighs the line above it. */
function Progress({
  className,
  children,
  size = "default",
  value,
  ...props
}: ProgressPrimitive.Root.Props & { size?: "default" | "sm" }) {
  return (
    <ProgressPrimitive.Root
      value={value}
      data-slot="progress"
      className={cn("flex flex-wrap gap-3", className)}
      {...props}
    >
      {children}
      <ProgressTrack className={cn(size === "sm" && "h-0.5")}>
        <ProgressIndicator className={cn(size === "sm" && "rounded-full")} />
      </ProgressTrack>
    </ProgressPrimitive.Root>
  );
}

function ProgressTrack({ className, ...props }: ProgressPrimitive.Track.Props) {
  return (
    <ProgressPrimitive.Track
      className={cn(
        "relative flex h-1 w-full items-center overflow-x-hidden rounded-xs bg-muted",
        className,
      )}
      data-slot="progress-track"
      {...props}
    />
  );
}

function ProgressIndicator({ className, ...props }: ProgressPrimitive.Indicator.Props) {
  return (
    <ProgressPrimitive.Indicator
      className={cn(
        "relative h-full rounded-xs bg-primary transition-[width] duration-normal ease-standard",
        className,
      )}
      data-slot="progress-indicator"
      {...props}
    />
  );
}

function ProgressLabel({ className, ...props }: ProgressPrimitive.Label.Props) {
  return (
    <ProgressPrimitive.Label
      className={cn("text-caption font-medium", className)}
      data-slot="progress-label"
      {...props}
    />
  );
}

function ProgressValue({ className, ...props }: ProgressPrimitive.Value.Props) {
  return (
    <ProgressPrimitive.Value
      className={cn("ml-auto font-mono text-caption text-muted-foreground tabular-nums", className)}
      data-slot="progress-value"
      {...props}
    />
  );
}

export { Progress, ProgressTrack, ProgressIndicator, ProgressLabel, ProgressValue };
