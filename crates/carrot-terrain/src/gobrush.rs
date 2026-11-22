//! goBrush: heightmap brushes driven by flint, ported from the GPL
//! reference (Arcaniax). The 18 heightmaps used on the reference server
//! ship embedded and seed `carrot/gobrush/brushes` on first enable.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use carrot_worldedit::edit::ensure_chunks;
use carrot_worldedit::states::is_air;
use pumpkin_core::entity::player::Player;
use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;
use rand::RngExt;

use crate::config::TerrainConfig;
use crate::state::TerrainState;

/// The heightmap PNGs from the reference server's goBrush folder.
const DEFAULT_BRUSHES: &[(&str, &[u8])] = &[
    (
        "china mountain 1.png",
        include_bytes!("../brushes/china mountain 1.png"),
    ),
    (
        "china mountain 2.png",
        include_bytes!("../brushes/china mountain 2.png"),
    ),
    ("circle.png", include_bytes!("../brushes/circle.png")),
    (
        "classic mountain 1.png",
        include_bytes!("../brushes/classic mountain 1.png"),
    ),
    (
        "classic mountain 2.png.png",
        include_bytes!("../brushes/classic mountain 2.png.png"),
    ),
    (
        "classic mountain 3.png.png",
        include_bytes!("../brushes/classic mountain 3.png.png"),
    ),
    ("cliff.png", include_bytes!("../brushes/cliff.png")),
    ("crater.png", include_bytes!("../brushes/crater.png")),
    ("hills.png", include_bytes!("../brushes/hills.png")),
    ("mesa.png", include_bytes!("../brushes/mesa.png")),
    (
        "mountain 1.png",
        include_bytes!("../brushes/mountain 1.png"),
    ),
    (
        "mountain 2.png",
        include_bytes!("../brushes/mountain 2.png"),
    ),
    (
        "mountain 3.png",
        include_bytes!("../brushes/mountain 3.png"),
    ),
    (
        "plains cliff.png",
        include_bytes!("../brushes/plains cliff.png"),
    ),
    (
        "sharp cliff.png",
        include_bytes!("../brushes/sharp cliff.png"),
    ),
    (
        "smooth sphere.png",
        include_bytes!("../brushes/smooth sphere.png"),
    ),
    ("sphere.png", include_bytes!("../brushes/sphere.png")),
    ("volcano.png", include_bytes!("../brushes/volcano.png")),
];

/// Writes the embedded heightmaps into `dir` when missing.
pub fn seed_default_brushes(dir: &Path) {
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    for (name, bytes) in DEFAULT_BRUSHES {
        let path = dir.join(name);
        if !path.exists() {
            let _ = std::fs::write(path, bytes);
        }
    }
}

/// Lists available brush file names (sorted).
#[must_use]
pub fn list_brushes(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "png"))
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        },
    );
    names.sort();
    names
}

/// A decoded heightmap (luminance per pixel).
pub struct HeightMap {
    width: u32,
    height: u32,
    luminance: Vec<u8>,
}

impl HeightMap {
    /// Decodes a PNG into a luminance map.
    #[must_use]
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let decoder = png::Decoder::new(bytes.as_slice());
        let mut reader = decoder.read_info().ok()?;
        let mut buffer = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buffer).ok()?;
        let width = frame.width;
        let height = frame.height;
        // Indexed pixels are palette slots; everything else is raw.
        let palette: Vec<[u8; 3]> =
            reader
                .info()
                .palette
                .as_deref()
                .map_or_else(Vec::new, |raw: &[u8]| {
                    raw.chunks(3)
                        .map(|triple| {
                            [
                                triple.first().copied().unwrap_or(0),
                                triple.get(1).copied().unwrap_or(0),
                                triple.get(2).copied().unwrap_or(0),
                            ]
                        })
                        .collect()
                });
        let stride: usize = match frame.color_type {
            png::ColorType::Grayscale | png::ColorType::Indexed => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
        };
        let mut luminance = Vec::with_capacity((width * height) as usize);
        for pixel in buffer.chunks(stride) {
            let value = match stride {
                1 | 2 => {
                    let index = pixel.first().copied().unwrap_or(0) as usize;
                    if frame.color_type == png::ColorType::Indexed {
                        let entry = palette.get(index).copied().unwrap_or([0, 0, 0]);
                        (u32::from(entry[0]) + u32::from(entry[1]) + u32::from(entry[2])) / 3
                    } else {
                        u32::from(index as u8)
                    }
                }
                _ => {
                    let red = u32::from(pixel.first().copied().unwrap_or(0));
                    let green = u32::from(pixel.get(1).copied().unwrap_or(0));
                    let blue = u32::from(pixel.get(2).copied().unwrap_or(0));
                    (red + green + blue) / 3
                }
            };
            luminance.push(value as u8);
        }
        Some(Self {
            width,
            height,
            luminance,
        })
    }

    /// Samples the map nearest-neighbor-resized to `size`x`size`
    /// (the Java original draws the pattern scaled by `drawImage`).
    #[must_use]
    pub fn gray(&self, x: u32, z: u32, size: u32) -> Option<f64> {
        if size == 0 || x >= size || z >= size {
            return None;
        }
        let sx = x * self.width / size;
        let sz = z * self.height / size;
        let index = (sz.min(self.height - 1) * self.width + sx.min(self.width - 1)) as usize;
        let value = self.luminance.get(index).copied().unwrap_or(0);
        Some(f64::from(value) / 255.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_circle_brush_decodes() {
        let (name, bytes) = DEFAULT_BRUSHES
            .iter()
            .find(|(name, _)| *name == "circle.png")
            .expect("circle.png embedded");
        let dir = std::env::temp_dir().join(format!("carrot-terrain-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join(name), bytes).expect("write temp brush");
        let map = HeightMap::load(&dir.join(name)).expect("decodes");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(map.width > 0 && map.height > 0);
        // Center of the circle heightmap should sample brighter than the
        // corner (white ring, black outside).
        let center = map
            .gray(map.width / 2, map.height / 2, map.width)
            .expect("center");
        let corner = map.gray(0, 0, map.width).expect("corner");
        assert!(center > corner);
    }

    #[test]
    fn all_embedded_brushes_ship() {
        assert_eq!(DEFAULT_BRUSHES.len(), 18);
    }
}

/// Per-player goBrush state (mirrors the reference `BrushPlayer`).
pub struct GoBrushPlayer {
    pub enabled: bool,
    pub size: i32,
    pub intensity: i32,
    pub direction_up: bool,
    pub flat_mode: bool,
    pub mode_3d: bool,
    pub auto_rotation: bool,
    pub brush: String,
}

impl GoBrushPlayer {
    /// Defaults from the module configuration (the reference config).
    #[must_use]
    pub fn new(config: &TerrainConfig) -> Self {
        Self {
            enabled: false,
            size: config.gobrush_default_size,
            intensity: config.gobrush_default_intensity,
            direction_up: config.gobrush_default_up,
            flat_mode: config.gobrush_default_flat,
            mode_3d: config.gobrush_default_3d,
            auto_rotation: config.gobrush_default_auto_rotation,
            brush: config.gobrush_default_brush.clone(),
        }
    }

    /// One-line status for `/gb` feedback.
    #[must_use]
    pub fn status(&self) -> String {
        format!(
            "goBrush: {} | brush {} | tamanho {} | intensidade {} | {} | {}{}{}",
            if self.enabled { "ativo" } else { "inativo" },
            self.brush,
            self.size,
            self.intensity,
            if self.direction_up { "cima" } else { "baixo" },
            if self.mode_3d { "3D " } else { "" },
            if self.flat_mode { "plano " } else { "" },
            if self.auto_rotation { "auto-rot" } else { "" },
        )
    }
}

/// Cardinal direction from yaw (goBrush `getCardinalDirection`).
fn cardinal_direction(yaw: f64) -> &'static str {
    let rotation = (yaw - 90.0).rem_euclid(360.0);
    match rotation {
        value if (0.0..45.0).contains(&value) => "W",
        value if (45.0..135.0).contains(&value) => "N",
        value if (135.0..225.0).contains(&value) => "E",
        value if (225.0..315.0).contains(&value) => "S",
        _ => "W",
    }
}

fn highest_non_air(world: &World, x: i32, z: i32, min_y: i32, max_y: i32) -> i32 {
    for y in (min_y..=max_y).rev() {
        let state = world.get_block_state_id(&BlockPos::new(x, y, z));
        if !is_air(state) {
            return y;
        }
    }
    min_y
}

fn distance(a: Vector3<f64>, b: Vector3<f64>) -> f64 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt()
}

/// Runs the goBrush stroke at `target` (goBrush `PlayerInteractListener`).
#[allow(clippy::too_many_lines, clippy::if_not_else)] // literal port of goBrush's listener
pub async fn paint(terrain: &Arc<TerrainState>, player: &Arc<Player>, target: BlockPos) {
    let (size, intensity, up, flat, mode_3d, auto_rotation, brush_name) = {
        let state = terrain.player(player.gameprofile.id);
        let guard = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        (
            guard.gobrush.size,
            guard.gobrush.intensity,
            guard.gobrush.direction_up,
            guard.gobrush.flat_mode,
            guard.gobrush.mode_3d,
            guard.gobrush.auto_rotation,
            guard.gobrush.brush.clone(),
        )
    };
    let Some(heightmap) = terrain.heightmap(&brush_name) else {
        player.send_system_message(&TextComponent::text(format!(
            "Brush {brush_name} não encontrado (/gb list)."
        )));
        return;
    };

    let world = player.world();
    let yaw = f64::from(player.living_entity.entity.yaw.load());
    let pitch = f64::from(player.living_entity.entity.pitch.load());
    let entity = &player.living_entity.entity;
    let eye = entity.get_eye_pos();
    let look = entity.rotation();

    let min = -(size / 2);
    let max = size / 2;
    // Drawn before any await: ThreadRng is not Send.
    let dither: f64 = rand::rng().random();

    // Pre-load every column the stroke can touch.
    let mut columns: Vec<BlockPos> = Vec::new();
    for x in min - 1..=max + 1 {
        for z in min - 1..=max + 1 {
            columns.push(BlockPos::new(target.0.x + x, target.0.y, target.0.z + z));
        }
    }
    ensure_chunks(&world, &columns, |pos: &BlockPos| pos).await;

    let min_y = world.get_bottom_y();
    let max_y = world.get_top_y();
    let cardinal = if auto_rotation {
        cardinal_direction(yaw)
    } else {
        "N"
    };
    let sample = |x: i32, z: i32| -> f64 {
        // Auto-rotation remaps the sample indices (reference getHeight).
        let (sx, sz) = match cardinal {
            "S" => (size - x, size - z),
            "W" => (size - z, x),
            "E" => (z, size - x),
            _ => (x, z),
        };
        if sx < 0 || sz < 0 || sx >= size || sz >= size {
            return 0.0;
        }
        heightmap
            .gray(sx.unsigned_abs(), sz.unsigned_abs(), size.unsigned_abs())
            .map_or(0.0, |gray| gray * f64::from(intensity))
    };

    let mut strokes: HashMap<BlockPos, BlockStateId> = HashMap::new();
    if !mode_3d {
        for x in min..=max {
            for z in min..=max {
                let column_x = target.0.x + x;
                let column_z = target.0.z + z;
                let top = highest_non_air(&world, column_x, column_z, min_y, max_y);
                let mut height = sample(x - min, z - min);
                let fraction = height.fract();
                if dither > 1.0 - fraction {
                    height += 1.0;
                }
                let surface = world.get_block_state_id(&BlockPos::new(column_x, top, column_z));
                if up {
                    for y in 1..height.floor().max(0.0) as i32 {
                        if !flat || top + y <= target.0.y {
                            strokes.insert(BlockPos::new(column_x, top + y, column_z), surface);
                        }
                    }
                } else {
                    for y in 0..height.floor().max(0.0) as i32 {
                        if (!flat || top - y > target.0.y) && top - y >= min_y {
                            strokes.insert(
                                BlockPos::new(column_x, top - y, column_z),
                                BlockStateId::AIR,
                            );
                        }
                    }
                }
            }
        }
    } else {
        // 3D mode: extrude the heightmap along the view direction, one
        // search per sample cell (goBrush's NestedFor + getClosest).
        let rotation = (yaw - 90.0).rem_euclid(360.0) / 360.0 * std::f64::consts::TAU;
        let rotation_pitch = pitch.rem_euclid(360.0) / 360.0 * std::f64::consts::TAU;
        let x_mov = rotation.cos() * 0.25;
        let z_mov = rotation.sin() * 0.25;
        let y_mod = rotation_pitch.cos() * 0.25;
        let look64 = Vector3::new(f64::from(look.x), f64::from(look.y), f64::from(look.z));
        let step = Vector3::new(look64.x * 0.5, look64.y * 0.5, look64.z * 0.5);
        let anchor = Vector3::new(
            f64::from(target.0.x) + 0.5,
            f64::from(target.0.y) + 0.5,
            f64::from(target.0.z) + 0.5,
        );
        let reach = f64::from(size) / 4.0;

        for i0 in min - 1..=max {
            for i1 in min - 1..=max {
                let mut loop_loc = Vector3::new(
                    eye.x + z_mov * f64::from(i0),
                    eye.y + f64::from(i1) * y_mod,
                    eye.z - x_mov * f64::from(i0),
                );
                let drift_x = (1.0 - y_mod) * x_mov * f64::from(i1);
                let drift_z = (1.0 - y_mod) * z_mov * f64::from(i1);
                if pitch < 0.0 {
                    loop_loc.x += drift_x;
                    loop_loc.z += drift_z;
                } else {
                    loop_loc.x -= drift_x;
                    loop_loc.z -= drift_z;
                }

                // Walk from loop_loc toward the surface, like getClosest.
                let mut cursor = loop_loc;
                let mut hit: Option<BlockPos> = None;
                for _ in 0..=(reach * 8.0) as usize {
                    if distance(cursor, anchor) >= reach {
                        break;
                    }
                    let block = BlockPos::containing(cursor.x, cursor.y, cursor.z);
                    if !is_air(world.get_block_state_id(&block)) {
                        hit = Some(block);
                        break;
                    }
                    cursor = Vector3::new(cursor.x + step.x, cursor.y + step.y, cursor.z + step.z);
                    if cursor.y <= f64::from(min_y) || cursor.y > f64::from(max_y) {
                        break;
                    }
                }
                let Some(block_loc) = hit else {
                    continue;
                };
                let mut height = sample(i0 - min, i1 - min);
                let fraction = height.fract();
                if dither > 1.0 - fraction {
                    height += 1.0;
                }
                let surface = world.get_block_state_id(&block_loc);
                let anchor_distance = distance(anchor, eye);
                for y in 0..height.floor().max(0.0) as i32 {
                    let offset = Vector3::new(
                        look64.x * f64::from(-y),
                        look64.y * f64::from(-y),
                        look64.z * f64::from(-y),
                    );
                    let place_flat = if up {
                        Vector3::new(
                            f64::from(block_loc.0.x) + 0.5 + look64.x * f64::from(-y),
                            f64::from(block_loc.0.y) + 0.5 + look64.y * f64::from(-y),
                            f64::from(block_loc.0.z) + 0.5 + look64.z * f64::from(-y),
                        )
                    } else {
                        Vector3::new(
                            f64::from(block_loc.0.x) + 0.5 + look64.x * f64::from(y - 1),
                            f64::from(block_loc.0.y) + 0.5 + look64.y * f64::from(y - 1),
                            f64::from(block_loc.0.z) + 0.5 + look64.z * f64::from(y - 1),
                        )
                    };
                    let paint = if !flat {
                        true
                    } else if up {
                        distance(place_flat, loop_loc) > anchor_distance
                    } else {
                        distance(place_flat, loop_loc) < anchor_distance - 1.0
                    };
                    if paint {
                        strokes.insert(
                            BlockPos::new(
                                block_loc.0.x + offset.x as i32,
                                block_loc.0.y + offset.y as i32,
                                block_loc.0.z + offset.z as i32,
                            ),
                            surface,
                        );
                    }
                }
            }
        }
    }

    let request = carrot_worldedit::edit::EditRequest {
        desired: strokes.into_iter().collect(),
        mask: None,
        block_entities: HashMap::new(),
        description: format!("goBrush {brush_name}"),
    };
    carrot_worldedit::edit::execute(&terrain.we, player, request).await;
}
