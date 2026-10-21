import { requireBridge } from "./bridge";

/** Forwards one line to the desktop log file the settings panel can open. */
export async function warn(message: string): Promise<void> {
  await requireBridge().log.write("warn", message);
}

export async function error(message: string): Promise<void> {
  await requireBridge().log.write("error", message);
}
