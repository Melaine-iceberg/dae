import { useEffect, useState } from "react";

import { commands } from "@/bindings";

import type { WslDistro } from "./types";

/** Lists installed WSL distros, refreshing when the window regains focus. */
export function useWslDistros(): WslDistro[] {
  const [distros, setDistros] = useState<WslDistro[]>([]);

  useEffect(() => {
    let disposed = false;

    const refresh = () => {
      void commands
        .listWslDistros()
        .then((nextDistros) => {
          if (!disposed) setDistros(nextDistros);
        })
        .catch((error: unknown) => console.warn("Unable to list WSL distros", error));
    };

    refresh();
    window.addEventListener("focus", refresh);

    return () => {
      disposed = true;
      window.removeEventListener("focus", refresh);
    };
  }, []);

  return distros;
}
