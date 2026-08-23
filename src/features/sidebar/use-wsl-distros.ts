import { useEffect, useState } from "react";

import { commands } from "@/bindings";

import type { WslDistro } from "./types";

/**
 * Lists installed WSL distros, refreshing when the window regains focus.
 * Returns `null` until the first fetch resolves so callers can tell loading
 * apart from "no distros installed". Meant to be mounted lazily — the probe
 * spawns `wsl.exe`, which cold start should never pay for.
 */
export function useWslDistros(): WslDistro[] | null {
  const [distros, setDistros] = useState<WslDistro[] | null>(null);

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
