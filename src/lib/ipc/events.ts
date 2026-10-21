import { getBridge, type BridgeEvent } from "./bridge";

export interface Event<T> {
  event: string;
  id: number;
  payload: T;
  target?: string | null;
}

export type EventCallback<T> = (event: Event<T>) => void;
export type UnlistenFn = () => void;

export type EventTarget =
  | string
  | { kind: "Any" }
  | { kind: "AnyLabel"; label: string }
  | { kind: "Window"; label: string }
  | { kind: "Webview"; label: string };

export type ListenOptions = {
  target?: EventTarget;
};

type Listener = {
  id: number;
  event: string;
  target: string | null;
  handler: EventCallback<unknown>;
  once: boolean;
};

const listeners = new Set<Listener>();
let nextListenerId = 1;
let subscribed = false;
let warnedWithoutBridge = false;

function targetLabel(target: EventTarget | undefined): string | null {
  if (!target) return null;
  if (typeof target === "string") return target;
  if (target.kind === "Any") return null;
  return target.label;
}

function payloadOf(event: BridgeEvent): unknown {
  if (event.payloadJson !== null) {
    try {
      return JSON.parse(event.payloadJson);
    } catch {
      return null;
    }
  }
  return event.data ?? null;
}

function dispatch(event: BridgeEvent): void {
  const payload = payloadOf(event);
  const target = event.target ?? null;

  // Snapshot: a handler may add or remove listeners while this event dispatches.
  for (const listener of Array.from(listeners)) {
    if (listener.event !== event.name) continue;
    // A global listener hears every event; a window-scoped one hears broadcasts
    // and the events addressed to it, never another window's traffic.
    if (listener.target !== null && target !== null && listener.target !== target) {
      continue;
    }
    if (listener.once) listeners.delete(listener);
    listener.handler({ event: event.name, id: listener.id, payload, target });
  }
}

function subscribe(): boolean {
  const bridge = getBridge();
  if (!bridge) return false;
  if (!subscribed) {
    subscribed = true;
    bridge.onEvent(dispatch);
  }
  return true;
}

function addListener<T>(
  event: string,
  handler: EventCallback<T>,
  target: string | null,
  once: boolean,
): Promise<UnlistenFn> {
  if (!subscribe()) {
    if (!warnedWithoutBridge) {
      warnedWithoutBridge = true;
      console.warn("Cannot listen for dae events: the desktop bridge is not installed");
    }
    return Promise.resolve(() => {});
  }

  const listener: Listener = {
    id: nextListenerId++,
    event,
    target,
    handler: handler as EventCallback<unknown>,
    once,
  };
  listeners.add(listener);
  return Promise.resolve(() => {
    listeners.delete(listener);
  });
}

/** Listens for one event, optionally scoped to a window label. */
export function listen<T>(
  event: string,
  handler: EventCallback<T>,
  options?: ListenOptions,
): Promise<UnlistenFn> {
  return addListener(event, handler, targetLabel(options?.target), false);
}

/** Listens for the next occurrence of one event. */
export function once<T>(
  event: string,
  handler: EventCallback<T>,
  options?: ListenOptions,
): Promise<UnlistenFn> {
  return addListener(event, handler, targetLabel(options?.target), true);
}

/**
 * Emits an event to this window's listeners.
 *
 * The frontend is the only emitter the app needs today; events from the core
 * arrive through the bridge instead. Dispatching locally keeps `events.x.emit`
 * meaningful without inventing a round trip through the main process.
 */
export async function emit<T>(event: string, payload?: T): Promise<void> {
  dispatch({
    name: event,
    target: null,
    payloadJson: payload === undefined ? null : JSON.stringify(payload),
    data: null,
  });
}
