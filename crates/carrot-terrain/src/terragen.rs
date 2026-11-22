//! `//terragen` — column noise terrain generation over the selection.
//!
//! Arceon `TerrainGenHandler` port: noise type, height, zoom, wave
//! domain-warp, `-e` add mode, `-n` extra octaves, `-r` cellular
//! cell values, `-s` snow caps and `-v` vignette falloff.

use std::collections::HashMap;
use std::sync::Arc;

use carrot_worldedit::states::is_air;
use pumpkin_core::entity::player::Player;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;

use crate::noise::{CellDistance, CellReturn, Cellular, Fractal, FractalKind, Simplex};
use crate::state::TerrainState;

/// The supported noise types (Arceon `NoiseType` enum).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoiseType {
    Simplex,
    Fractal,
    Billowy,
    Ridged,
    Turbulence,
    Electric,
    Euclidean,
    Manhattan,
    Natural,
    Poly,
}

impl NoiseType {
    /// Parses a type name (case-insensitive).
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "simplex" => Some(Self::Simplex),
            "fractal" => Some(Self::Fractal),
            "billowy" => Some(Self::Billowy),
            "ridged" => Some(Self::Ridged),
            "turbulence" => Some(Self::Turbulence),
            "electric" => Some(Self::Electric),
            "euclidean" => Some(Self::Euclidean),
            "manhattan" => Some(Self::Manhattan),
            "natural" => Some(Self::Natural),
            "poly" => Some(Self::Poly),
            _ => None,
        }
    }

    /// Every type name (for help).
    #[must_use]
    pub const fn names() -> &'static str {
        "simplex, fractal, billowy, ridged, turbulence, electric, euclidean, manhattan, natural, poly"
    }
}

/// Everything a terragen run needs.
pub struct TerragenParams {
    pub noise_type: NoiseType,
    pub pattern: carrot_worldedit::pattern::Pattern,
    pub height: f64,
    pub zoom: f64,
    pub wave: f64,
    pub add_mode: bool,
    pub extra_octaves: bool,
    pub cell_values: bool,
    pub snow: bool,
    pub vignette: bool,
}

/// Runs terragen over the player's selection.
#[allow(clippy::too_many_lines)] // literal port of Arceon's terragen loop
pub async fn run(terrain: &Arc<TerrainState>, player: &Arc<Player>, params: TerragenParams) {
    let world = player.world();
    let selection = {
        let session = terrain.we.session(player.gameprofile.id);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.selection
    };
    let Some([min, max]) = selection else {
        player.send_system_message(&TextComponent::text(
            "Faça uma seleção primeiro (wand, //pos1 e //pos2).".to_string(),
        ));
        return;
    };
    let min = BlockPos::min(min, max);
    let max = BlockPos::max(min, max);

    let mut columns: Vec<BlockPos> = Vec::new();
    for x in min.0.x..=max.0.x {
        for z in min.0.z..=max.0.z {
            columns.push(BlockPos::new(x, min.0.y, z));
        }
    }
    carrot_worldedit::edit::ensure_chunks(&world, &columns, |pos: &BlockPos| pos).await;

    let seed = crate::util::random_seed();
    let simplex = Simplex::new(seed);
    let aux = Simplex::new(seed + 101);
    let warp_noise = Simplex::new(seed + 202);
    let octaves: u32 = if params.extra_octaves { 8 } else { 4 };
    let fractal = Fractal::new(seed, octaves, FractalKind::Fbm);
    let billow = Fractal::new(seed, octaves, FractalKind::Billow);
    let rigid = Fractal::new(seed, octaves, FractalKind::Rigid);
    let electric_octaves: u32 = if params.extra_octaves { 5 } else { 3 };
    let electric = Fractal::new(seed, electric_octaves, FractalKind::Rigid);

    let frequency = 0.01 / params.zoom.max(f64::EPSILON);
    let sample = |x: i32, z: i32| -> f64 {
        let (fx, fz) = (f64::from(x) * frequency, f64::from(z) * frequency);
        let mut value = match params.noise_type {
            NoiseType::Simplex => simplex.noise2(fx, fz),
            NoiseType::Fractal => fractal.noise2(fx, fz),
            NoiseType::Billowy => billow.noise2(fx, fz),
            NoiseType::Ridged => rigid.noise2(fx, fz),
            NoiseType::Turbulence => {
                // Gradient-perturb approximated as a frequency-scaled warp.
                let warp = 100.0 * frequency;
                let wx = fx + warp * simplex.noise2(fx + 5.2, fz + 1.3);
                let wz = fz + warp * simplex.noise2(fx - 3.7, fz + 9.1);
                fractal.noise2(wx, wz)
            }
            NoiseType::Electric => {
                let cell = crate::noise::value_noise(seed as i32, fx * 100.0, 0.0, fz * 100.0);
                1.0 - electric.noise2(fx + cell * 0.1, fz)
            }
            NoiseType::Euclidean | NoiseType::Manhattan | NoiseType::Natural | NoiseType::Poly => {
                let mut cellular = Cellular::new(seed as i32);
                cellular.distance = match params.noise_type {
                    NoiseType::Manhattan => CellDistance::Manhattan,
                    NoiseType::Natural | NoiseType::Poly => CellDistance::Natural,
                    _ => CellDistance::Euclidean,
                };
                cellular.frequency = if params.noise_type == NoiseType::Poly {
                    0.03 / params.zoom.max(f64::EPSILON)
                } else {
                    0.1 / params.zoom.max(f64::EPSILON)
                };
                cellular.return_type = if params.cell_values {
                    CellReturn::CellValue
                } else {
                    CellReturn::Distance
                };
                cellular.noise(f64::from(x), 0.0, f64::from(z))
            }
        };
        if params.wave != 0.0 {
            let warp_frequency = 0.1 / params.zoom.max(f64::EPSILON);
            let nx = aux.noise2(f64::from(x) * warp_frequency, f64::from(z) * warp_frequency);
            let nz =
                warp_noise.noise2(f64::from(x) * warp_frequency, f64::from(z) * warp_frequency);
            value = match params.noise_type {
                NoiseType::Euclidean
                | NoiseType::Manhattan
                | NoiseType::Natural
                | NoiseType::Poly => value,
                _ => simplex.noise2(
                    f64::from(x) * frequency + params.wave * nx,
                    f64::from(z) * frequency + params.wave * nz,
                ),
            };
        }
        value
    };

    let width = f64::from(max.0.x - min.0.x + 1);
    let length = f64::from(max.0.z - min.0.z + 1);
    let vignette_amount = width.min(length) / 5.0;
    let center = (
        f64::from(min.0.x) + width / 2.0,
        f64::from(min.0.z) + length / 2.0,
    );

    let snapshot = crate::state::WeSnapshot::capture(&terrain.we, player.gameprofile.id);
    let mut desired: Vec<(BlockPos, BlockStateId)> = Vec::new();
    let min_y = world.get_bottom_y();
    let max_y = world.get_top_y();

    for x in min.0.x..=max.0.x {
        for z in min.0.z..=max.0.z {
            let falloff = if params.vignette {
                let dx = (f64::from(x) - center.0).abs();
                let dz = (f64::from(z) - center.1).abs();
                let distance = dx.max(dz) / vignette_amount.max(1.0);
                (1.0 - distance * distance).max(0.0)
            } else {
                1.0
            };
            let f = sample(x, z);
            if params.add_mode {
                // Raise/lower from the highest block by the noise amount.
                let mut top = min_y;
                for y in (min_y..=max_y).rev() {
                    let state = world.get_block_state_id(&BlockPos::new(x, y, z));
                    if !is_air(state) {
                        top = y;
                        break;
                    }
                }
                let delta = (f * params.height * falloff) as i32;
                if delta > 0 {
                    let surface = world.get_block_state_id(&BlockPos::new(x, top, z));
                    for step in 1..=delta.min(max_y - top) {
                        let pos = BlockPos::new(x, top + step, z);
                        if let Some(state) = snapshot.apply_pattern(&params.pattern, &pos) {
                            desired.push((pos, state));
                        } else {
                            desired.push((pos, surface));
                        }
                    }
                } else if delta < 0 {
                    for step in 0..delta.min(0).abs().min(top - min_y) {
                        let pos = BlockPos::new(x, top - step, z);
                        desired.push((pos, BlockStateId::AIR));
                    }
                }
            } else {
                let shaped = f + 1.0;
                let limit = shaped * (params.height / 2.0) * falloff + f64::from(min.0.y);
                let floor = limit.floor().clamp(f64::from(min.0.y), f64::from(max_y));
                let base = min.0.y.max(min_y);
                let top = floor as i32;
                for y in base..=top {
                    let pos = BlockPos::new(x, y, z);
                    if let Some(state) = snapshot.apply_pattern(&params.pattern, &pos) {
                        let state = if params.snow && y == top {
                            snow_cap(state, limit - f64::from(top))
                        } else {
                            state
                        };
                        desired.push((pos, state));
                    }
                }
            }
        }
    }

    let request = carrot_worldedit::edit::EditRequest {
        desired,
        mask: None,
        block_entities: HashMap::new(),
        description: format!("Terragen {:?}", params.noise_type),
    };
    carrot_worldedit::edit::execute(&terrain.we, player, request).await;
}

/// Snow caps: snow layers thickened by the fractional remainder.
fn snow_cap(state: BlockStateId, remainder: f64) -> BlockStateId {
    let block = state.to_block();
    if block.name.trim_start_matches("minecraft:") != "snow" {
        return state;
    }
    let layers = ((remainder * 8.0).round() as u8).clamp(1, 8);
    carrot_worldedit::states::parse_state_str(&format!("snow[layers={layers}]")).unwrap_or(state)
}
