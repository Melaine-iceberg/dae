import { lazy, Suspense, type ComponentType } from "react";
import { useAtomValue } from "jotai";

import { Skeleton } from "@/components/ui/skeleton";

import { OverviewView } from "./overview-view";
import { tabSurfaceFamily } from "./tab-surface";

// Only the Overview surface is rendered on the first frame; every other
// surface loads its chunk on demand so the initial JS parse stays lean.
const LazySplitExplorerView = lazy(() =>
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

type SplitExplorerProps = { tabId: string };

/**
 * The explorer module, hoisted out of `lazy` and loaded ahead of the first
 * folder open by `preloadExplorerSurface()` (see main.tsx).
 *
 * The explorer is the one surface whose first open is a click away from the
 * window's first frame, and `lazy()` resolves asynchronously: the surface
 * committed its skeleton immediately and only mounted the explorer once React
 * resumed the suspended boundary. That resume is scheduled as concurrent work
 * and yields to the browser between slices, so the mount landed ~300 ms after
 * the click (measured; the module itself was already in memory, and rendering
 * it directly instead took 16-80 ms). Preloading the module and rendering it
 * synchronously keeps the first folder open on the click's own frame while the
 * chunk stays out of the entry bundle.
 *
 * The binding is capitalized on purpose: the JSX transform classifies an
 * element by the resolved binding's name, so a lowercase binding compiles to
 * an intrinsic element (`jsx("splitExplorer", …)`) that React renders as an
 * unknown custom element — silently, with no error.
 */
let PreloadedSplitExplorer: ComponentType<SplitExplorerProps> | null = null;

/** Loads the explorer chunk. Safe to call repeatedly; never rejects. */
export function preloadExplorerSurface(): Promise<void> {
  if (PreloadedSplitExplorer) return Promise.resolve();

  return import("@/features/explorer/split-view").then(
    (module) => {
      PreloadedSplitExplorer = module.SplitExplorerView;
    },
    () => {
      // A failed preload is not fatal: the lazy fallback below retries it, and
      // surfaces its own error through the Suspense boundary.
    },
  );
}

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
      return PreloadedSplitExplorer ? (
        <PreloadedSplitExplorer tabId={tabId} />
      ) : (
        <Suspense fallback={<SurfaceSkeleton />}>
          <LazySplitExplorerView tabId={tabId} />
        </Suspense>
      );
  }
}
