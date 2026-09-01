import { useAtomValue } from "jotai";

import { SplitExplorerView } from "@/features/explorer/split-view";

import { FavoritesView } from "./favorites-view";
import { OverviewView } from "./overview-view";
import { RecentsView } from "./recents-view";
import { SpaceView } from "./space-view";
import { tabSurfaceFamily } from "./tab-surface";
import { TrashView } from "./trash-view";

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
      return <RecentsView />;
    case "favorites":
      return <FavoritesView />;
    case "trash":
      return <TrashView />;
    case "space":
      return <SpaceView key={surface.spaceId} spaceId={surface.spaceId} />;
    case "folder":
      return <SplitExplorerView tabId={tabId} />;
  }
}
