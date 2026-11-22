//! The VoxelSniper-style `/b` tool.
//!
//! Arrow and gunpowder items driving ball/cylinder/disc shapes, erosion
//! presets, blend ball/disc and erode-blend, with `/v` and `/vr` feeding
//! the pattern and replace mask (ported from `FastAsyncVoxelSniper`).

use std::collections::HashMap;
use std::sync::Arc;

use carrot_worldedit::edit::ensure_chunks;
use carrot_worldedit::mask::Mask;
use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::session::WeState;
use carrot_worldedit::states::is_air;
use pumpkin_core::entity::player::Player;
use pumpkin_core::world::World;
use pumpkin_data::{BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;
use rand::RngExt;

use crate::state::TerrainState;

/// Erosion parameters (face counts and recursion counts).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ErodeParams {
    pub erode_faces: u32,
    pub erode_rec: u32,
    pub fill_faces: u32,
    pub fill_rec: u32,
}

impl ErodeParams {
    /// The reference presets (`ErodeBrush.Preset`).
    #[allow(clippy::missing_const_for_fn)] // String lowering is not const
    #[must_use]
    pub fn preset(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "default" => Some(Self::DEFAULT),
            "melt" => Some(Self::MELT),
            "fill" => Some(Self::FILL),
            "smooth" => Some(Self::SMOOTH),
            "lift" => Some(Self::LIFT),
            "floatclean" => Some(Self::FLOAT_CLEAN),
            _ => None,
        }
    }

    /// Preset with erosion and fill swapped (gunpowder action).
    #[must_use]
    pub const fn inverted(self) -> Self {
        Self {
            erode_faces: self.fill_faces,
            erode_rec: self.fill_rec,
            fill_faces: self.erode_faces,
            fill_rec: self.erode_rec,
        }
    }

    pub const DEFAULT: Self = Self {
        erode_faces: 0,
        erode_rec: 1,
        fill_faces: 0,
        fill_rec: 1,
    };
    pub const MELT: Self = Self {
        erode_faces: 2,
        erode_rec: 1,
        fill_faces: 5,
        fill_rec: 1,
    };
    pub const FILL: Self = Self {
        erode_faces: 5,
        erode_rec: 1,
        fill_faces: 2,
        fill_rec: 1,
    };
    pub const SMOOTH: Self = Self {
        erode_faces: 3,
        erode_rec: 1,
        fill_faces: 3,
        fill_rec: 1,
    };
    pub const LIFT: Self = Self {
        erode_faces: 6,
        erode_rec: 0,
        fill_faces: 1,
        fill_rec: 1,
    };
    pub const FLOAT_CLEAN: Self = Self {
        erode_faces: 6,
        erode_rec: 1,
        fill_faces: 6,
        fill_rec: 1,
    };
}

/// The selected `/b` brush.
#[derive(Clone, Debug)]
pub enum SniperBrush {
    /// No brush bound.
    None,
    /// Solid sphere (`ball`, alias `b`).
    Ball,
    /// Vertical cylinder (`cylinder`, alias `c`).
    Cylinder,
    /// Flat disc (`disc`, alias `d`).
    Disc,
    /// Erosion (`erode`, alias `e`).
    Erode(ErodeParams),
    /// Erode then blend (`erode_blend`, alias `eb`).
    ErodeBlend(ErodeParams),
    /// 26-neighbor mode blend (`blend_ball`, alias `bb`).
    BlendBall,
    /// 8-neighbor disc blend (`blend_disc`, alias `bd`).
    BlendDisc,
    /// Ball with a random radius per snipe (`/brandom`).
    RandomBall { min: u32, max: u32 },
}

impl SniperBrush {
    /// Short label for feedback.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::None => "nenhum",
            Self::Ball => "ball",
            Self::Cylinder => "cylinder",
            Self::Disc => "disc",
            Self::Erode(_) => "erode",
            Self::ErodeBlend(_) => "erode_blend",
            Self::BlendBall => "blend_ball",
            Self::BlendDisc => "blend_disc",
            Self::RandomBall { .. } => "random ball",
        }
    }
}

/// Per-player `VoxelSniper` state.
pub struct SniperState {
    /// Selected brush.
    pub brush: SniperBrush,
    /// Brush radius.
    pub size: u32,
    /// `/v` pattern.
    pub pattern: Pattern,
    /// `/vr` replace restriction.
    pub replace: Option<Mask>,
    /// `trueCircle` (radius + 0.5).
    pub true_circle: bool,
    /// Cylinder height.
    pub voxel_height: i32,
    /// Cylinder center offset.
    pub cylinder_center: i32,
    /// Blend brushes exclude water from votes.
    pub water_excluded: bool,
}

impl Default for SniperState {
    fn default() -> Self {
        Self::new()
    }
}

impl SniperState {
    /// Defaults mirror FAVS: air pattern, radius 3, blend excludes water.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            brush: SniperBrush::None,
            size: 3,
            pattern: Pattern::Single(BlockStateId::AIR),
            replace: None,
            true_circle: false,
            voxel_height: 1,
            cylinder_center: 0,
            water_excluded: true,
        }
    }
}

/// Which item triggered the snipe (`arrow` or gunpowder).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnipeAction {
    /// Arrow item.
    Arrow,
    /// Gunpowder item.
    Gunpowder,
}

const fn is_water(state: BlockStateId) -> bool {
    state.to_state().is_liquid()
}

#[allow(clippy::missing_const_for_fn)] // float casts are not const
fn effective_radius(size: u32, true_circle: bool) -> f64 {
    f64::from(size) + f64::from(true_circle) * 0.5
}

/// Runs the player's sniper brush at `target`.
#[allow(clippy::too_many_lines)] // dispatch over every sniper brush
pub async fn snipe(
    terrain: &Arc<TerrainState>,
    player: &Arc<Player>,
    action: SnipeAction,
    target: BlockPos,
) {
    let (brush, size, pattern, replace, true_circle, voxel_height, cylinder_center, water_excluded) = {
        let state = terrain.player(player.gameprofile.id);
        let guard = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            guard.sniper.brush.clone(),
            guard.sniper.size,
            guard.sniper.pattern.clone(),
            guard.sniper.replace.clone(),
            guard.sniper.true_circle,
            guard.sniper.voxel_height,
            guard.sniper.cylinder_center,
            guard.sniper.water_excluded,
        )
    };

    let world = player.world();
    let snapshot = crate::state::WeSnapshot::capture(&terrain.we, player.gameprofile.id);

    // Random radius per snipe for the /brandom alias.
    let size = match &brush {
        SniperBrush::RandomBall { min, max } => {
            let mut rng = rand::rng();
            rng.random_range(*min..=(*max).max(*min))
        }
        _ => size,
    };

    let description = match &brush {
        SniperBrush::None => {
            player.send_system_message(&TextComponent::text(
                "Nenhum brush de sniper selecionado (/b <brush>).".to_string(),
            ));
            return;
        }
        other => format!("Snipe {} r={}", other.label(), size),
    };

    let mut mask = replace;
    let mut block_entities = HashMap::new();
    let desired = match &brush {
        SniperBrush::None => Vec::new(),
        SniperBrush::Ball | SniperBrush::RandomBall { .. } => {
            ball_desired(&snapshot, &pattern, target, size, true_circle)
        }
        SniperBrush::Cylinder => cylinder_desired(
            &snapshot,
            &pattern,
            target,
            size,
            voxel_height,
            cylinder_center,
            true_circle,
        ),
        SniperBrush::Disc => disc_desired(&snapshot, &pattern, target, size, true_circle),
        SniperBrush::Erode(params) => {
            let params = if action == SnipeAction::Gunpowder {
                params.inverted()
            } else {
                *params
            };
            mask = None;
            crate::fawe_brushes::erode(
                terrain,
                &world,
                target,
                size,
                params.erode_faces,
                params.erode_rec,
                params.fill_faces,
                params.fill_rec,
                true,
            )
            .await
        }
        SniperBrush::ErodeBlend(params) => {
            let params = if action == SnipeAction::Gunpowder {
                params.inverted()
            } else {
                *params
            };
            mask = None;
            let eroded = crate::fawe_brushes::erode(
                terrain,
                &world,
                target,
                size,
                params.erode_faces,
                params.erode_rec,
                params.fill_faces,
                params.fill_rec,
                true,
            )
            .await;
            let blended = blend_desired(
                terrain,
                &world,
                target,
                size,
                BlendShape::Ball,
                false,
                water_excluded,
            )
            .await;
            let mut merged: HashMap<BlockPos, BlockStateId> = HashMap::new();
            for (pos, state) in eroded.into_iter().chain(blended) {
                merged.insert(pos, state);
            }
            merged.into_iter().collect()
        }
        SniperBrush::BlendBall => {
            mask = None;
            blend_desired(
                terrain,
                &world,
                target,
                size,
                BlendShape::Ball,
                action == SnipeAction::Gunpowder,
                water_excluded,
            )
            .await
        }
        SniperBrush::BlendDisc => {
            mask = None;
            blend_desired(
                terrain,
                &world,
                target,
                size,
                BlendShape::Disc,
                action == SnipeAction::Gunpowder,
                water_excluded,
            )
            .await
        }
    };
    let _ = &mut block_entities;

    let request = carrot_worldedit::edit::EditRequest {
        desired,
        mask,
        block_entities,
        description,
    };
    carrot_worldedit::edit::execute(&terrain.we, player, request).await;
}

fn ball_desired(
    snapshot: &crate::state::WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    size: u32,
    true_circle: bool,
) -> Vec<(BlockPos, BlockStateId)> {
    let radius = effective_radius(size, true_circle);
    let squared = radius * radius;
    let r = size as i32;
    let mut desired = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                if f64::from(dx * dx + dy * dy + dz * dz) > squared {
                    continue;
                }
                let pos = BlockPos::new(target.0.x + dx, target.0.y + dy, target.0.z + dz);
                if let Some(state) = snapshot.apply_pattern(pattern, &pos) {
                    desired.push((pos, state));
                }
            }
        }
    }
    desired
}

#[allow(clippy::too_many_arguments)]
fn cylinder_desired(
    snapshot: &crate::state::WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    size: u32,
    voxel_height: i32,
    cylinder_center: i32,
    true_circle: bool,
) -> Vec<(BlockPos, BlockStateId)> {
    let radius = effective_radius(size, true_circle);
    let squared = radius * radius;
    let r = size as i32;
    let start = target.0.y + cylinder_center;
    let end = start + voxel_height;
    let (top, bottom) = if end < start {
        (start, start)
    } else {
        (end, start)
    };
    let mut desired = Vec::new();
    for y in bottom..=top {
        for dx in -r..=r {
            for dz in -r..=r {
                if f64::from(dx * dx + dz * dz) > squared {
                    continue;
                }
                let pos = BlockPos::new(target.0.x + dx, y, target.0.z + dz);
                if let Some(state) = snapshot.apply_pattern(pattern, &pos) {
                    desired.push((pos, state));
                }
            }
        }
    }
    desired
}

fn disc_desired(
    snapshot: &crate::state::WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    size: u32,
    true_circle: bool,
) -> Vec<(BlockPos, BlockStateId)> {
    let radius = effective_radius(size, true_circle);
    let squared = radius * radius;
    let r = size as i32;
    let mut desired = Vec::new();
    for dx in -r..=r {
        for dz in -r..=r {
            if f64::from(dx * dx + dz * dz) > squared {
                continue;
            }
            let pos = BlockPos::new(target.0.x + dx, target.0.y, target.0.z + dz);
            if let Some(state) = snapshot.apply_pattern(pattern, &pos) {
                desired.push((pos, state));
            }
        }
    }
    desired
}

/// Which neighborhood the blend votes over.
#[derive(Clone, Copy)]
enum BlendShape {
    /// Sphere volume with 26 neighbors.
    Ball,
    /// Flat disc with 8 neighbors.
    Disc,
}

#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)] // literal port of FAVS's blend brushes
async fn blend_desired(
    terrain: &Arc<TerrainState>,
    world: &Arc<World>,
    target: BlockPos,
    size: u32,
    shape: BlendShape,
    air_excluded: bool,
    water_excluded: bool,
) -> Vec<(BlockPos, BlockStateId)> {
    let _ = terrain;
    let radius = effective_radius(size, false);
    let squared = radius * radius;
    let r = size as i32;

    let mut region: Vec<BlockPos> = Vec::new();
    for x in -r - 1..=r + 1 {
        for y in -r - 1..=r + 1 {
            for z in -r - 1..=r + 1 {
                region.push(BlockPos::new(
                    target.0.x + x,
                    target.0.y + y,
                    target.0.z + z,
                ));
            }
        }
    }
    ensure_chunks(world, &region, |pos: &BlockPos| pos).await;

    let passes = |candidate: BlockStateId, air_excluded: bool, water_excluded: bool| {
        if air_excluded && is_air(candidate) {
            return false;
        }
        !(water_excluded && is_water(candidate))
    };

    let neighbor_offsets: Vec<(i32, i32, i32)> = match shape {
        BlendShape::Ball => (-1..=1)
            .flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| (x, y, z))))
            .filter(|(x, y, z)| !(*x == 0 && *y == 0 && *z == 0))
            .collect(),
        BlendShape::Disc => (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .filter(|(x, z)| !(*x == 0 && *z == 0))
            .map(|(x, z)| (x, 0, z))
            .collect(),
    };

    let in_shape = |dx: i32, dy: i32, dz: i32| match shape {
        BlendShape::Ball => f64::from(dx * dx + dy * dy + dz * dz) <= squared,
        BlendShape::Disc => dy == 0 && f64::from(dx * dx + dz * dz) <= squared,
    };

    let mut desired = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                if !in_shape(dx, dy, dz) {
                    continue;
                }
                let pos = BlockPos::new(target.0.x + dx, target.0.y + dy, target.0.z + dz);
                let current = world.get_block_state_id(&pos);
                let mut frequency: HashMap<BlockId, (u32, BlockStateId)> = HashMap::new();
                for (ox, oy, oz) in &neighbor_offsets {
                    let neighbor = BlockPos::new(pos.0.x + ox, pos.0.y + oy, pos.0.z + oz);
                    let state = world.get_block_state_id(&neighbor);
                    if !passes(state, air_excluded, water_excluded) {
                        continue;
                    }
                    let entry = frequency.entry(state.to_block_id()).or_insert((0, state));
                    entry.0 += 1;
                }
                let mut best_count = 0;
                let mut best_state = current;
                for (count, state) in frequency.values() {
                    // Strict `>` keeps the first entry on ties, like FAVS.
                    if *count > best_count {
                        best_count = *count;
                        best_state = *state;
                    }
                }
                if best_count > 0 && best_state.to_block_id() != current.to_block_id() {
                    desired.push((pos, best_state));
                }
            }
        }
    }
    desired
}

/// Applies the sniper pattern restrictions shared by `/v` and `/vr` (the
/// `/v` with no argument picks the targeted block state).
pub fn pattern_from_target(we: &Arc<WeState>, player: &Arc<Player>) -> Option<Pattern> {
    let _ = we;
    let world = player.world();
    player
        .get_target_block(&world, 256.0)
        .map(|pos| Pattern::Single(world.get_block_state_id(&pos)))
}
