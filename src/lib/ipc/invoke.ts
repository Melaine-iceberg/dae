import { requireBridge } from "./bridge";

let nextChannelId = 1;

/**
 * A Tauri-shaped IPC channel.
 *
 * The backend is handed only the `{ __dae_channel__ }` reference; the callback
 * stays in the renderer. Commands that stream (the terminal, packet listings)
 * pass one of these as an argument, so the shape has to survive the JSON pass
 * `invoke` performs below.
 */
export class Channel<T = unknown> {
  readonly id = nextChannelId++;
  onmessage: (message: T) => void = () => {};

  constructor(onmessage?: (message: T) => void) {
    if (onmessage) this.onmessage = onmessage;
  }

  toJSON(): { __dae_channel__: number } {
    return { __dae_channel__: this.id };
  }
}

/**
 * Runs one command through the desktop bridge.
 *
 * Arguments go through JSON — the same representation the bridge already hands
 * the core — so `Channel` instances become their references and values the
 * backend would not accept are dropped instead of failing structured cloning.
 */
export async function invoke<T>(command: string, args?: unknown): Promise<T> {
  const bridge = requireBridge();
  const payload = args === undefined ? null : JSON.parse(JSON.stringify(args));
  return (await bridge.invoke(command, payload)) as T;
}
