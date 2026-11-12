//! A real PTY shell driven through the libghostty-vt grid.
//!
//! The unit tests in `pty` cover the palette; these cover the path the app
//! actually takes: spawn a shell, parse its output, read styled snapshots and
//! move the viewport — all without Qt.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use dae_core::pty::{FLAG_SPACER, TerminalEvent, TerminalSession, TerminalSnapshot};

fn grid_text(snapshot: &TerminalSnapshot) -> String {
    let mut text = String::new();
    for row in snapshot.cells.chunks(snapshot.cols) {
        for cell in row {
            if cell.flags & FLAG_SPACER == 0 {
                text.push(cell.ch);
            }
        }
        text.push('\n');
    }
    text
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
    loop {
        if Instant::now() >= rewrite {
            session.write(command.as_bytes()).expect("write");
            rewrite = Instant::now() + Duration::from_secs(2);
        }
        let last = session.snapshot(true).map(|snapshot| grid_text(&snapshot));
        if let Some(text) = &last
            && text.lines().any(|line| line.trim() == needle)
        {
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

    let snapshot = session.snapshot(true).expect("snapshot");
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
    while Instant::now() < deadline {
        for line in 0..5 {
            session
                .write(format!("echo vt-churn-{line}\r").as_bytes())
                .expect("write");
        }
        if let Some(snapshot) = session.snapshot(true) {
            assert_eq!(snapshot.cells.len(), snapshot.cols * snapshot.rows);
            frames += 1;
        }
    }
    assert!(frames > 100, "took {frames} snapshots");
    assert!(!events.lock().unwrap().is_empty());
}
