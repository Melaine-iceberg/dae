import { Progress as ProgressPrimitive } from "@base-ui/react/progress";

import { cn } from "@/lib/utils";

/* Material 3 Expressive wavy progress: the indicator is filled but carries a
   travelling sine wave rendered with an SVG pattern, the signature expressive
   loading treatment. The wave uses the on-primary tone so it stays legible on
   both schemes. */
function WaveTexture() {
  return (
    <svg
      aria-hidden="true"
      className="absolute top-0 left-0 h-full w-[200%] animate-m3-wave text-primary-foreground/50"
    >
      <defs>
        <pattern height="6" id="m3-progress-wave" patternUnits="userSpaceOnUse" width="16">
          <path
            d="M0 3 Q4 1 8 3 T16 3"
            fill="none"
            stroke="currentColor"
            strokeLinecap="round"
            strokeWidth="2"
            vectorEffect="non-scaling-stroke"
          />
        </pattern>
      </defs>
      <rect fill="url(#m3-progress-wave)" height="100%" width="100%" x="-16" y="0" />
    </svg>
  );
}

function Progress({ className, children, value, ...props }: ProgressPrimitive.Root.Props) {
  return (
    <ProgressPrimitive.Root
      value={value}
      data-slot="progress"
      className={cn("flex flex-wrap gap-3", className)}
      {...props}
    >
      {children}
      <ProgressTrack>
        <ProgressIndicator />
      </ProgressTrack>
    </ProgressPrimitive.Root>
  );
}

function ProgressTrack({ className, ...props }: ProgressPrimitive.Track.Props) {
  return (
    <ProgressPrimitive.Track
      className={cn(
        "relative flex h-1.5 w-full items-center overflow-x-hidden rounded-full bg-muted",
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
      data-slot="progress-indicator"
      className={cn(
        "relative h-full overflow-hidden rounded-full bg-primary transition-[width] duration-200 ease-spring",
        className,
      )}
      {...props}
    >
      <WaveTexture />
    </ProgressPrimitive.Indicator>
  );
}

function ProgressLabel({ className, ...props }: ProgressPrimitive.Label.Props) {
  return (
    <ProgressPrimitive.Label
      className={cn("text-sm font-medium", className)}
      data-slot="progress-label"
      {...props}
    />
  );
}

function ProgressValue({ className, ...props }: ProgressPrimitive.Value.Props) {
  return (
    <ProgressPrimitive.Value
      className={cn("ml-auto text-sm text-muted-foreground tabular-nums", className)}
      data-slot="progress-value"
      {...props}
    />
  );
}

export { Progress, ProgressTrack, ProgressIndicator, ProgressLabel, ProgressValue };
