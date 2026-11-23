//! The edit engine: primitives that apply desired block states.
//!
//! Chunk loads run on the async runtime so ticks are never blocked;
//! the entry points hand edits to the incremental queue (`queue.rs`),
//! which slices huge edits into bounded pieces.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pumpkin_core::entity::player::Player;
use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_util::text::TextComponent;
use pumpkin_world::level::Level;
use pumpkin_world::world::BlockFlags;
use rayon::prelude::*;

use crate::history::Change;
use crate::mask::Mask;
use crate::session::WeState;

/// Flags for bulk edits: notify clients and skip drops/redstone shape
/// updates, like the vanilla `/fill` strict mode.
const APPLY_FLAGS: BlockFlags = BlockFlags::from_bits_truncate(
    BlockFlags::NOTIFY_LISTENERS.bits()
        | BlockFlags::UPDATE_KNOWN_SHAPE.bits()
        | BlockFlags::SKIP_DROPS.bits(),
);

/// How many blocks to place between block-update flushes and yields.
const FLUSH_INTERVAL: usize = 4096;

/// How many block reads to take between yields while diffing a region.
const READ_INTERVAL: usize = 65_536;

/// Holds every chunk touching an edit loaded for the edit's duration.
///
/// Without the lease the periodic chunk unload (`clean_memory`) can drop
/// chunks nobody watches mid-edit; `set_block_state` then silently
/// writes nothing and `get_block_state_id` reads air, losing most of a
/// huge edit. Force tickets keep the chunks alive exactly like FAWE's
/// edit chunk retention, and `Drop` releases them even on panic.
///
/// The edit queue leases per slice instead (see `queue.rs`); this
/// region-wide lease remains for one-shot applies (page replays) and
/// tests.
pub struct ChunkLease {
    level: Arc<Level>,
    chunks: Vec<Vector2<i32>>,
}

impl ChunkLease {
    /// Acquires the lease for every chunk touching `items`.
    pub fn acquire<T>(level: &Arc<Level>, items: &[T], key: impl Fn(&T) -> &BlockPos) -> Self {
        let chunks = chunk_keys(items, key);
        level.retain_chunks(&chunks);
        Self {
            level: Arc::clone(level),
            chunks,
        }
    }
}

impl Drop for ChunkLease {
    fn drop(&mut self) {
        self.level.release_chunks(&self.chunks);
    }
}

/// The distinct chunk coordinates touching `items`.
fn chunk_keys<T>(items: &[T], key: impl Fn(&T) -> &BlockPos) -> Vec<Vector2<i32>> {
    let mut chunks: HashSet<Vector2<i32>> = HashSet::new();
    for item in items {
        let pos = key(item);
        chunks.insert(Vector2::new(pos.0.x >> 4, pos.0.z >> 4));
    }
    chunks.into_iter().collect()
}

/// A computed edit request: the desired state for each position (patterns
/// already applied) plus per-command masks and block entity payloads.
pub struct EditRequest {
    /// `(position, desired state)` pairs.
    pub desired: Vec<(BlockPos, BlockStateId)>,
    /// Command-specific mask (`//replace` source) applied together with
    /// the session's `//gmask`.
    pub mask: Option<Mask>,
    /// Block entity NBT keyed by absolute position (pastes).
    pub block_entities: HashMap<BlockPos, NbtCompound>,
    /// Description used in feedback and confirmation prompts.
    pub description: String,
}

impl EditRequest {
    /// A request without masks or block entities.
    #[must_use]
    pub fn new(desired: Vec<(BlockPos, BlockStateId)>, description: String) -> Self {
        Self {
            desired,
            mask: None,
            block_entities: HashMap::new(),
            description,
        }
    }
}

/// Executes an edit request end to end through the edit queue.
///
/// The queue diffs the request into history pages slice by slice, asks
/// for `//confirm` when huge, applies page by page and reports progress
/// on a bossbar. Awaiting keeps the legacy `execute(...).await`
/// contract (brushes rely on it).
pub async fn execute(state: &Arc<WeState>, player: &Arc<Player>, request: EditRequest) {
    let gmask: Option<Mask> = {
        let session = state.session(player.gameprofile.id);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.gmask.clone()
    };
    let done = state.queue().enqueue_desired(player, request, gmask);
    let _ = done.await;
}

/// Applies a pending `//confirm` edit (pages already on disk; no
/// re-read of the world).
pub async fn confirm_pending(state: &Arc<WeState>, player: &Arc<Player>) {
    let uuid = player.gameprofile.id;
    let pending = {
        let session = state.session(uuid);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.pending.take()
    };
    let Some(pending) = pending else {
        player.send_system_message(&TextComponent::text("Nada para confirmar.".to_string()));
        return;
    };
    if !Arc::ptr_eq(&pending.world, &player.world()) {
        player.send_system_message(&TextComponent::text(
            "Você mudou de mundo desde a edição; ela foi descartada.".to_string(),
        ));
        let session = state.session(uuid);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.history.abort_seq(pending.seq);
        return;
    }
    let reader = {
        let session = state.session(uuid);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.history.open_pending(pending.seq)
    };
    let reader = match reader {
        Ok(reader) => reader,
        Err(err) => {
            player.send_system_message(&TextComponent::text(format!(
                "Falha ao retomar a edição ({err})."
            )));
            return;
        }
    };
    let description = format!("//confirm {}", pending.description);
    let done = state.queue().enqueue_replay(
        player,
        description,
        reader,
        false,
        Some(pending.seq),
        Some(pending.total),
    );
    let _ = done.await;
}

/// Diffs desired states against the world.
///
/// Applies the session (`gmask`) and command masks; only positions
/// whose state actually changes are returned. Yields periodically so
/// the runtime keeps serving other tasks.
pub async fn compute_changes(
    world: &Arc<World>,
    desired: &[(BlockPos, BlockStateId)],
    gmask: Option<&Mask>,
    mask: Option<&Mask>,
) -> Vec<Change> {
    let mut changes: Vec<Change> = Vec::with_capacity(desired.len());
    for (pos, after) in desired {
        if !world.is_in_build_limit(*pos) {
            continue;
        }
        let before = world.get_block_state_id(pos);
        if let Some(mask) = gmask
            && !mask.test_at(world, pos)
        {
            continue;
        }
        if let Some(mask) = mask
            && !mask.test_at(world, pos)
        {
            continue;
        }
        if before == *after {
            continue;
        }
        changes.push((*pos, before, *after));
        if changes.len().is_multiple_of(READ_INTERVAL) {
            tokio::task::yield_now().await;
        }
    }
    changes
}

/// Applies `(position, state)` pairs verbatim (no history, no masks).
///
/// Leases and loads the chunks of the given list only — the queue calls
/// this once per history page, so a huge edit pins one page of chunks
/// at a time.
pub async fn apply_state_list(world: &Arc<World>, targets: &[(BlockPos, BlockStateId)]) {
    let _lease = ChunkLease::acquire(&world.level, targets, |item| &item.0);
    ensure_chunks(world, targets, |item: &(BlockPos, BlockStateId)| &item.0).await;
    for (index, (pos, state)) in targets.iter().enumerate() {
        if !world.is_in_build_limit(*pos) {
            continue;
        }
        world.set_block_state(pos, *state, APPLY_FLAGS);
        if index % FLUSH_INTERVAL == FLUSH_INTERVAL - 1 {
            world.flush_block_updates();
            tokio::task::yield_now().await;
        }
    }
    world.flush_block_updates();
}

/// Reads the current state of every position (parallel over the
/// simulation pool for large regions).
#[must_use]
pub fn read_states(world: &World, positions: &[BlockPos]) -> Vec<BlockStateId> {
    if positions.len() > 65_536 {
        return positions
            .par_iter()
            .map(|pos| world.get_block_state_id(pos))
            .collect();
    }
    positions
        .iter()
        .map(|pos| world.get_block_state_id(pos))
        .collect()
}

/// Writes the changes to the world (block states plus block entities),
/// flushing block updates and yielding periodically.
// The concrete hasher never varies in this crate-internal API.
#[allow(clippy::implicit_hasher)]
pub async fn apply_changes(
    world: &Arc<World>,
    changes: &[Change],
    block_entities: &HashMap<BlockPos, NbtCompound>,
) {
    let _lease = ChunkLease::acquire(&world.level, changes, |item: &Change| &item.0);
    ensure_chunks(world, changes, |item: &Change| &item.0).await;
    for (index, (pos, _, after)) in changes.iter().enumerate() {
        world.set_block_state(pos, *after, APPLY_FLAGS);
        if let Some(nbt) = block_entities.get(pos) {
            let mut nbt = nbt.clone();
            nbt.put_int("x", pos.0.x);
            nbt.put_int("y", pos.0.y);
            nbt.put_int("z", pos.0.z);
            world.load_block_entity_nbt(*pos, &nbt);
        }
        if index % FLUSH_INTERVAL == FLUSH_INTERVAL - 1 {
            world.flush_block_updates();
            tokio::task::yield_now().await;
        }
    }
    world.flush_block_updates();
}

/// Loads every chunk touching the given positions.
///
/// Skips chunks already in memory and yields periodically so the runtime
/// keeps serving other tasks. Public so read-based brushes (terrain
/// module) can load before sampling.
pub async fn ensure_chunks<T>(world: &Arc<World>, items: &[T], key: impl Fn(&T) -> &BlockPos) {
    let chunks = chunk_keys(items, key);
    ensure_chunk_list(world, &chunks).await;
}

/// Loads the given chunk columns (skipping resident ones), yielding
/// periodically so the runtime keeps serving other tasks.
pub async fn ensure_chunk_list(world: &Arc<World>, chunks: &[Vector2<i32>]) {
    let mut fetched: usize = 0;
    for chunk in chunks {
        if !world.level.is_chunk_loaded(chunk) {
            world.level.get_or_fetch_chunk(*chunk, |_| ()).await;
            fetched += 1;
            if fetched.is_multiple_of(64) {
                tokio::task::yield_now().await;
            }
        }
    }
}
