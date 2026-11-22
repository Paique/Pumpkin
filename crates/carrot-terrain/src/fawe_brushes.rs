//! FAWE brush algorithms ported 1:1 from the reference sources.
//!
//! Shapes, `WorldEdit`'s Gaussian heightmap smoothing, erosion, blend
//! ball, the distorted blob and the shatter frontier brush
//! (`BrushCommands`, `HeightMap` convolution, `ErodeBrush`, `BlendBall`,
//! `EditSession.makeBlob`, `ShatterBrush`).

use std::collections::HashMap;
use std::sync::Arc;

use carrot_worldedit::edit::ensure_chunks;
use carrot_worldedit::mask::Mask;
use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::states::is_air;
use pumpkin_core::world::World;
use pumpkin_data::{BlockId, BlockStateId};
use pumpkin_util::math::position::BlockPos;

use crate::noise::Simplex;
use rand::RngExt;

use crate::state::{TerrainState, WeSnapshot};

type Desired = Vec<(BlockPos, BlockStateId)>;

/// A block that blocks movement (the reference's `isMovementBlocker`).
const fn solid(state: BlockStateId) -> bool {
    state.to_state().is_solid()
}

/// The six cardinal offsets (erosion faces).
const FACES: [(i32, i32, i32); 6] = [
    (0, 0, 1),
    (0, 0, -1),
    (0, 1, 0),
    (0, -1, 0),
    (1, 0, 0),
    (-1, 0, 0),
];

/// Positions of a filled (or hollow) integer sphere around `center`.
fn sphere_positions(center: BlockPos, radius: u32, hollow: bool) -> Vec<BlockPos> {
    let r = radius as i64;
    let r2 = r * r;
    let inner = (r - 1).max(0) * (r - 1).max(0);
    let mut positions = Vec::new();
    for dx in -r..=r {
        for dy in -r..=r {
            for dz in -r..=r {
                let dist = dx * dx + dy * dy + dz * dz;
                if dist > r2 || (hollow && dist < inner) {
                    continue;
                }
                positions.push(BlockPos::new(
                    center.0.x + dx as i32,
                    center.0.y + dy as i32,
                    center.0.z + dz as i32,
                ));
            }
        }
    }
    positions
}

/// `//br sphere` — filled or hollow sphere of a pattern.
#[must_use]
pub fn sphere(
    snapshot: &WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    radius: u32,
    hollow: bool,
) -> Desired {
    sphere_positions(target, radius, hollow)
        .into_iter()
        .filter_map(|pos| {
            snapshot
                .apply_pattern(pattern, &pos)
                .map(|state| (pos, state))
        })
        .collect()
}

/// `//br cylinder` — disc of radius `radius` rising `height` blocks from
/// the target; hollow keeps the wall ring plus caps.
#[allow(clippy::too_many_arguments)]
#[must_use]
pub fn cylinder(
    snapshot: &WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    radius: u32,
    height: i32,
    hollow: bool,
) -> Desired {
    let r = radius as i64;
    let r2 = r * r;
    let inner = (r - 1).max(0) * (r - 1).max(0);
    let steps = height.max(1);
    let mut desired = Vec::new();
    for dy in 0..steps {
        for dx in -r..=r {
            for dz in -r..=r {
                let dist = dx * dx + dz * dz;
                let ring = dist <= r2 && dist >= inner;
                let cap = dy == 0 || dy == steps - 1;
                if dist > r2 || (hollow && !ring && !cap) {
                    continue;
                }
                let pos = BlockPos::new(
                    target.0.x + dx as i32,
                    target.0.y + dy,
                    target.0.z + dz as i32,
                );
                if let Some(state) = snapshot.apply_pattern(pattern, &pos) {
                    desired.push((pos, state));
                }
            }
        }
    }
    desired
}

/// Highest y in `[min_y, max_y]` matching `mask` (or non-air without a
/// mask), like `WorldEdit`'s `getHighestTerrainBlock`.
fn highest_terrain(
    world: &World,
    x: i32,
    z: i32,
    min_y: i32,
    max_y: i32,
    mask: Option<&Mask>,
) -> i32 {
    for y in (min_y..=max_y).rev() {
        let state = world.get_block_state_id(&BlockPos::new(x, y, z));
        let matches = mask.map_or_else(|| !is_air(state), |mask| mask.test(state));
        if matches {
            return y;
        }
    }
    min_y
}

/// Gaussian kernel (radius 2, sigma 1) — `WorldEdit`'s `SmoothBrush`.
const SMOOTH_KERNEL_RADIUS: i32 = 2;

fn gaussian_kernel(radius: i32, sigma: f64) -> Vec<Vec<f64>> {
    let diameter = (radius * 2 + 1) as usize;
    let sigma22 = 2.0 * sigma * sigma;
    let constant = std::f64::consts::PI * sigma22;
    let mut data = vec![vec![0.0; diameter]; diameter];
    for (index_y, row) in data.iter_mut().enumerate() {
        for (index_x, cell) in row.iter_mut().enumerate() {
            let y = index_y as i32 - radius;
            let x = index_x as i32 - radius;
            *cell = (-(f64::from(x * x + y * y)) / sigma22).exp() / constant;
        }
    }
    data
}

/// `//br smooth` — iterate a normalized 5x5 Gaussian over the region
/// heightmap and regrow/trim each column to the smoothed height.
#[allow(clippy::too_many_lines)] // literal port of WorldEdit's convolution
pub async fn smooth(
    terrain: &Arc<TerrainState>,
    world: &Arc<World>,
    target: BlockPos,
    radius: u32,
    iterations: u32,
    mask: Option<&Mask>,
) -> Desired {
    let size = radius as i32;
    let min = BlockPos::new(target.0.x - size, target.0.y - size, target.0.z - size);
    let max = BlockPos::new(target.0.x + size, target.0.y + size + 10, target.0.z + size);
    let width = (max.0.x - min.0.x + 1).max(1) as usize;
    let length = (max.0.z - min.0.z + 1).max(1) as usize;

    let mut columns: Vec<BlockPos> = Vec::with_capacity(width * length);
    for z in min.0.z..=max.0.z {
        for x in min.0.x..=max.0.x {
            columns.push(BlockPos::new(x, target.0.y, z));
        }
    }
    ensure_chunks(world, &columns, |pos: &BlockPos| pos).await;

    let _ = terrain;
    // Current heightmap.
    let mut heights = vec![0i32; width * length];
    for (index, column) in columns.iter().enumerate() {
        heights[index] = highest_terrain(world, column.0.x, column.0.z, min.0.y, max.0.y, mask);
    }

    // Iterated normalized convolution with border clamping.
    let kernel = gaussian_kernel(SMOOTH_KERNEL_RADIUS, 1.0);
    let mut data = heights.clone();
    for _ in 0..iterations.max(1) {
        let mut next = vec![0i32; data.len()];
        for z in 0..length {
            for x in 0..width {
                let mut total = 0.0;
                let mut weight = 0.0;
                for (ky, krow) in kernel.iter().enumerate() {
                    let sy = z as i32 + ky as i32 - SMOOTH_KERNEL_RADIUS;
                    if sy < 0 || sy >= length as i32 {
                        continue;
                    }
                    for (kx, kcell) in krow.iter().enumerate() {
                        let sx = x as i32 + kx as i32 - SMOOTH_KERNEL_RADIUS;
                        if sx < 0 || sx >= width as i32 {
                            continue;
                        }
                        total += f64::from(data[sy as usize * width + sx as usize]) * kcell;
                        weight += kcell;
                    }
                }
                let value = if weight > 0.0 { total / weight } else { 0.0 };
                next[z * width + x] = (value + 0.5).floor() as i32;
            }
        }
        data = next;
    }

    // Apply the new heights (WorldEdit `HeightMap.apply` semantics).
    let mut desired: Desired = Vec::new();
    for (index, column) in columns.iter().enumerate() {
        let cur = heights[index];
        let new = data[index].min(max.0.y);
        let (x, z) = (column.0.x, column.0.z);
        if new > cur {
            let existing = world.get_block_state_id(&BlockPos::new(x, cur, z));
            if !solid(existing) {
                continue;
            }
            let mut tmp = BlockStateId::AIR;
            let mut set_y = new - 1;
            let mut get_y = cur - 1;
            while set_y >= cur {
                let get = world.get_block_state_id(&BlockPos::new(x, get_y, z));
                if !is_air(get) {
                    tmp = get;
                }
                desired.push((BlockPos::new(x, set_y, z), tmp));
                set_y -= 1;
                get_y -= 1;
            }
            desired.push((BlockPos::new(x, new, z), existing));
        } else if cur > new {
            // The reference shifts the surface segment above `new` down
            // to the region floor before trimming; reproduced verbatim.
            let mut set_y = min.0.y;
            let mut get_y = new;
            while set_y <= new {
                desired.push((
                    BlockPos::new(x, set_y, z),
                    world.get_block_state_id(&BlockPos::new(x, get_y, z)),
                ));
                set_y += 1;
                get_y += 1;
            }
            let existing = world.get_block_state_id(&BlockPos::new(x, cur, z));
            desired.push((BlockPos::new(x, new, z), existing));
            for y in (new + 1)..=cur {
                desired.push((BlockPos::new(x, y, z), BlockStateId::AIR));
            }
        }
    }
    desired
}

/// `//br erode` — FAWE's double-buffered erosion.
///
/// Solids with enough non-solid neighbors crumble to the most common
/// neighbor state, then holes with enough solid neighbors fill the same
/// way. With `erode_air` (`VoxelSniper` semantics) eroded blocks become
/// air.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)] // literal port of FAWE's double-buffered erosion
pub async fn erode(
    terrain: &Arc<TerrainState>,
    world: &Arc<World>,
    target: BlockPos,
    radius: u32,
    erode_faces: u32,
    erode_rec: u32,
    fill_faces: u32,
    fill_rec: u32,
    erode_air: bool,
) -> Desired {
    let r = radius as i32;
    let size = (radius as i64) * (radius as i64);
    let side = (r * 2 + 1) as usize;

    let mut region: Vec<BlockPos> = Vec::with_capacity(side * side * side);
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                region.push(BlockPos::new(
                    target.0.x + x,
                    target.0.y + y,
                    target.0.z + z,
                ));
            }
        }
    }
    ensure_chunks(world, &region, |pos: &BlockPos| pos).await;
    let _ = terrain;

    let index = |x: i32, y: i32, z: i32| -> usize {
        ((x + r) as usize * side + (y + r) as usize) * side + (z + r) as usize
    };
    let initial: Vec<BlockStateId> = region
        .iter()
        .map(|pos| world.get_block_state_id(pos))
        .collect();
    let mut buffer_a = initial.clone();
    let mut buffer_b = buffer_a.clone();

    let inside =
        |x: i32, y: i32, z: i32| x >= -r && x <= r && y >= -r && y <= r && z >= -r && z <= r;

    for iteration in 0..(erode_rec + fill_rec) {
        let eroding = iteration < erode_rec;
        let (current, next): (&Vec<BlockStateId>, &mut Vec<BlockStateId>) = if iteration % 2 == 0 {
            (&buffer_a, &mut buffer_b)
        } else {
            (&buffer_b, &mut buffer_a)
        };
        next.copy_from_slice(current);
        let needed = if eroding { erode_faces } else { fill_faces } as i32;

        for x in -r..=r {
            for y in -r..=r {
                for z in -r..=r {
                    if (i64::from(x * x) + i64::from(y * y) + i64::from(z * z)) >= size {
                        continue;
                    }
                    let here = current[index(x, y, z)];
                    let here_solid = solid(here);
                    if eroding != here_solid {
                        // Erosion works on solids, fill on gaps.
                        continue;
                    }
                    let mut frequency: HashMap<BlockId, u32> = HashMap::new();
                    let mut total = 0;
                    let mut highest_state = here;
                    let mut highest = 0;
                    for (fx, fy, fz) in FACES {
                        let (nx, ny, nz) = (x + fx, y + fy, z + fz);
                        if !inside(nx, ny, nz) {
                            continue;
                        }
                        let neighbor = current[index(nx, ny, nz)];
                        if solid(neighbor) == here_solid {
                            continue;
                        }
                        total += 1;
                        let key = neighbor.to_block_id();
                        let count = frequency.entry(key).or_insert(0);
                        *count += 1;
                        if *count > highest {
                            highest = *count;
                            highest_state = neighbor;
                        }
                    }
                    let replacement = if eroding && erode_air {
                        BlockStateId::AIR
                    } else {
                        highest_state
                    };
                    if total >= needed && replacement != here {
                        next[index(x, y, z)] = replacement;
                    }
                }
            }
        }
    }

    let final_buffer = if (erode_rec + fill_rec).is_multiple_of(2) {
        &buffer_a
    } else {
        &buffer_b
    };
    let mut desired = Desired::new();
    for (offset, pos) in region.iter().enumerate() {
        if final_buffer[offset] != initial[offset] {
            desired.push((*pos, final_buffer[offset]));
        }
    }
    desired
}

/// `//br blendball` — every block inside the sphere becomes the most
/// frequent block type among its 26 neighbors (frequency difference and
/// air-only comparison configurable).
#[allow(clippy::too_many_lines)] // literal port of FAWE's BlendBall filter
pub async fn blend_ball(
    terrain: &Arc<TerrainState>,
    world: &Arc<World>,
    target: BlockPos,
    radius: u32,
    min_freq_diff: u32,
    only_air: bool,
) -> Desired {
    let r = radius as i32;
    let sphere = sphere_positions(target, radius, false);
    let mut cube: Vec<BlockPos> = Vec::with_capacity(((2 * r + 3) as usize).pow(3));
    for x in -r - 1..=r + 1 {
        for y in -r - 1..=r + 1 {
            for z in -r - 1..=r + 1 {
                cube.push(BlockPos::new(
                    target.0.x + x,
                    target.0.y + y,
                    target.0.z + z,
                ));
            }
        }
    }
    ensure_chunks(world, &cube, |pos: &BlockPos| pos).await;
    let _ = terrain;

    let states = carrot_worldedit::edit::read_states(world, &cube);
    let lookup: HashMap<BlockPos, BlockStateId> = cube.iter().copied().zip(states).collect();

    let mut desired = Desired::new();
    for pos in &sphere {
        // The histogram below reads far better with let-else.
        #[allow(clippy::option_if_let_else)]
        let Some(&current) = lookup.get(pos) else {
            continue;
        };

        if only_air {
            // Binary vote: air versus existing blocks (self included).
            let current_is_air = is_air(current);
            let mut air = u32::from(current_is_air);
            let mut existing = u32::from(!current_is_air);
            let mut representative = if current_is_air {
                BlockStateId::AIR
            } else {
                current
            };
            for ox in -1..=1 {
                for oy in -1..=1 {
                    for oz in -1..=1 {
                        if ox == 0 && oy == 0 && oz == 0 {
                            continue;
                        }
                        let neighbor = BlockPos::new(pos.0.x + ox, pos.0.y + oy, pos.0.z + oz);
                        let Some(&state) = lookup.get(&neighbor) else {
                            continue;
                        };
                        if is_air(state) {
                            air += 1;
                        } else {
                            existing += 1;
                            if current_is_air || is_air(representative) {
                                representative = state;
                            }
                        }
                    }
                }
            }
            let (highest, highest_state) = match air.cmp(&existing) {
                std::cmp::Ordering::Greater => (air, BlockStateId::AIR),
                std::cmp::Ordering::Less => (existing, representative),
                // A tie keeps the current block.
                std::cmp::Ordering::Equal => continue,
            };
            if highest >= min_freq_diff.max(1) && highest_state != current {
                desired.push((*pos, highest_state));
            }
            continue;
        }

        let mut frequency: HashMap<BlockId, (u32, BlockStateId)> = HashMap::new();
        let current_key = current.to_block_id();
        let mut current_count = 1;
        for ox in -1..=1 {
            for oy in -1..=1 {
                for oz in -1..=1 {
                    if ox == 0 && oy == 0 && oz == 0 {
                        continue;
                    }
                    let neighbor = BlockPos::new(pos.0.x + ox, pos.0.y + oy, pos.0.z + oz);
                    let Some(&state) = lookup.get(&neighbor) else {
                        continue;
                    };
                    if state.to_block_id() == current_key {
                        current_count += 1;
                    }
                    let entry = frequency.entry(state.to_block_id()).or_insert((0, state));
                    entry.0 += 1;
                }
            }
        }

        let mut highest = 1;
        let mut highest_state = current;
        let mut tie = false;
        for (count, state) in frequency.values() {
            if count.saturating_sub(highest) >= min_freq_diff {
                highest = *count;
                highest_state = *state;
                tie = false;
            } else if *count == highest {
                tie = true;
            }
        }
        if !tie
            && highest_state != current
            && highest.saturating_sub(current_count) >= min_freq_diff
        {
            desired.push((*pos, highest_state));
        }
    }
    desired
}

/// `//br rock/blob` — `EditSession.makeBlob` port (both the spherical
/// and the manhattan/rotation paths, quirks included).
#[allow(clippy::too_many_arguments)]
pub fn blob(
    snapshot: &WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    radius: u32,
    radii: [f64; 3],
    sphericity: f64,
    frequency: f64,
    amplitude: f64,
) -> Desired {
    let mut rng = rand::rng();
    let seed_x: f64 = rng.random();
    let seed_y: f64 = rng.random();
    let seed_z: f64 = rng.random();

    let size = f64::from(radius);
    let distort = frequency / size;
    let max_radius = radii
        .iter()
        .copied()
        .fold(0.0f64, f64::max)
        .max(f64::EPSILON);
    let mods = [
        max_radius / radii[0].max(f64::EPSILON),
        max_radius / radii[1].max(f64::EPSILON),
        max_radius / radii[2].max(f64::EPSILON),
    ];
    let radius_sqr = size * size;
    let size_int = radius as i32 * 2;
    let noise = Simplex::new((seed_x * 65536.0) as i64 ^ (seed_y * 65536.0) as i64);
    // The non-spherical path rotates once per brush use, not per block.
    let rotation = if sphericity == 1.0 {
        (0.0, 0.0, 0.0)
    } else {
        (
            f64::from(rand::rng().random_range(0..360)),
            f64::from(rand::rng().random_range(0..360)),
            f64::from(rand::rng().random_range(0..360)),
        )
    };
    let man_scale = [
        1.25 + seed_x * 0.5,
        1.25 + seed_y * 0.5,
        1.25 + seed_z * 0.5,
    ];

    let mut desired = Desired::new();
    for x in -size_int..=size_int {
        for y in -size_int..=size_int {
            for z in -size_int..=size_int {
                let pos = BlockPos::new(target.0.x + x, target.0.y + y, target.0.z + z);
                let inside = if sphericity == 1.0 {
                    let distance = f64::from(x * x) * mods[0]
                        + f64::from(y * y) * mods[1]
                        + f64::from(z * z) * mods[2];
                    let sample = amplitude
                        * noise.noise3(
                            seed_x + f64::from(x) * distort,
                            seed_y + f64::from(y) * distort,
                            seed_z + f64::from(z) * distort,
                        );
                    distance + distance * sample < radius_sqr
                } else {
                    // Random rotation, then manhattan/ellipsoid blend.
                    let (rx, ry, rz) = rotation;
                    let (px, py, pz) =
                        rotate3(f64::from(x), f64::from(y), f64::from(z), rx, ry, rz);
                    let (px, py, pz) = (px.round() as i32, py.round() as i32, pz.round() as i32);
                    let x_scaled = f64::from(px.abs()) * mods[0];
                    let y_scaled = f64::from(py.abs()) * mods[1];
                    let z_scaled = f64::from(pz.abs()) * mods[2];
                    let man_dist = x_scaled + y_scaled + z_scaled;
                    let dist_sqr = f64::from(px * px) * mods[0]
                        + f64::from(py * py) * mods[1]
                        + f64::from(pz * pz) * mods[2];
                    let roughness = 1.0 - sphericity;
                    let distance = dist_sqr.sqrt() * sphericity
                        + man_dist
                            .max(x_scaled * man_scale[0])
                            .max(y_scaled * man_scale[1])
                            .max(z_scaled * man_scale[2])
                            * roughness;
                    let sample = amplitude
                        * noise.noise3(
                            seed_x + f64::from(x) * distort,
                            // The reference samples seedZ twice here.
                            seed_z + f64::from(z) * distort,
                            seed_z + f64::from(z) * distort,
                        );
                    distance + distance * sample < f64::from(radius)
                };
                if inside && let Some(state) = snapshot.apply_pattern(pattern, &pos) {
                    desired.push((pos, state));
                }
            }
        }
    }
    desired
}

/// Euler rotation in X, then Y, then Z order (degrees).
fn rotate3(x: f64, y: f64, z: f64, ax: f64, ay: f64, az: f64) -> (f64, f64, f64) {
    let rad = |deg: f64| deg.to_radians();
    // X
    let (sin, cos) = rad(ax).sin_cos();
    let (y, z) = (y * cos - z * sin, y * sin + z * cos);
    // Y
    let (sin, cos) = rad(ay).sin_cos();
    let (x, z) = (x * cos + z * sin, -x * sin + z * cos);
    // Z
    let (sin, cos) = rad(az).sin_cos();
    let (x, y) = (x * cos - y * sin, x * sin + y * cos);
    (x, y, z)
}

/// The 26 cube offsets sorted by length (`BreadthFirstSearch` order).
fn diagonal_directions() -> Vec<(i32, i32, i32)> {
    let mut dirs: Vec<(i32, i32, i32)> = Vec::with_capacity(26);
    for x in -1..=1 {
        for y in -1..=1 {
            for z in -1..=1 {
                if x != 0 || y != 0 || z != 0 {
                    dirs.push((x, y, z));
                }
            }
        }
    }
    dirs.sort_by_key(|(x, y, z)| x * x + y * y + z * z);
    dirs
}

/// `//br shatter` — scatter seeds over the surface, then grow competing
/// frontiers that paint the pattern wherever they collide.
#[allow(clippy::too_many_lines)] // literal port of FAWE's frontier brush
pub async fn shatter(
    snapshot: &WeSnapshot,
    terrain: &Arc<TerrainState>,
    world: &Arc<World>,
    pattern: &Pattern,
    target: BlockPos,
    radius: u32,
    count: u32,
) -> Desired {
    let r = radius as i32;
    let radius2 = (radius as i64) * (radius as i64);
    let directions = diagonal_directions();
    let mut cube: Vec<BlockPos> = Vec::new();
    for x in -r..=r {
        for y in -r..=r {
            for z in -r..=r {
                cube.push(BlockPos::new(
                    target.0.x + x,
                    target.0.y + y,
                    target.0.z + z,
                ));
            }
        }
    }
    ensure_chunks(world, &cube, |pos: &BlockPos| pos).await;
    let _ = terrain;

    let state_of = |pos: &BlockPos| world.get_block_state_id(pos);
    let is_surface = |pos: &BlockPos| {
        let above = BlockPos::new(pos.0.x, pos.0.y + 1, pos.0.z);
        solid(state_of(pos)) && !solid(state_of(&above))
    };
    let within = |pos: &BlockPos| {
        let dx = i64::from(pos.0.x - target.0.x);
        let dy = i64::from(pos.0.y - target.0.y);
        let dz = i64::from(pos.0.z - target.0.z);
        dx * dx + dy * dy + dz * dz <= radius2
    };

    // Surface BFS from the click.
    let mut visited: std::collections::HashSet<BlockPos> = std::collections::HashSet::new();
    let mut queue = std::collections::VecDeque::new();
    if is_surface(&target) {
        visited.insert(target);
        queue.push_back(target);
    }
    while let Some(pos) = queue.pop_front() {
        for (dx, dy, dz) in &directions {
            let next = BlockPos::new(pos.0.x + dx, pos.0.y + dy, pos.0.z + dz);
            if !within(&next) || visited.contains(&next) {
                continue;
            }
            if is_surface(&next) {
                visited.insert(next);
                queue.push_back(next);
            }
        }
    }

    let surface: Vec<BlockPos> = visited.into_iter().collect();
    if surface.is_empty() {
        return Vec::new();
    }

    // Scatter seeds (minimum separation of 1, like ScatterBrush).
    let mut rng = rand::rng();
    let mut placed: HashMap<BlockPos, usize> = HashMap::new();
    let mut seeds: Vec<BlockPos> = Vec::new();
    let mut fails = 0i32;
    while seeds.len() < count as usize && fails < 1000 {
        let candidate = surface[rng.random_range(0..surface.len())];
        let conflicts = placed.keys().any(|other| {
            (other.0.x - candidate.0.x).pow(2)
                + (other.0.y - candidate.0.y).pow(2)
                + (other.0.z - candidate.0.z).pow(2)
                <= 1
        });
        if conflicts {
            fails += 1;
            continue;
        }
        placed.insert(candidate, seeds.len());
        seeds.push(candidate);
    }

    // Grow frontiers until they all die out; paint on collisions.
    let mut frontiers: Vec<std::collections::HashSet<BlockPos>> = seeds
        .iter()
        .map(|seed| std::collections::HashSet::from([*seed]))
        .collect();
    let mut frontier_visited: Vec<std::collections::HashSet<BlockPos>> = seeds
        .iter()
        .map(|seed| std::collections::HashSet::from([*seed]))
        .collect();
    let mut painted: std::collections::HashSet<BlockPos> = std::collections::HashSet::new();
    let mut desired = Desired::new();
    loop {
        let mut any = false;
        for index in 0..frontiers.len() {
            if frontiers[index].is_empty() {
                continue;
            }
            any = true;
            let mut next: std::collections::HashSet<BlockPos> = std::collections::HashSet::new();
            let cells: Vec<BlockPos> = frontiers[index].iter().copied().collect();
            for cell in cells {
                if rng.random_range(0..2) == 0 {
                    next.insert(cell);
                    continue;
                }
                for (dx, dy, dz) in &directions {
                    let step = BlockPos::new(cell.0.x + dx, cell.0.y + dy, cell.0.z + dz);
                    if !within(&step) || !is_surface(&step) {
                        continue;
                    }
                    match placed.entry(step) {
                        std::collections::hash_map::Entry::Occupied(_) => {
                            if !frontier_visited[index].contains(&step) && !painted.contains(&step)
                            {
                                painted.insert(step);
                                if let Some(state) = snapshot.apply_pattern(pattern, &step) {
                                    desired.push((step, state));
                                }
                            }
                        }
                        std::collections::hash_map::Entry::Vacant(entry) => {
                            entry.insert(index);
                            next.insert(step);
                            frontier_visited[index].insert(step);
                        }
                    }
                }
            }
            frontiers[index] = next;
        }
        if !any {
            break;
        }
    }
    desired
}
