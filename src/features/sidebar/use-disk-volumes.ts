import { useEffect, useRef, useState } from "react";

import { commands } from "@/bindings";

import type { DiskVolume } from "./types";

const DISK_POLL_INTERVAL_MS = 60_000;
const NAVIGATION_REFRESH_THROTTLE_MS = 30_000;

/**
 * Lists disk volumes and keeps them fresh: polls on an interval, refreshes when
 * the window regains focus, and (throttled) whenever the viewed path changes,
 * since moving across volumes is when capacity or removable media change.
 *
 * Returns `null` until the first fetch resolves so callers can tell loading
 * apart from an empty machine. Meant to be mounted lazily (collapsed sidebar
 * sections should not pay for the probe).
 */
export function useDiskVolumes(viewedPath: string | null): DiskVolume[] | null {
  const [volumes, setVolumes] = useState<DiskVolume[] | null>(null);
  const lastRefreshAtRef = useRef(0);
  const refreshRef = useRef<(force?: boolean) => void>(() => {});

  useEffect(() => {
    let disposed = false;

    const refresh = (force = false) => {
      const now = Date.now();
      if (!force && now - lastRefreshAtRef.current < NAVIGATION_REFRESH_THROTTLE_MS) return;
      lastRefreshAtRef.current = now;

      void commands
        .listDisks()
        .then((nextVolumes) => {
          if (!disposed) setVolumes(nextVolumes);
        })
        .catch((error: unknown) => console.warn("Unable to list disks", error));
    };

    refreshRef.current = refresh;
    refresh(true);

    const interval = window.setInterval(() => refresh(true), DISK_POLL_INTERVAL_MS);
    const handleFocus = () => refresh(true);
    window.addEventListener("focus", handleFocus);

    return () => {
      disposed = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", handleFocus);
    };
  }, []);

  useEffect(() => {
    if (viewedPath !== null) {
      refreshRef.current();
    }
  }, [viewedPath]);

  return volumes;
}
