import { requireBridge, type BridgeDialogFilter } from "./bridge";

export type OpenDialogOptions = {
  title?: string;
  defaultPath?: string;
  directory?: boolean;
  multiple?: boolean;
  filters?: BridgeDialogFilter[];
};

/** Native open dialog. Returns the chosen path(s), or `null` when cancelled. */
export async function open(options: OpenDialogOptions = {}): Promise<string | string[] | null> {
  return requireBridge().dialog.open(options);
}
