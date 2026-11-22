//! Cuboid selections, cardinal directions and the block sets they produce.

use pumpkin_util::math::{position::BlockPos, vector3::Vector3};

/// One of the six cardinal directions plus the player's look direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Towards negative Z.
    North,
    /// Towards positive Z.
    South,
    /// Towards negative X.
    West,
    /// Towards positive X.
    East,
    /// Towards positive Y.
    Up,
    /// Towards negative Y.
    Down,
}

impl Direction {
    /// The unit vector of this direction.
    #[must_use]
    pub const fn offset(self) -> Vector3<i32> {
        match self {
            Self::North => Vector3::new(0, 0, -1),
            Self::South => Vector3::new(0, 0, 1),
            Self::West => Vector3::new(-1, 0, 0),
            Self::East => Vector3::new(1, 0, 0),
            Self::Up => Vector3::new(0, 1, 0),
            Self::Down => Vector3::new(0, -1, 0),
        }
    }

    /// Parses a direction keyword (`WorldEdit` names, both `north` and `n`).
    #[must_use]
    pub fn parse(keyword: &str) -> Option<Self> {
        match keyword.to_ascii_lowercase().as_str() {
            "north" | "n" | "norte" => Some(Self::North),
            "south" | "s" | "sul" => Some(Self::South),
            "west" | "w" | "oeste" => Some(Self::West),
            "east" | "e" | "leste" => Some(Self::East),
            "up" | "u" | "cima" => Some(Self::Up),
            "down" | "d" | "baixo" => Some(Self::Down),
            _ => None,
        }
    }

    /// Maps a horizontal yaw (Minecraft convention: 0 = south) to the
    /// cardinal direction the player faces.
    #[must_use]
    pub fn from_yaw(yaw: f32) -> Self {
        let index = (yaw.rem_euclid(360.0) / 90.0).round() as i32 & 3;
        match index {
            0 => Self::South,
            1 => Self::West,
            2 => Self::North,
            _ => Self::East,
        }
    }
}

/// An axis-aligned cuboid between two corners (inclusive).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cuboid {
    min: BlockPos,
    max: BlockPos,
}

impl Cuboid {
    /// Builds a cuboid from two arbitrary corners; corners are normalized.
    #[must_use]
    pub fn new(a: BlockPos, b: BlockPos) -> Self {
        let min = Vector3::new(a.0.x.min(b.0.x), a.0.y.min(b.0.y), a.0.z.min(b.0.z));
        let max = Vector3::new(a.0.x.max(b.0.x), a.0.y.max(b.0.y), a.0.z.max(b.0.z));
        Self {
            min: BlockPos(min),
            max: BlockPos(max),
        }
    }

    /// The minimum corner.
    #[must_use]
    pub const fn min(&self) -> BlockPos {
        self.min
    }

    /// The maximum corner.
    #[must_use]
    pub const fn max(&self) -> BlockPos {
        self.max
    }

    /// The inclusive size per axis.
    #[must_use]
    pub const fn size(&self) -> Vector3<i32> {
        Vector3::new(
            self.max.0.x - self.min.0.x + 1,
            self.max.0.y - self.min.0.y + 1,
            self.max.0.z - self.min.0.z + 1,
        )
    }

    /// The total number of blocks in the region.
    ///
    /// Returns `None` when the volume would overflow `u64` (absurd
    /// coordinates), so callers can reject before iterating.
    #[must_use]
    pub fn volume(&self) -> Option<u64> {
        let size = self.size();
        u64::try_from(size.x)
            .ok()?
            .checked_mul(u64::try_from(size.y).ok()?)?
            .checked_mul(u64::try_from(size.z).ok()?)
    }

    /// `true` when `pos` lies inside the cuboid (inclusive bounds).
    #[must_use]
    pub const fn contains(&self, pos: &BlockPos) -> bool {
        pos.0.x >= self.min.0.x
            && pos.0.x <= self.max.0.x
            && pos.0.y >= self.min.0.y
            && pos.0.y <= self.max.0.y
            && pos.0.z >= self.min.0.z
            && pos.0.z <= self.max.0.z
    }

    /// All positions in the region (Y-major iteration like vanilla fill).
    #[must_use]
    pub fn iterate(&self) -> Vec<BlockPos> {
        let mut positions = Vec::new();
        for y in self.min.0.y..=self.max.0.y {
            for z in self.min.0.z..=self.max.0.z {
                for x in self.min.0.x..=self.max.0.x {
                    positions.push(BlockPos(Vector3::new(x, y, z)));
                }
            }
        }
        positions
    }

    /// Grows the region `amount` blocks towards `direction` by moving the
    /// face that faces `direction` (negative amounts contract instead,
    /// matching `WorldEdit`).
    #[must_use]
    pub fn expand(&self, direction: Direction, amount: i32) -> Self {
        let min = self.min.0;
        let max = self.max.0;
        let (min, max) = match direction {
            Direction::North => (Vector3::new(min.x, min.y, min.z - amount), max),
            Direction::South => (min, Vector3::new(max.x, max.y, max.z + amount)),
            Direction::West => (Vector3::new(min.x - amount, min.y, min.z), max),
            Direction::East => (min, Vector3::new(max.x + amount, max.y, max.z)),
            Direction::Up => (min, Vector3::new(max.x, max.y + amount, max.z)),
            Direction::Down => (Vector3::new(min.x, min.y - amount, min.z), max),
        };
        Self::new(BlockPos(min), BlockPos(max))
    }

    /// Shrinks the region `amount` blocks from `direction` towards its
    /// inside (negative amounts expand instead, matching `WorldEdit`).
    #[must_use]
    pub fn contract(&self, direction: Direction, amount: i32) -> Self {
        self.expand(direction, -amount)
    }

    /// Moves the whole region `amount` blocks towards `direction`.
    #[must_use]
    pub const fn shift(&self, direction: Direction, amount: i32) -> Self {
        let off = direction.offset();
        let delta = Vector3::new(off.x * amount, off.y * amount, off.z * amount);
        let min = BlockPos(Vector3::new(
            self.min.0.x + delta.x,
            self.min.0.y + delta.y,
            self.min.0.z + delta.z,
        ));
        let max = BlockPos(Vector3::new(
            self.max.0.x + delta.x,
            self.max.0.y + delta.y,
            self.max.0.z + delta.z,
        ));
        Self { min, max }
    }

    /// Expands the region vertically to the full build height.
    #[must_use]
    pub const fn expand_vertical(&self, bottom_y: i32, top_y: i32) -> Self {
        Self {
            min: BlockPos(Vector3::new(self.min.0.x, bottom_y, self.min.0.z)),
            max: BlockPos(Vector3::new(self.max.0.x, top_y, self.max.0.z)),
        }
    }

    /// Positions on the six walls (faces) of the region.
    #[must_use]
    pub fn walls(&self) -> Vec<BlockPos> {
        let mut positions = Vec::new();
        for y in self.min.0.y..=self.max.0.y {
            for z in self.min.0.z..=self.max.0.z {
                for x in self.min.0.x..=self.max.0.x {
                    let on_wall = x == self.min.0.x
                        || x == self.max.0.x
                        || z == self.min.0.z
                        || z == self.max.0.z;
                    let on_floor_or_ceiling = y == self.min.0.y || y == self.max.0.y;
                    if on_wall || on_floor_or_ceiling {
                        positions.push(BlockPos(Vector3::new(x, y, z)));
                    }
                }
            }
        }
        positions
    }

    /// Positions on the shell of the region (walls, floor and ceiling),
    /// i.e. every block that has a neighbor outside the cuboid.
    #[must_use]
    pub fn outline(&self) -> Vec<BlockPos> {
        self.walls()
    }

    /// Positions on the floor and ceiling only (used by hollow fills).
    #[must_use]
    pub fn faces(&self) -> Vec<BlockPos> {
        let mut positions = Vec::new();
        for z in self.min.0.z..=self.max.0.z {
            for x in self.min.0.x..=self.max.0.x {
                positions.push(BlockPos(Vector3::new(x, self.min.0.y, z)));
                positions.push(BlockPos(Vector3::new(x, self.max.0.y, z)));
            }
        }
        positions
    }

    /// The geometric center block (floored), `WorldEdit` `//center` places a
    /// column here.
    #[must_use]
    pub const fn center(&self) -> BlockPos {
        BlockPos(Vector3::new(
            i32::midpoint(self.min.0.x, self.max.0.x),
            i32::midpoint(self.min.0.y, self.max.0.y),
            i32::midpoint(self.min.0.z, self.max.0.z),
        ))
    }

    /// The positions of a named sub-shape of the region.
    #[must_use]
    pub fn positions_of_kind(&self, kind: RegionKind) -> Vec<BlockPos> {
        match kind {
            RegionKind::All => self.iterate(),
            RegionKind::Walls => self.walls(),
            RegionKind::Outline => self.outline(),
        }
    }
}

/// Which part of a cuboid a command fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// Every block in the region.
    All,
    /// The four side faces.
    Walls,
    /// The whole shell (sides, floor and ceiling).
    Outline,
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: BlockPos = BlockPos(Vector3::new(0, 0, 0));
    const B: BlockPos = BlockPos(Vector3::new(2, 2, 2));

    #[test]
    fn normalizes_corners() {
        let region = Cuboid::new(B, A);
        assert_eq!(region.min(), A);
        assert_eq!(region.max(), B);
    }

    #[test]
    fn volume_counts_inclusive_blocks() {
        assert_eq!(Cuboid::new(A, B).volume(), Some(27));
        assert_eq!(Cuboid::new(A, A).volume(), Some(1));
    }

    #[test]
    fn iterate_yields_every_position() {
        assert_eq!(Cuboid::new(A, B).iterate().len(), 27);
    }

    #[test]
    fn expand_north_moves_min() {
        let region = Cuboid::new(A, B).expand(Direction::North, 5);
        assert_eq!(region.min().0.z, -5);
        assert_eq!(region.max().0.z, 2);
    }

    #[test]
    fn expand_south_moves_max() {
        let region = Cuboid::new(A, B).expand(Direction::South, 5);
        assert_eq!(region.min().0.z, 0);
        assert_eq!(region.max().0.z, 7);
    }

    #[test]
    fn contract_shrinks_region() {
        let region = Cuboid::new(A, B).contract(Direction::North, 1);
        assert_eq!(region.min().0.z, 1);
        assert_eq!(region.max().0.z, 2);
    }

    #[test]
    fn negative_expand_contracts() {
        let region = Cuboid::new(A, B).expand(Direction::South, -1);
        assert_eq!(region.max().0.z, 1);
    }

    #[test]
    fn shift_moves_both_corners() {
        let region = Cuboid::new(A, B).shift(Direction::East, 3);
        assert_eq!(region.min().0.x, 3);
        assert_eq!(region.max().0.x, 5);
    }

    #[test]
    fn walls_exclude_interior() {
        let walls = Cuboid::new(A, B).walls();
        assert_eq!(walls.len(), 26);
        assert!(!walls.contains(&BlockPos(Vector3::new(1, 1, 1))));
    }

    #[test]
    fn yaw_maps_cardinals() {
        assert_eq!(Direction::from_yaw(0.0), Direction::South);
        assert_eq!(Direction::from_yaw(90.0), Direction::West);
        assert_eq!(Direction::from_yaw(180.0), Direction::North);
        assert_eq!(Direction::from_yaw(270.0), Direction::East);
        assert_eq!(Direction::from_yaw(-90.0), Direction::East);
    }

    #[test]
    fn parses_direction_keywords() {
        assert_eq!(Direction::parse("NORTH"), Some(Direction::North));
        assert_eq!(Direction::parse("w"), Some(Direction::West));
        assert_eq!(Direction::parse("sideways"), None);
    }
}
