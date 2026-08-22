import { lazy, Suspense, useEffect, useState } from "react";
import { useAtomValue, useSetAtom } from "jotai";

import { events } from "@/bindings";
import { useLocaleSync } from "@/i18n/atoms";
import { ExplorerTabs } from "@/features/explorer/explorer-tabs";
import { propertiesTargetAtom } from "@/features/explorer/properties-atoms";
import { undoRedoAtom } from "@/features/explorer/tabs";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { commandBarOpenAtom } from "@/features/workspace/command-bar-atoms";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";

// Overlays that only appear on user action; their chunks load on demand so
// the first frame stays lean.
const PropertiesDialog = lazy(() =>
  import("@/features/explorer/properties-dialog").then((m) => ({
    default: m.PropertiesDialog,
  })),
);
const CommandBar = lazy(() =>
  import("@/features/workspace/command-bar").then((m) => ({ default: m.CommandBar })),
);

// The query devtools panel is debug-only; a dynamic import keeps it (and its
// dependency subtree) out of production bundles entirely.
const ReactQueryDevtools = import.meta.env.DEV
  ? lazy(() =>
      import("@tanstack/react-query-devtools").then((m) => ({
        default: m.ReactQueryDevtools,
      })),
    )
  : null;

/** Latches to true the first time `open` becomes true, then stays true so
 *  lazily loaded overlays keep their close animation and internal state. */
function useEverOpened(open: boolean) {
  const [opened, setOpened] = useState(open);
  useEffect(() => {
    if (open) setOpened(true);
  }, [open]);
  return opened;
}

function App() {
  const setCommandBarOpen = useSetAtom(commandBarOpenAtom);
  const setTerminalVisible = useSetAtom(terminalVisibleAtom);
  const setUndoRedo = useSetAtom(undoRedoAtom);
  const commandBarMounted = useEverOpened(useAtomValue(commandBarOpenAtom));
  const propertiesMounted = useEverOpened(useAtomValue(propertiesTargetAtom) !== null);

  useLocaleSync();

  useEffect(() => watchSystemTheme(applySystemTheme), []);

  // The undo/redo stacks live in the backend; mirror their availability so
  // every surface (not just the explorer that ran the last operation) can
  // keep Ctrl+Z / Ctrl+Shift+Z accurate.
  useEffect(() => {
    const unlistenPromise = events.explorerUndoRedoChanged.listen(({ payload }) => {
      setUndoRedo(payload);
    });
    return () => {
      void unlistenPromise.then((unlisten) => unlisten());
    };
  }, [setUndoRedo]);

  // Global shortcuts: Ctrl/Cmd+K toggles the command bar (SKILL.md §15/§30),
  // Ctrl+` toggles the integrated terminal (matching VS Code on all platforms).
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;

      if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandBarOpen((open) => !open);
        return;
      }

      if (event.ctrlKey && !event.altKey && event.key === "`") {
        event.preventDefault();
        setTerminalVisible((open) => !open);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [setCommandBarOpen, setTerminalVisible]);

  return (
    <>
      <ExplorerTabs />
      {commandBarMounted && (
        <Suspense fallback={null}>
          <CommandBar />
        </Suspense>
      )}
      {propertiesMounted && (
        <Suspense fallback={null}>
          <PropertiesDialog />
        </Suspense>
      )}
      {import.meta.env.DEV && ReactQueryDevtools && (
        <Suspense fallback={null}>
          <ReactQueryDevtools initialIsOpen={false} />
        </Suspense>
      )}
    </>
  );
}

export default App;
