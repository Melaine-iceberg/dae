import { getBridge } from "./bridge";

/**
 * Tells the window a tab came from that this window took it.
 *
 * This is the answer a tab drag cannot carry itself: the platform reports a
 * drop's *action* back to the source, but on Wayland that action comes from the
 * compositor and is treated as a cancellation when it is missing, so a drop
 * onto another window is not reliably distinguishable from a drop onto the
 * desktop. The shell forwards the token to the source window instead, which is
 * how the source learns that its copy of the tab is spent.
 *
 * Resolves `false` outside the desktop shell.
 */
export async function claimTabHandoff(source: string, token: string): Promise<boolean> {
  const bridge = getBridge();
  if (!bridge?.tab) return false;
  try {
    return Boolean(await bridge.tab.claim(source, token));
  } catch (error) {
    console.error("Failed to claim a tab handoff", error);
    return false;
  }
}
