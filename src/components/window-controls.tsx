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
      {/* Maximize is four corner brackets — a frame opening outward — rather
          than the closed square every platform draws. The square is the one
          glyph in this set made entirely of right angles, and at a 1px stroke
          those read as four clipped points instead of as a shape; the bracket
          keeps the corners and drops the edges that carried no information,
          saying "this fills the frame" with less than half the ink.

          The box is 1.5–10.5 with 2.5-unit arms and a 1-unit radius: lucide's
          `maximize`, the library the rest of the app's icons come from, at
          half scale — 9 units on the 12 grid, the 75% it gives the glyph on
          24. Each edge lands on a half-unit, so the hairline sits on the pixel
          instead of straddling it. */}
      {kind === "maximize" && (
        <>
          <path d="M4 1.5H2.5a1 1 0 0 0-1 1V4" />
          <path d="M10.5 4V2.5a1 1 0 0 0-1-1H8" />
          <path d="M1.5 8v1.5a1 1 0 0 0 1 1H4" />
          <path d="M8 10.5h1.5a1 1 0 0 0 1-1V8" />
        </>
      )}
      {/* Restore is what maximize would leave behind: the window you are in
          (5 units, front) lying on the window it came from (7, behind), the
          pair offset by 3 on the diagonal. The brackets and the stack share
          the same 9-unit box, so toggling changes the window inside the frame
          rather than the size of the glyph.

          The back card is an open path, not a rect — its lower-left corner is
          hidden by the front card, and the two legs that walk into that corner
          stop 1 unit short of the front's edge. The round cap spends the other
          half: the front's edge stroke ends at 5.0 and the back's cap begins
          at 4.5, so the outlines clear each other by exactly the half-unit of
          air that reads as "stacked". Stopping *on* the front's edges — the
          obvious first draft — welds the two strokes into one shape.

          Both cards wear a 20% radius (1 on 5, 1.4 on 7) so the size step
          does not break them into two shapes, and the offset leaves the back
          card's exposed left and bottom legs 1.1 units each: long enough to
          read as a card edge, short enough to stay a peek. */}
      {kind === "restore" && (
        <>
          <path d="M3.5 4V2.9a1.4 1.4 0 0 1 1.4-1.4H9.1a1.4 1.4 0 0 1 1.4 1.4V7.1a1.4 1.4 0 0 1-1.4 1.4H8" />
          <rect height="5" rx="1" width="5" x="1.5" y="5.5" />
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
