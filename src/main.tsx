import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./App.css";

import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { applySystemTheme } from "@/lib/theme";

if (import.meta.env.DEV) {
  setupDevInvoke();
}

applySystemTheme();

const queryClient = new QueryClient();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <QueryClientProvider client={queryClient}>
    <React.StrictMode>
      <App />
    </React.StrictMode>
  </QueryClientProvider>,
);

// The window is created hidden (see tauri.conf.json); reveal it once the
// first frame has actually painted so users never see a white flash.
if ("__TAURI_INTERNALS__" in window) {
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      const currentWindow = getCurrentWindow();
      void currentWindow.show().then(() => currentWindow.setFocus());
    });
  });
}
