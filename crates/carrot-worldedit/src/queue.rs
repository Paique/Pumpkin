//! FAWE-style edit queue: per-player job lanes drained round-robin by a
//! single worker, so huge edits run in bounded slices of memory instead
//! of materializing whole regions.
//!
//! Every edit becomes a two-phase job. The diff phase streams desired
//! states into on-disk history pages (chunk leases held per slice, not
//! per region); the apply phase replays those pages one at a time, each
//! under its own lease. RAM stays O(slice) no matter the edit size, and
//! the history file exists before the first block changes — a crash
//! mid-apply leaves a working `//undo`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use pumpkin_core::entity::player::Player;
use pumpkin_core::world::World;
use pumpkin_core::world::bossbar::Bossbar;
use pumpkin_data::BlockStateId;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector2::Vector2;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;
use tokio::sync::{Notify, oneshot};
use uuid::Uuid;

use crate::clipboard::Clipboard;
use crate::edit::{EditRequest, apply_state_list, ensure_chunk_list};
use crate::history::{Change, HistoryReader, HistoryWriter};
use crate::mask::Mask;
use crate::pattern::Pattern;
use crate::region::Cuboid;
use crate::session::{PendingApply, WeState};

/// How the worker sleeps when the queue is empty before re-checking
/// (the enqueue path also wakes it).
const IDLE_POLL: Duration = Duration::from_millis(250);

/// The result of a finished job, sent to whoever awaits it.
#[derive(Debug)]
pub enum JobOutcome {
    /// The edit applied (or replayed) fully; `changed` blocks.
    Done { changed: u64 },
    /// The diff finished but the edit waits for `//confirm`.
    WaitingConfirm { seq: u64, total: u64 },
    /// Cancelled by `//cancel`; `applied` blocks were already written
    /// (and committed to history, so `//undo` covers them).
    Cancelled { applied: u64 },
    /// The job failed.
    Failed(String),
}

/// What one slice of work left the job with.
enum SliceResult {
    /// The job continues; requeue it for the next turn.
    Running,
    /// The job is over (any reporting happens in `report_finish`).
    Finished(JobOutcome),
}

/// One job handed to the worker, with its shared cancel flag.
pub(crate) struct Turn<T> {
    pub(crate) uuid: Uuid,
    pub(crate) item: T,
    pub(crate) flag: Arc<AtomicBool>,
}

/// The per-player FIFO lanes plus the round-robin turn order.
///
/// One worker owns the executing job between slices; a lane holds at
/// most one running job (tracked by its cancel flag) and any number of
/// queued ones. Players rotate, so concurrent editors make progress
/// together instead of one huge edit starving the rest.
pub(crate) struct TurnQueue<T> {
    inner: Mutex<TurnQueueInner<T>>,
    notify: Notify,
}

#[derive(Default)]
struct TurnQueueInner<T> {
    lanes: HashMap<Uuid, Lane<T>>,
    order: VecDeque<Uuid>,
}

struct Lane<T> {
    queued: VecDeque<(T, Arc<AtomicBool>)>,
    running: Option<Arc<AtomicBool>>,
}

impl<T> TurnQueue<T> {
    /// An empty queue.
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(TurnQueueInner {
                lanes: HashMap::new(),
                order: VecDeque::new(),
            }),
            notify: Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TurnQueueInner<T>> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Enqueues `item` for `uuid`, returning its 1-based position in
    /// the lane (1 when it can start immediately).
    pub(crate) fn push(&self, uuid: Uuid, item: T) -> usize {
        let position;
        {
            let mut inner = self.lock();
            let TurnQueueInner { lanes, order } = &mut *inner;
            let lane = match lanes.entry(uuid) {
                std::collections::hash_map::Entry::Occupied(entry) => entry.into_mut(),
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    order.push_back(uuid);
                    vacant.insert(Lane {
                        queued: VecDeque::new(),
                        running: None,
                    })
                }
            };
            position = lane.queued.len() + usize::from(lane.running.is_some()) + 1;
            lane.queued
                .push_back((item, Arc::new(AtomicBool::new(false))));
        }
        self.notify.notify_one();
        position
    }

    /// Takes the next job (round-robin across players), if any.
    pub(crate) fn take_turn(&self) -> Option<Turn<T>> {
        let mut inner = self.lock();
        let max = inner.order.len();
        for _ in 0..max {
            let Some(&uuid) = inner.order.front() else {
                break;
            };
            // Rotate the player to the back before deciding, so the
            // next `take_turn` moves on to someone else.
            inner.order.push_back(uuid);
            inner.order.pop_front();

            let taken = inner.lanes.get_mut(&uuid).and_then(|lane| {
                let (item, flag) = lane.queued.pop_front()?;
                lane.running = Some(Arc::clone(&flag));
                Some(Turn { uuid, item, flag })
            });
            if let Some(turn) = taken {
                return Some(turn);
            }
            // An idle lane (nothing queued, nothing running) is dead;
            // one with a running job just waits for the worker.
            let idle = inner
                .lanes
                .get(&uuid)
                .is_some_and(|lane| lane.running.is_none());
            if idle {
                inner.lanes.remove(&uuid);
                inner.order.retain(|&pending| pending != uuid);
            }
        }
        None
    }

    /// Returns an unfinished job to the front of its lane.
    pub(crate) fn requeue(&self, turn: Turn<T>) {
        let uuid = turn.uuid;
        let mut inner = self.lock();
        if let Some(lane) = inner.lanes.get_mut(&uuid) {
            lane.running = None;
            lane.queued.push_front((turn.item, turn.flag));
        }
    }

    /// Marks a player's lane as finished (no running job anymore),
    /// removing the lane entirely when nothing is queued.
    pub(crate) fn finish(&self, uuid: &Uuid) {
        let mut inner = self.lock();
        if let Some(lane) = inner.lanes.get_mut(uuid) {
            lane.running = None;
            if lane.queued.is_empty() {
                inner.lanes.remove(uuid);
                inner.order.retain(|&pending| pending != *uuid);
            }
        }
    }

    /// Cancels a player's queued jobs, returning them so their awaiters
    /// can be answered, and flags the running job for cancellation.
    pub(crate) fn cancel(&self, uuid: &Uuid) -> (Vec<T>, bool) {
        let mut inner = self.lock();
        let Some(lane) = inner.lanes.get_mut(uuid) else {
            return (Vec::new(), false);
        };
        let drained: Vec<T> = lane.queued.drain(..).map(|(item, _)| item).collect();
        let running = lane.running.as_ref().is_some_and(|flag| {
            flag.store(true, Ordering::Relaxed);
            true
        });
        (drained, running)
    }

    /// Wakes when new work arrives (or spuriously; callers re-check).
    pub(crate) async fn wait(&self) {
        self.notify.notified().await;
    }
}

/// The queue as the rest of the module uses it.
pub struct EditQueue {
    turns: TurnQueue<QueuedJob>,
}

impl Default for EditQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl EditQueue {
    /// An empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self {
            turns: TurnQueue::new(),
        }
    }

    /// Enqueues a precomputed-desired edit (brushes, pastes, generation
    /// and every legacy `execute` caller), returning the outcome signal.
    pub(crate) fn enqueue_desired(
        &self,
        player: &Arc<Player>,
        request: EditRequest,
        gmask: Option<Mask>,
    ) -> oneshot::Receiver<JobOutcome> {
        let (job, done) = QueuedJob::desired(player, request, gmask);
        self.enqueue(player, job);
        done
    }

    /// Enqueues a lazy region edit (`//set`-family), returning the
    /// outcome signal.
    pub(crate) fn enqueue_region(
        &self,
        player: &Arc<Player>,
        spec: RegionSpec,
    ) -> oneshot::Receiver<JobOutcome> {
        let (job, done) = QueuedJob::region(player, spec);
        self.enqueue(player, job);
        done
    }

    /// Enqueues a history replay (`//undo`/`//redo` or a confirmed
    /// edit's resume), returning the outcome signal.
    pub(crate) fn enqueue_replay(
        &self,
        player: &Arc<Player>,
        description: String,
        reader: HistoryReader,
        undo: bool,
        commit_seq: Option<u64>,
        total: Option<u64>,
    ) -> oneshot::Receiver<JobOutcome> {
        let (job, done) = QueuedJob::replay(player, description, reader, undo, commit_seq, total);
        self.enqueue(player, job);
        done
    }

    fn enqueue(&self, player: &Arc<Player>, job: QueuedJob) {
        let position = self.turns.push(player.gameprofile.id, job);
        if position > 1 {
            player.send_system_message(&TextComponent::text(format!(
                "Edição enfileirada (posição {position})."
            )));
        }
    }

    /// Cancels a player's queued jobs, answering their awaiters; the
    /// running job (if any) stops at the end of its current slice.
    /// Returns how many queued jobs were dropped and whether a running
    /// job was flagged.
    pub(crate) fn cancel(&self, player: &Arc<Player>) -> (usize, bool) {
        let (drained, running) = self.turns.cancel(&player.gameprofile.id);
        let count = drained.len();
        for job in drained {
            job.answer_cancelled();
        }
        (count, running)
    }

    /// Takes the next turn, if any (worker only).
    pub(crate) fn take_turn(&self) -> Option<Turn<QueuedJob>> {
        self.turns.take_turn()
    }

    /// Returns a running job to its lane (worker only).
    pub(crate) fn requeue(&self, turn: Turn<QueuedJob>) {
        self.turns.requeue(turn);
    }

    /// Marks a lane finished (worker only).
    pub(crate) fn finish(&self, uuid: &Uuid) {
        self.turns.finish(uuid);
    }

    /// Wakes when new work arrives (worker only).
    pub(crate) async fn wait(&self) {
        self.turns.wait().await;
    }
}

/// A lazy region edit (`//set`-family): everything the diff phase needs
/// to generate desired states chunk-column by chunk-column.
pub(crate) struct RegionSpec {
    /// The selected cuboid.
    pub region: Cuboid,
    /// What to place at each position.
    pub pattern: Pattern,
    /// Session clipboard for `#clipboard` patterns.
    pub clipboard: Option<Arc<Clipboard>>,
    /// Command-specific mask (`//replace` source).
    pub mask: Option<Mask>,
    /// Serialized description shown to the player (`//set stone`).
    pub description: String,
    /// Session-wide mask applied to every edit.
    pub gmask: Option<Mask>,
}

/// The executing unit.
pub(crate) struct QueuedJob {
    player: Arc<Player>,
    world: Arc<World>,
    description: String,
    kind: JobKind,
    done: Option<oneshot::Sender<JobOutcome>>,
    /// Bossbar id once the first slice sent one.
    bar: Option<Uuid>,
}

enum JobKind {
    /// Two-phase edit: diff a source into history pages, gate on
    /// `//confirm` when huge, then apply the pages.
    Edit(EditJob),
    /// Apply history pages: undo/redo or a confirmed edit resuming.
    Replay(ReplayJob),
}

struct EditJob {
    source: DiffSource,
    gmask: Option<Mask>,
    mask: Option<Mask>,
    block_entities: HashMap<BlockPos, NbtCompound>,
    stage: EditStage,
}

enum DiffSource {
    /// Region positions generated chunk-column by chunk-column.
    Region {
        region: Cuboid,
        chunks: Vec<Vector2<i32>>,
        next_chunk: usize,
        pattern: Pattern,
        clipboard: Option<Arc<Clipboard>>,
        scanned: u64,
        volume: u64,
    },
    /// Precomputed `(position, state)` list (brushes, pastes, shapes).
    Desired {
        desired: Vec<(BlockPos, BlockStateId)>,
        next: usize,
    },
}

enum EditStage {
    /// Streaming the diff into history pages.
    Diff(Option<HistoryWriter>),
    /// Replaying recorded pages; `seq` commits as history at the end.
    Apply {
        reader: HistoryReader,
        seq: u64,
        total: u64,
        applied: u64,
    },
}

struct ReplayJob {
    reader: HistoryReader,
    /// Undo applies the `before` side, redo the `after` side.
    undo: bool,
    /// Sequence to commit once this replay finishes (confirmed edits);
    /// undo/redo replays leave the history stacks alone.
    commit_seq: Option<u64>,
    applied: u64,
    total: Option<u64>,
}

impl QueuedJob {
    /// A desired-list edit job plus its outcome signal.
    pub(crate) fn desired(
        player: &Arc<Player>,
        request: EditRequest,
        gmask: Option<Mask>,
    ) -> (Self, oneshot::Receiver<JobOutcome>) {
        let (done, signal) = oneshot::channel();
        (
            Self {
                player: Arc::clone(player),
                world: player.world(),
                description: request.description,
                kind: JobKind::Edit(EditJob {
                    source: DiffSource::Desired {
                        desired: request.desired,
                        next: 0,
                    },
                    gmask,
                    mask: request.mask,
                    block_entities: request.block_entities,
                    stage: EditStage::Diff(None),
                }),
                done: Some(done),
                bar: None,
            },
            signal,
        )
    }

    /// A lazy region edit job plus its outcome signal.
    pub(crate) fn region(
        player: &Arc<Player>,
        spec: RegionSpec,
    ) -> (Self, oneshot::Receiver<JobOutcome>) {
        let (done, signal) = oneshot::channel();
        let chunks = region_chunk_columns(&spec.region);
        let volume = spec.region.volume().unwrap_or(u64::MAX);
        (
            Self {
                player: Arc::clone(player),
                world: player.world(),
                description: spec.description,
                kind: JobKind::Edit(EditJob {
                    source: DiffSource::Region {
                        region: spec.region,
                        chunks,
                        next_chunk: 0,
                        pattern: spec.pattern,
                        clipboard: spec.clipboard,
                        scanned: 0,
                        volume,
                    },
                    gmask: spec.gmask,
                    mask: spec.mask,
                    block_entities: HashMap::new(),
                    stage: EditStage::Diff(None),
                }),
                done: Some(done),
                bar: None,
            },
            signal,
        )
    }

    /// A history replay job plus its outcome signal.
    pub(crate) fn replay(
        player: &Arc<Player>,
        description: String,
        reader: HistoryReader,
        undo: bool,
        commit_seq: Option<u64>,
        total: Option<u64>,
    ) -> (Self, oneshot::Receiver<JobOutcome>) {
        let (done, signal) = oneshot::channel();
        (
            Self {
                player: Arc::clone(player),
                world: player.world(),
                description,
                kind: JobKind::Replay(ReplayJob {
                    reader,
                    undo,
                    commit_seq,
                    applied: 0,
                    total,
                }),
                done: Some(done),
                bar: None,
            },
            signal,
        )
    }

    /// Answers a queued-but-never-run job's awaiter (from `//cancel`).
    pub(crate) fn answer_cancelled(self) {
        if let Some(done) = self.done {
            let _ = done.send(JobOutcome::Cancelled { applied: 0 });
        }
    }

    /// Sends or updates the progress bossbar (best effort).
    pub(crate) fn update_progress(&mut self) {
        let fraction = match &self.kind {
            JobKind::Edit(edit) => {
                match &edit.source {
                    DiffSource::Region {
                        scanned, volume, ..
                    } => *scanned as f32 / (*volume).max(1) as f32,
                    DiffSource::Desired { desired, next } => {
                        *next as f32 / desired.len().max(1) as f32
                    }
                }
                .min(1.0)
                    * match &edit.stage {
                        // The diff is half the work; apply is the other half.
                        EditStage::Diff(_) => 0.5,
                        EditStage::Apply { applied, total, .. } => {
                            0.5 + 0.5 * (*applied as f32 / (*total).max(1) as f32)
                        }
                    }
            }
            JobKind::Replay(replay) => replay
                .total
                .map_or(1.0, |total| replay.applied as f32 / total.max(1) as f32)
                .clamp(0.0, 1.0),
        };
        if self.bar.is_none() {
            let bar = Bossbar::new(TextComponent::text(format!(
                "WorldEdit: {}",
                self.description
            )));
            let id = bar.uuid;
            self.player.send_bossbar(&bar);
            self.bar = Some(id);
        }
        if let Some(id) = self.bar {
            self.player
                .update_bossbar_health(&id, fraction.clamp(0.0, 1.0));
        }
    }

    /// Runs one bounded slice of the job. `flag` is the shared cancel
    /// flag from the turn.
    async fn run_slice(
        &mut self,
        state: &Arc<WeState>,
        flag: &Arc<AtomicBool>,
        budget: usize,
    ) -> SliceResult {
        if flag.load(Ordering::Relaxed) {
            return self.cancelled(state);
        }
        let uuid = self.player.gameprofile.id;
        let result = match &mut self.kind {
            JobKind::Edit(edit) => run_edit_slice(&self.world, state, uuid, edit, budget).await,
            JobKind::Replay(replay) => run_replay_slice(&self.world, state, uuid, replay).await,
        };
        match result {
            Ok(SliceResult::Running) => SliceResult::Running,
            Ok(SliceResult::Finished(outcome)) => SliceResult::Finished(outcome),
            Err(message) => {
                // Settle the pages so a mid-edit failure (full disk,
                // corrupt page) leaves no tmp behind and whatever
                // applied stays undoable.
                self.settle(state);
                SliceResult::Finished(JobOutcome::Failed(message))
            }
        }
    }

    /// Stops a cancelled job, committing whatever already applied so
    /// `//undo` covers it (or discarding pages when nothing applied).
    fn cancelled(&self, state: &Arc<WeState>) -> SliceResult {
        let applied = self.settle(state);
        SliceResult::Finished(JobOutcome::Cancelled { applied })
    }

    /// Commits or discards whatever the job already produced, returning
    /// how many blocks were applied.
    fn settle(&self, state: &Arc<WeState>) -> u64 {
        let uuid = self.player.gameprofile.id;
        match &self.kind {
            JobKind::Edit(edit) => match &edit.stage {
                EditStage::Diff(Some(writer)) => {
                    abort_pending(state, uuid, writer.seq());
                    0
                }
                EditStage::Diff(None) => 0,
                EditStage::Apply { seq, applied, .. } => {
                    let seq = *seq;
                    let applied = *applied;
                    if applied > 0 {
                        let _ = commit_pending(state, uuid, seq);
                    } else {
                        abort_pending(state, uuid, seq);
                    }
                    applied
                }
            },
            JobKind::Replay(replay) => {
                let applied = replay.applied;
                match replay.commit_seq {
                    // A confirmed edit resuming: commit what applied so
                    // it stays undoable, discard the pages otherwise.
                    Some(seq) if applied > 0 => {
                        let _ = commit_pending(state, uuid, seq);
                    }
                    Some(seq) => abort_pending(state, uuid, seq),
                    // Plain undo/redo: the stacks already moved.
                    None => {}
                }
                applied
            }
        }
    }

    /// Final messaging, bossbar teardown, `//confirm` parking and
    /// awaiter signalling.
    fn report_finish(&mut self, state: &Arc<WeState>, outcome: &JobOutcome) {
        if let Some(id) = self.bar.take() {
            self.player.remove_bossbar(id);
        }
        let message = match outcome {
            JobOutcome::Done { changed } => {
                format!("{}: {changed} bloco(s) alterado(s).", self.description)
            }
            JobOutcome::WaitingConfirm { total, .. } => {
                format!("Edição de {total} blocos calculada. Use //confirm para aplicar.")
            }
            JobOutcome::Cancelled { applied } => format!(
                "Edição cancelada ({applied} bloco(s) já aplicados; //undo disponível quando houver)."
            ),
            JobOutcome::Failed(message) => format!("Falha na edição: {message}"),
        };
        self.player
            .send_system_message(&TextComponent::text(message));
        if let JobOutcome::WaitingConfirm { seq, total } = outcome {
            let session = state.session(self.player.gameprofile.id);
            let mut guard = session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            // Replacing a never-confirmed edit discards its sealed
            // pages so the tmp file cannot leak until a restart.
            if let Some(previous) = guard.pending.take() {
                guard.history.abort_seq(previous.seq);
            }
            guard.pending = Some(PendingApply {
                world: Arc::clone(&self.world),
                seq: *seq,
                total: *total,
                description: self.description.clone(),
            });
        }
        if let Some(done) = self.done.take() {
            let _ = done.send(clone_outcome(outcome));
        }
    }
}

fn clone_outcome(outcome: &JobOutcome) -> JobOutcome {
    match outcome {
        JobOutcome::Done { changed } => JobOutcome::Done { changed: *changed },
        JobOutcome::WaitingConfirm { seq, total } => JobOutcome::WaitingConfirm {
            seq: *seq,
            total: *total,
        },
        JobOutcome::Cancelled { applied } => JobOutcome::Cancelled { applied: *applied },
        JobOutcome::Failed(message) => JobOutcome::Failed(message.clone()),
    }
}

/// One slice of a two-phase edit.
async fn run_edit_slice(
    world: &Arc<World>,
    state: &Arc<WeState>,
    uuid: Uuid,
    edit: &mut EditJob,
    budget: usize,
) -> Result<SliceResult, String> {
    let block_entities = &edit.block_entities;
    match &mut edit.stage {
        EditStage::Diff(writer_slot) => {
            if writer_slot.is_none() {
                let writer = begin_pending(state, uuid).map_err(|err| format!("history: {err}"))?;
                *writer_slot = Some(writer);
            }
            if !diff_slice(world, edit, budget).await? {
                return Ok(SliceResult::Running);
            }
            // Diff done: seal the pages and gate on `//confirm`.
            let EditStage::Diff(writer_slot) = &mut edit.stage else {
                return Err("diff sem writer".to_string());
            };
            let Some(writer) = writer_slot.take() else {
                return Err("diff sem writer".to_string());
            };
            let seq = writer.seq();
            let total = writer.total();
            writer.finish().map_err(|err| format!("history: {err}"))?;
            let confirm_above = state.with_config(|config| config.confirm_above);
            if total > confirm_above {
                return Ok(SliceResult::Finished(JobOutcome::WaitingConfirm {
                    seq,
                    total,
                }));
            }
            edit.stage = EditStage::Apply {
                reader: open_pending(state, uuid, seq)?,
                seq,
                total,
                applied: 0,
            };
            Ok(SliceResult::Running)
        }
        EditStage::Apply {
            reader,
            seq,
            total,
            applied,
        } => {
            let page = reader
                .next_page()
                .map_err(|err| format!("history: {err}"))?;
            let Some(page) = page else {
                let applied = *applied;
                let seq = *seq;
                commit_pending(state, uuid, seq).map_err(|err| format!("history: {err}"))?;
                return Ok(SliceResult::Finished(JobOutcome::Done { changed: applied }));
            };
            let changed = apply_page(world, block_entities, page).await?;
            *applied += changed;
            Ok(SliceResult::Running)
        }
    }
}

/// Diffs up to `budget` positions into the writer. Returns `true` when
/// the source is exhausted.
async fn diff_slice(world: &Arc<World>, edit: &mut EditJob, budget: usize) -> Result<bool, String> {
    let gmask = edit.gmask.as_ref();
    let mask = edit.mask.as_ref();
    let writer = match &mut edit.stage {
        EditStage::Diff(writer) => writer.as_mut(),
        EditStage::Apply { .. } => None,
    };
    let Some(writer) = writer else {
        return Err("diff sem writer".to_string());
    };
    match &mut edit.source {
        DiffSource::Region {
            region,
            chunks,
            next_chunk,
            pattern,
            clipboard,
            scanned,
            ..
        } => {
            // Collect the chunk columns this slice covers.
            let mut slice_chunks: Vec<Vector2<i32>> = Vec::new();
            let mut covered = 0usize;
            while *next_chunk < chunks.len() && covered < budget {
                let chunk = chunks[*next_chunk];
                slice_chunks.push(chunk);
                covered += region_chunk_positions(region, chunk);
                *next_chunk += 1;
            }
            if slice_chunks.is_empty() {
                return Ok(true);
            }
            let lease = ChunkLeaseSlice::acquire(world, &slice_chunks).await;
            let min = region.min();
            let max = region.max();
            for chunk in slice_chunks {
                let (x0, x1) = column_range(min.0.x, max.0.x, chunk.x);
                let (z0, z1) = column_range(min.0.z, max.0.z, chunk.y);
                for y in min.0.y..=max.0.y {
                    for z in z0..=z1 {
                        for x in x0..=x1 {
                            let pos = BlockPos(Vector3::new(x, y, z));
                            *scanned += 1;
                            diff_position(
                                world,
                                writer,
                                gmask,
                                mask,
                                &pos,
                                pattern,
                                clipboard.as_deref(),
                            )?;
                        }
                    }
                }
            }
            drop(lease);
            Ok(*next_chunk >= chunks.len())
        }
        DiffSource::Desired { desired, next } => {
            let end = (*next + budget).min(desired.len());
            if end == *next {
                return Ok(true);
            }
            let slice = &desired[*next..end];
            let lease = ChunkLeaseSlice::acquire_positions(world, slice).await;
            for (pos, after) in slice {
                diff_desired(world, writer, gmask, mask, pos, *after)?;
            }
            drop(lease);
            *next = end;
            Ok(*next >= desired.len())
        }
    }
}

/// Diffs one pattern-generated position.
fn diff_position(
    world: &Arc<World>,
    writer: &mut HistoryWriter,
    gmask: Option<&Mask>,
    mask: Option<&Mask>,
    pos: &BlockPos,
    pattern: &Pattern,
    clipboard: Option<&Clipboard>,
) -> Result<(), String> {
    let Some(after) = pattern.apply(pos, clipboard) else {
        return Ok(());
    };
    diff_desired(world, writer, gmask, mask, pos, after)
}

/// Diffs one already-known desired state.
fn diff_desired(
    world: &Arc<World>,
    writer: &mut HistoryWriter,
    gmask: Option<&Mask>,
    mask: Option<&Mask>,
    pos: &BlockPos,
    after: BlockStateId,
) -> Result<(), String> {
    if !world.is_in_build_limit(*pos) {
        return Ok(());
    }
    let before = world.get_block_state_id(pos);
    if !masks_pass(gmask, mask, world, pos) || before == after {
        return Ok(());
    }
    writer
        .push((*pos, before, after))
        .map_err(|err| format!("history: {err}"))
}

/// True when both masks (session `gmask` and command mask) pass `pos`.
fn masks_pass(
    gmask: Option<&Mask>,
    mask: Option<&Mask>,
    world: &Arc<World>,
    pos: &BlockPos,
) -> bool {
    if let Some(mask) = gmask
        && !mask.test_at(world, pos)
    {
        return false;
    }
    if let Some(mask) = mask
        && !mask.test_at(world, pos)
    {
        return false;
    }
    true
}

/// Applies one page of changes (plus its block entities); returns how
/// many blocks were written.
async fn apply_page(
    world: &Arc<World>,
    block_entities: &HashMap<BlockPos, NbtCompound>,
    page: Vec<Change>,
) -> Result<u64, String> {
    let targets: Vec<(BlockPos, BlockStateId)> =
        page.iter().map(|(pos, _, after)| (*pos, *after)).collect();
    apply_state_list(world, &targets).await;
    for (pos, _, _) in &page {
        if let Some(nbt) = block_entities.get(pos) {
            let mut nbt = nbt.clone();
            nbt.put_int("x", pos.0.x);
            nbt.put_int("y", pos.0.y);
            nbt.put_int("z", pos.0.z);
            world.load_block_entity_nbt(*pos, &nbt);
        }
    }
    Ok(page.len() as u64)
}

/// One slice of a history replay.
async fn run_replay_slice(
    world: &Arc<World>,
    state: &Arc<WeState>,
    uuid: Uuid,
    replay: &mut ReplayJob,
) -> Result<SliceResult, String> {
    let page = replay
        .reader
        .next_page()
        .map_err(|err| format!("history: {err}"))?;
    let Some(page) = page else {
        if let Some(seq) = replay.commit_seq {
            commit_pending(state, uuid, seq).map_err(|err| format!("history: {err}"))?;
        }
        return Ok(SliceResult::Finished(JobOutcome::Done {
            changed: replay.applied,
        }));
    };
    let targets: Vec<(BlockPos, BlockStateId)> = page
        .iter()
        .map(|(pos, before, after)| (*pos, if replay.undo { *before } else { *after }))
        .collect();
    apply_state_list(world, &targets).await;
    replay.applied += page.len() as u64;
    Ok(SliceResult::Running)
}

/// Begins a streaming record on a player's store.
fn begin_pending(state: &Arc<WeState>, uuid: Uuid) -> Result<HistoryWriter, std::io::Error> {
    let session = state.session(uuid);
    let mut guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.history.begin_record()
}

/// Opens a finished-but-uncommitted record's pages.
fn open_pending(state: &Arc<WeState>, uuid: Uuid, seq: u64) -> Result<HistoryReader, String> {
    let session = state.session(uuid);
    let guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .history
        .open_pending(seq)
        .map_err(|err| format!("history: {err}"))
}

/// Commits a finished record into the undo stack.
fn commit_pending(state: &Arc<WeState>, uuid: Uuid, seq: u64) -> Result<(), std::io::Error> {
    let session = state.session(uuid);
    let mut guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.history.commit_seq(seq)
}

/// Discards a finished-but-uncommitted record.
fn abort_pending(state: &Arc<WeState>, uuid: Uuid, seq: u64) {
    let session = state.session(uuid);
    let mut guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.history.abort_seq(seq);
}

/// A per-slice chunk lease (the region-wide variant would pin every
/// chunk of a huge edit for the whole edit).
struct ChunkLeaseSlice {
    level: Arc<pumpkin_world::level::Level>,
    chunks: Vec<Vector2<i32>>,
}

impl ChunkLeaseSlice {
    async fn acquire(world: &Arc<World>, chunks: &[Vector2<i32>]) -> Self {
        let level = Arc::clone(&world.level);
        level.retain_chunks(chunks);
        ensure_chunk_list(world, chunks).await;
        Self {
            level,
            chunks: chunks.to_vec(),
        }
    }

    async fn acquire_positions(world: &Arc<World>, positions: &[(BlockPos, BlockStateId)]) -> Self {
        let mut set: HashSet<Vector2<i32>> = HashSet::new();
        for (pos, _) in positions {
            set.insert(Vector2::new(pos.0.x >> 4, pos.0.z >> 4));
        }
        let chunks: Vec<Vector2<i32>> = set.into_iter().collect();
        Self::acquire(world, &chunks).await
    }
}

impl Drop for ChunkLeaseSlice {
    fn drop(&mut self) {
        self.level.release_chunks(&self.chunks);
    }
}

/// The chunk columns a cuboid touches, in x-then-z order.
fn region_chunk_columns(region: &Cuboid) -> Vec<Vector2<i32>> {
    let min = region.min();
    let max = region.max();
    let mut chunks = Vec::new();
    for cx in (min.0.x >> 4)..=(max.0.x >> 4) {
        for cz in (min.0.z >> 4)..=(max.0.z >> 4) {
            chunks.push(Vector2::new(cx, cz));
        }
    }
    chunks
}

/// How many cuboid positions lie in one chunk column.
fn region_chunk_positions(region: &Cuboid, chunk: Vector2<i32>) -> usize {
    let min = region.min();
    let max = region.max();
    let (x0, x1) = column_range(min.0.x, max.0.x, chunk.x);
    let (z0, z1) = column_range(min.0.z, max.0.z, chunk.y);
    let dx = (x1 - x0 + 1).max(0) as u64;
    let dy = (max.0.y - min.0.y + 1).max(0) as u64;
    let dz = (z1 - z0 + 1).max(0) as u64;
    (dx * dy * dz) as usize
}

/// The inclusive world-axis range inside one chunk-local index.
fn column_range(min: i32, max: i32, chunk: i32) -> (i32, i32) {
    (min.max(chunk * 16), max.min(chunk * 16 + 15))
}

/// The queue worker: drains lanes round-robin, one slice per turn.
pub(crate) async fn run_worker(state: Arc<WeState>) {
    loop {
        let Some(mut turn) = state.queue().take_turn() else {
            tokio::select! {
                () = state.queue().wait() => {},
                () = tokio::time::sleep(IDLE_POLL) => {},
            }
            continue;
        };
        let budget = state.with_config(|config| config.queue_slice_blocks);
        let outcome = turn.item.run_slice(&state, &turn.flag, budget).await;
        match outcome {
            SliceResult::Running => {
                turn.item.update_progress();
                state.queue().requeue(turn);
            }
            SliceResult::Finished(result) => {
                turn.item.report_finish(&state, &result);
                state.queue().finish(&turn.uuid);
            }
        }
        tokio::task::yield_now().await;
    }
}

/// Runs a region edit to completion with the queue's own slice engine,
/// minus the player UX (bossbar, messages, confirmation gating — set
/// `confirm_above` above the edit size to skip `//confirm`).
///
/// Integration probes use this to measure the pipeline's memory
/// behavior; production goes through `EditQueue::enqueue_region` and
/// the worker, which call the very same slice functions.
///
/// # Errors
///
/// Returns the failure message when any slice fails.
pub async fn run_region_edit_probe(
    state: &Arc<WeState>,
    uuid: Uuid,
    world: &Arc<World>,
    region: Cuboid,
    pattern: Pattern,
) -> Result<u64, String> {
    let chunks = region_chunk_columns(&region);
    let volume = region.volume().unwrap_or(u64::MAX);
    let mut edit = EditJob {
        source: DiffSource::Region {
            region,
            chunks,
            next_chunk: 0,
            pattern,
            clipboard: None,
            scanned: 0,
            volume,
        },
        gmask: None,
        mask: None,
        block_entities: HashMap::new(),
        stage: EditStage::Diff(None),
    };
    let budget = state.with_config(|config| config.queue_slice_blocks);
    loop {
        match run_edit_slice(world, state, uuid, &mut edit, budget).await {
            Ok(SliceResult::Running) => {}
            Ok(SliceResult::Finished(JobOutcome::Done { changed })) => return Ok(changed),
            Ok(SliceResult::Finished(other)) => return Err(format!("{other:?}")),
            Err(message) => return Err(message),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue() -> TurnQueue<usize> {
        TurnQueue::new()
    }

    fn uuid(seed: u8) -> Uuid {
        Uuid::from_u64_pair(u128::from(seed) as u64, 0)
    }

    #[tokio::test]
    async fn round_robins_between_players() {
        let q = queue();
        let a = uuid(1);
        let b = uuid(2);
        q.push(a, 10);
        q.push(a, 11);
        q.push(b, 20);

        // Running jobs rotate between players; a requeue returns the
        // same job (its next slice), and a finished job frees the lane
        // for the next queued one.
        let first = q.take_turn().expect("turn 1");
        assert_eq!((first.uuid, first.item), (a, 10));
        q.requeue(first);

        let second = q.take_turn().expect("turn 2");
        assert_eq!((second.uuid, second.item), (b, 20));
        q.requeue(second);

        // Player a's first edit continues (second slice), not its
        // second edit.
        let third = q.take_turn().expect("turn 3");
        assert_eq!((third.uuid, third.item), (a, 10));
        q.finish(&third.uuid);

        let fourth = q.take_turn().expect("turn 4");
        assert_eq!(fourth.uuid, b);
        q.finish(&fourth.uuid);

        let fifth = q.take_turn().expect("turn 5");
        assert_eq!((fifth.uuid, fifth.item), (a, 11));
        q.finish(&fifth.uuid);

        assert!(q.take_turn().is_none());
    }

    #[tokio::test]
    async fn cancel_drains_queued_and_flags_running() {
        let q = queue();
        let a = uuid(1);
        q.push(a, 1);
        q.push(a, 2);
        q.push(a, 3);

        let running = q.take_turn().expect("turn");
        let (drained, has_running) = q.cancel(&a);
        assert_eq!(drained, vec![2, 3]);
        assert!(has_running);
        assert!(running.flag.load(Ordering::Relaxed));

        // After finishing the running job the lane disappears.
        q.finish(&a);
        assert!(q.take_turn().is_none());
    }

    #[tokio::test]
    async fn empty_queue_has_no_turns() {
        let q = queue();
        assert!(q.take_turn().is_none());
        q.push(uuid(9), 1);
        assert!(q.take_turn().is_some());
        q.finish(&uuid(9));
        assert!(q.take_turn().is_none());
    }

    #[test]
    fn chunk_helpers_agree_with_region_volume() {
        let region = Cuboid::new(
            BlockPos(Vector3::new(-5, 0, 33)),
            BlockPos(Vector3::new(20, 10, 47)),
        );
        let chunks = region_chunk_columns(&region);
        let counted: u64 = chunks
            .iter()
            .map(|chunk| region_chunk_positions(&region, *chunk) as u64)
            .sum();
        assert_eq!(counted, region.volume().expect("volume"));
    }
}
