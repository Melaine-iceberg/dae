//! Per-row selection state for the explorer's models.
//!
//! The selection used to live on the tab as a `path -> true` map, which made
//! every delegate re-evaluate a JS call on each change and every click copy the
//! whole map. It lives in the model now: a row carries its own flag, the model
//! publishes it as the `selected` role, and a change emits `dataChanged` for
//! the rows that actually changed. That is one signal per click instead of one
//! JS call per visible delegate, and it lets the status bar read a count that
//! is maintained rather than recounted.
//!
//! The row flags are the truth. [`Selection::indices`] is only a sorted index
//! into them, so that "clear" and "toggle" touch the selected rows instead of
//! scanning a 120,000-row listing per click; it is dropped by any row edit and
//! rebuilt from the flags on the next selection call.

use std::collections::HashSet;

/// A row that can carry selection state.
pub(crate) trait SelectableRow {
    fn is_selected(&self) -> bool;

    fn set_selected(&mut self, selected: bool);

    /// Identity that survives a row rebuild: the name for a row under the
    /// listing base, the whole path for a row that spells its own.
    fn selection_key(&self) -> &str;
}

/// A contiguous span of rows whose selection changed, `[first, last]`.
pub(crate) type Run = (usize, usize);

/// A rectangular band of cells in a grid of `columns` columns. The detail list
/// is the degenerate case: one column, so the band is a row range.
#[derive(Clone, Copy)]
pub(crate) struct Band {
    pub first_row: usize,
    pub last_row: usize,
    pub first_column: usize,
    pub last_column: usize,
    pub columns: usize,
}

impl Band {
    /// Clips the band to `len` rows and appends the cells it covers.
    fn append_indices(&self, len: usize, out: &mut Vec<usize>) {
        if self.columns == 0 {
            return;
        }
        for row in self.first_row..=self.last_row {
            for column in self.first_column..=self.last_column {
                let index = row * self.columns + column;
                if index >= len {
                    // Rows are the outer loop, so the rest of this row and
                    // every row after it are past the end too.
                    return;
                }
                out.push(index);
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct Selection {
    /// Sorted indices of the selected rows, or `None` once a row edit made
    /// them address different rows.
    indices: Option<Vec<usize>>,
    /// Number of rows with the flag set. Kept separately because the view
    /// reads it while `indices` is stale.
    count: i32,
    /// Paths from [`Selection::restore`] whose rows have not streamed in yet.
    /// They stay paths until the listing base they belong to is known — the
    /// stream sets it with its head batch, which is after a history move asks
    /// for the restore.
    pending: HashSet<String>,
    /// `pending`, converted to row keys once the base is known.
    pending_keys: Option<HashSet<String>>,
    /// The selected rows as they were when a marquee began.
    marquee: Option<Vec<usize>>,
}

impl Selection {
    pub(crate) fn count(&self) -> i32 {
        self.count
    }

    /// The keys of every selected row. Allocates one key per selected row, so
    /// it is a rebuild helper, not a per-click path.
    pub(crate) fn selected_keys<T: SelectableRow>(&self, rows: &[T]) -> Vec<String> {
        rows.iter()
            .filter(|row| row.is_selected())
            .map(|row| row.selection_key().to_owned())
            .collect()
    }

    /// Selects only `row`.
    pub(crate) fn select_only<T: SelectableRow>(&mut self, rows: &mut [T], row: usize) -> Vec<Run> {
        self.clear_pending();
        if row >= rows.len() {
            // A click on a row that vanished under the pointer means "nothing
            // here is selected", not "keep whatever was".
            return self.clear(rows);
        }
        let next = vec![row];
        self.write(rows, next)
    }

    /// Flips `row`.
    pub(crate) fn toggle<T: SelectableRow>(&mut self, rows: &mut [T], row: usize) -> Vec<Run> {
        self.clear_pending();
        if row >= rows.len() {
            return Vec::new();
        }
        let previous = self.take_indices(rows);
        let mut next = previous.clone();
        match next.binary_search(&row) {
            Ok(at) => {
                next.remove(at);
            }
            Err(at) => next.insert(at, row),
        }
        self.write_with(rows, previous, next)
    }

    /// Replaces the selection with `[first, last]`, clipped to the rows that
    /// exist.
    pub(crate) fn select_range<T: SelectableRow>(
        &mut self,
        rows: &mut [T],
        first: i32,
        last: i32,
    ) -> Vec<Run> {
        self.clear_pending();
        let next = match clipped_rows(rows.len(), first, last) {
            Some((start, end)) => (start..=end).collect(),
            None => Vec::new(),
        };
        self.write(rows, next)
    }

    pub(crate) fn select_all<T: SelectableRow>(&mut self, rows: &mut [T]) -> Vec<Run> {
        self.clear_pending();
        let next = (0..rows.len()).collect();
        self.write(rows, next)
    }

    pub(crate) fn clear<T: SelectableRow>(&mut self, rows: &mut [T]) -> Vec<Run> {
        self.clear_pending();
        self.marquee = None;
        self.write(rows, Vec::new())
    }

    /// Replaces the selection with the rows whose path is in `paths`. A path
    /// whose row has not arrived yet — a history entry restored before its
    /// listing finished streaming — is held until it does. `base` is the
    /// directory the rows are addressed from, empty while the stream has not
    /// said what it is.
    pub(crate) fn restore<T: SelectableRow>(
        &mut self,
        rows: &mut [T],
        paths: &[String],
        base: &str,
    ) -> Vec<Run> {
        self.marquee = None;
        self.pending = paths.iter().cloned().collect();
        self.pending_keys = None;
        self.resolve_pending(base);
        let mut next = Vec::new();
        if let Some(keys) = self.pending_keys.as_mut() {
            for (index, row) in rows.iter().enumerate() {
                if keys.remove(row.selection_key()) {
                    next.push(index);
                }
            }
        }
        self.write(rows, next)
    }

    /// Converts the pending paths to row keys, once. The base arrives with the
    /// head batch, which is why this is called from the insert path rather
    /// than from [`Selection::restore`] alone.
    pub(crate) fn resolve_pending(&mut self, base: &str) {
        if self.pending_keys.is_some() || base.is_empty() {
            return;
        }
        let keys: HashSet<String> = self
            .pending
            .iter()
            .map(|path| path_key(path, base))
            .collect();
        self.pending.clear();
        self.pending_keys = Some(keys);
    }

    /// Starts a rubber-band drag. `keep` is the Ctrl case: the selection the
    /// band extends, rather than the empty base a plain drag starts from.
    pub(crate) fn begin_marquee<T: SelectableRow>(
        &mut self,
        rows: &mut [T],
        keep: bool,
    ) -> Vec<Run> {
        self.clear_pending();
        if keep {
            let base = self.take_indices(rows);
            self.marquee = Some(base);
            return Vec::new();
        }
        self.marquee = Some(Vec::new());
        self.write(rows, Vec::new())
    }

    /// The selection a marquee drag describes: the base it started from plus
    /// the band under the pointer. `band` is `None` while the pointer is off
    /// the cells, which leaves the base alone.
    pub(crate) fn marquee_cells<T: SelectableRow>(
        &mut self,
        rows: &mut [T],
        band: Option<Band>,
    ) -> Vec<Run> {
        let previous = self.take_indices(rows);
        let base = self.marquee.take().unwrap_or_else(|| previous.clone());
        let mut next = base.clone();
        if let Some(band) = band {
            band.append_indices(rows.len(), &mut next);
        }
        next.sort_unstable();
        next.dedup();
        self.marquee = Some(base);
        self.write_with(rows, previous, next)
    }

    pub(crate) fn end_marquee(&mut self) {
        self.marquee = None;
    }

    /// New rows arrived. Flags the ones a pending restore is waiting for, and
    /// drops the index cache: every index past the insertion points elsewhere
    /// now.
    pub(crate) fn rows_inserted<T: SelectableRow>(&mut self, rows: &mut [T]) {
        if let Some(keys) = self.pending_keys.as_mut() {
            let mut flagged = 0;
            for row in rows {
                if keys.remove(row.selection_key()) {
                    row.set_selected(true);
                    flagged += 1;
                }
            }
            self.count += flagged;
        }
        self.invalidate();
    }

    /// Rows left the model; their flags leave the count with them.
    pub(crate) fn rows_removed<T: SelectableRow>(&mut self, rows: &[T]) {
        let dropped = rows.iter().filter(|row| row.is_selected()).count() as i32;
        self.count = (self.count - dropped).max(0);
        self.invalidate();
    }

    /// Rows were rebuilt: flags every row whose key is in `keys` (the rows that
    /// were selected before the rebuild) or that a pending restore is waiting
    /// for. Only used where the rebuild is local — the surrounding rows kept
    /// their own flags, so the count is adjusted rather than recomputed.
    pub(crate) fn flag<T: SelectableRow>(&mut self, rows: &mut [T], keys: &HashSet<String>) {
        let mut flagged = 0;
        for row in rows.iter_mut() {
            let wanted = keys.contains(row.selection_key())
                || self
                    .pending_keys
                    .as_mut()
                    .is_some_and(|pending| pending.remove(row.selection_key()));
            if wanted && !row.is_selected() {
                row.set_selected(true);
                flagged += 1;
            }
        }
        self.count += flagged;
        self.invalidate();
    }

    /// A row edit that keeps the flags where they are (a re-sort) still moves
    /// the indices.
    pub(crate) fn invalidate(&mut self) {
        self.indices = None;
        self.marquee = None;
    }

    /// The model is empty or showing another directory.
    pub(crate) fn reset(&mut self) {
        self.indices = Some(Vec::new());
        self.count = 0;
        self.pending.clear();
        self.pending_keys = None;
        self.marquee = None;
    }

    /// The stream ended: a path that never matched a row is a file that was
    /// renamed or deleted since the selection was captured.
    pub(crate) fn settle(&mut self) {
        self.pending.clear();
        self.pending_keys = None;
        self.indices = None;
    }

    fn clear_pending(&mut self) {
        self.pending.clear();
        self.pending_keys = None;
    }

    /// Takes the selected indices, rebuilding them from the row flags when a
    /// row edit dropped them.
    fn take_indices<T: SelectableRow>(&mut self, rows: &[T]) -> Vec<usize> {
        self.indices.take().unwrap_or_else(|| {
            rows.iter()
                .enumerate()
                .filter(|(_, row)| row.is_selected())
                .map(|(index, _)| index)
                .collect()
        })
    }

    fn write<T: SelectableRow>(&mut self, rows: &mut [T], next: Vec<usize>) -> Vec<Run> {
        let previous = self.take_indices(rows);
        self.write_with(rows, previous, next)
    }

    /// Writes `next` into the row flags and reports the spans that changed.
    fn write_with<T: SelectableRow>(
        &mut self,
        rows: &mut [T],
        previous: Vec<usize>,
        next: Vec<usize>,
    ) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        let mut left = 0;
        let mut right = 0;
        while left < previous.len() || right < next.len() {
            let (index, selected) = match (previous.get(left), next.get(right)) {
                (Some(&was), Some(&now)) if was == now => {
                    left += 1;
                    right += 1;
                    continue;
                }
                (Some(&was), Some(&now)) if was < now => {
                    left += 1;
                    (was, false)
                }
                (Some(_), Some(&now)) => {
                    right += 1;
                    (now, true)
                }
                (Some(&was), None) => {
                    left += 1;
                    (was, false)
                }
                (None, Some(&now)) => {
                    right += 1;
                    (now, true)
                }
                (None, None) => break,
            };
            rows[index].set_selected(selected);
            match runs.last_mut() {
                Some((_, last)) if *last + 1 == index => *last = index,
                _ => runs.push((index, index)),
            }
        }
        self.count = next.len() as i32;
        self.indices = Some(next);
        runs
    }
}

/// The key a row under `base` is addressed by, from a full path. The same
/// splitting the listing uses: a path below the base is that base's child, so
/// the key is the name; anything else keeps its whole spelling.
pub(crate) fn path_key(path: &str, base: &str) -> String {
    match path.strip_prefix(base) {
        Some(name) if !name.is_empty() => name.to_owned(),
        _ => path.to_owned(),
    }
}

/// `[first, last]` clipped to `len` rows, or `None` when the range addresses
/// nothing. A Shift-click and a marquee both reach past the last row on
/// purpose; the intersection is cheaper than letting the caller probe.
pub(crate) fn clipped_rows(len: usize, first: i32, last: i32) -> Option<(usize, usize)> {
    if first < 0 || last < first {
        return None;
    }
    let start = first as usize;
    if start >= len {
        return None;
    }
    let end = (last as usize).min(len - 1);
    Some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Row {
        key: String,
        selected: bool,
    }

    impl Row {
        fn new(key: &str) -> Self {
            Self {
                key: key.to_owned(),
                selected: false,
            }
        }
    }

    impl SelectableRow for Row {
        fn is_selected(&self) -> bool {
            self.selected
        }

        fn set_selected(&mut self, selected: bool) {
            self.selected = selected;
        }

        fn selection_key(&self) -> &str {
            &self.key
        }
    }

    fn rows(keys: &[&str]) -> Vec<Row> {
        keys.iter().map(|key| Row::new(key)).collect()
    }

    fn selected(rows: &[Row]) -> Vec<&str> {
        rows.iter()
            .filter(|row| row.selected)
            .map(|row| row.key.as_str())
            .collect()
    }

    #[test]
    fn only_one_row_survives_a_click() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c"]);
        selection.select_all(&mut rows);
        assert_eq!(selection.count(), 3);

        let runs = selection.select_only(&mut rows, 1);
        assert_eq!(selected(&rows), vec!["b"]);
        assert_eq!(selection.count(), 1);
        // a and c were deselected, b stayed: two spans, not three rows.
        assert_eq!(runs, vec![(0, 0), (2, 2)]);

        // Clicking the row that is already alone in the selection is a no-op,
        // signals included.
        assert!(selection.select_only(&mut rows, 1).is_empty());
        assert_eq!(selection.count(), 1);

        // A row past the end means the listing no longer has it: the click
        // clears instead of leaving the old selection in place.
        selection.select_only(&mut rows, 9);
        assert_eq!(selection.count(), 0);
    }

    #[test]
    fn a_toggle_flips_one_row_and_keeps_the_rest() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c"]);
        selection.select_only(&mut rows, 0);
        assert_eq!(selection.toggle(&mut rows, 2), vec![(2, 2)]);
        assert_eq!(selected(&rows), vec!["a", "c"]);
        assert_eq!(selection.count(), 2);

        selection.toggle(&mut rows, 0);
        assert_eq!(selected(&rows), vec!["c"]);
        assert_eq!(selection.count(), 1);
    }

    #[test]
    fn a_range_is_clipped_to_the_rows_that_exist() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c"]);
        assert_eq!(selection.select_range(&mut rows, 1, 9), vec![(1, 2)]);
        assert_eq!(selected(&rows), vec!["b", "c"]);

        // Past the last row the range addresses nothing, so the selection
        // empties instead of keeping the previous range.
        assert_eq!(selection.select_range(&mut rows, 3, 5), vec![(1, 2)]);
        assert_eq!(selection.count(), 0);
        assert!(selection.select_range(&mut rows, -1, 2).is_empty());
        assert!(selection.select_range(&mut rows, 2, 1).is_empty());
    }

    #[test]
    fn clearing_reports_every_selected_span() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c", "d"]);
        selection.select_range(&mut rows, 0, 1);
        selection.toggle(&mut rows, 3);
        assert_eq!(selection.clear(&mut rows), vec![(0, 1), (3, 3)]);
        assert_eq!(selection.count(), 0);
        assert!(selection.clear(&mut rows).is_empty());
    }

    #[test]
    fn a_restore_holds_the_paths_its_rows_have_not_reached_yet() {
        const BASE: &str = "root/";
        let mut selection = Selection::default();
        // The first call lands before the stream has said what the base is:
        // the paths stay paths, and no row is flagged yet.
        let mut empty: Vec<Row> = Vec::new();
        selection.restore(&mut empty, &["root/a".to_owned(), "root/z".to_owned()], "");
        assert_eq!(selection.count(), 0);

        // The head batch brings the base, and the rows it carries take the
        // flags their paths ask for.
        selection.resolve_pending(BASE);
        let mut first = rows(&["a", "b"]);
        selection.rows_inserted(&mut first);
        assert_eq!(selected(&first), vec!["a"]);
        assert_eq!(selection.count(), 1);

        // `z` arrives with a later batch and inherits the pending flag.
        let mut later = rows(&["c", "z"]);
        selection.rows_inserted(&mut later);
        assert_eq!(selected(&later), vec!["z"]);
        assert_eq!(selection.count(), 2);

        // The batch that settles the stream forgets a path whose row never
        // arrived, so a later listing cannot resurrect it.
        selection.settle();
        let mut never = rows(&["z"]);
        selection.rows_inserted(&mut never);
        assert!(selected(&never).is_empty());
    }

    #[test]
    fn a_restore_over_rows_that_already_arrived_flags_them_immediately() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c"]);
        let runs = selection.restore(
            &mut rows,
            &["root/c".to_owned(), "root/a".to_owned()],
            "root/",
        );
        assert_eq!(selected(&rows), vec!["a", "c"]);
        assert_eq!(runs, vec![(0, 0), (2, 2)]);
        assert_eq!(selection.count(), 2);
    }

    #[test]
    fn a_marquee_band_builds_on_the_base_it_started_from() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c", "d", "e"]);
        selection.select_only(&mut rows, 0);

        // Ctrl-drag: the row selected before the drag survives the band.
        selection.begin_marquee(&mut rows, true);
        let band = Band {
            first_row: 2,
            last_row: 3,
            first_column: 0,
            last_column: 0,
            columns: 1,
        };
        selection.marquee_cells(&mut rows, Some(band));
        assert_eq!(selected(&rows), vec!["a", "c", "d"]);

        // The pointer moves back: rows outside the new band return to the base.
        let band = Band {
            first_row: 2,
            last_row: 2,
            first_column: 0,
            last_column: 0,
            columns: 1,
        };
        selection.marquee_cells(&mut rows, Some(band));
        assert_eq!(selected(&rows), vec!["a", "c"]);

        // Off the cells: the base alone.
        selection.marquee_cells(&mut rows, None);
        assert_eq!(selected(&rows), vec!["a"]);
        selection.end_marquee();
        assert!(selection.marquee.is_none());
    }

    #[test]
    fn a_marquee_band_covers_the_cells_of_a_grid_row() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c", "d", "e", "f"]);
        selection.begin_marquee(&mut rows, false);
        let band = Band {
            first_row: 0,
            last_row: 1,
            first_column: 1,
            last_column: 2,
            columns: 3,
        };
        assert_eq!(
            selection.marquee_cells(&mut rows, Some(band)),
            vec![(1, 2), (4, 5)]
        );
        assert_eq!(selected(&rows), vec!["b", "c", "e", "f"]);
        // The band reaches past the last row: the cells that do not exist are
        // dropped instead of addressing the wrong row.
        let band = Band {
            first_row: 0,
            last_row: 3,
            first_column: 0,
            last_column: 2,
            columns: 3,
        };
        selection.marquee_cells(&mut rows, Some(band));
        assert_eq!(selection.count(), 6);
    }

    #[test]
    fn a_row_edit_makes_the_cached_indices_rebuild_from_the_flags() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c"]);
        selection.select_only(&mut rows, 2);
        // A sort moved the row: the flag travelled, the index did not.
        rows.swap(0, 2);
        selection.invalidate();
        assert_eq!(selection.count(), 1);
        assert_eq!(selected(&rows), vec!["c"]);
        // The next click finds the selected row from the flags, not from the
        // index the click before the sort left behind.
        assert_eq!(selection.toggle(&mut rows, 1), vec![(1, 1)]);
        assert_eq!(selected(&rows), vec!["c", "b"]);
    }

    #[test]
    fn removed_rows_leave_the_count() {
        let mut selection = Selection::default();
        let mut rows = rows(&["a", "b", "c"]);
        selection.select_range(&mut rows, 0, 2);
        let removed: Vec<Row> = rows.drain(..2).collect();
        selection.rows_removed(&removed);
        assert_eq!(selection.count(), 1);
        assert_eq!(selected(&rows), vec!["c"]);
        selection.reset();
        assert_eq!(selection.count(), 0);
    }
}
