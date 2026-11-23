//! Editing channel handlers: `axiom:set_block`, `axiom:set_buffer` and
//! `axiom:tick_blocks` (mirrors of the upstream `packet/impl` listeners).
//!
//! Handlers parse the payload through [`crate::wire::Reader`], apply the
//! same guards (collection caps, permission checks, backpressure), and
//! dispatch the world work to [`crate::apply`] on the async runtime.
//! Malformed payloads kick the player with the upstream-style message;
//! permission misses drop silently, exactly like upstream.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::Ordering};

use pumpkin_core::entity::player::Player;
use pumpkin_core::server::Server;
use pumpkin_data::BlockStateId;
use pumpkin_protocol::VarInt;
use pumpkin_protocol::java::client::play::CAcknowledgeBlockChange;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;

use crate::buffer::{self, BlockBuffer};
use crate::state::{AxiomState, read_client_dispatch};
use crate::wire::{Reader, WireError, unpack_block_pos};

/// Upstream `packetCollectionReadLimit`: maximum collection size on the
/// wire (maps, sets, position lists).
const COLLECTION_LIMIT: usize = 1024;

/// Result of a handler run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handled {
    /// The payload was consumed in full.
    FullyRead,
    /// The payload was deliberately not consumed to the end (unimplemented
    /// sub-features); the tunnel's full-read check must be skipped.
    Partial,
}

/// Whether the player holds an editing permission (op short-circuit, then
/// the literal node through children/wildcards).
fn has_perm(server: &Arc<Server>, player: &Arc<Player>, node: &str) -> bool {
    crate::is_op(player)
        || server.permission_manager.has_permission(
            &player.gameprofile.id,
            node,
            player.permission_lvl.load(),
        )
}

/// `axiom:set_block`: a batch of `(position, state)` with optional neighbor
/// suppression and a client sequence to acknowledge.
///
/// Wire: `VarInt` count + (`long` pos, `VarInt` state) pairs · bool
/// updateNeighbors · [`VarInt`] count + `long` preventUpdatesAt · `VarInt`
/// reason · bool breaking · `long` hit pos + `VarInt` direction (unused) ·
/// `VarInt` hand (unused) · `VarInt` sequence.
pub async fn handle_set_block(
    _state: &AxiomState,
    server: &Arc<Server>,
    player: &Arc<Player>,
    data: &[u8],
) -> Result<Handled, String> {
    if !has_perm(server, player, crate::permissions::BUILD_PLACE_NODE) {
        return Ok(Handled::FullyRead);
    }
    let world = player.world();
    let mut reader = Reader::new(data);

    let count = reader
        .read_var_int()
        .map_err(|err| wrap(err, "axiom:set_block"))?;
    if count < 0 || count as usize > COLLECTION_LIMIT {
        return Err(format!(
            "Error while processing packet axiom:set_block: Value {count} is larger than limit {COLLECTION_LIMIT}"
        ));
    }

    let mut edits: Vec<(BlockPos, BlockStateId)> = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let packed = reader.read_i64_be().map_err(|err| wrap(err, "axiom:set_block"))?;
        let (x, y, z) = unpack_block_pos(packed);
        let raw = reader.read_var_int().map_err(|err| wrap(err, "axiom:set_block"))?;
        let Some(id) = buffer::filter_state(u64::try_from(raw.max(0)).unwrap_or(u64::MAX)) else {
            // Void air or a disallowed state: upstream keeps the map entry
            // with the void-air sentinel and skips it at apply time.
            continue;
        };
        edits.push((BlockPos::new(x, y, z), id));
    }

    let update_neighbors = reader
        .read_bool()
        .map_err(|err| wrap(err, "axiom:set_block"))?;
    let mut prevent: HashSet<(i32, i32, i32)> = HashSet::new();
    if update_neighbors {
        let prevent_count = reader
            .read_var_int()
            .map_err(|err| wrap(err, "axiom:set_block"))?;
        if prevent_count < 0 || prevent_count as usize > COLLECTION_LIMIT {
            return Err(format!(
                "Error while processing packet axiom:set_block: Value {prevent_count} is larger than limit {COLLECTION_LIMIT}"
            ));
        }
        for _ in 0..prevent_count {
            let packed = reader.read_i64_be().map_err(|err| wrap(err, "axiom:set_block"))?;
            prevent.insert(unpack_block_pos(packed));
        }
    }

    // reason, breaking, hit result, hand and sequence close the payload.
    let _reason = reader.read_var_int().map_err(|err| wrap(err, "axiom:set_block"))?;
    let _breaking = reader
        .read_bool()
        .map_err(|err| wrap(err, "axiom:set_block"))?;
    let _hit = reader.read_i64_be().map_err(|err| wrap(err, "axiom:set_block"))?;
    let _direction = reader
        .read_var_int()
        .map_err(|err| wrap(err, "axiom:set_block"))?;
    let _hand = reader.read_var_int().map_err(|err| wrap(err, "axiom:set_block"))?;
    let sequence = reader
        .read_var_int()
        .map_err(|err| wrap(err, "axiom:set_block"))?;
    if reader.remaining() > 0 {
        return Err("Axiom: Failed to fully read axiom:set_block packet".to_string());
    }

    if sequence >= 0
        && let Some(java) = player.as_java()
    {
        java.0
            .try_send_client_packet(&CAcknowledgeBlockChange::new(VarInt(sequence)));
    }

    // A position in the prevent set (or adjacent to one) gets no neighbor
    // updates, mirroring the upstream per-direction check.
    let neighbors = move |pos: &BlockPos| {
        if !update_neighbors {
            return false;
        }
        if prevent.contains(&(pos.0.x, pos.0.y, pos.0.z)) {
            return false;
        }
        !prevent.contains(&(pos.0.x + 1, pos.0.y, pos.0.z))
            && !prevent.contains(&(pos.0.x - 1, pos.0.y, pos.0.z))
            && !prevent.contains(&(pos.0.x, pos.0.y + 1, pos.0.z))
            && !prevent.contains(&(pos.0.x, pos.0.y - 1, pos.0.z))
            && !prevent.contains(&(pos.0.x, pos.0.y, pos.0.z + 1))
            && !prevent.contains(&(pos.0.x, pos.0.y, pos.0.z - 1))
    };

    crate::apply::apply_edits(&world, &edits, neighbors, &HashMap::new()).await;
    Ok(Handled::FullyRead)
}

/// `axiom:set_buffer`: a bulk section buffer (blocks in AX-1).
///
/// Wire: `Utf8` dimension · uuid (discarded) · byte type (0 = blocks,
/// 1 = biomes) · buffer body · `VarInt` clientAvailableDispatchSends.
// Async for uniform dispatch; the world work is spawned, not awaited.
#[allow(clippy::unused_async)]
pub async fn handle_set_buffer(
    state: &AxiomState,
    server: &Arc<Server>,
    player: &Arc<Player>,
    data: &[u8],
) -> Result<Handled, String> {
    let uuid = player.gameprofile.id;
    let config = state.config();
    let pending = state.pending_sections(uuid);

    // Backpressure hard cap before any parsing (upstream order).
    if pending.load(Ordering::Relaxed) > config.max_pending_sections_per_player_hard as usize {
        return Err("Axiom: too many pending operations".to_string());
    }

    let world = player.world();
    let mut reader = Reader::new(data);
    let dimension = reader
        .read_string()
        .map_err(|err| wrap(err, "axiom:set_buffer"))?;
    let _discarded_uuid = reader
        .read_uuid_bytes()
        .map_err(|err| wrap(err, "axiom:set_buffer"))?;

    let buffer_type = reader
        .read_byte()
        .map_err(|err| wrap(err, "axiom:set_buffer"))?;
    if buffer_type != 0 {
        // Biome buffers (type 1) are phase AX-4; the payload is legal but
        // unsupported, so it is dropped with a note instead of a kick.
        tracing::debug!("carrot-axiom: dropping set_buffer type {buffer_type} (AX-4)");
        return Ok(Handled::Partial);
    }

    let block_buffer = BlockBuffer::load(&mut reader).map_err(|err| {
        format!("Error while processing packet axiom:set_buffer: {err}")
    })?;
    let client_available = read_client_dispatch(&mut reader);
    if reader.remaining() > 0 {
        return Err("Axiom: Failed to fully read axiom:set_buffer packet".to_string());
    }

    // Dimension must match the player's current world (upstream returns
    // silently on mismatch).
    if dimension != world.dimension.minecraft_name {
        return Ok(Handled::FullyRead);
    }

    let sections = block_buffer.section_count();
    if sections > 0 {
        // Dispatch budget (token bucket): reserves this buffer's cost.
        let violation = state.with_player(uuid, |entry| {
            entry
                .dispatch
                .consume(&config, pending.load(Ordering::Relaxed), sections, client_available)
                .err()
        });
        if let Some(reason) = violation {
            return Err(reason.to_string());
        }
        pending.fetch_add(sections, Ordering::Relaxed);
    }

    if !has_perm(server, player, crate::permissions::BUILD_SECTION_NODE) {
        // Refund: permission miss drops the edit but must not eat budget.
        pending.fetch_sub(sections, Ordering::Relaxed);
        return Ok(Handled::FullyRead);
    }
    let allow_nbt = has_perm(server, player, crate::permissions::BUILD_NBT_NODE);

    let buffer = Arc::new(block_buffer);
    tokio::spawn(async move {
        crate::apply::apply_buffer(&world, &buffer, allow_nbt, pending).await;
    });
    Ok(Handled::FullyRead)
}

/// `axiom:tick_blocks`: forced ticking of a position set or AABB.
///
/// Wire: `Utf8` dimension · byte type (0 = position set, 1 = AABB) ·
/// body (packed 16×16 column masks or two `long` positions).
pub async fn handle_tick_blocks(
    _state: &AxiomState,
    server: &Arc<Server>,
    player: &Arc<Player>,
    data: &[u8],
) -> Result<Handled, String> {
    if !has_perm(server, player, crate::permissions::BUILD_DANGEROUS_TICK_NODE) {
        return Ok(Handled::FullyRead);
    }
    let world = player.world();
    let mut reader = Reader::new(data);
    let dimension = reader
        .read_string()
        .map_err(|err| wrap(err, "axiom:tick_blocks"))?;
    let tick_type = reader
        .read_byte()
        .map_err(|err| wrap(err, "axiom:tick_blocks"))?;

    let positions = match tick_type {
        0 => read_position_set(&mut reader)?,
        1 => {
            let min = unpack_block_pos(
                reader
                    .read_i64_be()
                    .map_err(|err| wrap(err, "axiom:tick_blocks"))?,
            );
            let max = unpack_block_pos(
                reader
                    .read_i64_be()
                    .map_err(|err| wrap(err, "axiom:tick_blocks"))?,
            );
            aabb_positions(min, max)
        }
        other => {
            return Err(format!(
                "Error while processing packet axiom:tick_blocks: Unknown type: {other}"
            ));
        }
    };
    if reader.remaining() > 0 {
        return Err("Axiom: Failed to fully read axiom:tick_blocks packet".to_string());
    }

    if dimension != world.dimension.minecraft_name {
        return Ok(Handled::FullyRead);
    }

    let count = positions.len();
    if count > 1_048_576 {
        let name = &player.gameprofile.name;
        broadcast(
            server,
            &format!("{name} updated & ticked {count} blocks using Axiom. The server may lag..."),
        );
        let estimate = (count / 2_097_152).max(1);
        broadcast(
            server,
            &format!("Estimated Time (varies depending on server hardware): {estimate}s"),
        );
        if estimate > 30 {
            broadcast(server, "Estimated time is >30s, expect to be kicked from the server");
        }
    }

    crate::apply::tick_positions(&world, &positions).await;
    Ok(Handled::FullyRead)
}

/// Reads a packed `PositionSet`: `VarInt` section count + per section a
/// `long` position and 256 `short` row masks (y-major over z, x in bits).
fn read_position_set(reader: &mut Reader<'_>) -> Result<Vec<BlockPos>, String> {
    let sections = reader
        .read_var_int()
        .map_err(|err| wrap(err, "axiom:tick_blocks"))?;
    // The wire count is bounded by the packet itself (≤ 2 MiB / 514 bytes
    // per section); a hostile count cannot allocate more than that.
    if sections < 0 || sections as usize > COLLECTION_LIMIT {
        return Err(format!(
            "Error while processing packet axiom:tick_blocks: Value {sections} is larger than limit {COLLECTION_LIMIT}"
        ));
    }

    let mut positions = Vec::new();
    for _ in 0..sections {
        let packed = reader
            .read_i64_be()
            .map_err(|err| wrap(err, "axiom:tick_blocks"))?;
        let (sx, sy, sz) = unpack_block_pos(packed);
        let mut rows = [0u16; 256];
        for row in &mut rows {
            *row = reader
                .read_i16_be()
                .map_err(|err| wrap(err, "axiom:tick_blocks"))? as u16;
        }
        // Row layout matches PositionSet.forEach: index = z*16 + y, bits = x.
        for (index, mask) in rows.iter().enumerate() {
            let z = (index % 16) as i32;
            let y = (index / 16) as i32;
            if *mask == 0xFFFF {
                for x in 0..16 {
                    positions.push(BlockPos::new(
                        sx * 16 + x,
                        sy * 16 + y,
                        sz * 16 + z,
                    ));
                }
            } else {
                for x in 0..16 {
                    if mask & (1 << x) != 0 {
                        positions.push(BlockPos::new(
                            sx * 16 + x,
                            sy * 16 + y,
                            sz * 16 + z,
                        ));
                    }
                }
            }
        }
    }
    Ok(positions)
}

/// Expands an AABB pair into positions (upstream iterates the inclusive
/// box; the cap keeps a hostile box from allocating gigabytes).
fn aabb_positions(min: (i32, i32, i32), max: (i32, i32, i32)) -> Vec<BlockPos> {
    const AABB_CAP: usize = 1_048_576;
    let (min_x, max_x) = (min.0.min(max.0), min.0.max(max.0));
    let (min_y, max_y) = (min.1.min(max.1), min.1.max(max.1));
    let (min_z, max_z) = (min.2.min(max.2), min.2.max(max.2));
    let mut positions = Vec::new();
    'outer: for x in min_x..=max_x {
        for y in min_y..=max_y {
            for z in min_z..=max_z {
                positions.push(BlockPos::new(x, y, z));
                if positions.len() >= AABB_CAP {
                    tracing::warn!(
                        "carrot-axiom: tick_blocks AABB clamped to {AABB_CAP} positions"
                    );
                    break 'outer;
                }
            }
        }
    }
    positions
}

/// Broadcasts a system message to every online player.
fn broadcast(server: &Arc<Server>, message: &str) {
    for player in server.get_all_players() {
        player.send_system_message(&TextComponent::text(message.to_string()));
    }
}

/// Formats a wire error the way the dedicated-channel wrapper does.
// The borrowed error is only formatted; passing by value would force clones
// at every call site that already owns its reader.
#[allow(clippy::needless_pass_by_value)]
fn wrap(err: WireError, channel: &str) -> String {
    format!("Error while processing packet {channel}: {err}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aabb_expands_inclusively_and_clamps() {
        let positions = aabb_positions((0, 0, 0), (1, 1, 1));
        assert_eq!(positions.len(), 8);
        let positions = aabb_positions((5, 5, 5), (0, 0, 0));
        assert_eq!(positions.len(), 216);
        let positions = aabb_positions((0, 0, 0), (200, 200, 200));
        assert_eq!(positions.len(), 1_048_576);
    }

    #[test]
    fn position_set_rows_decode() {
        let mut writer = crate::wire::Writer::new();
        writer.write_var_int(1);
        writer.write_i64_be(crate::wire::pack_block_pos(1, -1, 2));
        for row in 0..256 {
            // All bits set in row z=0/y=0, single bit x=3 in row z=1/y=0.
            if row == 0 {
                writer.write_i16_be(-1);
            } else if row == 1 {
                writer.write_i16_be(1 << 3);
            } else {
                writer.write_i16_be(0);
            }
        }
        let bytes = writer.into_bytes();
        let mut reader = Reader::new(&bytes);
        let positions = read_position_set(&mut reader).unwrap();
        assert_eq!(positions.len(), 17);
        assert!(positions.contains(&BlockPos::new(16, -16, 32))); // row 0, x=0
        assert!(positions.contains(&BlockPos::new(31, -16, 32))); // row 0, x=15
        assert!(positions.contains(&BlockPos::new(19, -16, 33))); // row 1, x=3
    }
}
