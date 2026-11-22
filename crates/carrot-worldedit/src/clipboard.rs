//! The session clipboard: a copied cuboid with its states, block entities
//! and copy origin, plus rotate/flip transforms around that origin.

use std::collections::HashMap;

use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_data::block_rotation::{Mirror, Rotation};
use pumpkin_nbt::NbtCompound;
use pumpkin_util::math::{position::BlockPos, vector3::Vector3};
use rayon::prelude::*;

use crate::region::Cuboid;

/// Horizontal axis used by `//flip`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlipAxis {
    /// Mirror east-west (`Mirror::FrontBack`).
    X,
    /// Mirror north-south (`Mirror::LeftRight`).
    Z,
}

/// A copied region. Coordinates are the absolute world coordinates at copy
/// time; pasting anchors on `origin` (where the player stood).
pub struct Clipboard {
    min: BlockPos,
    origin: BlockPos,
    size: Vector3<i32>,
    blocks: Vec<BlockStateId>,
    block_entities: HashMap<usize, NbtCompound>,
}

impl Clipboard {
    /// Copies `region` from `world`, remembering `origin` as the anchor.
    #[must_use]
    pub fn from_world(world: &World, region: &Cuboid, origin: BlockPos) -> Self {
        let size = region.size();
        let min = region.min();
        let positions: Vec<BlockPos> = region.iterate();
        let blocks: Vec<BlockStateId> = if positions.len() > 65_536 {
            positions
                .par_iter()
                .map(|pos| world.get_block_state_id(pos))
                .collect()
        } else {
            positions
                .iter()
                .map(|pos| world.get_block_state_id(pos))
                .collect()
        };

        let mut block_entities = HashMap::new();
        for pos in &positions {
            if let Some(nbt) = world.get_block_entity_nbt(pos)
                && let Some(index) = relative_index(&min, &size, pos)
            {
                block_entities.insert(index, nbt);
            }
        }

        Self {
            min,
            origin,
            size,
            blocks,
            block_entities,
        }
    }

    /// The minimum corner (absolute, at copy time).
    #[must_use]
    pub const fn min(&self) -> BlockPos {
        self.min
    }

    /// The paste anchor (absolute, where the player stood when copying).
    #[must_use]
    pub const fn origin(&self) -> BlockPos {
        self.origin
    }

    /// The inclusive dimensions.
    #[must_use]
    pub const fn size(&self) -> Vector3<i32> {
        self.size
    }

    /// Total blocks in the clipboard.
    #[must_use]
    pub const fn volume(&self) -> usize {
        self.blocks.len()
    }

    /// State at an absolute position, or `None` outside the copied region.
    #[must_use]
    pub fn state_at_absolute(&self, pos: &BlockPos) -> Option<BlockStateId> {
        let index = relative_index(&self.min, &self.size, pos)?;
        self.blocks.get(index).copied()
    }

    /// Iterates `(absolute position, state)` pairs of the whole clipboard.
    pub fn iter_blocks(&self) -> impl Iterator<Item = (BlockPos, BlockStateId)> + '_ {
        let min = self.min;
        let size = self.size;
        self.blocks.iter().enumerate().map(move |(index, state)| {
            let (x, y, z) = decompose(index, size);
            (
                BlockPos(Vector3::new(min.0.x + x, min.0.y + y, min.0.z + z)),
                *state,
            )
        })
    }

    /// Block entity NBT by linear index (position stays absolute inside).
    #[must_use]
    pub const fn block_entities(&self) -> &HashMap<usize, NbtCompound> {
        &self.block_entities
    }

    /// Assembles a clipboard from raw parts (used by schematic loading).
    #[must_use]
    pub const fn from_parts(
        min: BlockPos,
        origin: BlockPos,
        size: Vector3<i32>,
        blocks: Vec<BlockStateId>,
        block_entities: HashMap<usize, NbtCompound>,
    ) -> Self {
        Self {
            min,
            origin,
            size,
            blocks,
            block_entities,
        }
    }

    /// State by linear index (`x + z*W + y*W*L`).
    #[must_use]
    pub fn block_at_index(&self, index: usize) -> Option<BlockStateId> {
        self.blocks.get(index).copied()
    }

    /// Absolute position of a linear index, inverse of the block order.
    #[must_use]
    pub const fn position_of_index(&self, index: usize) -> Option<BlockPos> {
        if index >= self.blocks.len() {
            return None;
        }
        let (x, y, z) = decompose(index, self.size);
        Some(BlockPos(Vector3::new(
            self.min.0.x + x,
            self.min.0.y + y,
            self.min.0.z + z,
        )))
    }

    /// Takes the block entities map (used when pasting).
    #[must_use]
    pub fn take_block_entities(&mut self) -> HashMap<usize, NbtCompound> {
        std::mem::take(&mut self.block_entities)
    }

    /// Returns a rotated copy, turning clockwise in `steps` of 90 degrees
    /// around the copy origin (0-3).
    #[must_use]
    pub fn rotated(&self, steps: u8) -> Self {
        match steps & 3 {
            0 => Self {
                min: self.min,
                origin: self.origin,
                size: self.size,
                blocks: self.blocks.clone(),
                block_entities: clone_entities(&self.block_entities),
            },
            1 => self.transformed(
                |rel| Vector3::new(self.size.z - 1 - rel.z, rel.y, rel.x),
                |vec| Vector3::new(-vec.z, vec.y, vec.x),
                |state| state.rotate(Rotation::Clockwise90).id,
            ),
            2 => self.transformed(
                |rel| Vector3::new(self.size.x - 1 - rel.x, rel.y, self.size.z - 1 - rel.z),
                |vec| Vector3::new(-vec.x, vec.y, -vec.z),
                |state| state.rotate(Rotation::Rotate180).id,
            ),
            _ => self.transformed(
                |rel| Vector3::new(rel.z, rel.y, self.size.x - 1 - rel.x),
                |vec| Vector3::new(vec.z, vec.y, -vec.x),
                |state| state.rotate(Rotation::CounterClockwise90).id,
            ),
        }
    }

    /// Returns a mirrored copy over `axis`, keeping the origin in place.
    #[must_use]
    pub fn flipped(&self, axis: FlipAxis) -> Self {
        match axis {
            FlipAxis::X => self.transformed(
                |rel| Vector3::new(self.size.x - 1 - rel.x, rel.y, rel.z),
                |vec| Vector3::new(-vec.x, vec.y, vec.z),
                |state| state.mirror(Mirror::FrontBack).id,
            ),
            FlipAxis::Z => self.transformed(
                |rel| Vector3::new(rel.x, rel.y, self.size.z - 1 - rel.z),
                |vec| Vector3::new(vec.x, vec.y, -vec.z),
                |state| state.mirror(Mirror::LeftRight).id,
            ),
        }
    }

    /// Core transform. `index_map` remaps block coordinates relative to
    /// the old minimum into coordinates relative to the new minimum;
    /// `vector_map` is the same rotation/mirror as a free vector (no
    /// bounding shift), used to place the new box around the unchanged
    /// origin. Keeping them apart is what makes the transform turn around
    /// the origin instead of the region minimum.
    fn transformed<F, G, S>(&self, index_map: F, vector_map: G, state_map: S) -> Self
    where
        F: Fn(Vector3<i32>) -> Vector3<i32> + Copy,
        G: Fn(Vector3<i32>) -> Vector3<i32>,
        S: Fn(BlockStateId) -> BlockStateId,
    {
        let min_rel = Vector3::new(
            self.min.0.x - self.origin.0.x,
            self.min.0.y - self.origin.0.y,
            self.min.0.z - self.origin.0.z,
        );
        let max_rel = Vector3::new(
            min_rel.x + self.size.x - 1,
            min_rel.y + self.size.y - 1,
            min_rel.z + self.size.z - 1,
        );
        let a = vector_map(min_rel);
        let b = vector_map(max_rel);
        let mapped_min = component_min(a, b);
        let mapped_max = component_max(a, b);
        let new_min = BlockPos(Vector3::new(
            self.origin.0.x + mapped_min.x,
            self.origin.0.y + mapped_min.y,
            self.origin.0.z + mapped_min.z,
        ));
        let new_size = Vector3::new(
            mapped_max.x - mapped_min.x + 1,
            mapped_max.y - mapped_min.y + 1,
            mapped_max.z - mapped_min.z + 1,
        );

        let mut blocks = vec![BlockStateId::AIR; (new_size.x * new_size.y * new_size.z) as usize];
        let mut entities = HashMap::with_capacity(self.block_entities.len());
        for y in 0..self.size.y {
            for z in 0..self.size.z {
                for x in 0..self.size.x {
                    let source = linear_index(x, y, z, self.size);
                    let target_rel = index_map(Vector3::new(x, y, z));
                    let target = linear_index(target_rel.x, target_rel.y, target_rel.z, new_size);
                    blocks[target] = state_map(self.blocks[source]);
                    if let Some(nbt) = self.block_entities.get(&source) {
                        entities.insert(target, nbt.clone());
                    }
                }
            }
        }

        Self {
            min: new_min,
            origin: self.origin,
            size: new_size,
            blocks,
            block_entities: entities,
        }
    }
}

fn clone_entities(entities: &HashMap<usize, NbtCompound>) -> HashMap<usize, NbtCompound> {
    entities
        .iter()
        .map(|(index, nbt)| (*index, nbt.clone()))
        .collect()
}

fn component_min(a: Vector3<i32>, b: Vector3<i32>) -> Vector3<i32> {
    Vector3::new(a.x.min(b.x), a.y.min(b.y), a.z.min(b.z))
}

fn component_max(a: Vector3<i32>, b: Vector3<i32>) -> Vector3<i32> {
    Vector3::new(a.x.max(b.x), a.y.max(b.y), a.z.max(b.z))
}

const fn linear_index(x: i32, y: i32, z: i32, size: Vector3<i32>) -> usize {
    (x + z * size.x + y * size.x * size.z) as usize
}

const fn decompose(index: usize, size: Vector3<i32>) -> (i32, i32, i32) {
    let area = size.x * size.z;
    let y = (index as i32) / area;
    let rest = (index as i32) - y * area;
    let z = rest / size.x;
    let x = rest - z * size.x;
    (x, y, z)
}

const fn relative_index(min: &BlockPos, size: &Vector3<i32>, pos: &BlockPos) -> Option<usize> {
    let x = pos.0.x - min.0.x;
    let y = pos.0.y - min.0.y;
    let z = pos.0.z - min.0.z;
    if x < 0 || y < 0 || z < 0 || x >= size.x || y >= size.y || z >= size.z {
        return None;
    }
    Some(linear_index(x, y, z, *size))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: BlockPos = BlockPos(Vector3::new(100, 64, 100));
    const ORIGIN: BlockPos = BlockPos(Vector3::new(101, 64, 101));

    fn clip(size: Vector3<i32>, fill: impl Fn(i32, i32, i32) -> BlockStateId) -> Clipboard {
        let mut blocks = Vec::new();
        for y in 0..size.y {
            for z in 0..size.z {
                for x in 0..size.x {
                    blocks.push(fill(x, y, z));
                }
            }
        }
        Clipboard {
            min: MIN,
            origin: ORIGIN,
            size,
            blocks,
            block_entities: HashMap::new(),
        }
    }

    fn state(name: &str) -> BlockStateId {
        crate::states::parse_state_str(name).expect("valid state")
    }

    #[test]
    fn round_trips_positions() {
        let clipboard = clip(Vector3::new(2, 1, 3), |x, _, _| {
            state(if x == 0 { "stone" } else { "dirt" })
        });
        let inside = BlockPos(Vector3::new(101, 64, 102));
        let outside = BlockPos(Vector3::new(0, 0, 0));
        assert_eq!(clipboard.state_at_absolute(&inside), Some(state("dirt")));
        assert_eq!(clipboard.state_at_absolute(&outside), None);
        assert_eq!(clipboard.iter_blocks().count(), 6);
    }

    #[test]
    fn clockwise_rotation_places_blocks_correctly() {
        let stone = state("stone");
        let dirt = state("dirt");
        // 2 wide (X), 1 tall, 1 deep: stone at x=0, dirt at x=1.
        let clipboard = clip(
            Vector3::new(2, 1, 1),
            |x, _, _| {
                if x == 0 { stone } else { dirt }
            },
        );
        let rotated = clipboard.rotated(1);
        // Sizes swap X/Z and the origin stays.
        assert_eq!(rotated.size(), Vector3::new(1, 1, 2));
        assert_eq!(rotated.origin(), ORIGIN);
        // Rotating the old box rel-origin corners (-1,-1)/(0,0) with
        // (dx,dz)->(-dz,dx) lands the box at (1,-1)/(1,0): min (102,64,100).
        assert_eq!(rotated.min(), BlockPos(Vector3::new(102, 64, 100)));
        // Stone (rel-origin (-1,-1)) maps to (1,-1) => absolute (102,64,100).
        assert_eq!(
            rotated.state_at_absolute(&BlockPos(Vector3::new(102, 64, 100))),
            Some(stone)
        );
        // Dirt (rel-origin (0,-1)) maps to (1,0) => absolute (102,64,101).
        assert_eq!(
            rotated.state_at_absolute(&BlockPos(Vector3::new(102, 64, 101))),
            Some(dirt)
        );
    }

    #[test]
    fn four_rotations_return_the_same_region() {
        let clipboard = clip(Vector3::new(2, 2, 3), |x, y, z| {
            let _ = (x, y, z);
            state("stone")
        });
        let mut current = clipboard.rotated(1);
        current = current.rotated(1);
        current = current.rotated(1);
        current = current.rotated(1);
        assert_eq!(current.size(), clipboard.size());
        assert_eq!(current.min(), clipboard.min());
        assert_eq!(current.volume(), clipboard.volume());
    }

    #[test]
    fn flip_x_swaps_east_west_of_origin() {
        let east = state("dirt");
        let clipboard = clip(Vector3::new(2, 1, 1), |x, _, _| {
            let _ = x;
            east
        });
        let flipped = clipboard.flipped(FlipAxis::X);
        assert_eq!(flipped.size(), clipboard.size());
        assert_eq!(flipped.origin(), ORIGIN);
        // Box rel-origin x range [-1,0] mirrors to [0,1]: min x=101.
        assert_eq!(flipped.min().0.x, 101);
        assert_eq!(flipped.volume(), 2);
    }
}
