//! The transfer job state machine: the queue, the conflict answers and the
//! running slots, owned by the core so every driver — the Qt shell today, an
//! elevated child later — runs the same lifecycle instead of reimplementing it.
//!
//! The scheduler is deliberately not a global: the shell owns one instance and
//! injects its runtime and [`Host`], which keeps this crate engine-free. All
//! public methods are synchronous and safe to call from the UI thread; the
//! async work (conflict check, the transfer itself) is spawned on the injected
//! runtime and reports back through [`JobEvent`].
//!
//! # Slots
//!
//! [`MAX_RUNNING`] transfers run at once; the rest wait in the queue. The
//! pre-run phase (conflict check, then the dialog) is serial: at most one job
//! is not running, because the shell has one conflict dialog and two jobs
//! asking questions through it would interleave their prompts. A job that
//! starts running frees the pre-run position for the next waiting job, so a
//! long copy never holds up the queue behind it.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::FileSystemError;
use crate::host::{Event, Host};
use crate::types::{ConflictAction, TransferConflict, TransferItem};

use super::service;

static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);

/// How many transfers run at once. Two is what a desktop disk can keep busy
/// without the panel becoming a wall of rows; the queue absorbs the rest.
pub const MAX_RUNNING: usize = 2;

/// One copy or move, from request to completion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferRequest {
    pub sources: Vec<String>,
    pub destination: String,
    pub cut: bool,
    /// Continue a cancelled copy: every source merges into the destination
    /// and the conflict check is skipped entirely.
    pub resume: bool,
    /// Shell-owned flag, echoed back in [`JobEvent::Started`] and never read
    /// here. The cut clipboard is spent when the transfer *runs*, not when it
    /// is requested, so a conflict the user cancels keeps the cut available.
    pub spends_clipboard: bool,
}

/// What the shell has to be told about one job's lifecycle.
///
/// Payload-only, like every [`Event`]: wording an error or formatting a label
/// is the shell's business. Every variant carries the `job_id` minted at
/// acceptance, so the shell can route it to the right row even while several
/// jobs run at once. Events for one job arrive in lifecycle order —
/// `Accepted`, then either `ConflictPrompt`* → `Started` → `Finished`, or
/// `CheckFailed`, or nothing further when the user dismisses the dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobEvent {
    /// A job left the queue and owns a slot (running or in its pre-run phase).
    Accepted {
        job_id: u64,
        request: TransferRequest,
    },
    /// One collision for the dialog. `index` is 0-based; `total` is the batch.
    ConflictPrompt {
        job_id: u64,
        conflict: TransferConflict,
        index: usize,
        total: usize,
        cut: bool,
    },
    /// Every decision is in (or none was needed) and the batch is running.
    /// Pause, cancel and the speed limit address it by `operation_id`.
    Started {
        job_id: u64,
        operation_id: String,
        request: TransferRequest,
    },
    /// The batch reported back and its slot is free. `error` is the raw
    /// engine error; a cancellation is not a failure.
    Finished {
        job_id: u64,
        cut: bool,
        error: Option<String>,
        cancelled: bool,
    },
    /// The conflict check itself failed, before anything ran or was asked.
    CheckFailed {
        job_id: u64,
        cut: bool,
        error: String,
    },
}

/// One accepted job, from acceptance until it reports back.
#[derive(Debug)]
struct ActiveJob {
    /// Minted at acceptance, never reused: it addresses the job's row in the
    /// shell and doubles as the token that drops stale async callbacks.
    id: u64,
    request: TransferRequest,
    /// Collisions found before transferring. Empty while the copy runs.
    conflicts: Vec<TransferConflict>,
    /// The conflict the dialog is showing; `conflicts.len()` means every
    /// decision is in.
    index: usize,
    /// Source path → resolution. A source the check found nothing for keeps
    /// `Fail`: the dialog answers collisions that were on disk when the batch
    /// was checked, and one that appears between the check and the copy is a
    /// race, not a choice.
    decisions: HashMap<String, ConflictAction>,
    running: bool,
}

impl ActiveJob {
    fn new(id: u64, request: TransferRequest) -> Self {
        Self {
            id,
            request,
            conflicts: Vec::new(),
            index: 0,
            decisions: HashMap::new(),
            running: false,
        }
    }

    /// Records `action` for the conflict on screen and steps to the next one.
    /// Returns true when every conflict has an answer, which is the moment the
    /// batch can finally run.
    fn answer(&mut self, action: ConflictAction, apply_to_all: bool) -> bool {
        let Some(conflict) = self.conflicts.get(self.index) else {
            return true;
        };
        self.decisions.insert(conflict.source_path.clone(), action);
        if apply_to_all {
            for rest in &self.conflicts[self.index + 1..] {
                self.decisions.insert(rest.source_path.clone(), action);
            }
            self.index = self.conflicts.len();
        } else {
            self.index += 1;
        }
        self.index >= self.conflicts.len()
    }

    fn prompt(&self) -> Option<JobEvent> {
        let conflict = self.conflicts.get(self.index)?;
        Some(JobEvent::ConflictPrompt {
            job_id: self.id,
            conflict: conflict.clone(),
            index: self.index,
            total: self.conflicts.len(),
            cut: self.request.cut,
        })
    }

    /// The batch with its decisions frozen. A resume merges every source; an
    /// unanswered collision fails rather than guessing.
    fn items(&self) -> Vec<TransferItem> {
        self.request
            .sources
            .iter()
            .map(|path| TransferItem {
                path: path.clone(),
                on_conflict: self.decisions.get(path).copied().unwrap_or({
                    if self.request.resume {
                        ConflictAction::Merge
                    } else {
                        ConflictAction::Fail
                    }
                }),
            })
            .collect()
    }
}

#[derive(Default)]
struct State {
    /// Copies and moves waiting for a slot, oldest first. Accepted jobs are
    /// not in here.
    queue: Vec<TransferRequest>,
    /// Accepted jobs: checking, awaiting conflict answers, or running.
    jobs: Vec<ActiveJob>,
    next_id: u64,
}

/// The async work a state transition hands back to the scheduler.
enum Spawn {
    Check {
        job_id: u64,
        sources: Vec<String>,
        destination: String,
    },
    Run {
        job_id: u64,
        items: Vec<TransferItem>,
        destination: String,
        cut: bool,
        operation_id: String,
    },
}

impl State {
    /// A slot is free *and* no job is in its pre-run phase: the one conflict
    /// dialog cannot serve two jobs at once.
    fn can_accept(&self) -> bool {
        self.jobs.len() < MAX_RUNNING && self.jobs.iter().all(|job| job.running)
    }

    fn job(&mut self, job_id: u64) -> Option<&mut ActiveJob> {
        self.jobs.iter_mut().find(|job| job.id == job_id)
    }

    /// The one job still in its pre-run phase, if any.
    fn pending_id(&self) -> Option<u64> {
        self.jobs.iter().find(|job| !job.running).map(|job| job.id)
    }

    fn remove(&mut self, job_id: u64) -> Option<ActiveJob> {
        let position = self.jobs.iter().position(|job| job.id == job_id)?;
        Some(self.jobs.remove(position))
    }

    /// Takes a slot for `request`. Pure: the caller spawns the returned work.
    fn accept(&mut self, request: TransferRequest) -> (Vec<JobEvent>, Spawn) {
        self.next_id += 1;
        let id = self.next_id;
        let resume = request.resume;
        let mut events = vec![JobEvent::Accepted {
            job_id: id,
            request: request.clone(),
        }];
        self.jobs.push(ActiveJob::new(id, request));
        if resume {
            let (more, spawn) = self.start_running(id);
            events.extend(more);
            (events, spawn)
        } else {
            let job = self.jobs.last().expect("slot just taken");
            let spawn = Spawn::Check {
                job_id: id,
                sources: job.request.sources.clone(),
                destination: job.request.destination.clone(),
            };
            (events, spawn)
        }
    }

    /// Moves a job to running: decisions frozen, operation id minted.
    fn start_running(&mut self, job_id: u64) -> (Vec<JobEvent>, Spawn) {
        let job = self.job(job_id).expect("job");
        job.running = true;
        let operation_id = format!(
            "transfer-{}",
            NEXT_OPERATION.fetch_add(1, Ordering::Relaxed)
        );
        let event = JobEvent::Started {
            job_id,
            operation_id: operation_id.clone(),
            request: job.request.clone(),
        };
        let spawn = Spawn::Run {
            job_id,
            items: job.items(),
            destination: job.request.destination.clone(),
            cut: job.request.cut,
            operation_id,
        };
        (vec![event], spawn)
    }

    /// Accepts waiting jobs while a slot and the pre-run position allow. A
    /// resume starts running inside `accept`, which can free the position
    /// again — hence the loop rather than a single accept.
    fn pump(&mut self) -> (Vec<JobEvent>, Vec<Spawn>) {
        let mut events = Vec::new();
        let mut spawns = Vec::new();
        while self.can_accept() && !self.queue.is_empty() {
            let request = self.queue.remove(0);
            let (more, spawn) = self.accept(request);
            events.extend(more);
            spawns.push(spawn);
        }
        (events, spawns)
    }

    /// A job starts running, then the queue moves up behind it.
    fn run_and_pump(&mut self, job_id: u64) -> (Vec<JobEvent>, Vec<Spawn>) {
        let (mut events, spawn) = self.start_running(job_id);
        let (more, mut spawns) = self.pump();
        events.extend(more);
        spawns.insert(0, spawn);
        (events, spawns)
    }
}

/// The transfer queue: [`MAX_RUNNING`] operation slots, one serial pre-run
/// phase, and a waiting line behind them.
pub struct JobScheduler {
    state: Mutex<State>,
    runtime: tokio::runtime::Handle,
    host: Arc<dyn Host>,
}

impl JobScheduler {
    pub fn new(runtime: tokio::runtime::Handle, host: Arc<dyn Host>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            runtime,
            host,
        })
    }

    /// Takes a transfer into the queue, or starts it when a slot is free.
    pub fn submit(self: &Arc<Self>, request: TransferRequest) {
        let (events, spawns) = {
            let mut state = self.lock();
            if !state.can_accept() {
                state.queue.push(request);
                return;
            }
            let (mut events, spawn) = state.accept(request);
            let (more, mut spawns) = state.pump();
            events.extend(more);
            spawns.insert(0, spawn);
            (events, spawns)
        };
        self.deliver(events, spawns);
    }

    /// Answers the conflict the dialog is showing; runs the batch when the
    /// last answer lands. The dialog always belongs to the one pending job.
    pub fn answer(self: &Arc<Self>, action: ConflictAction, apply_to_all: bool) {
        let (events, spawns) = {
            let mut state = self.lock();
            let Some(job_id) = state.pending_id() else {
                return;
            };
            let answered = state
                .job(job_id)
                .expect("pending job")
                .answer(action, apply_to_all);
            if answered {
                state.run_and_pump(job_id)
            } else {
                let job = state.job(job_id).expect("pending job");
                (job.prompt().into_iter().collect(), Vec::new())
            }
        };
        self.deliver(events, spawns);
    }

    /// The conflict dialog was dismissed: nothing runs, the queue advances.
    /// A running transfer is untouched — its cancel goes through the control
    /// registered under its operation id, not here.
    pub fn cancel_pending(self: &Arc<Self>) {
        let (events, spawns) = {
            let mut state = self.lock();
            let Some(job_id) = state.pending_id() else {
                return;
            };
            state.remove(job_id);
            state.pump()
        };
        self.deliver(events, spawns);
    }

    /// Drops the job waiting at `index`. Out-of-range is a no-op: the panel
    /// can be one frame behind a queue that already moved on.
    pub fn drop_queued(&self, index: usize) {
        let mut state = self.lock();
        if index < state.queue.len() {
            state.queue.remove(index);
        }
    }

    pub fn clear_queued(&self) {
        self.lock().queue.clear();
    }

    /// The waiting jobs, oldest first, for the queue panel.
    pub fn queued(&self) -> Vec<TransferRequest> {
        self.lock().queue.clone()
    }

    fn adopt_conflicts(
        self: &Arc<Self>,
        job_id: u64,
        result: Result<Vec<TransferConflict>, FileSystemError>,
    ) {
        let (events, spawns) = {
            let mut state = self.lock();
            if state.job(job_id).is_none() {
                return; // the job was dismissed while the check ran
            }
            match result {
                Ok(conflicts) if conflicts.is_empty() => state.run_and_pump(job_id),
                Ok(conflicts) => {
                    let job = state.job(job_id).expect("job");
                    job.conflicts = conflicts;
                    job.index = 0;
                    (job.prompt().into_iter().collect(), Vec::new())
                }
                Err(error) => {
                    let cut = state.job(job_id).expect("job").request.cut;
                    state.remove(job_id);
                    let (mut events, spawns) = state.pump();
                    events.insert(
                        0,
                        JobEvent::CheckFailed {
                            job_id,
                            cut,
                            error: error.to_string(),
                        },
                    );
                    (events, spawns)
                }
            }
        };
        self.deliver(events, spawns);
    }

    fn finish(self: &Arc<Self>, job_id: u64, result: Result<(), FileSystemError>) {
        let (events, spawns) = {
            let mut state = self.lock();
            let Some(job) = state.remove(job_id) else {
                return; // stale callback for a job already gone
            };
            let cancelled = matches!(result, Err(FileSystemError::Cancelled(_)));
            let (mut events, spawns) = state.pump();
            events.insert(
                0,
                JobEvent::Finished {
                    job_id,
                    cut: job.request.cut,
                    error: result.err().map(|error| error.to_string()),
                    cancelled,
                },
            );
            (events, spawns)
        };
        self.deliver(events, spawns);
    }

    /// Emits outside the lock, then spawns: a host that delivers inline (the
    /// test doubles) must never re-enter the scheduler under its own guard.
    fn deliver(self: &Arc<Self>, events: Vec<JobEvent>, spawns: Vec<Spawn>) {
        for event in events {
            self.host.emit(Event::TransferJob(event));
        }
        for spawn in spawns {
            match spawn {
                Spawn::Check {
                    job_id,
                    sources,
                    destination,
                } => {
                    let this = Arc::clone(self);
                    self.runtime.spawn(async move {
                        let result =
                            service::check_transfer_conflicts(sources, destination).await;
                        this.adopt_conflicts(job_id, result);
                    });
                }
                Spawn::Run {
                    job_id,
                    items,
                    destination,
                    cut,
                    operation_id,
                } => {
                    let this = Arc::clone(self);
                    let host = Arc::clone(&self.host);
                    self.runtime.spawn(async move {
                        let result = if cut {
                            service::move_entries(items, destination, operation_id, host).await
                        } else {
                            service::copy_entries(items, destination, operation_id, host).await
                        };
                        this.finish(job_id, result);
                    });
                }
            }
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().expect("job scheduler lock poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::EntryKind;

    fn conflict(path: &str) -> TransferConflict {
        TransferConflict {
            source_path: path.to_owned(),
            target_path: format!("{path}-target"),
            name: path.to_owned(),
            source_kind: EntryKind::File,
            source_size: Some(1),
            source_modified_at: None,
            target_kind: EntryKind::File,
            target_size: Some(2),
            target_modified_at: None,
        }
    }

    fn request(sources: &[&str], destination: &str, cut: bool) -> TransferRequest {
        TransferRequest {
            sources: sources.iter().map(|s| s.to_string()).collect(),
            destination: destination.to_owned(),
            cut,
            resume: false,
            spends_clipboard: false,
        }
    }

    fn awaiting(paths: &[&str]) -> ActiveJob {
        let mut job = ActiveJob::new(1, request(paths, "C:/dst", false));
        job.conflicts = paths.iter().map(|p| conflict(p)).collect();
        job
    }

    #[test]
    fn a_conflict_batch_waits_for_every_answer_unless_applied_to_all() {
        let mut active = awaiting(&["C:/a", "C:/b", "C:/c"]);

        // One answer at a time: the batch runs only after the third.
        assert!(!active.answer(ConflictAction::Skip, false));
        assert_eq!(active.index, 1);
        assert_eq!(active.decisions.get("C:/a"), Some(&ConflictAction::Skip));
        assert!(!active.answer(ConflictAction::KeepBoth, false));
        assert!(active.answer(ConflictAction::Replace, false));
        assert_eq!(active.decisions.len(), 3);
        assert_eq!(active.decisions.get("C:/c"), Some(&ConflictAction::Replace));
    }

    #[test]
    fn apply_to_all_answers_the_rest_with_the_same_action() {
        let mut active = awaiting(&["C:/a", "C:/b", "C:/c"]);
        active.index = 1;
        active.decisions.insert("C:/a".into(), ConflictAction::Skip);

        assert!(active.answer(ConflictAction::Replace, true));
        assert_eq!(active.decisions.get("C:/b"), Some(&ConflictAction::Replace));
        assert_eq!(active.decisions.get("C:/c"), Some(&ConflictAction::Replace));
        // The one already answered keeps its own answer.
        assert_eq!(active.decisions.get("C:/a"), Some(&ConflictAction::Skip));
    }

    #[test]
    fn an_unanswered_collision_fails_and_a_resume_merges() {
        let mut active = awaiting(&["C:/a", "C:/b"]);
        active.decisions.insert("C:/a".into(), ConflictAction::Skip);
        let items = active.items();
        assert_eq!(items[0].on_conflict, ConflictAction::Skip);
        assert_eq!(items[1].on_conflict, ConflictAction::Fail);

        active.request.resume = true;
        let items = active.items();
        assert_eq!(items[0].on_conflict, ConflictAction::Skip);
        assert_eq!(items[1].on_conflict, ConflictAction::Merge);
    }

    /// Two slots run at once; a third waits. The pre-run phase is serial: a
    /// job still checking its conflicts holds the position even with a slot
    /// free, because the shell has one dialog.
    #[test]
    fn two_slots_run_and_the_pre_run_phase_stays_serial() {
        let mut state = State::default();
        state.queue.push(request(&["C:/a"], "C:/one", false));
        state.queue.push(request(&["C:/b"], "C:/two", true));
        state.queue.push(request(&["C:/c"], "C:/three", false));

        // The first job is accepted and checks; the second must wait even
        // though a slot is free — the first is not running yet.
        let (events, spawns) = state.pump();
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], JobEvent::Accepted { job_id: 1, .. }));
        assert_eq!(spawns.len(), 1);
        assert_eq!(state.queue.len(), 2);

        // Once it runs, the second is accepted; the third still waits.
        state.start_running(1);
        let (events, _) = state.pump();
        assert!(matches!(events[0], JobEvent::Accepted { job_id: 2, .. }));
        assert_eq!(state.queue.len(), 1);

        // Both running: no capacity left for the third.
        state.start_running(2);
        let (events, spawns) = state.pump();
        assert!(events.is_empty());
        assert!(spawns.is_empty());
        assert_eq!(state.jobs.len(), MAX_RUNNING);

        // A finished job frees a slot and the third moves up.
        state.remove(1);
        let (events, _) = state.pump();
        assert!(matches!(events[0], JobEvent::Accepted { job_id: 3, .. }));
        assert!(state.queue.is_empty());
    }

    /// A resume skips the check and starts running inside `accept`, so one
    /// pump can take several waiting jobs when slots allow.
    #[test]
    fn a_resume_takes_a_slot_and_runs_without_a_check() {
        let mut state = State::default();
        let mut req = request(&["C:/a"], "C:/dst", false);
        req.resume = true;
        let (events, spawn) = state.accept(req);

        assert!(matches!(events[0], JobEvent::Accepted { job_id: 1, .. }));
        assert!(matches!(events[1], JobEvent::Started { job_id: 1, .. }));
        match spawn {
            Spawn::Run { items, .. } => assert_eq!(items[0].on_conflict, ConflictAction::Merge),
            Spawn::Check { .. } => panic!("a resume must skip the conflict check"),
        }
    }

    #[test]
    fn dropping_a_queued_transfer_leaves_the_rest_and_ignores_a_stale_index() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        let host = Arc::new(crate::host::testing::RecordingHost::default());
        let scheduler =
            JobScheduler::new(runtime.handle().clone(), host as Arc<dyn Host>);
        // The first request takes a slot; the next two wait behind the
        // pre-run phase and the slot count.
        scheduler.submit(request(&["C:/a"], "C:/one", false));
        scheduler.submit(request(&["C:/b"], "C:/two", false));
        scheduler.submit(request(&["C:/c"], "C:/three", false));

        scheduler.drop_queued(7);
        assert_eq!(scheduler.queued().len(), 2);
        scheduler.drop_queued(0);
        let queued = scheduler.queued();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].destination, "C:/three");
        scheduler.clear_queued();
        assert!(scheduler.queued().is_empty());
    }

    /// Callbacks carry the job id they were spawned for; one whose job is
    /// already gone must change nothing.
    #[test]
    fn a_stale_callback_for_a_dismissed_job_is_dropped() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("test runtime");
        let host = Arc::new(crate::host::testing::RecordingHost::default());
        let scheduler =
            JobScheduler::new(runtime.handle().clone(), Arc::clone(&host) as Arc<dyn Host>);
        scheduler.submit(request(&["C:/a"], "C:/one", false));
        scheduler.cancel_pending();
        host.take();

        scheduler.finish(1, Ok(()));
        scheduler.adopt_conflicts(1, Ok(Vec::new()));
        assert!(host.take().is_empty());
        assert!(scheduler.queued().is_empty());
    }

    /// The whole loop on a real runtime: submit, conflict prompt, answer,
    /// finished — with the file actually on disk at the end.
    #[test]
    fn a_conflicted_copy_runs_once_the_answer_lands() {
        use crate::host::testing::RecordingHost;

        let dir = std::env::temp_dir().join(format!(
            "dae-jobs-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("dst")).expect("test dirs");
        std::fs::write(dir.join("a.txt"), b"new").expect("source");
        std::fs::write(dir.join("dst/a.txt"), b"old").expect("collision");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let host = Arc::new(RecordingHost::default());
        let scheduler = JobScheduler::new(runtime.handle().clone(), Arc::clone(&host) as Arc<dyn Host>);

        let source = dir.join("a.txt").to_string_lossy().into_owned();
        let destination = dir.join("dst").to_string_lossy().into_owned();

        runtime.block_on(async {
            scheduler.submit(TransferRequest {
                sources: vec![source],
                destination,
                cut: false,
                resume: false,
                spends_clipboard: false,
            });

            let prompt = wait_for(&host, |event| {
                matches!(event, Event::TransferJob(JobEvent::ConflictPrompt { .. }))
            })
            .await;
            let Event::TransferJob(JobEvent::ConflictPrompt { total, .. }) = prompt else {
                unreachable!()
            };
            assert_eq!(total, 1);

            scheduler.answer(ConflictAction::Replace, false);

            let finished = wait_for(&host, |event| {
                matches!(event, Event::TransferJob(JobEvent::Finished { .. }))
            })
            .await;
            assert_eq!(
                finished,
                Event::TransferJob(JobEvent::Finished {
                    job_id: 1,
                    cut: false,
                    error: None,
                    cancelled: false,
                })
            );
        });

        assert_eq!(
            std::fs::read(dir.join("dst/a.txt")).expect("copied"),
            b"new"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two copies submitted back to back both reach the disk. (That the slots
    /// genuinely overlap is proven deterministically by the state-machine
    /// test above; here the blocking pool's timing is not ours to pin.)
    #[test]
    fn two_copies_run_through_the_two_slots() {
        use crate::host::testing::RecordingHost;

        let dir = std::env::temp_dir().join(format!(
            "dae-jobs-two-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("one")).expect("test dirs");
        std::fs::create_dir_all(dir.join("two")).expect("test dirs");
        std::fs::write(dir.join("a.txt"), b"a").expect("source a");
        std::fs::write(dir.join("b.txt"), b"b").expect("source b");

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let host = Arc::new(RecordingHost::default());
        let scheduler = JobScheduler::new(runtime.handle().clone(), Arc::clone(&host) as Arc<dyn Host>);

        let path = |name: &str| dir.join(name).to_string_lossy().into_owned();
        runtime.block_on(async {
            scheduler.submit(request(&[&path("a.txt")], &path("one"), false));
            scheduler.submit(request(&[&path("b.txt")], &path("two"), false));

            let mut finished = 0;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while finished < 2 {
                for event in host.take() {
                    if matches!(event, Event::TransferJob(JobEvent::Finished { error: None, .. })) {
                        finished += 1;
                    }
                }
                assert!(std::time::Instant::now() < deadline, "jobs never finished");
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        });

        assert_eq!(std::fs::read(dir.join("one/a.txt")).expect("copied a"), b"a");
        assert_eq!(std::fs::read(dir.join("two/b.txt")).expect("copied b"), b"b");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Polls the recording host until `predicate` matches or five seconds
    /// pass; the scheduler's work lands on the runtime this polls inside.
    async fn wait_for(
        host: &crate::host::testing::RecordingHost,
        predicate: impl Fn(&Event) -> bool,
    ) -> Event {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if let Some(event) = host.take().into_iter().find(|e| predicate(e)) {
                return event;
            }
            assert!(std::time::Instant::now() < deadline, "job event never arrived");
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}