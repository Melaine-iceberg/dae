import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";

import { setupDevInvoke } from "tauri-plugin-dev-invoke-api";

if (import.meta.env.DEV) {
  setupDevInvoke();
}

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
