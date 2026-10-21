import { getCurrentWindow, hasDesktopBridge, type Window } from "@/lib/ipc";

// A plain browser has no native window to drive, so everything that would ask
// for one gets `null` instead. The focus seam already has a DOM fallback for
// exactly this case.
export function getAppWindow(): Window | null {
  return hasDesktopBridge() ? getCurrentWindow() : null;
}
