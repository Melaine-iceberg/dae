import { useEffect } from "react";
import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

import { ExplorerView } from "@/features/explorer/explorer-view";
import { applySystemTheme, watchSystemTheme } from "@/lib/theme";

function App() {
  useEffect(() => watchSystemTheme(applySystemTheme), []);

  return (
    <>
      <ExplorerView />
      <ReactQueryDevtools initialIsOpen={false} />
    </>
  );
}

export default App;
