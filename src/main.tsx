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
  // can be: React producing DOM, the event loop coming back, and a frame being
  // painted. Creating the observer is free; only observing costs anything, and
  // that happens only when the probe is on.
  const observer = new MutationObserver(() => {
    tabPerfMark("commit");
    observer.disconnect();
  });

  if (tabPerfEnabled()) {
    tabPerfMark("render");
    observer.observe(root, { childList: true, subtree: true });
  }

  ReactDOM.createRoot(root).render(
    <QueryClientProvider client={queryClient}>
      <React.StrictMode>
        <App />
      </React.StrictMode>
    </QueryClientProvider>,
  );

  // The last mark is the frame that replaces the splash with real content, and
  // it is the one the user is really waiting on. `tick` is queued before the
  // frame request so FIFO runs it first: a late tick means the main thread was
  // busy, an early one means React deferred its own work and the window was
  // waiting rather than working.
  if (tabPerfEnabled()) {
    setTimeout(() => tabPerfMark("tick"), 0);
    requestAnimationFrame(() => {
      tabPerfMark("first-frame");
      // Reports only after `tick` has had its turn, for the same FIFO reason.
      // Deliberately not marked: `first-frame` should stay the last row so that
      // TOTAL keeps meaning "time until the user can see content".
      setTimeout(tabPerfReport, 0);
    });
  }

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
