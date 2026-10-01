import { useSyncExternalStore } from "react";
import { CircleAlert, CircleCheck, CircleX, Info, LoaderCircle } from "lucide-react";
import { Toaster as Sonner, type ToasterProps } from "sonner";

/**
 * The shell's notification host — the single mount point every transient
 * message in the app reports through. Call sites do not render this; they call
 * `notify` from `@/lib/notifications`, which is the only module permitted to
 * import `sonner` directly (see that file for the vocabulary).
 *
 * Sonner owns the queue, the stacking, the timers and the swipe; this file owns
 * the parts that must agree with the shell:
 *
 * - the THEME, which Sonner cannot read by itself. `theme` is a class on
 *   `<html>` rather than the OS preference (src/lib/theme.ts lets the user pin
 *   light on a dark desktop), so `prefers-color-scheme` is the wrong answer and
 *   the class is observed instead.
 * - the ICONS. Sonner's own set is a filled circle glyph family that belongs to
 *   no icon system in this app; the types carry the same lucide glyphs and the
 *   same semantic tokens the rest of the shell uses (`--success`, `--warning`,
 *   `--info`), so a notification's status reads identically to a Git badge's.
 *   `richColors` stays off deliberately: a toast painted end to end in its
 *   status hue is the one treatment App.css reserves for `Alert`.
 * - the GEOMETRY, which is denser than Sonner's default and lives in App.css
 *   next to the other third-party overrides — Tailwind utilities cannot win
 *   that cascade (Sonner injects its stylesheet unlayered, after the entry CSS).
 *
 * Colour is not handled here at all: App.css points Sonner's `--normal-*`
 * variables at `--popover` / `--popover-foreground` / `--border`, which already
 * flip with the theme. `theme` only has to keep Sonner's own few
 * theme-dependent defaults in step.
 */

/** Reads the shell's resolved theme off `<html>`, where `lib/theme.ts` writes it. */
function subscribeToDocumentTheme(onStoreChange: () => void): () => void {
  const observer = new MutationObserver(onStoreChange);
  observer.observe(document.documentElement, { attributeFilter: ["class"], attributes: true });
  return () => observer.disconnect();
}

function readDocumentTheme(): "light" | "dark" {
  return document.documentElement.classList.contains("dark") ? "dark" : "light";
}

function Toaster({ position = "bottom-right", ...props }: ToasterProps) {
  const theme = useSyncExternalStore(subscribeToDocumentTheme, readDocumentTheme, () => "light");

  return (
    <Sonner
      closeButton
      gap={8}
      icons={{
        // 16px is the cell Sonner reserves for the glyph; the tokens are what
        // makes a "did this work" glance possible without reading the sentence.
        success: <CircleCheck className="size-4 text-success" />,
        error: <CircleX className="size-4 text-destructive" />,
        warning: <CircleAlert className="size-4 text-warning" />,
        info: <Info className="size-4 text-info" />,
        loading: <LoaderCircle className="size-4 animate-spin text-muted-foreground" />,
      }}
      offset={16}
      position={position}
      theme={theme}
      {...props}
    />
  );
}

export { Toaster };
