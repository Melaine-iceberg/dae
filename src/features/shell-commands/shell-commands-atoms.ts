/**
 * Third-party commands for a selection — the right-click items the installed
 * applications contribute, which each platform's OS exposes through a different
 * mechanism (`shell_commands` in the backend hosts all three).
 *
 * These are deliberately *not* declared to dae the way the "应用扩展" manifest
 * extensions are: the platform owns the list, the apps own the wording, and the
 * only way to know what applies is to ask. Asking costs real work per selection
 * — a COM round trip on Windows, an `Info.plist` scan on macOS, a service-menu
 * scan on Linux — so the answer is cached here and the menu renders from the
 * cache.
 *
 * The cache key is the whole selection — every path *and* the right-clicked
 * entry — because a provider's title and visibility legitimately depend on both:
 * PowerRename hides itself for a selection it cannot rename, and VS Code's
 * command is a different string in a different language per install.
 *
 * Until a selection has been resolved the section renders nothing and the items
 * appear in place once the reply lands.
 */

import { atom, getDefaultStore, useAtomValue } from "jotai";
import { useEffect } from "react";

import { commands, type ShellCommand } from "@/bindings";
import { getFileOperationErrorMessage } from "@/i18n/errors";

const store = getDefaultStore();

/** Resolved commands per selection signature. A missing key means "not asked". */
const shellCommandsAtom = atom<ReadonlyMap<string, readonly ShellCommand[]>>(new Map());

/** Signatures with a request in flight, so a re-render does not ask twice. */
const inFlight = new Set<string>();

/**
 * Last failure from starting a command, surfaced by the explorer view. A
 * command that starts says nothing back, so this is the only place a wrong
 * CLSID or a vanished app can show up.
 */
export const shellCommandErrorAtom = atom<string | null>(null);

/** Stable empty result, so an unresolved selection never re-renders needlessly. */
const NO_COMMANDS: readonly ShellCommand[] = [];

/**
 * Identifies one menu's worth of commands. `\u0000` cannot appear in a path, so
 * two different selections can never collide on the joined string.
 */
function selectionSignature(paths: readonly string[], primary: string): string {
  return `${primary}\u0000${paths.join("\u0000")}`;
}

function storeCommands(signature: string, found: readonly ShellCommand[]): void {
  const cache = store.get(shellCommandsAtom);
  const updated = new Map(cache);
  updated.set(signature, found);
  store.set(shellCommandsAtom, updated);
}

/**
 * Asks the backend for a selection's commands unless it already has, caching an
 * empty result on failure: a provider that refuses to load must not be retried
 * every time the same menu opens.
 */
function ensureShellCommands(paths: readonly string[], primary: string): void {
  const signature = selectionSignature(paths, primary);
  if (store.get(shellCommandsAtom).has(signature) || inFlight.has(signature)) return;
  inFlight.add(signature);

  void commands
    .listShellCommands([...paths], primary)
    .then((found) => storeCommands(signature, found))
    .catch((error) => {
      console.warn("Unable to list shell commands", getFileOperationErrorMessage(error));
      storeCommands(signature, NO_COMMANDS);
    })
    .finally(() => inFlight.delete(signature));
}

/** Reads a selection's commands, asking for them on the first render. */
export function useShellCommands(paths: readonly string[], primary: string): readonly ShellCommand[] {
  const cache = useAtomValue(shellCommandsAtom);
  const signature = selectionSignature(paths, primary);

  useEffect(() => {
    ensureShellCommands(paths, primary);
    // Depends on `signature` alone: it *is* the identity of the request, while
    // the arrays are rebuilt on every render and would restart the effect.
  }, [signature]);

  return cache.get(signature) ?? NO_COMMANDS;
}

/**
 * Starts a command. Returns the failure message for the caller to surface, or
 * `null` when it ran; a command that starts successfully says nothing.
 *
 * On macOS the reply only means the service was *dispatched* — see the module
 * docs in `shell_commands/macos.rs` for why it is not awaited there.
 */
export async function invokeShellCommand(
  id: string,
  paths: readonly string[],
): Promise<string | null> {
  try {
    await commands.invokeShellCommand(id, [...paths]);
    return null;
  } catch (error) {
    return getFileOperationErrorMessage(error);
  }
}

/**
 * Primes the backend before the first right-click reaches it.
 *
 * The menu can only ask for a selection's commands once it is open, and the
 * first answer is what pays for everything the platform has not done yet:
 * Windows starts an STA thread and activates each provider's COM surrogate
 * (measured at 211-228 ms cold against 19 ms once warm, against the popup's own
 * 120 ms open animation), macOS reads every installed bundle's `Info.plist`, and
 * Linux reads the service-menu directories. The first right-click therefore
 * lands the section *after* the menu has settled, which is the flicker this
 * exists to remove; every one after it lands before.
 *
 * Called once at startup, after the window is revealed, so the cost is spent on
 * nothing instead of on the user's first menu. A failure is logged and
 * forgotten: the menu asks for itself when it opens and behaves exactly as it
 * did before — just with the delay back.
 */
export function warmShellCommands(): void {
  void commands.warmShellCommands().catch((error) => {
    console.warn("Unable to warm shell commands", getFileOperationErrorMessage(error));
  });
}
