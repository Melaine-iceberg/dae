import { getBridge, requireBridge } from "./bridge";
import type { DaeBridge } from "./bridge";

let nextChannelId = 1;

const channels = new Map<number, Channel<unknown>>();
let channelSubscription: (() => void) | null = null;
let channelSubscriptionBridge: DaeBridge | null = null;

/**
 * Routes core-to-renderer channel messages to the `Channel` that created them.
 *
 * The bridge owns the transport: the core only knows channel ids, so `main`
 * remembers which window invoked a command and preload delivers the message
 * here. Byte payloads are turned into `ArrayBuffer` because that is the shape
 * `Channel<ArrayBuffer>` consumers (packet listings) expect.
 */
function ensureChannelDelivery(): void {
  const bridge = getBridge();
  if (!bridge?.onChannelMessage || channelSubscriptionBridge === bridge) return;

  channelSubscription?.();
  channelSubscriptionBridge = bridge;
  channelSubscription = bridge.onChannelMessage((message) => {
    const channel = channels.get(message.channelId);
    if (!channel) return;

    if (message.payloadJson !== null) {
      channel.onmessage(JSON.parse(message.payloadJson));
      return;
    }
    if (message.data) {
      const { data } = message;
      channel.onmessage(
        data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength) as unknown,
      );
      return;
    }
    channel.onmessage(null as unknown);
  });
}

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
    channels.set(this.id, this as Channel<unknown>);
    ensureChannelDelivery();
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
  ensureChannelDelivery();
  const payload = args === undefined ? null : JSON.parse(JSON.stringify(args));
  return (await bridge.invoke(command, payload)) as T;
}
