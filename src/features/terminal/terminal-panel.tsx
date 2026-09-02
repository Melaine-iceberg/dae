import { useEffect, useRef, useState } from "react";
import { getDefaultStore, useAtomValue, useSetAtom } from "jotai";
import { useTranslation } from "react-i18next";
import { Channel, invoke } from "@tauri-apps/api/core";
import { Terminal, type ITheme } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { CanvasAddon } from "@xterm/addon-canvas";
import { ArrowCounterClockwiseIcon, XIcon } from "@phosphor-icons/react";

import { activePaneNavigatorAtom, activeTabIdAtom } from "@/features/explorer/tabs";
import { appSettingsAtom } from "@/features/settings/settings-atoms";
import { tabSurfaceFamily } from "@/features/workspace/tab-surface";
import { translateBackendMessage } from "@/i18n/errors";
import { cn } from "@/lib/utils";

import { terminalVisibleAtom } from "./terminal-atoms";
import { resolveAnsiPalette, type AnsiPalette } from "./terminal-palette";

import "@xterm/xterm/css/xterm.css";

const FONT_STACK =
  '"Cascadia Code", Consolas, Menlo, Monaco, "DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono CJK SC", monospace';

const DEFAULT_FONT_SIZE = 13;
const DEFAULT_LINE_HEIGHT = 1.2;

const MIN_PANEL_HEIGHT = 140;

/** Resolves the folder shown by the active tab's focused pane, if any, as
 *  the shell cwd. */
function currentWorkingDirectory(): string | undefined {
  const store = getDefaultStore();
  const tabId = store.get(activeTabIdAtom);
  if (!tabId) return undefined;
  const surface = store.get(tabSurfaceFamily(tabId));
  if (surface.kind !== "folder") return undefined;
  try {
    return store.get(activePaneNavigatorAtom).getSnapshot().directory?.path;
  } catch {
    return undefined;
  }
}

/** Maps the app's semantic color tokens onto the xterm color scheme. */
function readTerminalTheme(ansiOverride: AnsiPalette | null): ITheme {
  const style = getComputedStyle(document.documentElement);
  const token = (name: string, fallback: string) =>
    style.getPropertyValue(name).trim() || fallback;
  const dark = document.documentElement.classList.contains("dark");
  const ansi = resolveAnsiPalette(ansiOverride, dark);
  return {
    background: token("--card", "#211f26"),
    foreground: token("--foreground", "#e6e0e9"),
    cursor: token("--foreground", "#e6e0e9"),
    cursorAccent: token("--card", "#211f26"),
    selectionBackground: token("--accent", "#49454f"),
    black: ansi[0],
    red: ansi[1],
    green: ansi[2],
    yellow: ansi[3],
    blue: ansi[4],
    magenta: ansi[5],
    cyan: ansi[6],
    white: ansi[7],
    brightBlack: ansi[8],
    brightRed: ansi[9],
    brightGreen: ansi[10],
    brightYellow: ansi[11],
    brightBlue: ansi[12],
    brightMagenta: ansi[13],
    brightCyan: ansi[14],
    brightWhite: ansi[15],
  };
}

/** Reads the live terminal settings from the store (no re-render needed). */
function readTerminalSettings() {
  return getDefaultStore().get(appSettingsAtom)?.terminal;
}

/** Fit that tolerates mid-layout containers; the next resize tick recovers. */
function safeFit(fit: FitAddon): void {
  try {
    fit.fit();
  } catch {
    // Ignore transient layout races.
  }
}

/**
 * Bottom terminal panel hosting a single PTY session. The session spawns on
 * first reveal, survives being hidden and only dies on restart, shell exit
 * or app shutdown.
 */
export function TerminalPanel() {
  const { t } = useTranslation("terminal");
  const visible = useAtomValue(terminalVisibleAtom);
  const setVisible = useSetAtom(terminalVisibleAtom);
  const containerRef = useRef<HTMLDivElement | null>(null);
  const fitRef = useRef<FitAddon | null>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const sessionIdRef = useRef<number | null>(null);
  const [hasOpened, setHasOpened] = useState(false);
  const [restartCount, setRestartCount] = useState(0);
  const [exitCode, setExitCode] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [height, setHeight] = useState(288);

  useEffect(() => {
    if (visible) setHasOpened(true);
  }, [visible]);

  useEffect(() => {
    if (!hasOpened) return;
    const container = containerRef.current;
    if (!container) return;

    let disposed = false;
    let boundId: number | null = null;
    let dataDisposable: { dispose(): void } | null = null;
    setError(null);
    setExitCode(null);

    const fit = new FitAddon();
    const settings = readTerminalSettings();
    const terminal = new Terminal({
      fontFamily: settings?.fontFamily ?? FONT_STACK,
      fontSize: settings?.fontSize ?? DEFAULT_FONT_SIZE,
      lineHeight: settings?.lineHeight ?? DEFAULT_LINE_HEIGHT,
      scrollback: 5000,
      cursorBlink: true,
      theme: readTerminalTheme(settings?.ansiColors ?? null),
    });
    terminal.loadAddon(fit);
    terminal.open(container);
    fitRef.current = fit;
    terminalRef.current = terminal;

    // xterm 6 ships no built-in renderer; prefer WebGL and fall back to
    // canvas on init failure or GPU context loss.
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => {
        webgl.dispose();
        try {
          terminal.loadAddon(new CanvasAddon());
        } catch {
          // Renderer unavailable; the terminal surface stays blank.
        }
      });
      terminal.loadAddon(webgl);
    } catch {
      try {
        terminal.loadAddon(new CanvasAddon());
      } catch {
        // See above.
      }
    }

    // Fit before proposing dimensions so the terminal and the PTY agree on
    // cols/rows from the first byte; a mismatch makes shell line redraws
    // (PSReadLine echoes each keypress) jump while typing.
    safeFit(fit);
    const dimensions = fit.proposeDimensions();
    const outputChannel = new Channel<ArrayBuffer>();
    outputChannel.onmessage = (data) => terminal.write(new Uint8Array(data));
    const exitChannel = new Channel<number>();
    exitChannel.onmessage = (code) => {
      if (disposed || sessionIdRef.current !== boundId) return;
      setExitCode(code);
    };

    invoke<number>("terminal_create", {
      cwd: currentWorkingDirectory(),
      cols: dimensions?.cols ?? 80,
      rows: dimensions?.rows ?? 24,
      onOutput: outputChannel,
      onExit: exitChannel,
    })
      .then((id) => {
        if (disposed) {
          void invoke("terminal_kill", { id });
          return;
        }
        boundId = id;
        sessionIdRef.current = id;
        dataDisposable = terminal.onData((data) => {
          void invoke("terminal_write", { id, data });
        });
      })
      .catch((reason) => {
        if (!disposed) setError(String(reason));
      });

    // Refit on container changes, but only when the grid actually changes —
    // repeated fit/resize round-trips make the shell redraw and flicker.
    let lastCols = dimensions?.cols ?? 0;
    let lastRows = dimensions?.rows ?? 0;
    let fitFrame: number | null = null;
    const observer = new ResizeObserver(() => {
      if (fitFrame != null) return;
      fitFrame = requestAnimationFrame(() => {
        fitFrame = null;
        if (container.clientWidth === 0 || container.clientHeight === 0) return;
        safeFit(fit);
        const next = fit.proposeDimensions();
        if (!next || (next.cols === lastCols && next.rows === lastRows)) return;
        lastCols = next.cols;
        lastRows = next.rows;
        const sessionId = sessionIdRef.current;
        if (sessionId != null) {
          void invoke("terminal_resize", { id: sessionId, cols: next.cols, rows: next.rows });
        }
      });
    });
    observer.observe(container);

    const handleThemeChange = () => {
      terminal.options.theme = readTerminalTheme(readTerminalSettings()?.ansiColors ?? null);
    };
    window.addEventListener("app-theme-change", handleThemeChange);

    // Live-apply terminal settings (font family/size, line height, ANSI
    // palette) without tearing down the PTY session. Mutating `options` in
    // place re-renders the grid; font/line-height changes alter cell metrics,
    // so refit and resync the PTY dimensions.
    const applyTerminalSettings = () => {
      const next = readTerminalSettings();
      terminal.options.fontFamily = next?.fontFamily ?? FONT_STACK;
      terminal.options.fontSize = next?.fontSize ?? DEFAULT_FONT_SIZE;
      terminal.options.lineHeight = next?.lineHeight ?? DEFAULT_LINE_HEIGHT;
      terminal.options.theme = readTerminalTheme(next?.ansiColors ?? null);
      safeFit(fit);
      const dims = fit.proposeDimensions();
      const sessionId = sessionIdRef.current;
      if (dims && sessionId != null && (dims.cols !== lastCols || dims.rows !== lastRows)) {
        lastCols = dims.cols;
        lastRows = dims.rows;
        void invoke("terminal_resize", { id: sessionId, cols: dims.cols, rows: dims.rows });
      }
    };
    const unsubSettings = getDefaultStore().sub(appSettingsAtom, applyTerminalSettings);

    return () => {
      disposed = true;
      if (fitFrame != null) cancelAnimationFrame(fitFrame);
      observer.disconnect();
      window.removeEventListener("app-theme-change", handleThemeChange);
      unsubSettings();
      dataDisposable?.dispose();
      fitRef.current = null;
      terminalRef.current = null;
      const sessionId = sessionIdRef.current;
      sessionIdRef.current = null;
      if (sessionId != null) void invoke("terminal_kill", { id: sessionId });
      terminal.dispose();
    };
  }, [hasOpened, restartCount]);

  // Refit and focus after the panel reappears with real layout dimensions.
  useEffect(() => {
    if (!visible) return;
    const frame = requestAnimationFrame(() => {
      const fit = fitRef.current;
      if (fit) safeFit(fit);
      terminalRef.current?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [visible, restartCount]);

  const restart = () => {
    const sessionId = sessionIdRef.current;
    sessionIdRef.current = null;
    if (sessionId != null) void invoke("terminal_kill", { id: sessionId });
    setExitCode(null);
    setRestartCount((count) => count + 1);
  };

  const startResizeDrag = (event: React.PointerEvent<HTMLDivElement>) => {
    event.preventDefault();
    const panel = event.currentTarget.parentElement;
    if (!panel) return;
    const bottom = panel.getBoundingClientRect().bottom;
    const onMove = (move: PointerEvent) => {
      setHeight(Math.max(MIN_PANEL_HEIGHT, bottom - move.clientY));
    };
    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
    };
    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
  };

  return (
    <section
      aria-label={t("panel.label")}
      className={cn(
        "flex shrink-0 flex-col overflow-hidden rounded-xl border bg-card shadow-ambient-xs",
        !visible && "hidden",
      )}
      style={{ height }}
    >
      <div
        aria-hidden="true"
        className="group h-1 w-full shrink-0 cursor-row-resize"
        onPointerDown={startResizeDrag}
      />
      <header className="flex h-8 shrink-0 items-center gap-1 border-b border-border/60 px-2">
        <span className="text-xs font-medium text-muted-foreground select-none">
          {t("panel.title")}
        </span>
        <div className="ml-auto flex items-center gap-0.5">
          <button
            aria-label={t("panel.restart.label")}
            className="flex size-6 items-center justify-center rounded-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={restart}
            title={t("panel.restart.label")}
            type="button"
          >
            <ArrowCounterClockwiseIcon className="size-3.5" />
          </button>
          <button
            aria-label={t("panel.close.label")}
            className="flex size-6 items-center justify-center rounded-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onClick={() => setVisible(false)}
            title={t("panel.close.title")}
            type="button"
          >
            <XIcon className="size-3.5" />
          </button>
        </div>
      </header>
      <div className="relative min-h-0 flex-1 px-1 pb-1">
        <div ref={containerRef} className="h-full w-full" />
        {exitCode != null && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-card text-sm text-muted-foreground">
            <span>{t("panel.sessionEnded", { code: exitCode })}</span>
            <button
              className="flex h-7 items-center gap-1 rounded-lg border px-3 transition-colors hover:bg-accent hover:text-foreground"
              onClick={restart}
              type="button"
            >
              <ArrowCounterClockwiseIcon className="size-3.5" />
              {t("panel.restart.action")}
            </button>
          </div>
        )}
        {error && (
          <div className="absolute inset-0 flex items-center justify-center bg-card p-4 text-center text-sm text-destructive">
            {translateBackendMessage(error)}
          </div>
        )}
      </div>
    </section>
  );
}
