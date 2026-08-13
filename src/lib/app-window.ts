import { getCurrentWindow, type Window } from "@tauri-apps/api/window";

type TauriInternals = { __TAURI_INTERNALS__?: { metadata?: unknown } };

// 在纯浏览器(经 dev-invoke 桥接)中 __TAURI_INTERNALS__ 只有 invoke，
// 窗口 API 不可用，此时返回 null 而不是抛错。
export function getAppWindow(): Window | null {
  const internals = (globalThis as typeof globalThis & TauriInternals).__TAURI_INTERNALS__;
  if (!internals?.metadata) return null;
  return getCurrentWindow();
}
