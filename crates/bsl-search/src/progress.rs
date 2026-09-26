//! Generation-fenced progress; snapshots never wait on indexing work.
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

static NEXT_PASS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPassState {
    Waiting,
    Running,
    Ready,
    Disabled,
    Failed,
    Cancelled,
    Superseded,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexPhase {
    Initializing,
    Parsing,
    LexicalIndexing,
    Embedding,
    Persisting,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexCounters {
    pub total_files: usize,
    pub total_chunks: Option<usize>,
    pub total_batches: Option<usize>,
    pub done_chunks: usize,
    pub done_batches: usize,
}
#[derive(Debug, Clone)]
pub struct IndexProgressSnapshot {
    pub active: bool,
    pub state: IndexPassState,
    pub phase: Option<IndexPhase>,
    pub pass_id: Option<String>,
    pub counters: Option<IndexCounters>,
}
#[derive(Debug)]
struct Record {
    generation: u64,
    snapshot: IndexProgressSnapshot,
}
#[derive(Debug)]
pub struct IndexProgress {
    active: AtomicBool,
    record: Mutex<Record>,
}
impl Default for IndexProgress {
    fn default() -> Self {
        Self {
            active: AtomicBool::new(false),
            record: Mutex::new(Record {
                generation: 0,
                snapshot: IndexProgressSnapshot {
                    active: false,
                    state: IndexPassState::Waiting,
                    phase: None,
                    pass_id: None,
                    counters: None,
                },
            }),
        }
    }
}
impl IndexProgress {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
    /// Suspend the current attempt's liveness during a retry backoff.
    pub fn pause_pass(self: &Arc<Self>) -> PausedPass {
        let mut record = self.record.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = record.generation;
        record.snapshot.active = false;
        self.active.store(false, Ordering::Relaxed);
        PausedPass { progress: Arc::clone(self), generation }
    }
    pub fn snapshot(&self) -> Option<IndexProgressSnapshot> {
        self.record.try_lock().ok().map(|record| record.snapshot.clone())
    }
    pub fn percent(&self) -> usize {
        self.snapshot()
            .and_then(|s| s.counters)
            .filter(|c| c.total_chunks.is_some_and(|t| t > 0 && c.done_chunks <= t))
            .map_or(0, |c| {
                ((c.done_chunks as u128 * 100) / c.total_chunks.unwrap() as u128) as usize
            })
    }
    pub fn reset(&self) {
        let mut record = self.record.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // Consume a generation so callbacks from the reset attempt cannot publish again.
        record.generation = 0;
        record.snapshot = IndexProgressSnapshot {
            active: false,
            state: IndexPassState::Waiting,
            phase: None,
            pass_id: None,
            counters: None,
        };
        self.active.store(false, Ordering::Relaxed);
    }
    pub fn begin_pass(self: &Arc<Self>) -> ActivePass {
        self.begin_pass_with_counter(&NEXT_PASS)
    }

    fn begin_pass_with_counter(self: &Arc<Self>, sequence: &AtomicU64) -> ActivePass {
        // Formatting/process identity lookup occurs outside the bounded record lock.
        let prefix =
            blake3::hash(crate::lifecycle::process_id().as_bytes()).to_hex()[..32].to_owned();
        let generation = sequence
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
            .ok()
            .map(|n| n + 1);
        let pass_id = generation.map(|n| format!("{prefix}:{n}"));
        let mut record = self.record.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(generation) = generation {
            record.generation = generation;
            record.snapshot = IndexProgressSnapshot {
                active: true,
                state: IndexPassState::Running,
                phase: Some(IndexPhase::Initializing),
                pass_id,
                counters: None,
            };
            self.active.store(true, Ordering::Relaxed);
        } else {
            record.snapshot = IndexProgressSnapshot {
                active: false,
                state: IndexPassState::Unknown,
                phase: None,
                pass_id: None,
                counters: None,
            };
            self.active.store(false, Ordering::Relaxed);
        }
        ActivePass {
            token: IndexPassToken { progress: Arc::clone(self), generation },
            finished: false,
        }
    }
}
/// Restores liveness only while the suspended attempt still owns the running pass.
pub struct PausedPass {
    progress: Arc<IndexProgress>,
    generation: u64,
}
impl Drop for PausedPass {
    fn drop(&mut self) {
        let mut record =
            self.progress.record.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if record.generation == self.generation && record.snapshot.state == IndexPassState::Running
        {
            record.snapshot.active = true;
            self.progress.active.store(record.snapshot.active, Ordering::Relaxed);
        }
    }
}
#[derive(Debug, Clone)]
pub struct IndexPassToken {
    progress: Arc<IndexProgress>,
    generation: Option<u64>,
}
impl IndexPassToken {
    fn update(&self, update: impl FnOnce(&mut IndexProgressSnapshot)) {
        let mut record =
            self.progress.record.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.generation != Some(record.generation)
            || record.snapshot.state != IndexPassState::Running
        {
            return;
        }
        update(&mut record.snapshot);
    }
    fn release(&self) {
        let mut record =
            self.progress.record.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if self.generation == Some(record.generation) {
            record.snapshot.active = false;
            self.progress.active.store(false, Ordering::Relaxed);
        }
    }
    pub fn phase(&self, phase: IndexPhase) {
        self.update(|s| {
            s.phase = Some(phase);
            s.counters = None;
        });
    }
    pub fn set_totals(&self, files: usize, chunks: usize, batches: usize) {
        self.update(|s| {
            s.phase = Some(IndexPhase::Embedding);
            s.counters = Some(IndexCounters {
                total_files: files,
                total_chunks: Some(chunks),
                total_batches: Some(batches),
                done_chunks: 0,
                done_batches: 0,
            });
        });
    }
    /// Extend the batch total for a pass that plans its requests one owner at a time: the
    /// byte-bounded split of an owner is known only once its missing texts are.
    pub fn add_batches(&self, batches: usize) {
        self.update(|s| {
            if let Some(c) = s.counters.as_mut() {
                match c.total_batches.map(|total| total.checked_add(batches)) {
                    Some(Some(total)) => c.total_batches = Some(total),
                    Some(None) => s.counters = None,
                    None => {}
                }
            }
        });
    }
    pub fn advance(&self, chunks: usize, batches: usize) {
        self.update(|s| {
            if let Some(c) = s.counters.as_mut() {
                match (c.done_chunks.checked_add(chunks), c.done_batches.checked_add(batches)) {
                    (Some(chunks), Some(batches))
                        if c.total_chunks.is_none_or(|total| chunks <= total)
                            && c.total_batches.is_none_or(|total| batches <= total) =>
                    {
                        c.done_chunks = chunks;
                        c.done_batches = batches;
                    }
                    _ => s.counters = None,
                }
            }
        });
    }
    pub fn finish(&self, state: IndexPassState) {
        self.update(|s| {
            s.state = state;
            s.phase = None;
            s.counters = None;
        });
    }
}
pub struct ActivePass {
    token: IndexPassToken,
    finished: bool,
}
impl ActivePass {
    pub fn token(&self) -> IndexPassToken {
        self.token.clone()
    }
    pub fn finish(&mut self, state: IndexPassState) {
        self.token.finish(state);
        self.token.release();
        self.finished = true;
    }
}
impl Drop for ActivePass {
    fn drop(&mut self) {
        if !self.finished {
            self.token.finish(IndexPassState::Failed);
            self.token.release();
        }
    }
}

#[cfg(test)]
mod indexing_pass_lifecycle {
    use super::*;
    #[test]
    fn a_pause_restores_only_its_running_attempt() {
        let progress = IndexProgress::new();
        let mut first = progress.begin_pass();
        let paused = progress.pause_pass();
        assert!(!progress.is_active());
        drop(paused);
        assert!(progress.is_active());
        let stale_pause = progress.pause_pass();
        first.finish(IndexPassState::Ready);
        let mut next = progress.begin_pass();
        let next_pause = progress.pause_pass();
        drop(stale_pause);
        assert!(!progress.is_active());
        next.finish(IndexPassState::Ready);
        drop(next_pause);
        assert!(!progress.is_active());
    }
    #[test]
    fn stale_callback_and_drop_cannot_finish_new_pass() {
        let progress = IndexProgress::new();
        let old = progress.begin_pass();
        let stale = old.token();
        let mut new = progress.begin_pass();
        let current = new.token();
        current.set_totals(2, 4, 2);
        current.advance(2, 1);
        stale.finish(IndexPassState::Cancelled);
        drop(old);
        let sample = progress.snapshot().unwrap();
        assert_eq!(sample.state, IndexPassState::Running);
        assert_eq!(sample.counters.unwrap().done_chunks, 2);
        current.phase(IndexPhase::Persisting);
        assert!(progress.snapshot().unwrap().counters.is_none());
        new.finish(IndexPassState::Ready);
        assert!(!progress.is_active());
        let terminal = progress.snapshot().unwrap();
        assert_eq!(terminal.state, IndexPassState::Ready);
        assert_eq!(terminal.pass_id, sample.pass_id);
    }
    #[test]
    fn contention_overflow_and_reset_are_fail_closed() {
        let progress = IndexProgress::new();
        let pass = progress.begin_pass();
        let token = pass.token();
        let lock = progress.record.lock().unwrap();
        assert!(progress.snapshot().is_none());
        drop(lock);
        progress.reset();
        token.finish(IndexPassState::Ready);
        drop(pass);
        assert_eq!(progress.snapshot().unwrap().state, IndexPassState::Waiting);
        let sequence = AtomicU64::new(u64::MAX);
        let overflow = progress.begin_pass_with_counter(&sequence);
        let snapshot = progress.snapshot().unwrap();
        assert_eq!(snapshot.state, IndexPassState::Unknown);
        assert!(snapshot.pass_id.is_none());
        assert!(snapshot.counters.is_none());
        assert!(!progress.is_active());
        drop(overflow);
        assert_eq!(progress.snapshot().unwrap().state, IndexPassState::Unknown);
    }
    #[test]
    fn terminal_failure_survives_cleanup() {
        let progress = IndexProgress::new();
        let pass = progress.begin_pass();
        pass.token().finish(IndexPassState::Superseded);
        assert!(progress.is_active());
        drop(pass);
        assert!(!progress.is_active());
        assert_eq!(progress.snapshot().unwrap().state, IndexPassState::Superseded);
        let _pass = progress.begin_pass();
        assert_eq!(progress.snapshot().unwrap().state, IndexPassState::Running);
    }
    /// Without the extension, a byte-bounded split that outgrows the count-only estimate
    /// would make `advance` drop the counters as inconsistent.
    #[test]
    fn a_plan_made_per_owner_extends_the_batch_total() {
        let progress = IndexProgress::new();
        let mut pass = progress.begin_pass();
        let token = pass.token();
        let counters = || progress.snapshot().unwrap().counters.unwrap();
        token.set_totals(2, 5, 0);
        token.add_batches(2);
        token.advance(3, 2);
        token.add_batches(1);
        token.advance(2, 1);
        assert_eq!(counters().total_batches, Some(3));
        assert_eq!((counters().done_chunks, counters().done_batches), (5, 3));
        token.advance(0, 1);
        assert!(progress.snapshot().unwrap().counters.is_none(), "overrun drops counters");
        token.set_totals(1, 1, usize::MAX);
        token.add_batches(1);
        assert!(progress.snapshot().unwrap().counters.is_none(), "overflow drops counters");
        pass.finish(IndexPassState::Ready);
        token.add_batches(1);
        assert!(progress.snapshot().unwrap().counters.is_none());
    }
}
