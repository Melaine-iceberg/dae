//! A real PTY shell driven through the libghostty-vt grid.
//!
//! The unit tests in `pty` cover the palette; these cover the path the app
//! actually takes: spawn a shell, parse its output, read styled snapshots and
//! move the viewport — all without Qt.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dae_core::pty::{
    CellSnapshot, FLAG_SPACER, SnapshotKind, TerminalEvent, TerminalSession, TerminalSnapshot,
};

struct ShadowGrid {
    cols: usize,
    rows: usize,
    cells: Vec<CellSnapshot>,
}

impl ShadowGrid {
    fn new() -> Self {
        Self {
            cols: 0,
            rows: 0,
            cells: Vec::new(),
        }
    }

    fn apply(&mut self, snapshot: &TerminalSnapshot) {
        match &snapshot.kind {
            SnapshotKind::Full => {
                self.cols = snapshot.cols;
                self.rows = snapshot.rows;
                self.cells = snapshot.cells.clone();
            }
            SnapshotKind::Shift(k) => {
                assert_eq!((self.cols, self.rows), (snapshot.cols, snapshot.rows));
                let total = self.cols * self.rows;
                let shift = k * self.cols;
                self.cells.copy_within(shift..total, 0);
                self.cells[total - shift..].copy_from_slice(&snapshot.cells);
            }
            SnapshotKind::Rows(dirty) => {
                assert_eq!((self.cols, self.rows), (snapshot.cols, snapshot.rows));
                for row in dirty {
                    let start = usize::from(row.row) * self.cols;
                    self.cells[start..start + self.cols].copy_from_slice(&row.cells);
                }
            }
        }
    }

    fn text(&self) -> String {
        let mut text = String::new();
        for row in self.cells.chunks(self.cols.max(1)) {
            for cell in row {
                if cell.flags & FLAG_SPACER == 0 {
                    text.push(cell.ch);
                }
            }
            text.push('\n');
        }
        text
    }
}

/// Runs `echo <needle>` and waits until the answer appears on a line of its own.
///
/// Two startup hazards, one answer. The echoed input also contains the needle,
/// so a substring match would return before the shell has run the command; and
/// input written while the shell is still starting can be swallowed before it is
/// read (a greeting that reads stdin, a line-discipline flush), so the command is
/// rewritten until it is answered. No prompt shape is a reliable ready signal
/// across shells, and `echo` is idempotent, so a duplicate answer is harmless.
fn answered(session: &TerminalSession, needle: &str) {
    let command = format!("echo {needle}\r");
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut rewrite = Instant::now();
    let mut grid = ShadowGrid::new();
    let mut initialized = false;
    loop {
        if Instant::now() >= rewrite {
            session.write(command.as_bytes()).expect("write");
            rewrite = Instant::now() + Duration::from_secs(2);
        }
        let snapshot = if !initialized {
            initialized = true;
            session.snapshot_full(true)
        } else {
            session.snapshot(true)
        };
        if let Some(snapshot) = snapshot {
            grid.apply(&snapshot);
        }
        let last = grid.text();
        if last.lines().any(|line| line.trim() == needle) {
            return;
        }
        if Instant::now() >= deadline {
            panic!("{needle:?} never got a line of its own; last snapshot:\n{last:?}");
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn spawn_session() -> (TerminalSession, Arc<Mutex<Vec<TerminalEvent>>>) {
    let events = Arc::new(Mutex::new(Vec::new()));
    let notify_events = Arc::clone(&events);
    let session = TerminalSession::spawn(
        None,
        80,
        24,
        Arc::new(move |event| notify_events.lock().unwrap().push(event)),
        None,
    )
    .expect("spawn a shell");
    (session, events)
}

#[test]
fn the_shell_answers_a_command_in_the_grid() {
    let (session, _events) = spawn_session();

    // The answer proves the library answered the shell's terminal queries
    // through the write-pty callback; without that reply PowerShell never
    // finishes drawing its first prompt.
    answered(&session, "vt-smoke-ok");

    let snapshot = session.snapshot_full(true).expect("snapshot");
    assert_eq!((snapshot.cols, snapshot.rows), (80, 24));
    assert_eq!(snapshot.cells.len(), 80 * 24);
    assert!(snapshot.cursor_visible);
}

#[test]
fn the_viewport_scrolls_and_returns() {
    let (session, _events) = spawn_session();
    answered(&session, "vt-scroll-ok");

    for line in 0..40 {
        session
            .write(format!("echo vt-line-{line}\r").as_bytes())
            .expect("write");
    }
    answered(&session, "vt-line-39");

    let snapshot = session.snapshot(true).expect("snapshot");
    assert!(
        snapshot.history_size > 0,
        "40 lines on a 24-row grid must leave history"
    );

    session.scroll(5);
    let scrolled = session.snapshot(true).expect("snapshot");
    assert_eq!(scrolled.display_offset, 5);

    session.scroll(-10);
    let back = session.snapshot(true).expect("snapshot");
    assert_eq!(back.display_offset, 0);

    session.scroll_to(snapshot.history_size);
    let top = session.snapshot(true).expect("snapshot");
    assert_eq!(top.display_offset, snapshot.history_size);
}

#[test]
fn resizing_rebuilds_the_grid() {
    let (session, _events) = spawn_session();
    session.resize(100, 30).expect("resize");
    let snapshot = session.snapshot(true).expect("snapshot");
    assert_eq!((snapshot.cols, snapshot.rows), (100, 30));
    assert_eq!(snapshot.cells.len(), 3000);

    answered(&session, "vt-resize-ok");
}

#[test]
fn snapshots_stay_consistent_while_output_streams() {
    let (session, events) = spawn_session();
    answered(&session, "vt-stream-ok");

    let deadline = Instant::now() + Duration::from_secs(3);
    let mut frames = 0usize;
    let mut full = 0usize;
    let mut shifted = 0usize;
    let mut partial = 0usize;
    let mut first = true;
    while Instant::now() < deadline {
        for line in 0..5 {
            session
                .write(format!("echo vt-churn-{line}\r").as_bytes())
                .expect("write");
        }
        let snapshot = if first {
            first = false;
            session.snapshot_full(true)
        } else {
            session.snapshot(true)
        };
        if let Some(snapshot) = snapshot {
            match snapshot.kind {
                SnapshotKind::Full => {
                    assert_eq!(snapshot.cells.len(), snapshot.cols * snapshot.rows);
                    full += 1;
                }
                SnapshotKind::Shift(k) => {
                    // Only the k lines the scroll brought in cross.
                    assert!(k >= 1 && k < snapshot.rows, "shift {k}");
                    assert_eq!(snapshot.cells.len(), k * snapshot.cols);
                    shifted += 1;
                }
                SnapshotKind::Rows(rows) => {
                    assert!(rows.iter().all(|r| usize::from(r.row) < snapshot.rows));
                    assert!(rows.iter().all(|r| r.cells.len() == snapshot.cols));
                    partial += 1;
                }
            }
            frames += 1;
        }
    }
    assert!(frames > 100, "took {frames} snapshots");
    assert!(shifted > 0, "a streaming shell never sent a shift frame");
    assert!(full > 0, "the first frame was never full");
    assert!(partial > 0, "idle/partial snapshots never emitted Rows");
    assert!(!events.lock().unwrap().is_empty());
}

#[test]
fn search_finds_matches_and_steps_through_them() {
    let (session, _events) = spawn_session();
    answered(&session, "vt-search-ready");
    for line in 0..3 {
        session
            .write(format!("echo vt-needle-{line}\r").as_bytes())
            .expect("write");
    }
    // Give the shell a moment to print, then search for what it printed.
    thread::sleep(Duration::from_millis(600));
    session.set_search_needle(Some("vt-needle-"));

    let (total, _) = session.search_step(true).expect("a match");
    assert!(total >= 3, "found {total} matches for the echoed lines");

    // Every step stays inside the match list and reports a consistent count.
    let (total_again, _) = session.search_step(true).expect("another match");
    assert_eq!(total_again, total);

    // The echoed lines are on screen, so the viewport has rectangles.
    let rects = session.search_viewport();
    assert!(!rects.is_empty(), "no viewport rectangles for on-screen text");
    assert!(rects.iter().all(|rect| rect.columns > 0));

    // Clearing the needle clears the highlights.
    session.set_search_needle(None);
    assert!(session.search_viewport().is_empty());
    assert!(session.search_step(true).is_none());
}
