import { atom, getDefaultStore } from "jotai";

/**
 * Commands the command bar can dispatch to the active tab's explorer
 * surface. The explorer view owns the actual selection/operation state, so
 * the command bar only emits an intent here and the mounted explorer
 * executes it.
 */
export type ExplorerCommandId =
  | "create-folder"
  | "create-file"
  | "rename"
  | "delete"
  | "copy"
  | "cut"
  | "paste"
  | "copy-paths"
  | "select-all"
  | "refresh"
  | "go-back"
  | "go-forward"
  | "go-up"
  | "open-terminal"
  | "toggle-favorite";

export type PendingExplorerCommand = {
  id: number;
  command: ExplorerCommandId;
};

const store = getDefaultStore();
let nextCommandId = 0;

export const pendingExplorerCommandAtom = atom<PendingExplorerCommand | null>(null);

export function dispatchExplorerCommand(command: ExplorerCommandId): void {
  nextCommandId += 1;
  store.set(pendingExplorerCommandAtom, { id: nextCommandId, command });
}

export function clearPendingExplorerCommand(): void {
  store.set(pendingExplorerCommandAtom, null);
}
