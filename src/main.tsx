import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { restoreInitialTabHandoff, takeInjectedTabHandoff } from "@/features/explorer/tabs";
import { warmShellCommands } from "@/features/shell-commands/shell-commands-atoms";
import { preloadExplorerSurface } from "@/features/workspace/workspace-surface";
import { i18nReady } from "@/i18n";
import { getAppWindow } from "@/lib/app-window";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";
import { watchAccentSeam } from "@/lib/system-accent";
import { setupNativeClipboardBridge } from "@/lib/clipboard-bridge";
import { setupExternalLinkGuard } from "@/lib/external-links";
import { forwardConsoleToLogFile } from "@/lib/logging";
import { setupNativeContextMenuGuard } from "@/lib/native-context-menu";
import {
  startupPerfEnabled,
  tabPerfEnabled,
  tabPerfMark,
  tabPerfOnFirstData,
  tabPerfReport,
} from "@/lib/tab-perf";
import { TabPerfProfiler } from "@/lib/tab-perf-profiler";

// The last instant that belongs to loading rather than to running: every import
// of the entry graph has been evaluated by the time this module's own body
// starts, so this is where parse+execute ends and the app's own start-up
// begins. `boot-eval-start` (in `index.html`) is the other end.
tabPerfMark("boot-eval-end");

// First, so that anything failing during the rest of startup is recorded: the
// release build's webview has no console anyone will read, and this is what
// turns its errors into a file the user can attach to a bug report.
forwardConsoleToLogFile();

if (import.meta.env.DEV) {
  void setupDevInvoke();
}

setupNativeClipboardBridge();

setupExternalLinkGuard();

setupNativeContextMenuGuard();

applySystemTheme();

// Subscribe to the OS appearance here rather than in an `App` effect: the
// reading that matters on Linux comes back over IPC, and `App` does not mount
// until the locale resources have loaded. Starting the read now lets it land
// during that wait, so the shell does not paint the GTK theme's colour and then
// snap to the desktop's when React mounts. The disposer is dropped on purpose —
// this lives for the life of the window, exactly like `applySystemTheme` above.
watchSystemTheme(applySystemTheme);

// The accent for the same reason, and with one extra: its reading arrives over
// IPC on Windows and macOS, and the seam only paints while the Appearance
// preference says `system` — so a window that asked for its desktop accent
// would otherwise spend the locale wait painted blue and then snap. Starting
// the read here lets the answer land first; on the default (`brand`) the seam
// writes nothing at all and the shipped blue is simply the colour.
watchAccentSeam();

const queryClient = new QueryClient();

// The startup locale's resources may load from a lazy chunk; wait for
// i18next so the first paint never shows raw translation keys.
async function bootstrap() {
  tabPerfMark("bundle");

  // The explorer is the surface a click reaches first, and its chunk is what
  // makes that click wait (see `preloadExplorerSurface`); warm it alongside
  // the locale work rather than in idle time after the first paint.
  const explorerPreload = preloadExplorerSurface();

  await i18nReady;
  tabPerfMark("locale");

  let bootsOnFolder = false;
  const injectedHandoff = takeInjectedTabHandoff();
  if (injectedHandoff === undefined) {
    // A window built without the injected channel still gets its snapshot the
    // old way. Every window this app builds carries `null` or the payload, so on
    // the shipped paths this branch never runs — but losing a tab is worse than
    // the round trip this channel exists to avoid.
    const appWindow = getAppWindow();
    if (appWindow) {
      try {
        const handoff = await commands.takeTabHandoff(appWindow.label);
        if (handoff) bootsOnFolder = restoreInitialTabHandoff(handoff).kind === "folder";
      } catch (error) {
        // A malformed or unavailable handoff must not abort startup; the window
        // can still open normally on the Overview surface.
        console.error("Failed to restore detached tab", error);
      }
    }
  } else if (injectedHandoff !== null) {
    try {
      bootsOnFolder = restoreInitialTabHandoff(injectedHandoff).kind === "folder";
    } catch (error) {
      console.error("Failed to restore detached tab", error);
    }
  }
  tabPerfMark("handoff");

  // Only a window that opens onto a folder waits for the explorer chunk. It is
  // the chunk's one guaranteed first-frame consumer; every other window mounts
  // on the locale and leaves the preload in flight, so the 352 KB of listing,
  // dialog and Git machinery that a folder surface needs never sits between the
  // splash and the Overview. It is still in memory before a click can reach a
  // folder, and `WorkspaceSurfaceView` keeps its own Suspense fallback for the
  // window that somehow beats it there.
  if (bootsOnFolder) await explorerPreload;
  tabPerfMark("chunk");

  // DIAGNOSTIC for the §2 A/B (`docs/cold-start-js.md` §8 第 3 条), off unless Rust
  // injected the flag. The measured answer to "does a pre-rendered first
  // screen get painted" was "no frame is produced at all until after React's
  // first commit" — which cannot tell two very different causes apart: the
  // boot is one continuous block of main-thread work (parse → bundle → mount),
  // or the engine simply will not paint this document before the mount. Yielding
  // two frames here is the smallest change that separates them, and it is also
  // exactly what showing the skeleton would cost: the screen stays on the
  // static markup for the length of the mount instead of on an empty window.
  if ((globalThis as { __DAE_BOOT_YIELD?: number }).__DAE_BOOT_YIELD === 1) {
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
  }

  const root = document.getElementById("root") as HTMLElement;

  // `render -> first-frame` used to be one number, so it could not say whether
  // the main thread was busy or idle. These split it into the three things it
  // can be: React producing DOM, the event loop coming back, and the first frame
  // that can actually show the result. Creating the observer is free; only
  // observing costs anything, and that happens only when the probe is on.
  let reported = false;
  const report = () => {
    if (reported) return;
    reported = true;
    // Paint entries are kept by the engine, and they are the only record of the
    // instant the splash actually reached the screen: that happens before React
    // commits and nothing in script can observe it. A window that painted its
    // splash late and then committed late has two different problems, and this
    // row is what tells them apart.
    for (const entry of performance.getEntriesByType("paint")) {
      tabPerfMark(`paint:${entry.name}`, performance.timeOrigin + entry.startTime);
    }
    // Resource timing splits the bundle span into fetch and eval: the entry
    // script's `responseEnd` is when its bytes were all here, so
    // `boot-eval-end - responseEnd` is parse+execute and `responseEnd -
    // boot-eval-start` is protocol round-trip plus download. Matched on this
    // module's own URL so the later explorer chunk cannot stand in for it.
    const resource = (performance.getEntriesByType("resource") as PerformanceResourceTiming[])
      .filter((entry) => entry.name === import.meta.url)
      // `dist/index.html` carries a `<link rel="modulepreload">` for the entry
      // graph, so the same URL has two entries: the preload and the module
      // script that reused it. The last one's `responseEnd` is when the bytes
      // were actually all in hand.
      .reduce<PerformanceResourceTiming | undefined>(
        (latest, entry) => (!latest || entry.responseEnd > latest.responseEnd ? entry : latest),
        undefined,
      );
    if (resource) {
      tabPerfMark("bundle:fetched", performance.timeOrigin + resource.responseEnd);
    }
    tabPerfReport();
  };

  // A cold start waits for one thing a tear-off does not: the first payload.
  // Reporting on the frame alone would put `first-data` after the table's last
  // row, and the report is a one-shot (`reported`), so it would never appear at
  // all — which would hide whether the first frame waited on the backend. The
  // two-second cap keeps a surface that fetches nothing from hiding the frame it
  // did paint.
  let painted = false;
  let dataArrived = false;
  const reportWhenSettled = () => {
    if (!painted) return;
    if (!startupPerfEnabled()) {
      setTimeout(report, 0);
      return;
    }
    if (dataArrived) {
      setTimeout(report, 0);
      return;
    }
    setTimeout(report, 2_000);
  };
  tabPerfOnFirstData(() => {
    dataArrived = true;
    reportWhenSettled();
  });

  const observer = new MutationObserver(() => {
    tabPerfMark("commit");
    observer.disconnect();
    // A frame requested back at `render` can land before this commit and paint
    // the splash, which is how `first-frame` once read 269ms for a window that
    // did not commit until 410ms. The frame that can show React's DOM is the
    // first one after the commit, so it is measured from here.
    requestAnimationFrame(() => {
      tabPerfMark("first-frame");
      // How big the tree React just committed actually is. The commit phase
      // (see `tab-perf-profiler.tsx`) is DOM insertion plus layout effects, so
      // the node count is what makes that number readable: a few hundred nodes
      // and tens of milliseconds means forced layout, not insertion volume.
      tabPerfMark(`dom-nodes:${document.getElementsByTagName("*").length}`);
      // The warm-pool question, in one row. If this says `hidden`, a window
      // nobody has looked at still reached a frame, so a pool could bank the
      // work. If it only ever says `visible`, WebKit held the work back until
      // the window was shown and a pool would have banked nothing at all.
      tabPerfMark(`gfx:${document.visibilityState}`);
      painted = true;
      reportWhenSettled();
    });
  });

  if (tabPerfEnabled()) {
    tabPerfMark("render");
    // Data arrival. OverviewView is imported eagerly, so whatever the first
    // frame waits for cannot be that surface's chunk; the cache is what tells a
    // fetch apart from a chunk load, and when the fetch settled.
    queryClient.getQueryCache().subscribe((event) => {
      // Only fresh data landing counts. Every observer attaching, and every
      // result it re-reads, also fires this callback while the query's status is
      // already "success" - so filtering on status alone reported a mark for
      // each of those and made one fetch look like twenty.
      if (event.type !== "updated" || event.action?.type !== "success") return;
      const query = event.query as { queryKey?: unknown } | undefined;
      const key = query?.queryKey;
      const label = Array.isArray(key) ? String(key[0]) : String(key);
      tabPerfMark(`query[${label}]:success`);
    });
    observer.observe(root, { childList: true, subtree: true });
    // Was the main thread busy during the gaps? A long task inside one means work
    // that can be moved or removed; silence means the wait belonged to the
    // scheduler or the compositor, which are fixed in different places. Recording
    // which of "observed" and "unsupported" happened matters, because an engine
    // that cannot report long tasks would otherwise look exactly like an idle
    // one, and the two call for opposite conclusions.
    // Registering is not the same as being able to report: an engine without
    // long-task support accepts the observer and then says nothing, which reads
    // exactly like an idle main thread - and those two call for opposite
    // conclusions. `supportedEntryTypes` separates them, so its answer is
    // recorded before any entry could be.
    try {
      const longTasks = PerformanceObserver.supportedEntryTypes?.includes("longtask") ?? false;
      tabPerfMark(longTasks ? "longtask:supported" : "longtask:unsupported");
      if (longTasks) {
        new PerformanceObserver((list) => {
          for (const entry of list.getEntries()) {
            tabPerfMark(`longtask:${Math.round(entry.duration)}ms`);
          }
        }).observe({ entryTypes: ["longtask"] });
      }
    } catch {
      tabPerfMark("longtask:unsupported");
    }
    // How quickly the event loop came back. Within a millisecond or two means
    // React was not starving it, and any later commit delay is React's own work.
    setTimeout(() => tabPerfMark("tick"), 0);
    // Nothing should stop the commit, but a report that never arrives cannot be
    // told apart from the probe being switched off, so say which one it is.
    setTimeout(() => {
      if (reported) return;
      // A hidden window gets no frames, so silence is what it is supposed to do
      // until something shows it. The fallback is for a window that is on
      // screen and still has not painted, which is a failure worth naming.
      if (document.visibilityState === "hidden") return;
      tabPerfMark("report-fallback");
      report();
    }, 5_000);
  }

  ReactDOM.createRoot(root).render(
    <QueryClientProvider client={queryClient}>
      <React.StrictMode>
        <TabPerfProfiler id="app">
          <App />
        </TabPerfProfiler>
      </React.StrictMode>
    </QueryClientProvider>,
  );

  // The window is visible from creation (tauri.conf visible:true), so nothing
  // waits on React here: what covers the bundle load is the `#splash` in
  // index.html, which is already on screen by the time this runs. Deferred one
  // frame so the warm-up below competes with nothing the user can see yet.
  requestAnimationFrame(() => {
    // The context menu's "应用扩展" section can only be asked for once the
    // menu is open, and the first answer pays for a COM surrogate per
    // provider — 211-228 ms measured, against 19 ms once warm, behind the
    // popup's 120 ms open animation. Spending that here keeps it off the
    // user's first right-click, where it arrives after the menu has settled
    // and reads as a flicker. See `warmShellCommands`.
    warmShellCommands();
  });
}

void bootstrap();
