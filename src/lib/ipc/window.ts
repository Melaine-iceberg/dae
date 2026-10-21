import { getBridge, requireBridge, type WindowAction } from "./bridge";
import { emit, listen, once, type EventCallback, type UnlistenFn } from "./events";

/** Tauri's synthetic window event names, kept so call sites read unchanged. */
export const TauriEvent = {
  WINDOW_RESIZED: "tauri://resize",
  WINDOW_FOCUS: "tauri://focus",
  WINDOW_BLUR: "tauri://blur",
  DRAG_ENTER: "tauri://drag-enter",
  DRAG_OVER: "tauri://drag-over",
  DRAG_DROP: "tauri://drag-drop",
  DRAG_LEAVE: "tauri://drag-leave",
} as const;

/** A position in physical pixels, as the platform reports it. */
export class PhysicalPosition {
  constructor(
    readonly x: number,
    readonly y: number,
  ) {}

  toLogical(scaleFactor: number): { x: number; y: number } {
    return { x: this.x / scaleFactor, y: this.y / scaleFactor };
  }
}

/** A size in physical pixels, as the platform reports it. */
export class PhysicalSize {
  constructor(
    readonly width: number,
    readonly height: number,
  ) {}

  toLogical(scaleFactor: number): { width: number; height: number } {
    return { width: this.width / scaleFactor, height: this.height / scaleFactor };
  }
}

export type DragDropEvent =
  | { type: "enter"; paths: string[]; position: PhysicalPosition }
  | { type: "over"; position: PhysicalPosition }
  | { type: "drop"; paths: string[]; position: PhysicalPosition }
  | { type: "leave" };

async function control<T>(action: WindowAction): Promise<T> {
  return (await requireBridge().window.control(action)) as T;
}

/**
 * The current window, with the subset of Tauri's window API this shell uses.
 *
 * Window facts that the OS owns (position, scale, focus, resize) come from the
 * main process; file drag-and-drop is observed in the preload and delivered on
 * the same event names Tauri used.
 */
export class Window {
  constructor(readonly label: string) {}

  listen<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
    return listen(event, handler, { target: { kind: "Window", label: this.label } });
  }

  once<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
    return once(event, handler, { target: { kind: "Window", label: this.label } });
  }

  emit<T>(event: string, payload?: T): Promise<void> {
    return emit(event, payload);
  }

  minimize(): Promise<void> {
    return control("minimize");
  }

  toggleMaximize(): Promise<void> {
    return control("toggleMaximize");
  }

  isMaximized(): Promise<boolean> {
    return control("isMaximized");
  }

  isFocused(): Promise<boolean> {
    return control("isFocused");
  }

  setFocus(): Promise<void> {
    return control("setFocus");
  }

  close(): Promise<void> {
    return control("close");
  }

  async outerPosition(): Promise<PhysicalPosition> {
    const position = await control<{ x: number; y: number }>("outerPosition");
    return new PhysicalPosition(position.x, position.y);
  }

  scaleFactor(): Promise<number> {
    return control("scaleFactor");
  }

  async onResized(handler: EventCallback<PhysicalSize>): Promise<UnlistenFn> {
    return this.listen<{ width: number; height: number }>(
      TauriEvent.WINDOW_RESIZED,
      (event) => {
        handler({
          ...event,
          payload: new PhysicalSize(event.payload.width, event.payload.height),
        });
      },
    );
  }

  async onFocusChanged(handler: EventCallback<boolean>): Promise<UnlistenFn> {
    const unlistenFocus = await this.listen(TauriEvent.WINDOW_FOCUS, (event) => {
      handler({ ...event, payload: true });
    });
    const unlistenBlur = await this.listen(TauriEvent.WINDOW_BLUR, (event) => {
      handler({ ...event, payload: false });
    });
    return () => {
      unlistenFocus();
      unlistenBlur();
    };
  }

  async onDragDropEvent(handler: EventCallback<DragDropEvent>): Promise<UnlistenFn> {
    type Position = { x: number; y: number };
    type Paths = { paths: string[] };

    const unlisten = await Promise.all([
      this.listen<Paths & { position: Position }>(TauriEvent.DRAG_ENTER, (event) => {
        handler({
          ...event,
          payload: {
            type: "enter",
            paths: event.payload.paths,
            position: new PhysicalPosition(event.payload.position.x, event.payload.position.y),
          },
        });
      }),
      this.listen<{ position: Position }>(TauriEvent.DRAG_OVER, (event) => {
        handler({
          ...event,
          payload: {
            type: "over",
            position: new PhysicalPosition(event.payload.position.x, event.payload.position.y),
          },
        });
      }),
      this.listen<Paths & { position: Position }>(TauriEvent.DRAG_DROP, (event) => {
        handler({
          ...event,
          payload: {
            type: "drop",
            paths: event.payload.paths,
            position: new PhysicalPosition(event.payload.position.x, event.payload.position.y),
          },
        });
      }),
      this.listen(TauriEvent.DRAG_LEAVE, (event) => {
        handler({ ...event, payload: { type: "leave" } });
      }),
    ]);

    return () => {
      for (const stop of unlisten) stop();
    };
  }
}

/** Webview-scoped events use the same transport as window events here. */
export class Webview extends Window {}

let currentWindow: Window | null = null;

export function getCurrentWindow(): Window {
  currentWindow ??= new Window(getBridge()?.label ?? "main");
  return currentWindow;
}
