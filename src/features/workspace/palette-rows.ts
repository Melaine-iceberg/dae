/**
 * Pure ordering + geometry for the command palette's result list.
 *
 * It lives outside `command-bar.tsx` for one reason: the palette renders a
 * virtualized list, and the two facts that make virtualization correct — the
 * order results are laid out in, and the row index each result ends up at —
 * are exactly the facts worth testing without a browser. Everything here is
 * data in, data out; no React, no Jotai, no Tauri.
 *
 * The ordering rule is the interesting half. Results are ranked by fuzzy
 * score globally, and a global order interleaves categories: with `doc` in the
 * box the list alternates command / favorite / command, so a section header
 * has nothing left to head and the palette used to drop the headers entirely
 * as soon as a query was typed. Bucketing by category first and ordering the
 * buckets by their best match keeps the headers meaningful while still letting
 * a strong hit float its whole section upward.
 */

import type { RankedResult } from "@/lib/fuzzy";

export interface PaletteSection<TGroup extends string, TItem> {
  group: TGroup;
  /** Best match first. Never empty. */
  entries: RankedResult<TItem>[];
}

export type PaletteRow<TGroup extends string, TItem> =
  | {
      kind: "header";
      key: string;
      group: TGroup;
      /** The section before it needs a hairline to read as a new block. */
      separator: boolean;
    }
  | {
      kind: "item";
      key: string;
      group: TGroup;
      entry: RankedResult<TItem>;
      /** Position in the flat result list — what keyboard navigation moves in. */
      resultIndex: number;
    };

/**
 * Buckets already-rankable results by category and orders the categories by
 * their strongest member.
 *
 * `ranked` must already be in score order; ties inside a bucket keep the
 * catalogue's own order (which is what makes the empty-query list identical to
 * before: every score is 0, so the buckets fall back to `groupOrder` and the
 * entries inside them stay in listing order).
 */
export function rankIntoGroups<TGroup extends string, TItem>(
  ranked: readonly RankedResult<TItem>[],
  groupOf: (item: TItem) => TGroup,
  groupOrder: readonly TGroup[],
): PaletteSection<TGroup, TItem>[] {
  const buckets = new Map<TGroup, RankedResult<TItem>[]>();

  for (const result of ranked) {
    const group = groupOf(result.item);
    const bucket = buckets.get(group);
    if (bucket) bucket.push(result);
    else buckets.set(group, [result]);
  }

  const sections: PaletteSection<TGroup, TItem>[] = [];
  for (const [group, entries] of buckets) {
    sections.push({ group, entries: [...entries].sort((left, right) => right.score - left.score) });
  }

  sections.sort((left, right) => {
    const delta = (right.entries[0]?.score ?? 0) - (left.entries[0]?.score ?? 0);
    return delta !== 0 ? delta : groupOrder.indexOf(left.group) - groupOrder.indexOf(right.group);
  });

  return sections;
}

/**
 * Flattens sections into the row list the virtualizer walks: one header row
 * per section followed by its items.
 *
 * Headers and items share one index space, so `resultIndex` (which the
 * keyboard uses) is not the same as the row index the virtualizer needs —
 * the caller keeps the mapping. Item keys are derived from the result
 * position rather than the command id: two commands may legitimately share an
 * id across modes, and duplicate React keys inside a virtualized list are
 * silent.
 */
export function buildPaletteRows<TGroup extends string, TItem>(
  sections: readonly PaletteSection<TGroup, TItem>[],
): PaletteRow<TGroup, TItem>[] {
  const rows: PaletteRow<TGroup, TItem>[] = [];
  let resultIndex = 0;

  sections.forEach((section, sectionIndex) => {
    rows.push({
      group: section.group,
      key: `header:${section.group}`,
      kind: "header",
      separator: sectionIndex > 0,
    });

    for (const entry of section.entries) {
      rows.push({
        entry,
        group: section.group,
        key: `item:${resultIndex}`,
        kind: "item",
        resultIndex: resultIndex++,
      });
    }
  });

  return rows;
}

/** Maps each result index to the row index that renders it. */
export function rowIndicesByResult<TGroup extends string, TItem>(
  rows: readonly PaletteRow<TGroup, TItem>[],
): number[] {
  const indices: number[] = [];
  rows.forEach((row, rowIndex) => {
    if (row.kind === "item") indices[row.resultIndex] = rowIndex;
  });
  return indices;
}
