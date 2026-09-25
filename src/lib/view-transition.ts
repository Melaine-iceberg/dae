/**
 * Shared-element motion — the one place this shell spends it.
 *
 * The gesture is Finder's Quick Look: a preview opens *out of the file the
 * user is looking at*. The entry's visual (its icon tile, or its thumbnail for
 * an image) grows into the preview's hero, so the panel never appears to come
 * from nowhere. It is functional rather than decorative: it answers "where did
 * the thing I pressed Space on go".
 *
 * The View Transitions API is the whole implementation — `startViewTransition`
 * snapshots the page, runs the state update, and morphs any element whose
 * `view-transition-name` matches between the two snapshots. Two halves:
 *
 *   - the *source* (this module): named on the entry's visual in the old
 *     snapshot only, via `findEntryVisual`, and stood down again inside the
 *     update so the new snapshot has exactly one element carrying the name;
 *   - the *target* (`entry-preview.tsx`): the class `entry-preview-hero`,
 *     declared in App.css, on the preview's hero — the thumbnail plate for an
 *     image, the header icon otherwise.
 *
 * Everything else — the panel arriving, the listing narrowing — is left to the
 * transition's own root crossfade, which is the same fade the panel's
 * `animate-in` would have drawn.
 *
 * Deliberately not taken further:
 *
 *   - the close is a plain fade. The gesture is the opening; running the morph
 *     backwards would also need the source to be named in the *new* snapshot
 *     while the hero keeps the old one, which is the one bookkeeping this API
 *     makes awkward for no extra meaning;
 *   - no transition at all where the API is missing (WebKitGTK builds before
 *     the Safari 18 implementation) or where the user asked for reduced
 *     motion — both fall through to the state update, which is what the code
 *     did before this existed.
 */

import { flushSync } from "react-dom";

/** The name both halves of the pair carry; mirrors `.entry-preview-hero`. */
export const ENTRY_PREVIEW_HERO_NAME = "entry-preview-hero";

type ViewTransitionHandle = { finished: Promise<void> };
type ViewTransitionDocument = Document & {
  startViewTransition?: (update: () => void) => ViewTransitionHandle;
};

/**
 * Finds the entry visual a transition should start from — the `EntryIconFrame`
 * span, which wraps every icon variant (type tile, native bitmap, thumbnail)
 * and carries `data-entry-visual={entry.path}`.
 *
 * Imperative lookup on purpose: the View Transitions API works on the DOM that
 * is on screen at the moment of the change, and the listing is virtualized —
 * the entry may legitimately have no visual to start from, which is the
 * `null` this returns and the caller treats as "no morph".
 */
export function findEntryVisual(path: string | null): HTMLElement | null {
  if (!path) return null;
  for (const element of document.querySelectorAll<HTMLElement>("[data-entry-visual]")) {
    if (element.dataset.entryVisual === path) return element;
  }
  return null;
}

/**
 * Runs `update` with `source` morphing into whatever carries
 * `ENTRY_PREVIEW_HERO_NAME` afterwards.
 *
 * `flushSync` inside the transition callback is the documented pairing: the
 * API needs the new DOM *now* to take its snapshot, and React would otherwise
 * schedule the commit.
 */
export function withSharedElement(source: HTMLElement | null, update: () => void): void {
  const doc = document as ViewTransitionDocument;
  const reduced = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (!doc.startViewTransition || reduced || !source) {
    update();
    return;
  }

  source.style.setProperty("view-transition-name", ENTRY_PREVIEW_HERO_NAME);
  let transition: ViewTransitionHandle;
  try {
    transition = doc.startViewTransition(() => {
      flushSync(update);
      // Stood down before the new snapshot: with the hero carrying the name
      // too, two elements would share it and the API drops the morph.
      source.style.removeProperty("view-transition-name");
    });
  } catch (error) {
    source.style.removeProperty("view-transition-name");
    throw error;
  }

  void transition.finished
    .catch(() => {})
    .then(() => source.style.removeProperty("view-transition-name"));
}
