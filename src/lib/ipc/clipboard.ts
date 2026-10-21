import { requireBridge } from "./bridge";

export async function readText(): Promise<string> {
  return requireBridge().clipboard.readText();
}

export async function writeText(text: string): Promise<void> {
  await requireBridge().clipboard.writeText(text);
}
