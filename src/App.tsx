import { ReactQueryDevtools } from "@tanstack/react-query-devtools";

import { ExplorerView } from "@/features/explorer/explorer-view";

function App() {
  return (
    <>
      <ExplorerView />
      <ReactQueryDevtools initialIsOpen={false} />
    </>
  );
}

export default App;
