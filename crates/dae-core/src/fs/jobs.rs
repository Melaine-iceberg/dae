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
//! [`MAX_SLOTS`] jobs are held at once, each owning a slot from acceptance
//! until it reports back — the conflict check, the answers and the copy all
//! happen inside the slot. The rest wait in the queue. A slot is a job, not
//! a phase (the lesson borrowed from Opus and the KIO Scheduler), so a
//! crawling remote check or a dialog nobody answers parks one slot instead
//! of pausing the whole line behind it. The price is that the shell must
//! serve several jobs' questions at once: it queues their prompts, asks
//! them one at a time, and every answer and dismissal carries a job id.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::error::FileSystemError;
use crate::host::{Event, Host};
use crate::types::{ConflictAction, TransferConflict, TransferItem};

use super::service;

static NEXT_OPERATION: AtomicU64 = AtomicU64::new(1);

/// How many jobs the scheduler holds at once, a slot each, from acceptance
/// (conflict check, conflict answers) until the batch reports back. Four
/// keeps a crawling SFTP transfer from ever being the only thing moving:
/// the queue behind it still runs local copies, and the global token
/// bucket — not this count — is what keeps the wire honest.
pub const MAX_SLOTS: usize = 4;

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
    /// A slot is free. A job holds its slot through its whole pre-run phase
    /// — the shell serves each job's prompts independently — so one job
    /// checking or asking never holds up the next.
    fn can_accept(&self) -> bool {
        self.jobs.len() < MAX_SLOTS
    }

    fn job(&mut self, job_id: u64) -> Option<&mut ActiveJob> {
        self.jobs.iter_mut().find(|job| job.id == job_id)
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

    /// Accepts waiting jobs while a slot allows. A resume starts running
    /// inside `accept` without freeing a slot, so the loop simply continues
    /// until the slots are full or the queue is empty.
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

    /// Answers the conflict `job_id` is asking about; the batch runs when the
    /// last answer lands, and its slot keeps holding the job either way.
    /// Several jobs can be asking at once, so the address is per job: a
    /// job that is already running is not asking anything, and a stale
    /// answer from a dialog the queue moved past changes nothing.
    fn answer(
        &mut self,
        job_id: u64,
        action: ConflictAction,
        apply_to_all: bool,
    ) -> (Vec<JobEvent>, Vec<Spawn>) {
        let Some(job) = self.job(job_id) else {
            return (Vec::new(), Vec::new());
        };
        if job.running {
            return (Vec::new(), Vec::new());
        }
        let answered = job.answer(action, apply_to_all);
        if !answered {
            let prompt = self
                .job(job_id)
                .expect("job just answered")
                .prompt()
                .expect("an unanswered batch has a next conflict");
            return (vec![prompt], Vec::new());
        }
        self.run_and_pump(job_id)
    }

    /// The dialog for `job_id` was dismissed: nothing of that job runs, its
    /// slot frees and the queue moves up. Other jobs' questions — and every
    /// running transfer — are untouched.
    fn dismiss(&mut self, job_id: u64) -> (Vec<JobEvent>, Vec<Spawn>) {
        let Some(job) = self.job(job_id) else {
            return (Vec::new(), Vec::new());
        };
        if job.running {
            return (Vec::new(), Vec::new());
        }
        self.remove(job_id);
        self.pump()
    }
}

/// The transfer queue: [`MAX_SLOTS`] job slots — checking, answering,
/// running, in any mix — and a waiting line behind them.
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

    /// Answers the conflict the dialog is showing for `job_id`; runs the
    /// batch when the last answer lands. Several jobs can be asking at once,
    /// so the address is per job.
    pub fn answer(self: &Arc<Self>, job_id: u64, action: ConflictAction, apply_to_all: bool) {
        let (events, spawns) = {
            let mut state = self.lock();
            state.answer(job_id, action, apply_to_all)
        };
        self.deliver(events, spawns);
    }

    /// The conflict dialog for `job_id` was dismissed: nothing of that job
    /// runs, its slot frees and the queue advances. Other jobs — running or
    /// asking — are untouched; a running transfer's cancel goes through the
    /// control registered under its operation id, not here.
    pub fn dismiss(self: &Arc<Self>, job_id: u64) {
        let (events, spawns) = {
            let mut state = self.lock();
            state.dismiss(job_id)
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

    /// A slot belongs to a whole job — checking, awaiting answers, running,
    /// any mix. The queue therefore accepts the next job while one is still
    /// in its pre-run phase: a crawling remote check or a dialog nobody
    /// answers parks one slot, not the line behind it.
    #[test]
    fn slots_hold_whole_jobs_so_a_slow_pre_run_never_blocks_the_queue() {
        let mut state = State::default();
        state.queue.push(request(&["C:/a"], "C:/one", false));
        state.queue.push(request(&["C:/b"], "C:/two", true));
        state.queue.push(request(&["C:/c"], "C:/three", false));
        state.queue.push(request(&["C:/d"], "C:/four", false));

        // Every job is accepted at once, each with its conflict check in
        // flight — impossible when the pre-run phase was one global
        // position the shell's single dialog had to serve.
        let (events, spawns) = state.pump();
        assert_eq!(events.len(), MAX_SLOTS);
        assert_eq!(spawns.len(), MAX_SLOTS);
        assert!(spawns.iter().all(|spawn| matches!(spawn, Spawn::Check { .. })));
        assert!(state.queue.is_empty());
        assert_eq!(state.jobs.len(), MAX_SLOTS);

        // The slots are full: a fifth waits.
        state.queue.push(request(&["C:/e"], "C:/five", false));
        let (events, spawns) = state.pump();
        assert!(events.is_empty());
        assert!(spawns.is_empty());

        // One job answers its collisions and starts running — still one job
        // per slot, so the fifth keeps waiting.
        let job = state.job(2).expect("job");
        job.conflicts = vec![conflict("C:/b")];
        let (events, _) = state.answer(2, ConflictAction::Replace, false);
        assert!(matches!(events[..], [JobEvent::Started { job_id: 2, .. }]));
        let (events, _) = state.pump();
        assert!(events.is_empty());

        // A finished job frees its slot and the fifth moves up.
        state.remove(4);
        let (events, _) = state.pump();
        assert!(matches!(events[..], [JobEvent::Accepted { job_id: 5, .. }]));
        assert!(state.queue.is_empty());
    }

    /// Several jobs can be awaiting answers at once; an answer reaches the
    /// job it names and nothing else, and a dismissal only ever drops the
    /// job it names.
    #[test]
    fn answers_and_dismissals_reach_the_named_job_and_only_that_one() {
        let mut state = State::default();
        state.queue.push(request(&["C:/a"], "C:/one", false));
        state.queue.push(request(&["C:/b"], "C:/two", false));
        state.pump();
        for (id, path) in [(1, "C:/a"), (2, "C:/b")] {
            let job = state.job(id).expect("job");
            job.conflicts = vec![conflict(path), conflict(&format!("{path}-2"))];
        }

        // Job 2's first answer advances job 2 alone; job 1's batch is frozen
        // until its own turn.
        let (events, spawns) = state.answer(2, ConflictAction::Skip, false);
        assert!(matches!(
            events[..],
            [JobEvent::ConflictPrompt { job_id: 2, index: 1, total: 2, .. }]
        ));
        assert!(spawns.is_empty());
        assert!(!state.job(1).expect("job").decisions.contains_key("C:/a"));
        assert_eq!(state.job(2).expect("job").index, 1);

        // A stale answer — a job already running, or one that never was —
        // changes nothing.
        state.start_running(1);
        let (events, _) = state.answer(1, ConflictAction::Skip, false);
        assert!(events.is_empty());
        let (events, _) = state.answer(7, ConflictAction::Skip, false);
        assert!(events.is_empty());

        // Job 2's second answer runs its batch; a running job is not
        // dismissible, so both jobs survive a dismissal addressed at them.
        let (events, _) = state.answer(2, ConflictAction::Replace, true);
        assert!(matches!(events[..], [JobEvent::Started { job_id: 2, .. }]));
        for id in [1, 2] {
            let (events, spawns) = state.dismiss(id);
            assert!(events.is_empty());
            assert!(spawns.is_empty());
            assert!(state.job(id).is_some());
        }

        // A job that is still asking, though, is dismissible: it goes, and
        // with the queue empty the event list says nothing else moved.
        state.queue.push(request(&["C:/c"], "C:/three", false));
        let (events, _) = state.pump();
        assert!(matches!(events[..], [JobEvent::Accepted { job_id: 3, .. }]));
        let job = state.job(3).expect("job");
        job.conflicts = vec![conflict("C:/c")];
        let (events, spawns) = state.dismiss(3);
        assert!(events.is_empty());
        assert!(spawns.is_empty());
        assert!(state.job(3).is_none());
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
        // The first four submissions take the four slots and sit in their
        // checks (this runtime is never polled); the fifth waits in the
        // queue.
        scheduler.submit(request(&["C:/a"], "C:/one", false));
        scheduler.submit(request(&["C:/b"], "C:/two", false));
        scheduler.submit(request(&["C:/c"], "C:/three", false));
        scheduler.submit(request(&["C:/d"], "C:/four", false));
        scheduler.submit(request(&["C:/e"], "C:/five", false));

        scheduler.drop_queued(7);
        assert_eq!(scheduler.queued().len(), 1);
        scheduler.drop_queued(0);
        assert!(scheduler.queued().is_empty());
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
        scheduler.dismiss(1);
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

            scheduler.answer(1, ConflictAction::Replace, false);

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

    /// Two copies submitted back to back both reach the disk, each holding
    /// its own slot. (That the slots genuinely overlap is proven
    /// deterministically by the state-machine test above; here the blocking
    /// pool's timing is not ours to pin.)
    #[test]
    fn two_copies_run_through_the_slots() {
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