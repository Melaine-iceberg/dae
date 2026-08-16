import { useAtomValue } from "jotai";

import { ExplorerView } from "@/features/explorer/explorer-view";
import { getTabNavigator } from "@/features/explorer/tabs";

import { FavoritesView } from "./favorites-view";
import { OverviewView } from "./overview-view";
import { RecentsView } from "./recents-view";
import { SpaceView } from "./space-view";
import { tabSurfaceFamily } from "./tab-surface";

/**
 * Renders the active surface of one tab: the workspace surfaces (Overview,
 * Recents, Favorites, Space) or the classic folder explorer.
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
    case "space":
      return <SpaceView key={surface.spaceId} spaceId={surface.spaceId} />;
    case "folder":
      return <ExplorerView navigator={getTabNavigator(tabId)} />;
  }
}
