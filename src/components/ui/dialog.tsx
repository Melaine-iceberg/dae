import * as React from "react";
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { CloseIcon } from "@solar-icons/react/line-duotone";

/**
 * Entrance/exit motion for a dialog. `float` is the house behaviour: the popup
 * fades in with a 2px rise. `none` puts it on screen and off screen inside a
 * single frame.
 *
 * `none` exists for dialogs where the fade reads as the *window* flickering
 * rather than as the dialog arriving. That is what an animated fade does when
 * a window material is active: the canvas is translucent, so anything animating
 * at this level spends the duration below `--pane-alpha` and the desktop shows
 * through. The Open With picker is the dialog that asked for it.
 */
type DialogMotion = "float" | "none";

const BACKDROP_MOTION =
  "duration-fast data-open:animate-in data-open:fade-in-0 data-closed:animate-out data-closed:fade-out-0 data-closed:fill-mode-forwards";

const POPUP_MOTION =
  "data-open:animate-float-in data-closed:animate-out data-closed:fade-out-0 data-closed:duration-instant data-closed:ease-standard-accelerate data-closed:fill-mode-forwards";

function Dialog({ ...props }: DialogPrimitive.Root.Props) {
  return <DialogPrimitive.Root data-slot="dialog" {...props} />;
}

function DialogTrigger({ ...props }: DialogPrimitive.Trigger.Props) {
  return <DialogPrimitive.Trigger data-slot="dialog-trigger" {...props} />;
}

function DialogPortal({ ...props }: DialogPrimitive.Portal.Props) {
  return <DialogPrimitive.Portal data-slot="dialog-portal" {...props} />;
}

function DialogClose({ ...props }: DialogPrimitive.Close.Props) {
  return <DialogPrimitive.Close data-slot="dialog-close" {...props} />;
}

function DialogOverlay({
  className,
  motion = "float",
  ...props
}: DialogPrimitive.Backdrop.Props & { motion?: DialogMotion }) {
  return (
    <DialogPrimitive.Backdrop
      data-slot="dialog-overlay"
      className={cn(
        // Deliberately unfilled: nothing dims behind a dialog in this shell.
        // The backdrop still exists to catch the click that dismisses the
        // dialog, and to keep the pointer off the listing behind it — it just
        // does not paint. The dialog's own border and shadow are the whole
        // figure/ground signal.
        "fixed inset-0 isolate z-50",
        motion === "float" && BACKDROP_MOTION,
        className,
      )}
      {...props}
    />
  );
}

function DialogContent({
  className,
  children,
  showCloseButton = true,
  motion = "float",
  ...props
}: DialogPrimitive.Popup.Props & {
  showCloseButton?: boolean;
  motion?: DialogMotion;
}) {
  return (
    <DialogPortal>
      <DialogOverlay motion={motion} />
      <DialogPrimitive.Popup
        data-slot="dialog-content"
        className={cn(
          // A dialog is the one surface whose height is not chosen by the app:
          // a properties pane or a conflict list is as tall as its content, and
          // a short window would push the title bar and both buttons off-screen
          // with nothing to scroll. The cap plus the scroll container is that
          // guarantee, applied once here instead of per dialog. Dialogs that
          // manage their own height (settings) are unaffected: their body
          // already caps below this, so this scrollport never engages.
          //
          // Entrance is `POPUP_MOTION` — fade plus a 2px rise on the house
          // standard curve, or nothing at all under `motion="none"`. A dialog
          // that scales up from the centre is the one motion this language
          // does not have, and it would also fight the centring translate
          // below.
          //
          // Fill: the solid popover plane, like a menu — a dialog is not a
          // glass surface, so nothing behind it may show through the panel
          // itself. With the backdrop unpainted this fill is what separates
          // the dialog from the listing under it.
          //
          // `rounded-xl`, the one step above the menu's `rounded-md`. A dialog
          // is a lifted layer rather than a window, so there is no frame for it
          // to be rounder than, and at the menu's corner it read as a menu that
          // had grown into a dialog. See the radius scale in App.css.
          "fixed top-1/2 left-1/2 z-50 grid max-h-[calc(100dvh-2rem)] w-full max-w-[calc(100%-2rem)] -translate-x-1/2 -translate-y-1/2 gap-4 overflow-y-auto rounded-xl border border-border bg-popover p-5 text-body text-popover-foreground shadow-ambient-lg outline-none sm:max-w-sm",
          motion === "float" && POPUP_MOTION,
          className,
        )}
        {...props}
      >
        {children}
        {showCloseButton && (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            render={
              <Button variant="ghost" className="absolute top-2.5 right-2.5" size="icon-sm" />
            }
          >
            <CloseIcon />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        )}
      </DialogPrimitive.Popup>
    </DialogPortal>
  );
}

function DialogHeader({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div data-slot="dialog-header" className={cn("flex flex-col gap-1.5", className)} {...props} />
  );
}

/**
 * Footer: the dialog's action row, divided from the body by a hairline rather
 * than tinted into a second surface — the shell's depth ladder has no rung for
 * a footer band. Cancel sits left of the primary action (ghost, outline or
 * destructive), right-aligned.
 */
function DialogFooter({
  className,
  showCloseButton = false,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  showCloseButton?: boolean;
}) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn(
        "-mx-5 -mb-5 flex flex-col-reverse gap-2 rounded-b-xl border-t border-border px-5 py-4 sm:flex-row sm:justify-end",
        className,
      )}
      {...props}
    >
      {children}
      {showCloseButton && (
        <DialogPrimitive.Close render={<Button variant="outline" />}>Close</DialogPrimitive.Close>
      )}
    </div>
  );
}

function DialogTitle({ className, ...props }: DialogPrimitive.Title.Props) {
  return (
    <DialogPrimitive.Title
      data-slot="dialog-title"
      className={cn("font-heading text-title leading-tight", className)}
      {...props}
    />
  );
}

function DialogDescription({ className, ...props }: DialogPrimitive.Description.Props) {
  return (
    <DialogPrimitive.Description
      data-slot="dialog-description"
      className={cn(
        "text-caption text-muted-foreground *:[a]:underline *:[a]:underline-offset-3 *:[a]:hover:text-foreground",
        className,
      )}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
