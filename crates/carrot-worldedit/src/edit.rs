//! The edit engine: turns a list of desired states into applied world
//! changes with on-disk history, running chunk loads on the async
//! runtime so ticks are never blocked.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use pumpkin_core::entity::player::Player;
use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_util::text::TextComponent;
use pumpkin_world::world::BlockFlags;
use rayon::prelude::*;

use crate::history::Change;
use crate::mask::Mask;
use crate::session::{PendingEdit, WeState};

/// Flags for bulk edits: notify clients and skip drops/redstone shape
/// updates, like the vanilla `/fill` strict mode.
const APPLY_FLAGS: BlockFlags = BlockFlags::from_bits_truncate(
    BlockFlags::NOTIFY_LISTENERS.bits()
        | BlockFlags::UPDATE_KNOWN_SHAPE.bits()
        | BlockFlags::SKIP_DROPS.bits(),
);

/// How many blocks to place between block-update flushes and yields.
const FLUSH_INTERVAL: usize = 4096;

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

/// Executes an edit request end to end.
///
/// Loads chunks, applies the global mask, asks for `//confirm` when
/// huge, applies, records history and sends feedback. Runs inside a
/// tokio task, never on the tick thread.
pub async fn execute(state: &Arc<WeState>, player: &Arc<Player>, request: EditRequest) {
    let uuid = player.gameprofile.id;
    let world = player.world();
    let gmask: Option<Mask> = {
        let session = state.session(uuid);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.gmask.clone()
    };

    ensure_chunks(
        &world,
        &request.desired,
        |item: &(BlockPos, BlockStateId)| &item.0,
    )
    .await;

    let mut changes: Vec<Change> = Vec::with_capacity(request.desired.len());
    for (pos, after) in &request.desired {
        if !world.is_in_build_limit(*pos) {
            continue;
        }
        let before = world.get_block_state_id(pos);
        if let Some(mask) = &gmask
            && !mask.test_at(&world, pos)
        {
            continue;
        }
        if let Some(mask) = &request.mask
            && !mask.test_at(&world, pos)
        {
            continue;
        }
        if before == *after {
            continue;
        }
        changes.push((*pos, before, *after));
    }

    let confirm_above = state.with_config(|config| config.confirm_above);
    if u64::try_from(changes.len()).unwrap_or(u64::MAX) > confirm_above {
        let total = changes.len();
        let session = state.session(uuid);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.pending = Some(PendingEdit {
            world: Arc::clone(&world),
            description: request.description,
            changes,
            block_entities: request.block_entities,
        });
        drop(guard);
        player.send_system_message(&TextComponent::text(format!(
            "Edição de {total} blocos. Use //confirm para aplicar."
        )));
        return;
    }

    apply_and_record(
        state,
        player,
        &world,
        changes,
        request.block_entities,
        &request.description,
    )
    .await;
}

/// Applies a pending `//confirm` edit (already computed, no re-read).
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
    apply_and_record(
        state,
        player,
        &pending.world,
        pending.changes,
        pending.block_entities,
        &pending.description,
    )
    .await;
}

/// Writes the changes to the world and records them in the player's
/// on-disk history.
// The concrete hasher never varies in this crate-internal API.
#[allow(clippy::implicit_hasher)]
pub async fn apply_and_record(
    state: &Arc<WeState>,
    player: &Arc<Player>,
    world: &Arc<World>,
    changes: Vec<Change>,
    block_entities: HashMap<BlockPos, NbtCompound>,
    description: &str,
) {
    let total = changes.len();
    apply_changes(world, &changes, &block_entities).await;

    let session = state.session(player.gameprofile.id);
    {
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Err(err) = guard.history.record(&changes) {
            tracing::warn!("carrot worldedit: falha ao salvar history ({err})");
        }
    }
    player.send_system_message(&TextComponent::text(format!(
        "{description}: {total} bloco(s) alterado(s)."
    )));
}

/// Re-applies stored history entries (undo uses `before`, redo `after`).
pub async fn replay_entries(world: &Arc<World>, entries: &[Change], use_before: bool) -> usize {
    let targets: Vec<(BlockPos, BlockStateId)> = entries
        .iter()
        .map(|(pos, before, after)| (*pos, if use_before { *before } else { *after }))
        .collect();
    apply_state_list(world, &targets).await;
    targets.len()
}

/// Applies `(position, state)` pairs verbatim (no history, no masks).
pub async fn apply_state_list(world: &Arc<World>, targets: &[(BlockPos, BlockStateId)]) {
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

async fn apply_changes(
    world: &Arc<World>,
    changes: &[Change],
    block_entities: &HashMap<BlockPos, NbtCompound>,
) {
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
    let mut chunks: HashSet<Vector2<i32>> = HashSet::new();
    for item in items {
        let pos = key(item);
        chunks.insert(Vector2::new(pos.0.x >> 4, pos.0.z >> 4));
    }

    let mut fetched: usize = 0;
    for chunk in chunks {
        if !world.level.is_chunk_loaded(&chunk) {
            world.level.get_or_fetch_chunk(chunk, |_| ()).await;
            fetched += 1;
            if fetched.is_multiple_of(64) {
                tokio::task::yield_now().await;
            }
        }
    }
}
