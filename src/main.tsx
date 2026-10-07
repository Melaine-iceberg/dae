import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { commands } from "@/bindings";
import { restoreInitialTabHandoff } from "@/features/explorer/tabs";
import { warmShellCommands } from "@/features/shell-commands/shell-commands-atoms";
import { preloadExplorerSurface } from "@/features/workspace/workspace-surface";
import { i18nReady } from "@/i18n";
import { getAppWindow } from "@/lib/app-window";
import { applySystemTheme } from "@/lib/theme";
import { setupNativeClipboardBridge } from "@/lib/clipboard-bridge";
import { setupExternalLinkGuard } from "@/lib/external-links";
import { forwardConsoleToLogFile } from "@/lib/logging";
import { setupNativeContextMenuGuard } from "@/lib/native-context-menu";
import { tabPerfEnabled, tabPerfMark, tabPerfReport } from "@/lib/tab-perf";
import { TabPerfProfiler } from "@/lib/tab-perf-profiler";

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
    tabPerfReport();
  };

  const observer = new MutationObserver(() => {
    tabPerfMark("commit");
    observer.disconnect();
    // A frame requested back at `render` can land before this commit and paint
    // the splash, which is how `first-frame` once read 269ms for a window that
    // did not commit until 410ms. The frame that can show React's DOM is the
    // first one after the commit, so it is measured from here.
    requestAnimationFrame(() => {
      tabPerfMark("first-frame");
      // The warm-pool question, in one row. If this says `hidden`, a window
      // nobody has looked at still reached a frame, so a pool could bank the
      // work. If it only ever says `visible`, WebKit held the work back until
      // the window was shown and a pool would have banked nothing at all.
      tabPerfMark(`gfx:${document.visibilityState}`);
      setTimeout(report, 0);
    });
  });

  if (tabPerfEnabled()) {
    tabPerfMark("render");
    // Data arrival. OverviewView is imported eagerly, so whatever the first
    // frame waits for cannot be that surface's chunk; the cache is what tells a
    // fetch apart from a chunk load, and when the fetch settled.
    queryClient.getQueryCache().subscribe((event) => {
      const query = (event as { query?: { queryKey?: unknown; state?: { status?: string } } })
        .query;
      const status = query?.state?.status;
      if (status !== "success" && status !== "error") return;
      const key = query?.queryKey;
      const label = Array.isArray(key) ? String(key[0]) : String(key);
      tabPerfMark(`query[${label}]:${status}`);
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
      tabPerfMark("commit-missing");
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
