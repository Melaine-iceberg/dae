import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { i18nReady } from "@/i18n";
import { getAppWindow } from "@/lib/app-window";
import { applySystemTheme } from "@/lib/theme";
import { setupNativeClipboardBridge } from "@/lib/clipboard-bridge";
import { setupExternalLinkGuard } from "@/lib/external-links";

if (import.meta.env.DEV) {
  setupDevInvoke();
}

setupNativeClipboardBridge();

setupExternalLinkGuard();

applySystemTheme();

const queryClient = new QueryClient();

// The startup locale's resources may load from a lazy chunk; wait for
// i18next so the first paint never shows raw translation keys.
async function bootstrap() {
  await i18nReady;
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
      getAppWindow()?.show();
    });
  });
}

void bootstrap();
