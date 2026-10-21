import { requireBridge } from "./bridge";

export async function getVersion(): Promise<string> {
  return requireBridge().app.version();
}

export async function appLogDir(): Promise<string> {
  return requireBridge().app.logDir();
}
