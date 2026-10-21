/**
 * The preload bridge the Electron shell exposes as `window.daeBridge`.
 *
 * Everything in `@/lib/ipc` is a thin Tauri-shaped layer over this one object:
 * `src/bindings.ts` is generated against the facade rather than
 * `@tauri-apps/api`, so the renderer keeps the command names, window methods
 * and event vocabulary it was written against while the transport underneath
 * is Electron IPC.
 */

/** One event as the bridge delivers it. Payloads are JSON; byte streams arrive separately. */
export type BridgeEvent = {
  name: string;
  target: string | null;
  payloadJson: string | null;
  data: Uint8Array | null;
};

export type WindowAction =
  | "close"
  | "minimize"
  | "toggleMaximize"
  | "isMaximized"
  | "isFocused"
  | "setFocus"
  | "outerPosition"
  | "scaleFactor";

export type BridgeDialogFilter = {
  name: string;
  extensions: string[];
};

export type BridgeOpenDialogOptions = {
  title?: string;
  defaultPath?: string;
  directory?: boolean;
  multiple?: boolean;
  filters?: BridgeDialogFilter[];
};

export type DaeBridge = {
  /** The label of this window, passed down as an additional renderer argument. */
  label: string;
  invoke(command: string, args: unknown): Promise<unknown>;
  invokeJson(command: string, args: unknown): Promise<string>;
  invokeRaw(command: string, args: unknown): Promise<Uint8Array | null>;
  onEvent(listener: (event: BridgeEvent) => void): () => void;
  window: {
    control(action: WindowAction): Promise<unknown>;
  };
  dialog: {
    open(options: BridgeOpenDialogOptions): Promise<string | string[] | null>;
  };
  opener: {
    openPath(path: string): Promise<void>;
    openUrl(url: string): Promise<void>;
  };
  clipboard: {
    readText(): Promise<string>;
    writeText(text: string): Promise<void>;
  };
  log: {
    write(level: "warn" | "error", message: string): Promise<void>;
  };
  app: {
    version(): Promise<string>;
    logDir(): Promise<string>;
  };
};

type BridgeHost = {
  daeBridge?: DaeBridge;
};

/** The bridge, or `null` when the page is not running inside the desktop shell. */
export function getBridge(): DaeBridge | null {
  return (globalThis as BridgeHost).daeBridge ?? null;
}

export function hasDesktopBridge(): boolean {
  return getBridge() !== null;
}

/** The bridge, for call sites that cannot do anything sensible without it. */
export function requireBridge(): DaeBridge {
  const bridge = getBridge();
  if (!bridge) {
    throw new Error("dae IPC is unavailable: the desktop bridge is not installed");
  }
  return bridge;
}
