import { useEffect } from "react";
import { useSetAtom } from "jotai";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

import { ExplorerTabs } from "@/features/explorer/explorer-tabs";
import { CommandBar, commandBarOpenAtom } from "@/features/workspace/command-bar";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";

function App() {
  const setCommandBarOpen = useSetAtom(commandBarOpenAtom);

  useEffect(() => watchSystemTheme(applySystemTheme), []);

  // Global shortcut: Ctrl/Cmd+K toggles the command bar (SKILL.md §15/§30).
  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.isComposing) return;

      if ((event.ctrlKey || event.metaKey) && !event.altKey && event.key.toLowerCase() === "k") {
        event.preventDefault();
        setCommandBarOpen((open) => !open);
      }
    };

    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [setCommandBarOpen]);

  return (
    <>
      <ExplorerTabs />
      <CommandBar />
      <ReactQueryDevtools initialIsOpen={false} />
    </>
  );
}

export default App;
