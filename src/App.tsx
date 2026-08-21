import { useEffect } from "react";
import { useSetAtom } from "jotai";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

import { events } from "@/bindings";
import { ExplorerTabs } from "@/features/explorer/explorer-tabs";
import { PropertiesDialog } from "@/features/explorer/properties-dialog";
import { undoRedoAtom } from "@/features/explorer/tabs";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { CommandBar, commandBarOpenAtom } from "@/features/workspace/command-bar";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";

function App() {
  const setCommandBarOpen = useSetAtom(commandBarOpenAtom);
  const setTerminalVisible = useSetAtom(terminalVisibleAtom);
  const setUndoRedo = useSetAtom(undoRedoAtom);

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
      <CommandBar />
      <PropertiesDialog />
      <ReactQueryDevtools initialIsOpen={false} />
    </>
  );
}

export default App;
