//! Small shared helpers for terrain masks and generators.

use carrot_worldedit::states::is_air;
use pumpkin_core::world::World;
use pumpkin_util::math::position::BlockPos;
use rand::RngExt;

/// The position holds air.
pub fn is_air_at(world: &World, pos: &BlockPos) -> bool {
    is_air(world.get_block_state_id(pos))
}

/// A random seed in the reference's `nextInt(100000000)` range.
#[must_use]
pub fn random_seed() -> i64 {
    rand::rng().random_range(0..100_000_000)
}
