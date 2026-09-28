import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { getAppWindow } from "@/lib/app-window";
import { isMacPlatform } from "@/lib/platform";
import { watchWindowFocus } from "@/lib/window-focus";

const appWindow = getAppWindow();

type GlyphKind = "close" | "maximize" | "minimize" | "restore";

/**
 * Window chrome for a frameless window. `tauri.conf.json` sets
 * `decorations: false` on every platform, so the close/minimize/zoom buttons
 * are the app's job everywhere — there is no OS-drawn fallback to lean on.
 *
 * Two frames, three operations, one component:
 *
 *  * The glyphs are inline SVG, never a private-use font. The old code drew
 *    them out of "Segoe Fluent Icons", which ships with Windows 10 and with
 *    nothing else — on macOS and Linux every window had four tofu boxes in the
 *    corner. A 1px stroke in a small box also stays crisp at any DPI, where a
 *    font glyph rasterises once and scales blurry.
 *
 *  * The frame follows the platform while the operations do not. macOS puts
 *    traffic lights at the leading edge, as a group whose glyphs only appear
 *    under the pointer and whose colours drain to grey while the window is
 *    inactive. Windows and Linux put the caption cluster at the trailing edge,
 *    where it is three *plates* on a 4px rhythm rather than three full-bleed
 *    slabs — `.window-control` in App.css is where that lives, and why.
 */
function CaptionGlyph({ kind, size = 12 }: { kind: GlyphKind; size?: number }) {
  return (
    <svg
      aria-hidden="true"
      fill="none"
      height={size}
      stroke="currentColor"
      strokeLinecap="round"
      strokeLinejoin="round"
      /* One hairline at either scale: the geometry is written once on the 12
         grid and `size` maps it, so the stroke has to travel with it or the
         stoplights would draw four times as heavy as the caption cluster. */
      strokeWidth={12 / size}
      viewBox="0 0 12 12"
      width={size}
    >
      {kind === "minimize" && <path d="M2.5 6h7" />}
      {kind === "maximize" && <rect height="7" width="7" x="2.5" y="2.5" />}
      {/* Restore is two 5-unit squares offset by 3, not by 2. At a 1px stroke a
          2-unit offset leaves only 1px of channel between the pair's two right
          edges, and at 12px that reads as one smudged double line rather than
          as two windows; 3 leaves 2px of air, the least that still separates
          them. The pair then spans 9 units against the single square's 8, so
          toggling maximize does not read as the glyph shrinking.

          The back plate is an open path that stops on the front square's own
          edges, so the two never cross: its leg starts on the front's top edge
          and ends on its right edge, which is what makes the pair read as
          stacked rather than as overlapping. */}
      {kind === "restore" && (
        <>
          <path d="M5 5V2h5v5h-3" />
          <rect height="5" width="5" x="2" y="5" />
        </>
      )}
      {/* Round caps put the X's ends a hair past its box, so it is drawn a
          half-step inside the square's 7 units — at this size the diagonal
          carries more ink than the square it sits beside, and matching the two
          boxes would make Close read heavier than Maximize. */}
      {kind === "close" && <path d="M3 3l6 6M9 3l-6 6" />}
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
          glyph={<CaptionGlyph kind="close" size={10} />}
          kind="close"
          label={t("windowControls.close")}
          onClick={actions.close}
        />
        <TrafficLight
          glyph={<CaptionGlyph kind="minimize" size={10} />}
          kind="minimize"
          label={t("windowControls.minimize")}
          onClick={actions.minimize}
        />
        <TrafficLight
          glyph={<CaptionGlyph kind={maximized ? "restore" : "maximize"} size={10} />}
          kind="zoom"
          label={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
          onClick={actions.toggleMaximize}
        />
      </div>
    );
  }

  return (
    <div
      className="window-controls flex h-full shrink-0 items-stretch"
      data-focused={focused ? "true" : "false"}
      data-slot="window-controls"
    >
      <CaptionButton
        action="minimize"
        glyph={<CaptionGlyph kind="minimize" />}
        label={t("windowControls.minimize")}
        onClick={actions.minimize}
      />
      <CaptionButton
        action="maximize"
        glyph={<CaptionGlyph kind={maximized ? "restore" : "maximize"} />}
        // Named by the snap layout plugin in src-tauri/src/lib.rs, which finds
        // this button by id to hang its flyout off. Not a styling hook.
        id="window-maximize"
        label={maximized ? t("windowControls.restore") : t("windowControls.maximize")}
        onClick={actions.toggleMaximize}
      />
      <CaptionButton
        action="close"
        glyph={<CaptionGlyph kind="close" />}
        label={t("windowControls.close")}
        onClick={actions.close}
      />
    </div>
  );
}

/**
 * A caption button: the full-height target, with the painted plate inside it.
 * The two are separate elements because they are separate jobs — the target
 * has to reach the window edge to keep Fitts' law intact, the plate has to sit
 * inset so it can be a rounded key with air around it. Styling is in App.css,
 * because the maximize button's hover state has to be reachable as
 * `.is-hovered`, which the snap layout plugin applies itself.
 */
function CaptionButton({
  action,
  glyph,
  id,
  label,
  onClick,
}: {
  action: "close" | "maximize" | "minimize";
  glyph: React.ReactNode;
  id?: string;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      aria-label={label}
      className="window-control flex h-full w-window-control shrink-0 items-center justify-center"
      data-action={action}
      /* The bar is a `deep` drag region, so the one thing inside it that is a
         control has to say so — otherwise a click on Close starts a drag. */
      data-tauri-drag-region="false"
      id={id}
      onClick={onClick}
      title={label}
      type="button"
    >
      <span className="window-control-plate flex size-window-control shrink-0 items-center justify-center rounded-sm">
        {glyph}
      </span>
    </button>
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
      data-tauri-drag-region="false"
      onClick={onClick}
      title={label}
      type="button"
    >
      <span className="traffic-glyph">{glyph}</span>
    </button>
  );
}
