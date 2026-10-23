import { lazy, Suspense, type ComponentType } from "react";
import { useAtomValue } from "jotai";

import { Skeleton } from "@/components/ui/skeleton";

import type { EntryPreviewProps } from "./entry-preview";
import { inspectorAtom } from "./inspector-atom";

/**
 * The window's inspector column: the reference's third column, beside the main
 * column rather than inside it. `ExplorerTabs` renders it after `main`, so the
 * terminal drawer and the status strip stay *in* `main` and stop at the
 * inspector's hairline — the composition the reference draws.
 *
 * There is one column for the whole window and it shows the focused pane's
 * panel: in a split the two panes each keep their own open flag and their own
 * selection, and only the focused one publishes into `inspectorAtom`. Panes of
 * an unselected tab publish nothing at all, so the Overview and the other
 * workspace surfaces get no column.
 *
 * It subscribes to the atom in this leaf rather than in `ExplorerTabs` for the
 * same reason `FocusedPaneStatusBar` does: the atom is republished on every
 * selection change in the listing, and a subscription one level up would
 * re-render the tab strip, the rail and every kept-alive surface with it.
 */

/**
 * The preview panel is the only thing that reaches TanStack Markdown and the
 * highlight grammars: keeping it out of the explorer chunk took that chunk
 * from 306 KB to 242 KB, with the preview chain left as a 66 KB chunk of its
 * own. It sits behind the inspector toggle, so nothing on the first frame needs
 * it.
 *
 * `PreloadedEntryPreview` mirrors `preloadExplorerSurface` in
 * `workspace-surface.tsx`, for the same reason: `lazy()` resumes its boundary
 * as scheduled concurrent work, which would put the first Quick Look a frame or
 * two behind the click that started it — and the shared-element transition from
 * the listing row to the preview's hero plate wants the plate present on that
 * click's own frame.
 */
const LazyEntryPreview = lazy(() =>
  import("./entry-preview").then((m) => ({ default: m.EntryPreview })),
);
let PreloadedEntryPreview: ComponentType<EntryPreviewProps> | null = null;

/** Warms the preview chunk. Safe to call repeatedly; never rejects. */
export function preloadInspectorPanel(): void {
  if (PreloadedEntryPreview) return;

  void import("./entry-preview").then(
    (module) => {
      PreloadedEntryPreview = module.EntryPreview;
    },
    () => {
      // A failed warm is not fatal: the lazy boundary below retries it, and the
      // panel then arrives a frame after the toggle instead of with it.
    },
  );
}

/** The panel's shape while its chunk loads, so the column doesn't reflow when
 *  the real one lands. */
function InspectorSkeleton() {
  return (
    <aside className="flex h-full w-preview shrink-0 flex-col gap-3 overflow-hidden border-l border-border p-3">
      <Skeleton className="h-5 w-2/3" />
      <Skeleton className="min-h-48 flex-1 rounded-xl" />
      <Skeleton className="h-16 shrink-0 rounded-md" />
    </aside>
  );
}

export function InspectorPanel() {
  const snapshot = useAtomValue(inspectorAtom);
  if (!snapshot || !snapshot.isOpen) return null;

  const props: EntryPreviewProps = {
    entries: snapshot.entries,
    onClose: snapshot.onClose,
    onOpen: snapshot.onOpen,
  };

  // Keyed by owner: handing the column to the other split pane swaps the
  // *content*, and the panel's own preview state (the read text, the media
  // probe, the scroll position) belongs to the pane that put it up. Without the
  // key React would reuse the instance and paint one frame of the previous
  // pane's file under the new one's name.
  return PreloadedEntryPreview ? (
    <PreloadedEntryPreview key={snapshot.owner} {...props} />
  ) : (
    <Suspense fallback={<InspectorSkeleton />}>
      <LazyEntryPreview key={snapshot.owner} {...props} />
    </Suspense>
  );
}
