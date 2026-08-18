import { useEffect } from "react";
import { useSetAtom } from "jotai";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

import { ExplorerTabs } from "@/features/explorer/explorer-tabs";
import { terminalVisibleAtom } from "@/features/terminal/terminal-atoms";
import { CommandBar, commandBarOpenAtom } from "@/features/workspace/command-bar";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";

function App() {
  const setCommandBarOpen = useSetAtom(commandBarOpenAtom);
  const setTerminalVisible = useSetAtom(terminalVisibleAtom);

  useEffect(() => watchSystemTheme(applySystemTheme), []);

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
      <ReactQueryDevtools initialIsOpen={false} />
    </>
  );
}

export default App;
