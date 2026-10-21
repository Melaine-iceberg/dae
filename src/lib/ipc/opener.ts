import { requireBridge } from "./bridge";

/** Opens a filesystem path with the OS default handler. */
export async function openPath(path: string): Promise<void> {
  await requireBridge().opener.openPath(path);
}

/** Opens a URL in the user's browser or mail client. */
export async function openUrl(url: string): Promise<void> {
  await requireBridge().opener.openUrl(url);
}
