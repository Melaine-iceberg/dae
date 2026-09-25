import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { cn } from "@/lib/utils";
import { getAppWindow } from "@/lib/app-window";
import { isMacPlatform } from "@/lib/platform";
import { watchWindowFocus } from "@/lib/window-focus";

const appWindow = getAppWindow();

/**
 * Window chrome for a frameless window. `tauri.conf.json` sets
 * `decorations: false` on every platform, so the close/minimize/zoom buttons
 * are the app's job everywhere — there is no OS-drawn fallback to lean on.
 *
 * Two things this has to get right that the previous version did not:
 *
 *  * The glyphs are inline SVG, never a private-use font. The old code drew
 *    them out of "Segoe Fluent Icons", which ships with Windows 10 and with
 *    nothing else — on macOS and Linux every window had four tofu boxes in
 *    the corner. A 1px stroke in a 10×10 box also stays crisp at any DPI
 *    where a font glyph rasterises once and scales blurry.
 *
 *  * The frame follows the platform while the operations do not. Windows and
 *    Linux put square caption buttons at the trailing edge (46px wide, the
 *    caption width those toolkits use) with the close button going red on
 *    hover. macOS puts traffic lights at the leading edge, as a group whose
 *    glyphs only appear under the pointer and whose colours drain to grey
 *    when the window is inactive. Same three operations, two frames — one
 *    component, because splitting it would duplicate the state plumbing.
 */

/** Hairline caption glyphs. 10×10 at 1px stroke = Windows' caption weight. */
function CaptionGlyph({ kind }: { kind: "close" | "maximize" | "minimize" | "restore" }) {
  return (
    <svg
      aria-hidden="true"
      className="size-2.5"
      fill="none"
      stroke="currentColor"
      strokeWidth={1}
      viewBox="0 0 10 10"
    >
      {kind === "minimize" && <path d="M0 5.5h10" />}
      {kind === "maximize" && <rect height="9" width="9" x="0.5" y="0.5" />}
      {/* The back square is an open path that stops where the front one
          starts, so the two never cross. Starting the path on the front
          square's own corner is what makes the pair read as stacked. */}
      {kind === "restore" && (
        <>
          <path d="M3.5 3.5V0.5h6v6h-3" />
          <rect height="6" width="6" x="0.5" y="3.5" />
        </>
      )}
      {kind === "close" && <path d="M0.5 0.5l9 9m0-9l-9 9" />}
    </svg>
  );
}

export function WindowControls() {
  const { t } = useTranslation("common");
  const [maximized, setMaximized] = useState(false);
  // The window's focus is a seam of its own (src/lib/window-focus.ts) — this
  // component is a second consumer of the same reading, not a second reader.
  const [focused, setFocused] = useState(true);

  useEffect(() => watchWindowFocus(setFocused), []);

  useEffect(() => {
    if (!appWindow) return;
    let disposed = false;
    const sync = () =>
      void appWindow.isMaximized().then((value) => {
        if (!disposed) setMaximized(value);
      });

    sync();
    const unlistenResizePromise = appWindow.onResized(sync);

    return () => {
      disposed = true;
      void unlistenResizePromise.then((unlisten) => unlisten());
    };
  }, []);

  const actions = {
    close: () => void appWindow?.close(),
    minimize: () => void appWindow?.minimize(),
    toggleMaximize: () => void appWindow?.toggleMaximize(),
  };

  if (isMacPlatform) {
    return (
      // `order-first`, not a second mount point: which edge the controls sit
      // on is part of being a window frame, so the frame decides it. AppKit
      // puts them at the leading edge ahead of everything else in the bar.
      <div
        className="traffic-lights order-first flex h-full w-traffic-lights shrink-0 items-center gap-2 pl-2"
        data-focused={focused ? "true" : "false"}
        data-slot="window-controls"
      >
        <TrafficLight
          glyph={<CaptionGlyph kind="close" />}
          kind="close"
          label={t("windowControls.close")}
          onClick={actions.close}
        />
        <TrafficLight
          glyph={<CaptionGlyph kind="minimize" />}
          kind="minimize"
          label={t("windowControls.minimize")}
          onClick={actions.minimize}
        />
        <TrafficLight
          glyph={<CaptionGlyph kind={maximized ? "restore" : "maximize"} />}
          kind="zoom"
          label={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
          onClick={actions.toggleMaximize}
        />
      </div>
    );
  }

  const buttonClassName = cn(
    "flex h-full w-window-control shrink-0 items-center justify-center transition-colors duration-instant hover:bg-accent hover:text-accent-foreground",
    focused ? "text-foreground" : "text-muted-foreground",
  );

  return (
    <div className="flex h-full shrink-0 items-stretch" data-slot="window-controls">
      <button
        aria-label={t("windowControls.minimize")}
        className={buttonClassName}
        onClick={actions.minimize}
        title={t("windowControls.minimize")}
        type="button"
      >
        <CaptionGlyph kind="minimize" />
      </button>
      <button
        id="window-maximize"
        aria-label={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
        className={buttonClassName}
        onClick={actions.toggleMaximize}
        title={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
        type="button"
      >
        <CaptionGlyph kind={maximized ? "restore" : "maximize"} />
      </button>
      <button
        aria-label={t("windowControls.close")}
        className={cn(buttonClassName, "hover:bg-destructive hover:text-on-destructive")}
        onClick={actions.close}
        title={t("windowControls.close")}
        type="button"
      >
        <CaptionGlyph kind="close" />
      </button>
    </div>
  );
}

function TrafficLight({
  glyph,
  kind,
  label,
  onClick,
}: {
  glyph: React.ReactNode;
  kind: "close" | "minimize" | "zoom";
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className="traffic-light"
      data-light={kind}
      onClick={onClick}
      title={label}
      type="button"
    >
      {/* ~6px, the weight AppKit draws inside a 12px stoplight. */}
      <span className="traffic-glyph [&>svg]:size-1.5">{glyph}</span>
    </button>
  );
}
