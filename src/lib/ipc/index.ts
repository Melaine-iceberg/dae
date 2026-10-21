/**
 * The renderer's half of the desktop IPC seam.
 *
 * `src/bindings.ts` is generated against this module rather than
 * `@tauri-apps/api`, and the window, event and plugin helpers the shell used to
 * import from Tauri live here too. The transport underneath is the Electron
 * preload bridge (`window.daeBridge`); this module gives it the Tauri-shaped
 * surface the application is written against.
 */

export { hasDesktopBridge } from "./bridge";
export { Channel, invoke } from "./invoke";
export {
  emit,
  listen,
  once,
  type Event,
  type EventCallback,
  type EventTarget,
  type ListenOptions,
  type UnlistenFn,
} from "./events";
export {
  getCurrentWindow,
  PhysicalPosition,
  PhysicalSize,
  TauriEvent,
  Webview,
  Window,
  type DragDropEvent,
} from "./window";
export { open, type OpenDialogOptions } from "./dialog";
export { openPath, openUrl } from "./opener";
export { readText, writeText } from "./clipboard";
export { error, warn } from "./log";
export { appLogDir, getVersion } from "./app";
