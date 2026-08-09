import { useEffect } from "react";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

import { ExplorerTabs } from "@/features/explorer/explorer-tabs";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";

function App() {
  useEffect(() => watchSystemTheme(applySystemTheme), []);

  return (
    <>
      <ExplorerTabs />
      <ReactQueryDevtools initialIsOpen={false} />
    </>
  );
}

export default App;
