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
import { setupNativeContextMenuGuard } from "@/lib/native-context-menu";

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
  // The explorer is the surface a click reaches first, and its chunk is what
  // makes that click wait (see `preloadExplorerSurface`); warm it alongside
  // the locale work rather than in idle time after the first paint.
  const explorerPreload = preloadExplorerSurface();

  await i18nReady;
  // The explorer chunk started loading above, in parallel with the locale. By
  // the time the window is revealed it is in memory, so the first folder open
  // renders it synchronously instead of suspending on a `lazy()` payload.
  await explorerPreload;

  const appWindow = getAppWindow();
  if (appWindow) {
    try {
      const handoff = await commands.takeTabHandoff(appWindow.label);
      if (handoff) restoreInitialTabHandoff(handoff);
    } catch (error) {
      // A malformed or unavailable handoff must not strand a hidden window;
      // it can still open normally on the Overview surface.
      console.error("Failed to restore detached tab", error);
    }
  }

  ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
    <QueryClientProvider client={queryClient}>
      <React.StrictMode>
        <App />
      </React.StrictMode>
    </QueryClientProvider>,
  );

  // The window starts hidden (tauri.conf visible:false) to avoid a white
  // flash while the JS bundle loads. Reveal it after the browser has
  // painted the first frame so the user sees the fully rendered UI.
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      void getAppWindow()?.show();
      // The context menu's "应用扩展" section can only be asked for once the
      // menu is open, and the first answer pays for a COM surrogate per
      // provider — 211-228 ms measured, against 19 ms once warm, behind the
      // popup's 120 ms open animation. Spending that here keeps it off the
      // user's first right-click, where it arrives after the menu has settled
      // and reads as a flicker. See `warmShellCommands`.
      warmShellCommands();
    });
  });
}

void bootstrap();
