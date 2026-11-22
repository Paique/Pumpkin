//! `//eztexture`: palette texturing, the ezEdits subset the builders
//! actually used — `ambient`, `sunlight`, `pointlight`, `curvature`,
//! `axisgradient` and `shift`, plus `//ezpalette` world-fetch palette
//! management.
//!
//! The field math ports ezEdits faithfully: the cross-shaped separable
//! box blur (`smoothed`), central-difference normals from the smoothed
//! mask scalar field, surface extraction, neighbour-distance curvature
//! and the normal-weighted gaussian shadow blur, all over a dense grid
//! of the selection cuboid.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use carrot_core::ModuleContext;
use carrot_worldedit::edit::{EditRequest, ensure_chunks, execute};
use carrot_worldedit::mask::Mask;
use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::region::Direction;
use carrot_worldedit::states::{parse_state_str, state_to_string};
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use crate::commands::{ONLY_PLAYERS, PERM_BRUSH, held_block, player_of, reply};
use crate::state::TerrainState;

/// Where `//ezpalette` files live (`carrot/ezpalettes`).
#[derive(Default)]
pub struct PaletteStore {
    dir: Mutex<PathBuf>,
    cache: Mutex<HashMap<String, Vec<BlockStateId>>>,
}

impl PaletteStore {
    /// Points the store at the module data folder (seeded on enable).
    pub fn configure(&self, dir: PathBuf) {
        let _ = std::fs::create_dir_all(&dir);
        *self.dir.lock().unwrap_or_else(PoisonError::into_inner) = dir;
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    fn load(&self, name: &str) -> Option<Vec<BlockStateId>> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(states) = cache.get(name) {
            return Some(states.clone());
        }
        let dir = self
            .dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let text = std::fs::read_to_string(dir.join(format!("{name}.txt"))).ok()?;
        let states = text
            .lines()
            .filter_map(|line| parse_state_str(line.trim()).ok())
            .collect::<Vec<_>>();
        cache.insert(name.to_string(), states.clone());
        Some(states)
    }

    fn save(&self, name: &str, states: &[BlockStateId]) {
        let dir = self
            .dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let text = states
            .iter()
            .map(|state| state_to_string(*state))
            .collect::<Vec<_>>()
            .join("\n");
        let _ = std::fs::write(dir.join(format!("{name}.txt")), text);
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(name.to_string(), states.to_vec());
    }

    fn names(&self) -> Vec<String> {
        let dir = self
            .dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        let name = path.file_name()?.to_str()?.to_string();
                        path.extension()
                            .is_some_and(|ext| ext == "txt")
                            .then(|| name.trim_end_matches(".txt").to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn delete(&self, name: &str) {
        let dir = self
            .dir
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let _ = std::fs::remove_file(dir.join(format!("{name}.txt")));
        self.cache
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }
}

/// Registers `//eztexture`, `//ezt`, `//ezpalette` and `//ezp`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    for (name, kind) in [("//eztexture", Kind::Texture), ("//ezt", Kind::Texture)] {
        let executor = TextureExecutor {
            state: Arc::clone(state),
            kind,
        };
        context.register_command(
            name,
            command(name, "Palette texturing suite.")
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
            PERM_BRUSH,
        );
    }
    for name in ["//ezpalette", "//ezp"] {
        let executor = TextureExecutor {
            state: Arc::clone(state),
            kind: Kind::Palette,
        };
        context.register_command(
            name,
            command(name, "Named palette management.")
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
            PERM_BRUSH,
        );
    }
}

/// Which command tree an executor runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Texture,
    Palette,
}

/// One greedy-argument executor per tree.
#[derive(Clone)]
struct TextureExecutor {
    state: Arc<TerrainState>,
    kind: Kind,
}

impl CommandExecutor for TextureExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args")?;
        match self.kind {
            Kind::Palette => self.palette(context, &player, raw),
            Kind::Texture => self.texture(context, &player, raw),
        }
        Ok(1)
    }
}

impl TextureExecutor {
    /// `//ezpalette fetch|save|list|delete ...`.
    #[allow(clippy::too_many_lines)] // four subcommands, one dispatcher
    fn palette(&self, context: &CommandContext, player: &Arc<Player>, raw: &str) {
        let mut tokens = raw.split_whitespace();
        match tokens.next().unwrap_or_default() {
            "list" => {
                let names = self.state.palettes.names().join(", ");
                reply(
                    context,
                    if names.is_empty() {
                        "Nenhuma paleta salva.".to_string()
                    } else {
                        format!("Paletas: {names}")
                    },
                );
            }
            "delete" | "remove" => {
                let Some(name) = tokens.next() else {
                    reply(context, "use: //ezpalette delete <nome>".to_string());
                    return;
                };
                self.state.palettes.delete(name);
                reply(context, format!("Paleta {name} removida."));
            }
            "save" => {
                let (Some(name), Some(palette_token)) = (tokens.next(), tokens.next()) else {
                    reply(context, "use: //ezpalette save <nome> <padrão>".to_string());
                    return;
                };
                let hand = held_block(player);
                let pattern = match Pattern::parse(palette_token, hand) {
                    Ok(Pattern::Single(state)) => state,
                    Ok(_) => {
                        reply(
                            context,
                            "paleta precisa de blocos fixos (sem % aleatorio)".to_string(),
                        );
                        return;
                    }
                    Err(message) => {
                        reply(context, format!("Padrão inválido: {message}"));
                        return;
                    }
                };
                // A single-state save is degenerate; ezEdits parses the
                // comma list itself, so split the raw token list.
                let mut states = vec![pattern];
                for token in tokens {
                    match parse_state_str(token) {
                        Ok(state) => states.push(state),
                        Err(message) => {
                            reply(context, format!("Bloco inválido {token}: {message}"));
                            return;
                        }
                    }
                }
                self.state.palettes.save(name, &states);
                reply(
                    context,
                    format!("Paleta {name} salva com {} blocos.", states.len()),
                );
            }
            "fetch" => {
                // Fetch a row of blocks from the world starting under
                // the player, walking in the given (or facing) direction.
                let (Some(name), count) = (
                    tokens.next(),
                    tokens.next().map_or(0, |t| t.parse::<usize>().unwrap_or(0)),
                ) else {
                    reply(
                        context,
                        "use: //ezpalette fetch <nome> [quantidade] [-d direção]".to_string(),
                    );
                    return;
                };
                let mut direction_token = String::new();
                while let Some(token) = tokens.next() {
                    if token == "-d"
                        && let Some(next) = tokens.next()
                    {
                        direction_token = next.to_string();
                    }
                }
                let direction = if direction_token.is_empty() {
                    facing_direction(player)
                } else {
                    let Some(direction) = Direction::parse(&direction_token) else {
                        reply(context, format!("direção inválida: {direction_token}"));
                        return;
                    };
                    direction
                };

                let world = player.world();
                let feet = player.position();
                let mut pos = BlockPos::new(
                    feet.x.floor() as i32,
                    feet.y.floor() as i32 - 1,
                    feet.z.floor() as i32,
                );
                let offset = direction.offset();
                let mut states = Vec::new();
                let limit = if count == 0 { 128 } else { count };
                let mut steps = 0;
                while steps < limit {
                    let state = world.get_block_state_id(&pos);
                    if state == BlockStateId::AIR || state.to_state().is_liquid() {
                        if count == 0 {
                            break;
                        }
                    } else {
                        states.push(state);
                    }
                    pos = BlockPos::new(pos.0.x + offset.x, pos.0.y + offset.y, pos.0.z + offset.z);
                    steps += 1;
                }
                if states.is_empty() {
                    reply(
                        context,
                        "Nenhum bloco encontrado para a paleta.".to_string(),
                    );
                    return;
                }
                self.state.palettes.save(name, &states);
                reply(
                    context,
                    format!("Paleta {name} salva com {} blocos.", states.len()),
                );
            }
            other => {
                reply(
                    context,
                    format!("subcomando desconhecido: {other} (fetch, save, list, delete)"),
                );
            }
        }
    }

    /// `//eztexture <mode> <mask> <palette> [args] [-flags]`.
    fn texture(&self, context: &CommandContext, player: &Arc<Player>, raw: &str) {
        let mut tokens = raw.split_whitespace();
        let mode = tokens.next().unwrap_or_default().to_string();
        let (Some(mask_token), Some(palette_token)) = (tokens.next(), tokens.next()) else {
            reply(
                context,
                "use: //eztexture <ambient|sunlight|pointlight|curvature|axisgradient|shift> <máscara> <paleta> [args]".to_string(),
            );
            return;
        };
        let hand = held_block(player);
        let Some(mask) = Mask::parse(mask_token, hand).ok() else {
            reply(context, format!("Máscara inválida: {mask_token}"));
            return;
        };
        let Some(palette) = self.state.palettes.load(palette_token) else {
            let names = self.state.palettes.names().join(", ");
            reply(
                context,
                format!("Paleta desconhecida: {palette_token}. Disponíveis: {names}"),
            );
            return;
        };
        if palette.is_empty() {
            reply(context, format!("Paleta {palette_token} está vazia."));
            return;
        }

        let rest: Vec<String> = tokens.map(std::string::ToString::to_string).collect();
        let mode_kind = match mode.as_str() {
            "ambient" => Mode::Ambient,
            "sunlight" => Mode::Sunlight,
            "pointlight" => Mode::Pointlight,
            "curvature" => Mode::Curvature,
            "axisgradient" => Mode::AxisGradient,
            "shift" => Mode::Shift,
            other => {
                reply(context, format!("modo desconhecido: {other}"));
                return;
            }
        };
        let params = match Params::parse(mode_kind, &rest) {
            Ok(params) => params,
            Err(message) => {
                reply(context, message);
                return;
            }
        };

        let state = Arc::clone(&self.state);
        let player = Arc::clone(player);
        let description = format!("//eztexture {}", raw.trim());
        tokio::spawn(async move {
            run_texture(&state, &player, &mask, &palette, params, description).await;
        });
        reply(context, "Texturizando...".to_string());
    }
}

/// The supported texture modes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ambient,
    Sunlight,
    Pointlight,
    Curvature,
    AxisGradient,
    Shift,
}

/// Parsed parameters shared by the modes (defaults per ezEdits).
struct Params {
    mode: Mode,
    radius: i32,
    brightness: f32,
    contrast: f32,
    min_angle: i32,
    max_angle: i32,
    direction: Vector3<f64>,
    occlusion: f32,
    smoothing: f32,
    falloff_range: i32,
    falloff: bool,
    light_pos: Option<Vector3<f64>>,
    shadows: bool,
    axis: char,
    relative: bool,
    shift: i32,
}

impl Params {
    #[allow(clippy::too_many_lines)] // one parser, six modes
    fn parse(mode: Mode, tokens: &[String]) -> Result<Self, String> {
        let mut params = Self {
            mode,
            radius: if mode == Mode::Curvature || mode == Mode::Ambient {
                3
            } else {
                1
            },
            brightness: 0.0,
            contrast: 0.0,
            min_angle: 0,
            max_angle: 180,
            direction: Vector3::new(0.0, -1.0, 0.0),
            occlusion: 0.0,
            smoothing: 1.0,
            falloff_range: 5,
            falloff: false,
            light_pos: None,
            shadows: false,
            axis: 'y',
            relative: false,
            shift: 0,
        };
        let mut numbers: Vec<f32> = Vec::new();
        let mut iter = tokens.iter();
        while let Some(token) = iter.next() {
            match token.as_str() {
                "-l" => {
                    let Some(name) = iter.next() else {
                        return Err("flag -l precisa de direção".to_string());
                    };
                    let direction = Direction::parse(name)
                        .ok_or_else(|| format!("direção inválida: {name}"))?;
                    let offset = direction.offset();
                    params.direction = Vector3::new(
                        f64::from(offset.x),
                        f64::from(offset.y),
                        f64::from(offset.z),
                    );
                }
                "-p" => {
                    let Some(raw) = iter.next() else {
                        return Err("flag -p precisa de x,y,z".to_string());
                    };
                    let parts: Vec<f64> = raw
                        .split(',')
                        .filter_map(|part| part.parse::<f64>().ok())
                        .collect();
                    if parts.len() != 3 {
                        return Err(format!("posição inválida: {raw}"));
                    }
                    params.light_pos = Some(Vector3::new(parts[0], parts[1], parts[2]));
                }
                "-o" => {
                    params.occlusion = parse_flag_value(iter.next(), "flag -o")?;
                }
                "-r" => {
                    let value = parse_flag_value::<f32>(iter.next(), "flag -r")?;
                    params.smoothing = value;
                    if mode == Mode::Ambient || mode == Mode::Curvature {
                        params.brightness = value;
                    }
                }
                "-s" => params.shadows = true,
                "-f" => {
                    if mode == Mode::Pointlight {
                        params.falloff_range = parse_flag_value::<i32>(iter.next(), "flag -f")?;
                    }
                    params.falloff = true;
                }
                "-rel" => params.relative = true,
                "x" | "y" | "z" if mode == Mode::AxisGradient => {
                    params.axis = token.chars().next().unwrap_or('y');
                }
                other => match other.parse::<f32>() {
                    Ok(value) => numbers.push(value),
                    Err(_) => return Err(format!("argumento inválido: {other}")),
                },
            }
        }
        match mode {
            Mode::Ambient | Mode::Curvature => {
                if let Some(value) = numbers.first() {
                    params.radius = *value as i32;
                }
                if let Some(value) = numbers.get(1) {
                    params.brightness = *value;
                }
                if let Some(value) = numbers.get(2) {
                    params.contrast = *value;
                }
            }
            Mode::Sunlight | Mode::Pointlight => {
                if let Some(value) = numbers.first() {
                    params.radius = *value as i32;
                }
                if let Some(value) = numbers.get(1) {
                    params.min_angle = *value as i32;
                }
                if let Some(value) = numbers.get(2) {
                    params.max_angle = *value as i32;
                }
            }
            Mode::Shift => {
                params.shift = numbers.first().copied().unwrap_or(0.0) as i32;
            }
            Mode::AxisGradient => {}
        }
        if params.max_angle <= params.min_angle {
            return Err("ângulo máximo deve ser maior que o mínimo".to_string());
        }
        Ok(params)
    }
}

fn parse_flag_value<T: std::str::FromStr>(
    token: Option<&String>,
    label: &str,
) -> Result<T, String> {
    token
        .and_then(|value| value.parse::<T>().ok())
        .ok_or_else(|| format!("{label} precisa de um número"))
}

/// The horizontal direction the player faces.
fn facing_direction(player: &Player) -> Direction {
    Direction::from_yaw(player.living_entity.entity.yaw.load())
}

/// The ezEdits tone curve: `(v + b/(2(1+c))) * (1+c) - c*0.5`.
fn tone_curve(value: f32, brightness: f32, contrast: f32) -> f32 {
    (value + brightness / (2.0 * (1.0 + contrast))) * (1.0 + contrast) - contrast * 0.5
}

/// `mapClamped`: linear map between ranges, clamped to the output.
fn map_clamped(value: f32, in_min: f32, in_max: f32, out_min: f32, out_max: f32) -> f32 {
    if in_max <= in_min {
        return out_min;
    }
    let t = ((value - in_min) / (in_max - in_min)).clamp(0.0, 1.0);
    out_min + t * (out_max - out_min)
}

/// Dense scalar grid over the selection cuboid.
struct Grid {
    min: BlockPos,
    size: Vector3<usize>,
    values: Vec<f32>,
}

impl Grid {
    fn new(min: BlockPos, size: Vector3<usize>) -> Self {
        Self {
            min,
            size,
            values: vec![0.0; size.x * size.y * size.z],
        }
    }

    const fn index(&self, x: usize, y: usize, z: usize) -> usize {
        (y * self.size.z + z) * self.size.x + x
    }

    fn get(&self, x: i32, y: i32, z: i32) -> Option<f32> {
        if x < 0 || y < 0 || z < 0 {
            return None;
        }
        let (x, y, z) = (x as usize, y as usize, z as usize);
        (x < self.size.x && y < self.size.y && z < self.size.z)
            .then(|| self.values[self.index(x, y, z)])
    }

    fn set(&mut self, x: usize, y: usize, z: usize, value: f32) {
        let index = self.index(x, y, z);
        self.values[index] = value;
    }

    /// The cross-shaped separable box blur, `iterations` passes.
    fn smoothed(&self, radius: i32, iterations: u32) -> Self {
        let mut current = Self {
            min: self.min,
            size: self.size,
            values: self.values.clone(),
        };
        for _ in 0..iterations {
            let mut next = Self {
                min: self.min,
                size: self.size,
                values: vec![0.0; self.values.len()],
            };
            for y in 0..self.size.y {
                for z in 0..self.size.z {
                    for x in 0..self.size.x {
                        let (xi, yi, zi) = (x as i32, y as i32, z as i32);
                        let mut sum = 0.0;
                        let mut count = 0;
                        for axis in 0..3 {
                            for step in -radius..=radius {
                                let (nx, ny, nz) = match axis {
                                    0 => (xi + step, yi, zi),
                                    1 => (xi, yi + step, zi),
                                    _ => (xi, yi, zi + step),
                                };
                                if let Some(value) = current.get(nx, ny, nz) {
                                    sum += value;
                                    count += 1;
                                }
                            }
                        }
                        next.set(x, y, z, if count == 0 { 0.0 } else { sum / count as f32 });
                    }
                }
            }
            current = next;
        }
        current
    }

    /// Central-difference unit normals (out-of-bounds samples 0.5).
    fn normals(&self) -> Vec<Option<Vector3<f32>>> {
        let mut normals = vec![None; self.values.len()];
        for y in 0..self.size.y {
            for z in 0..self.size.z {
                for x in 0..self.size.x {
                    let (xi, yi, zi) = (x as i32, y as i32, z as i32);
                    let neighbor = |dx: i32, dy: i32, dz: i32| {
                        self.get(xi + dx, yi + dy, zi + dz).unwrap_or(0.5)
                    };
                    let dx = neighbor(1, 0, 0) - neighbor(-1, 0, 0);
                    let dy = neighbor(0, 1, 0) - neighbor(0, -1, 0);
                    let dz = neighbor(0, 0, 1) - neighbor(0, 0, -1);
                    if dx != 0.0 || dy != 0.0 || dz != 0.0 {
                        let length = (dx * dx + dy * dy + dz * dz).sqrt();
                        normals[self.index(x, y, z)] =
                            Some(Vector3::new(dx / length, dy / length, dz / length));
                    }
                }
            }
        }
        normals
    }
}

/// Runs a texture mode over the player's selection.
#[allow(clippy::too_many_lines)] // one dispatch, five literal ports
async fn run_texture(
    state: &Arc<TerrainState>,
    player: &Arc<Player>,
    mask: &Mask,
    palette: &[BlockStateId],
    params: Params,
    description: String,
) {
    let selection = {
        let session = state.we.session(player.gameprofile.id);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.selection
    };
    let Some([corner_a, corner_b]) = selection else {
        player.send_system_message(&TextComponent::text(
            "Faça uma seleção primeiro (wand, //pos1 e //pos2).".to_string(),
        ));
        return;
    };
    let min = BlockPos::min(corner_a, corner_b);
    let max = BlockPos::max(corner_a, corner_b);
    let world = player.world();

    let corners = vec![min, max];
    ensure_chunks(&world, &corners, |pos: &BlockPos| pos).await;

    let size = Vector3::new(
        (max.0.x - min.0.x + 1) as usize,
        (max.0.y - min.0.y + 1) as usize,
        (max.0.z - min.0.z + 1) as usize,
    );

    // The boolean input field: mask membership over the selection.
    let mut field = Grid::new(min, size);
    let mut desired: Vec<(BlockPos, BlockStateId)> = Vec::new();

    if params.mode_is(Mode::Shift) {
        for y in 0..size.y {
            for z in 0..size.z {
                for x in 0..size.x {
                    let pos =
                        BlockPos::new(min.0.x + x as i32, min.0.y + y as i32, min.0.z + z as i32);
                    let before = world.get_block_state_id(&pos);
                    if let Some(shifted) = palette_shift(palette, before, params.shift) {
                        desired.push((pos, shifted));
                    }
                }
            }
        }
        execute(&state.we, player, EditRequest::new(desired, description)).await;
        return;
    }

    for y in 0..size.y {
        for z in 0..size.z {
            for x in 0..size.x {
                let pos = BlockPos::new(min.0.x + x as i32, min.0.y + y as i32, min.0.z + z as i32);
                if mask.test_at(&world, &pos) {
                    field.set(x, y, z, 1.0);
                }
            }
        }
    }

    let values = match params_kind(&params) {
        Mode::Ambient => {
            let smoothed = field.smoothed(params.radius, 5);
            (0..field.values.len())
                .map(|index| {
                    let raw = field.values[index];
                    let smooth = smoothed.values[index];
                    let difference = (smooth - raw).abs();
                    tone_curve(
                        map_clamped(difference, 0.0, 0.5, 0.0, 1.0),
                        params.brightness,
                        params.contrast,
                    )
                })
                .collect::<Vec<_>>()
        }
        Mode::Curvature => {
            let normals = field.smoothed(params.radius, 5).normals();
            (0..field.values.len())
                .map(|index| {
                    let Some(normal) = normals[index] else {
                        return 0.0;
                    };
                    let (x, y, z) = field.coords_of(index);
                    let mut total = 0.0;
                    let mut count = 0.0;
                    for (dx, dy, dz) in [
                        (1, 0, 0),
                        (-1, 0, 0),
                        (0, 1, 0),
                        (0, -1, 0),
                        (0, 0, 1),
                        (0, 0, -1),
                    ] {
                        if let Some(nindex) =
                            field.index_opt(x as i32 + dx, y as i32 + dy, z as i32 + dz)
                            && let Some(Some(neighbor)) = normals.get(nindex)
                        {
                            let delta = normal - *neighbor;
                            total +=
                                (delta.x * delta.x + delta.y * delta.y + delta.z * delta.z).sqrt();
                            count += 1.0;
                        }
                    }
                    let curvature = if count == 0.0 { 0.0 } else { total / count };
                    tone_curve(curvature * 3.0, params.brightness, params.contrast)
                })
                .collect::<Vec<_>>()
        }
        Mode::Sunlight | Mode::Pointlight => light_values(&field, &params, player, palette),
        Mode::AxisGradient => {
            let (axis_index, axis_size) = match params.axis {
                'x' => (0usize, size.x),
                'z' => (2usize, size.z),
                _ => (1usize, size.y),
            };
            // Column bases per (x,z) for the relative mode.
            let mut values = vec![0.0f32; field.values.len()];
            for y in 0..size.y {
                for z in 0..size.z {
                    for x in 0..size.x {
                        let index = field.index(x, y, z);
                        let coordinate = match axis_index {
                            0 => x,
                            1 => y,
                            _ => z,
                        };
                        let value = if params.relative {
                            let run = axis_run(&field, axis_index, x, y, z);
                            (coordinate as f32 - run.0 as f32 + 0.5) / (run.1 - run.0 + 1) as f32
                        } else {
                            (coordinate as f32 + 0.5) / axis_size as f32
                        };
                        values[index] = 1.0 - value;
                    }
                }
            }
            values
        }
        Mode::Shift => Vec::new(), // handled before the field pass
    };

    for (index, value) in values.iter().enumerate() {
        if field.values[index] == 0.0 {
            continue;
        }
        let (x, y, z) = field.coords_of(index);
        let pos = BlockPos::new(min.0.x + x as i32, min.0.y + y as i32, min.0.z + z as i32);
        if let Some(state) = palette_normalized(palette, *value) {
            desired.push((pos, state));
        }
    }

    let request = EditRequest::new(desired, description);
    execute(&state.we, player, request).await;
}

/// Which mode the params carry (helper keeping the big match readable).
const fn params_kind(params: &Params) -> Mode {
    params.mode
}

impl Params {
    fn mode_is(&self, mode: Mode) -> bool {
        self.mode == mode
    }
}

/// The sunlight/pointlight shared pipeline: normals, facing, occlusion
/// shadows and the shadow subtraction pass.
#[allow(clippy::too_many_lines)]
fn light_values(
    field: &Grid,
    params: &Params,
    player: &Arc<Player>,
    _palette: &[BlockStateId],
) -> Vec<f32> {
    let normals = field.smoothed(params.radius, 6).normals();
    let min_angle = map_clamped(params.min_angle as f32, 0.0, 180.0, -1.0, 1.0);
    let max_angle = map_clamped(params.max_angle as f32, 0.0, 180.0, -1.0, 1.0);

    // Bounding-box diagonal: shadow rays run this far.
    let diagonal = Vector3::new(
        f64::from(field.size.x as i32),
        f64::from(field.size.y as i32),
        f64::from(field.size.z as i32),
    )
    .length();

    let feet = player.position();
    let light_pos = params
        .light_pos
        .unwrap_or(Vector3::new(feet.x, feet.y, feet.z));
    let falloff_sq = f64::from(params.falloff_range * params.falloff_range);

    let mut facing = vec![0.0f32; field.values.len()];
    let mut occlusion = vec![-1.0f32; field.values.len()];
    let mut surface = vec![false; field.values.len()];

    for y in 0..field.size.y {
        for z in 0..field.size.z {
            for x in 0..field.size.x {
                let index = field.index(x, y, z);
                if field.values[index] == 0.0 {
                    continue;
                }
                // Surface: any false (or outside) 6-neighbour.
                let mut is_surface = false;
                for (dx, dy, dz) in [
                    (1, 0, 0),
                    (-1, 0, 0),
                    (0, 1, 0),
                    (0, -1, 0),
                    (0, 0, 1),
                    (0, 0, -1),
                ] {
                    if field.get(x as i32 + dx, y as i32 + dy, z as i32 + dz) != Some(1.0) {
                        is_surface = true;
                        break;
                    }
                }
                surface[index] = is_surface;

                let Some(normal) = normals[index] else {
                    continue;
                };
                let block_center = Vector3::new(
                    f64::from(field.min.0.x + x as i32) + 0.5,
                    f64::from(field.min.0.y + y as i32) + 0.5,
                    f64::from(field.min.0.z + z as i32) + 0.5,
                );

                let light_dir = Vector3::new(
                    -params.direction.x,
                    -params.direction.y,
                    -params.direction.z,
                );
                let (direction_to_light, falloff_factor) = if params.mode == Mode::Sunlight {
                    (light_dir, 1.0f32)
                } else {
                    let delta = light_pos - block_center;
                    let distance_sq = delta.length_squared().max(1.0);
                    let falloff = if params.falloff {
                        (falloff_sq / distance_sq).min(1.0) as f32
                    } else {
                        1.0
                    };
                    (delta.normalize(), falloff)
                };
                let dot = normal.x as f64 * direction_to_light.x
                    + normal.y as f64 * direction_to_light.y
                    + normal.z as f64 * direction_to_light.z;
                facing[index] = falloff_factor
                    * (1.0 - map_clamped(dot as f32, min_angle, max_angle, 0.0, 1.0));

                // Shadow ray: from the light side towards the block.
                if params.shadows && is_surface {
                    let ray_end = if params.mode == Mode::Sunlight {
                        block_center - params.direction * diagonal
                    } else {
                        light_pos
                    };
                    for sample in line_blocks(block_center, ray_end) {
                        let local = (
                            sample.0.x - field.min.0.x,
                            sample.0.y - field.min.0.y,
                            sample.0.z - field.min.0.z,
                        );
                        if local == (x as i32, y as i32, z as i32) {
                            continue;
                        }
                        if field.get(local.0, local.1, local.2) == Some(1.0) {
                            occlusion[index] = 1.0;
                            break;
                        }
                    }
                }
            }
        }
    }

    // Shadow blur (normal-weighted gaussian over surface cells) and the
    // subtraction pass, the `a(...)` tail of ObfHr.sunlight/pointlight.
    if params.shadows {
        let blurred = if params.smoothing.abs() > 0.0 {
            selective_blur(
                field,
                &occlusion,
                &surface,
                &normals,
                params.smoothing.abs(),
            )
        } else {
            occlusion.clone()
        };
        for index in 0..facing.len() {
            if field.values[index] > 0.0 {
                facing[index] -= blurred[index].max(0.0) * params.occlusion;
            }
        }
    }
    facing
}

/// Normal-weighted gaussian blur restricted to surface cells.
fn selective_blur(
    field: &Grid,
    values: &[f32],
    surface: &[bool],
    normals: &[Option<Vector3<f32>>],
    sigma: f32,
) -> Vec<f32> {
    let reach = (2.0 * sigma).ceil() as i32;
    let mut result = values.to_vec();
    for y in 0..field.size.y {
        for z in 0..field.size.z {
            for x in 0..field.size.x {
                let index = field.index(x, y, z);
                if !surface[index] {
                    continue;
                }
                let Some(normal) = normals[index] else {
                    continue;
                };
                let mut weighted_sum = 0.0;
                let mut weight_total = 0.0;
                for dy in -reach..=reach {
                    for dz in -reach..=reach {
                        for dx in -reach..=reach {
                            let (lx, ly, lz) = (x as i32 + dx, y as i32 + dy, z as i32 + dz);
                            let Some(neighbor_value) = field.get(lx, ly, lz) else {
                                continue;
                            };
                            let Some(nindex) = field.index_opt(lx, ly, lz) else {
                                continue;
                            };
                            if neighbor_value != 1.0
                                || !surface.get(nindex).copied().unwrap_or(false)
                                || !normals.get(nindex).is_some_and(Option::is_some)
                            {
                                continue;
                            }
                            if let Some(Some(neighbor_normal)) = normals.get(nindex) {
                                let alignment = (normal.x * neighbor_normal.x
                                    + normal.y * neighbor_normal.y
                                    + normal.z * neighbor_normal.z)
                                    .max(0.0);
                                let distance_sq = (dx * dx + dy * dy + dz * dz) as f32;
                                let weight =
                                    (-(distance_sq / (2.0 * sigma * sigma))).exp() * alignment;
                                weighted_sum += values[nindex] * weight;
                                weight_total += weight;
                            }
                        }
                    }
                }
                if weight_total != 0.0 {
                    result[index] = weighted_sum / weight_total;
                }
            }
        }
    }
    result
}

impl Grid {
    /// Flat index from local coordinates, `None` when out of bounds.
    fn index_opt(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        if x < 0 || y < 0 || z < 0 {
            return None;
        }
        let (x, y, z) = (x as usize, y as usize, z as usize);
        (x < self.size.x && y < self.size.y && z < self.size.z).then(|| self.index(x, y, z))
    }

    /// Local coordinates of a flat index.
    const fn coords_of(&self, index: usize) -> (usize, usize, usize) {
        let x = index % self.size.x;
        let rest = index / self.size.x;
        let z = rest % self.size.z;
        let y = rest / self.size.z;
        (x, y, z)
    }
}

/// First and last occupied coordinate along an axis within the same
/// perpendicular line (relative gradients).
fn axis_run(field: &Grid, axis: usize, x: usize, y: usize, z: usize) -> (usize, usize) {
    let axis_size = match axis {
        0 => field.size.x,
        1 => field.size.y,
        _ => field.size.z,
    };
    let mut first = 0;
    let mut last = axis_size - 1;
    for step in 0..axis_size {
        let index = match axis {
            0 => field.index(step, y, z),
            1 => field.index(x, step, z),
            _ => field.index(x, y, step),
        };
        if field.values[index] > 0.0 {
            first = step;
            break;
        }
    }
    for step in (0..axis_size).rev() {
        let index = match axis {
            0 => field.index(step, y, z),
            1 => field.index(x, step, z),
            _ => field.index(x, y, step),
        };
        if field.values[index] > 0.0 {
            last = step;
            break;
        }
    }
    (first, last)
}

/// `Palette.getNormalized`: clamp to `[0, 0.999]` and scale.
fn palette_normalized(palette: &[BlockStateId], value: f32) -> Option<BlockStateId> {
    let clamped = value.clamp(0.0, 0.999);
    palette
        .get((clamped * palette.len() as f32) as usize)
        .copied()
}

/// `Palette.getShifted`: walk `amount` distinct palette entries from
/// the block's current position.
fn palette_shift(
    palette: &[BlockStateId],
    block: BlockStateId,
    amount: i32,
) -> Option<BlockStateId> {
    let start = palette.iter().position(|state| *state == block)?;
    let sign = amount.signum();
    let mut remaining = amount.unsigned_abs() as usize;
    let mut offset = 0usize;
    let mut current = block;
    while remaining > 0 {
        let candidate = start as i32 + offset as i32 * sign;
        if candidate < 0 || candidate as usize >= palette.len() {
            break;
        }
        let next = palette[candidate as usize];
        if next != current {
            remaining -= 1;
        }
        offset += 1;
        current = next;
    }
    Some(current)
}

/// Dominant-axis block line between two points (ObfIu.getBlocksInLine).
fn line_blocks(a: Vector3<f64>, b: Vector3<f64>) -> Vec<BlockPos> {
    let (x1, y1, z1) = (a.x.floor() as i32, a.y.floor() as i32, a.z.floor() as i32);
    let (x2, y2, z2) = (b.x.floor() as i32, b.y.floor() as i32, b.z.floor() as i32);
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let dz = (z2 - z1).abs();
    let mut blocks = Vec::new();
    if dx + dy + dz == 0 {
        blocks.push(BlockPos::new(x1, y1, z1));
        return blocks;
    }
    let sign = |from: i32, to: i32| i32::from(to > from) - i32::from(from > to);
    let d_max = dx.max(dy).max(dz);
    if d_max == dx {
        for step in 0..=dx {
            blocks.push(BlockPos::new(
                x1 + step * sign(x1, x2),
                y1 + ((step * dy) as f32 / dx as f32).round() as i32 * sign(y1, y2),
                z1 + ((step * dz) as f32 / dx as f32).round() as i32 * sign(z1, z2),
            ));
        }
    } else if d_max == dy {
        for step in 0..=dy {
            blocks.push(BlockPos::new(
                x1 + ((step * dx) as f32 / dy as f32).round() as i32 * sign(x1, x2),
                y1 + step * sign(y1, y2),
                z1 + ((step * dz) as f32 / dy as f32).round() as i32 * sign(z1, z2),
            ));
        }
    } else {
        for step in 0..=dz {
            blocks.push(BlockPos::new(
                x1 + ((step * dx) as f32 / dz as f32).round() as i32 * sign(x1, x2),
                y1 + ((step * dy) as f32 / dz as f32).round() as i32 * sign(y1, y2),
                z1 + step * sign(z1, z2),
            ));
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_curve_matches_ezedits() {
        // b(0.5, 0, 0) == 0.5
        assert!((tone_curve(0.5, 0.0, 0.0) - 0.5).abs() < 1e-6);
        // contrast 1 doubles the slope around the shifted midpoint
        assert!(tone_curve(0.25, 0.0, 1.0) < 0.25);
    }

    #[test]
    fn map_clamped_clamps() {
        assert_eq!(map_clamped(0.5, 0.0, 1.0, 0.0, 1.0), 0.5);
        assert_eq!(map_clamped(-1.0, 0.0, 1.0, 0.0, 1.0), 0.0);
        assert_eq!(map_clamped(2.0, 0.0, 1.0, 0.0, 1.0), 1.0);
        assert_eq!(map_clamped(90.0, 0.0, 180.0, -1.0, 1.0), 0.0);
    }

    #[test]
    fn palette_normalized_maps_range() {
        let palette = vec![
            parse_state_str("stone").unwrap(),
            parse_state_str("dirt").unwrap(),
            parse_state_str("grass_block").unwrap(),
        ];
        assert_eq!(palette_normalized(&palette, 0.0), Some(palette[0]));
        assert_eq!(palette_normalized(&palette, 0.5), Some(palette[1]));
        assert_eq!(palette_normalized(&palette, 1.0), Some(palette[2]));
    }

    #[test]
    fn palette_shift_walks_entries() {
        let palette: Vec<BlockStateId> = ["stone", "dirt", "grass_block", "sand"]
            .iter()
            .map(|name| parse_state_str(name).unwrap())
            .collect();
        let shifted = palette_shift(&palette, palette[1], 2).unwrap();
        assert_eq!(shifted, palette[3]);
        let back = palette_shift(&palette, palette[3], -2).unwrap();
        assert_eq!(back, palette[1]);
        // Unrelated blocks stay untouched.
        let foreign = parse_state_str("glass").unwrap();
        assert_eq!(palette_shift(&palette, foreign, 2), None);
    }

    #[test]
    fn cross_blur_smooths_a_spike() {
        let mut grid = Grid::new(BlockPos::new(0, 0, 0), Vector3::new(5, 5, 5));
        grid.set(2, 2, 2, 1.0);
        let smoothed = grid.smoothed(1, 1);
        // The spike value dropped and neighbours picked some up.
        assert!(smoothed.values[grid.index(2, 2, 2)] < 1.0);
        assert!(smoothed.values[grid.index(2, 2, 1)] > 0.0);
        // Far cells stay zero.
        assert_eq!(smoothed.values[grid.index(0, 0, 0)], 0.0);
    }

    #[test]
    fn line_hits_endpoints() {
        let a = Vector3::new(0.0, 0.0, 0.0);
        let b = Vector3::new(4.0, 0.0, 0.0);
        let blocks = line_blocks(a, b);
        assert_eq!(blocks.len(), 5);
        assert_eq!(blocks.last(), Some(&BlockPos::new(4, 0, 0)));
    }
}
