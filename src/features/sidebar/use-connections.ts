import { useCallback, useEffect, useState } from "react";

import { commands, type StoredConnection } from "@/bindings";

/** Lists saved network connections; callers refresh after save/delete. */
export function useConnections() {
  const [connections, setConnections] = useState<StoredConnection[]>([]);

  const refresh = useCallback(() => {
    void commands
      .listConnections()
      .then(setConnections)
      .catch((error: unknown) => console.warn("Unable to list connections", error));
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  return { connections, refresh };
}
