//! World application of Axiom edits: unitary `set_block` batches, section
//! buffers and forced ticks (phase AX-1).
//!
//! This is the `carrot-worldedit` edit engine pattern, re-implemented per
//! the plan (reusing the pattern, not the crate): chunk leases pin every
//! touched chunk so the periodic unload cannot eat an in-flight edit,
//! loads happen on the async runtime, writes use the no-physics bulk
//! flags, and block-update flushes + yields happen periodically so ticks
//! are never starved.
//!
//! Sections carry an RAII pending guard: the per-player backpressure
//! counter is released even when a task bails early.

use std::collections::{HashMap, HashSet};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use pumpkin_core::world::World;
use pumpkin_data::{BlockStateId, fluid::Fluid};
use pumpkin_util::math::{position::BlockPos, vector2::Vector2};
use pumpkin_world::level::Level;
use pumpkin_world::tick::TickPriority;
use pumpkin_world::world::BlockFlags;

use crate::buffer::BlockBuffer;

/// Flags for bulk edits: notify clients and skip drops/redstone shape
/// updates, like the vanilla `/fill` strict mode (same as worldedit's
/// `APPLY_FLAGS`).
const APPLY_FLAGS: BlockFlags = BlockFlags::from_bits_truncate(
    BlockFlags::NOTIFY_LISTENERS.bits()
        | BlockFlags::UPDATE_KNOWN_SHAPE.bits()
        | BlockFlags::SKIP_DROPS.bits(),
);

/// Flags when the client asked for neighbor updates (vanilla `setBlock`
/// flag 3: block update + clients).
const NEIGHBOR_FLAGS: BlockFlags = BlockFlags::from_bits_truncate(
    BlockFlags::NOTIFY_NEIGHBORS.bits() | BlockFlags::NOTIFY_LISTENERS.bits(),
);

/// How many block writes to take between block-update flushes and yields.
const FLUSH_INTERVAL: usize = 4096;

/// Holds every chunk touching an edit loaded for the edit's duration.
///
/// Without the lease the periodic chunk unload can drop chunks nobody
/// watches mid-edit; `set_block_state` then silently writes nothing (the
/// bug already lived in carrot-worldedit before `ChunkLease`).
pub struct ChunkLease {
    level: Arc<Level>,
    chunks: Vec<Vector2<i32>>,
}

impl ChunkLease {
    /// Acquires the lease for the given chunk columns.
    fn acquire(level: &Arc<Level>, chunks: Vec<Vector2<i32>>) -> Self {
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

/// Releases the reserved pending sections when the apply finishes (RAII:
/// holds even through early returns).
pub struct PendingGuard {
    pending: Arc<AtomicUsize>,
    count: usize,
}

impl PendingGuard {
    /// Reserves `count` sections (already added to the counter by the
    /// caller; released here).
    const fn new(pending: Arc<AtomicUsize>, count: usize) -> Self {
        Self { pending, count }
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.pending.fetch_sub(self.count, Ordering::Relaxed);
    }
}

/// Loads every chunk in `chunks` that is not in memory yet.
async fn ensure_chunks(world: &Arc<World>, chunks: &[Vector2<i32>]) {
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

/// Applies `(position, state)` pairs, optionally with neighbor updates,
/// plus any block-entity NBT keyed by absolute position.
///
/// Neighbor updates are decided per position through `neighbors(pos)` so
/// the `preventUpdatesAt` set can downgrade individual writes.
// The concrete hasher never varies in this crate-internal API.
#[allow(clippy::implicit_hasher)]
pub async fn apply_edits(
    world: &Arc<World>,
    edits: &[(BlockPos, BlockStateId)],
    neighbors: impl Fn(&BlockPos) -> bool,
    block_entities: &HashMap<BlockPos, pumpkin_nbt::NbtCompound>,
) {
    let chunks: Vec<Vector2<i32>> = edits
        .iter()
        .map(|(pos, _)| Vector2::new(pos.0.x >> 4, pos.0.z >> 4))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let _lease = ChunkLease::acquire(&world.level, chunks.clone());
    ensure_chunks(world, &chunks).await;

    for (index, (pos, state)) in edits.iter().enumerate() {
        if !world.is_in_build_limit(*pos) {
            continue;
        }
        let flags = if neighbors(pos) {
            NEIGHBOR_FLAGS
        } else {
            APPLY_FLAGS
        };
        world.set_block_state(pos, *state, flags);
        if let Some(nbt) = block_entities.get(pos) {
            world.load_block_entity_nbt(*pos, nbt);
        }
        if index % FLUSH_INTERVAL == FLUSH_INTERVAL - 1 {
            world.flush_block_updates();
            tokio::task::yield_now().await;
        }
    }
    world.flush_block_updates();
}

/// Applies a decoded block buffer: sections in wire order, block entities
/// (when `allow_nbt`) after their section's blocks.
///
/// Sections outside the world's vertical range are dropped (upstream
/// behavior); their pending cost is still released by the guard.
pub async fn apply_buffer(
    world: &Arc<World>,
    buffer: &BlockBuffer,
    allow_nbt: bool,
    pending: Arc<AtomicUsize>,
) {
    let _guard = PendingGuard::new(pending, buffer.section_count());

    let min_section = world.dimension.min_y >> 4;
    let max_section = (world.dimension.min_y + world.dimension.height - 1) >> 4;
    let sections: Vec<&crate::buffer::BufferSection> = buffer
        .sections
        .iter()
        .filter(|section| section.sy >= min_section && section.sy <= max_section)
        .collect();
    if sections.is_empty() {
        return;
    }

    let chunks: Vec<Vector2<i32>> = sections
        .iter()
        .map(|section| Vector2::new(section.sx, section.sz))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let _lease = ChunkLease::acquire(&world.level, chunks.clone());
    ensure_chunks(world, &chunks).await;

    let mut writes: usize = 0;
    for section in sections {
        for (lx, ly, lz, state) in section.changes() {
            let pos = BlockPos::new(
                section.sx * 16 + i32::from(lx),
                section.sy * 16 + i32::from(ly),
                section.sz * 16 + i32::from(lz),
            );
            if !world.is_in_build_limit(pos) {
                continue;
            }
            world.set_block_state(&pos, state, APPLY_FLAGS);
            writes += 1;
            if writes.is_multiple_of(FLUSH_INTERVAL) {
                world.flush_block_updates();
                tokio::task::yield_now().await;
            }
        }

        if allow_nbt
            && let Some(table) = buffer
                .block_entities
                .get(&crate::wire::pack_block_pos(section.sx, section.sy, section.sz))
        {
            for (offset, entity) in table {
                let pos = BlockPos::new(
                    section.sx * 16 + i32::from(offset & 0xF),
                    section.sy * 16 + i32::from(offset >> 4 & 0xF),
                    section.sz * 16 + i32::from(offset >> 8 & 0xF),
                );
                match entity.decompress() {
                    Ok(mut nbt) => {
                        nbt.put_int("x", pos.0.x);
                        nbt.put_int("y", pos.0.y);
                        nbt.put_int("z", pos.0.z);
                        world.load_block_entity_nbt(pos, &nbt);
                    }
                    Err(err) => {
                        tracing::debug!("carrot-axiom: dropping block entity at {pos:?}: {err}");
                    }
                }
            }
        }
    }
    world.flush_block_updates();
}

/// Forces ticks for the given positions (mirror of
/// `TickBlocksPacketListener`'s consumer, adapted to Pumpkin's tick
/// scheduler).
///
/// Fluids and liquid blocks schedule a fluid tick; other blocks get
/// neighbor updates instead of vanilla's `updateFromNeighbourShapes`,
/// which Pumpkin does not expose publicly (documented divergence: the
/// common use case — making placed water/lava flow — is exact).
pub async fn tick_positions(world: &Arc<World>, positions: &[BlockPos]) {
    let chunks: Vec<Vector2<i32>> = positions
        .iter()
        .map(|pos| Vector2::new(pos.0.x >> 4, pos.0.z >> 4))
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let _lease = ChunkLease::acquire(&world.level, chunks.clone());
    ensure_chunks(world, &chunks).await;

    let mut processed: usize = 0;
    for pos in positions {
        let state_id = world.get_block_state_id(pos);
        let state = state_id.to_state();
        if state.is_air() {
            continue;
        }
        let block = state_id.to_block();
        if state.is_liquid() {
            if let Some(fluid) = fluid_for(block) {
                world.schedule_fluid_tick(fluid, *pos, 0, TickPriority::High);
            }
        } else {
            world.update_neighbors_at(pos, block, None);
        }
        processed += 1;
        if processed.is_multiple_of(65_536) {
            tokio::task::yield_now().await;
        }
    }
}

/// The fluid associated with a liquid block (water/lava families).
const fn fluid_for(block: &pumpkin_data::Block) -> Option<&'static Fluid> {
    match block.id {
        pumpkin_data::BlockId::WATER => Some(&Fluid::WATER),
        pumpkin_data::BlockId::LAVA => Some(&Fluid::LAVA),
        _ => None,
    }
}
