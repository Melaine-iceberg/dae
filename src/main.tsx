import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import "@/i18n";
import { applySystemTheme } from "@/lib/theme";
import { getAppWindow } from "@/lib/app-window";
import { setupNativeClipboardBridge } from "@/lib/clipboard-bridge";

if (import.meta.env.DEV) {
  setupDevInvoke();
}

setupNativeClipboardBridge();

applySystemTheme();

const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <QueryClientProvider client={queryClient}>
    <React.StrictMode>
      <App />
    </React.StrictMode>
  </QueryClientProvider>,
);

// The window is created hidden (see tauri.conf.json); reveal it immediately
// after the initial render. index.html carries the app background color, so
// the window appears without a white flash even before the first paint.
const appWindow = getAppWindow();
if (appWindow) {
  void appWindow.show().then(() => appWindow.setFocus());
}
