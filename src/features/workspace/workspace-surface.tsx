import { lazy, Suspense } from "react";
import { useAtomValue } from "jotai";

import { Skeleton } from "@/components/ui/skeleton";

import { OverviewView } from "./overview-view";
import { tabSurfaceFamily } from "./tab-surface";

// Only the Overview surface is rendered on the first frame; every other
// surface loads its chunk on demand so the initial JS parse stays lean.
const SplitExplorerView = lazy(() =>
  import("@/features/explorer/split-view").then((m) => ({ default: m.SplitExplorerView })),
);
const RecentsView = lazy(() =>
  import("./recents-view").then((m) => ({ default: m.RecentsView })),
);
const FavoritesView = lazy(() =>
  import("./favorites-view").then((m) => ({ default: m.FavoritesView })),
);
const TrashView = lazy(() =>
  import("./trash-view").then((m) => ({ default: m.TrashView })),
);
const SpaceView = lazy(() =>
  import("./space-view").then((m) => ({ default: m.SpaceView })),
);

/** Lightweight placeholder while a lazy surface chunk loads. */
function SurfaceSkeleton() {
  return (
    <div className="flex flex-1 flex-col gap-3 p-4">
      <Skeleton className="h-5 w-32" />
      <Skeleton className="h-full w-full rounded-lg" />
    </div>
  );
}

/**
 * Renders the active surface of one tab: the workspace surfaces (Overview,
 * Recents, Favorites, Space) or the classic folder explorer, which itself
 * switches between the single and dual-pane layouts.
 */
export function WorkspaceSurfaceView({ tabId }: { tabId: string }) {
  const surface = useAtomValue(tabSurfaceFamily(tabId));

  switch (surface.kind) {
    case "overview":
      return <OverviewView />;
    case "recents":
      return <Suspense fallback={<SurfaceSkeleton />}><RecentsView /></Suspense>;
    case "favorites":
      return <Suspense fallback={<SurfaceSkeleton />}><FavoritesView /></Suspense>;
    case "trash":
      return <Suspense fallback={<SurfaceSkeleton />}><TrashView /></Suspense>;
    case "space":
      return (
        <Suspense fallback={<SurfaceSkeleton />}>
          <SpaceView key={surface.spaceId} spaceId={surface.spaceId} />
        </Suspense>
      );
    case "folder":
      return (
        <Suspense fallback={<SurfaceSkeleton />}>
          <SplitExplorerView tabId={tabId} />
        </Suspense>
      );
  }
}
