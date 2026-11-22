//! `//revolve`: revolves the selection around the player (Arceon port).
//!
//! Sub-block sampling against same-type neighbours, rotated
//! `facing`/`rotation`/`axis` properties and the `-p`/`-s`
//! line-connected variants.

use std::collections::HashSet;
use std::sync::Arc;

use carrot_core::ModuleContext;
use carrot_worldedit::edit::{EditRequest, ensure_chunks, execute};
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::text::TextComponent;

use crate::commands::{ONLY_PLAYERS, PERM_BRUSH, player_of, reply};
use crate::state::TerrainState;

/// Registers `//revolve` and `//rev`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    for name in ["//revolve", "//rev"] {
        let executor = RevolveExecutor {
            state: Arc::clone(state),
        };
        context.register_command(
            name,
            command(name, "Revolves the selection around you.")
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
            PERM_BRUSH,
        );
    }
}

/// `//revolve <amount> [start=0] [end=360] [heightDiff=0] [-p] [-s] [-r]`.
#[derive(Clone)]
struct RevolveExecutor {
    state: Arc<TerrainState>,
}

impl CommandExecutor for RevolveExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args")?;
        let mut tokens = raw.split_whitespace();

        let Some(amount) = tokens.next().and_then(|token| token.parse::<i32>().ok()) else {
            reply(
                context,
                "use: //revolve <copias> [inicio] [fim] [altura] [-p] [-s] [-r]".to_string(),
            );
            return Ok(0);
        };
        let mut numbers: Vec<f64> = Vec::new();
        let mut poly = false;
        let mut start_flag = false;
        let mut reverse = false;
        for token in tokens {
            match token {
                "-p" => poly = true,
                "-s" => start_flag = true,
                "-r" => reverse = true,
                other => {
                    let Ok(value) = other.parse::<f64>() else {
                        reply(context, format!("argumento inválido: {other}"));
                        return Ok(0);
                    };
                    numbers.push(value);
                }
            }
        }
        // Positional: [start] [end] [heightDiff].
        let start = numbers.first().copied().unwrap_or(0.0);
        let end = numbers.get(1).copied().unwrap_or(360.0);
        let height = numbers.get(2).copied().unwrap_or(0.0);
        if amount <= 0 || amount > 10_000 {
            reply(context, "copias entre 1 e 10000".to_string());
            return Ok(0);
        }

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        let params = Params {
            amount,
            start,
            end,
            height,
            poly,
            start_flag,
            reverse,
        };
        tokio::spawn(async move {
            run(&state, &player, params).await;
        });
        reply(context, "Revolucionando...".to_string());
        Ok(1)
    }
}

/// Parsed `//revolve` parameters.
struct Params {
    amount: i32,
    start: f64,
    end: f64,
    height: f64,
    poly: bool,
    start_flag: bool,
    reverse: bool,
}

/// Arceon `getRadian` quirk: `deg / 360 * TAU`, kept from the `//ab`
/// port so both tools agree on angles.
fn get_radian(deg: f64) -> f64 {
    deg / 360.0 * std::f64::consts::TAU
}

/// Java `Math.round` semantics (half towards positive infinity).
fn java_round(value: f64) -> f64 {
    (value + 0.5).floor()
}

/// Runs the revolve over the player's selection.
#[allow(clippy::too_many_lines)] // literal port of Arceon's revolve
async fn run(terrain: &Arc<TerrainState>, player: &Arc<Player>, params: Params) {
    let world = player.world();
    let selection = {
        let session = terrain.we.session(player.gameprofile.id);
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

    let corners = vec![min, max];
    ensure_chunks(&world, &corners, |pos: &BlockPos| pos).await;

    // Center: the player's block position, plus 0.5 like Arceon.
    let feet = player.position();
    let center =
        Vector3::new(feet.x.floor(), feet.y.floor(), feet.z.floor()) + Vector3::new(0.5, 0.5, 0.5);

    // Sub-block samples: one per block plus extra samples wherever a
    // same-type neighbour exists (Arceon's anti-aliased cache).
    let mut cache: Vec<(Vector3<f64>, BlockStateId)> = Vec::new();
    for y in min.0.y..=max.0.y {
        for z in min.0.z..=max.0.z {
            for x in min.0.x..=max.0.x {
                let pos = BlockPos::new(x, y, z);
                let state = world.get_block_state_id(&pos);
                if state == BlockStateId::AIR {
                    continue;
                }
                let block = state.to_block();
                let corner = Vector3::new(f64::from(x), f64::from(y), f64::from(z));
                let offset = |fx: f64, fz: f64| {
                    Vector3::new(
                        center.x - (corner.x + fx),
                        center.y - (corner.y + 0.5),
                        center.z - (corner.z + fz),
                    )
                };
                cache.push((offset(0.5, 0.5), state));

                for (dx, dz, fx, fz) in [
                    (-1, -1, 0.35, 0.35),
                    (-1, 0, 0.35, 0.5),
                    (-1, 1, 0.35, 0.65),
                    (0, -1, 0.5, 0.35),
                    (0, 1, 0.5, 0.65),
                    (1, -1, 0.65, 0.35),
                    (1, 0, 0.65, 0.5),
                    (1, 1, 0.65, 0.65),
                ] {
                    let neighbor = BlockPos::new(x + dx, y, z + dz);
                    let inside = neighbor.0.x >= min.0.x
                        && neighbor.0.x <= max.0.x
                        && neighbor.0.z >= min.0.z
                        && neighbor.0.z <= max.0.z;
                    if inside && world.get_block_state_id(&neighbor).to_block() == block {
                        cache.push((offset(fx, fz), state));
                    }
                }
            }
        }
    }

    let mut placed = Vec::new();
    let mut seen: HashSet<BlockPos> = HashSet::new();
    let every_degrees = (params.end - params.start) / f64::from(params.amount)
        * if params.reverse { -1.0 } else { 1.0 };

    for step in 0..params.amount {
        let degrees = every_degrees * f64::from(step) + params.start;
        let radian = get_radian(degrees);
        let x_distance = radian.sin();
        let z_distance = radian.cos();
        let step_height = params.height / f64::from(params.amount) * f64::from(step);

        let rotated = |offset: Vector3<f64>| {
            Vector3::new(
                center.x - z_distance * offset.x + x_distance * offset.z,
                center.y - offset.y + step_height,
                center.z - x_distance * offset.x - z_distance * offset.z,
            )
        };

        if !params.poly && !params.start_flag {
            for (offset, state) in &cache {
                emit(
                    &world,
                    &mut placed,
                    &mut seen,
                    rotated(*offset),
                    *state,
                    degrees,
                );
            }
        } else {
            // Line-connected variants: draw Math3D.line (5 samples per
            // block) between consecutive rotated copies.
            let connect_degrees = if params.start_flag {
                params.start
            } else {
                every_degrees * f64::from(step + 1) + params.start
            };
            let connect_height = if params.start_flag {
                params.height / f64::from(params.amount)
            } else {
                params.height / f64::from(params.amount) * f64::from(step + 1)
            };
            let connect_radian = get_radian(connect_degrees);
            let connect_x = connect_radian.sin();
            let connect_z = connect_radian.cos();
            for (offset, state) in &cache {
                let from = rotated(*offset);
                let to = Vector3::new(
                    center.x - connect_z * offset.x + connect_x * offset.z,
                    center.y - offset.y + connect_height,
                    center.z - connect_x * offset.x - connect_z * offset.z,
                );
                for sample in line(from, to) {
                    emit(&world, &mut placed, &mut seen, sample, *state, 0.0);
                }
            }
        }
    }

    let description = format!("//revolve {}", params.amount);
    execute(&terrain.we, player, EditRequest::new(placed, description)).await;
}

/// Floors the sample onto a block position and records it once.
fn emit(
    world: &Arc<pumpkin_core::world::World>,
    placed: &mut Vec<(BlockPos, BlockStateId)>,
    seen: &mut HashSet<BlockPos>,
    target: Vector3<f64>,
    state: BlockStateId,
    degrees: f64,
) {
    let pos = BlockPos::new(
        target.x.floor() as i32,
        target.y.floor() as i32,
        target.z.floor() as i32,
    );
    if world.is_in_build_limit(pos) && seen.insert(pos) {
        placed.push((pos, rotate_state(state, degrees)));
    }
}

/// `Math3D.line`: 5 samples per block of distance.
fn line(a: Vector3<f64>, b: Vector3<f64>) -> Vec<Vector3<f64>> {
    let distance = (b - a).length();
    let steps = ((distance * 5.0).ceil() as i32).max(1);
    (0..=steps)
        .map(|step| {
            let t = f64::from(step) / f64::from(steps);
            a + (b - a) * t
        })
        .collect()
}

/// Rotates `facing`, `rotation` and `axis` properties by `degrees`,
/// exactly the Arceon property ladder.
fn rotate_state(state: BlockStateId, degrees: f64) -> BlockStateId {
    let block = state.to_block();
    let Some(properties) = block.properties(state) else {
        return state;
    };
    let mut changed = false;
    let mut parsed = Vec::new();
    for property in properties.to_props() {
        let key = property.key_str();
        let value = property.value_str();
        let rotated = match key {
            "facing" => Some(rotate_facing(value, degrees)),
            "rotation" => Some(rotate_rotation(value, degrees)),
            "axis" => Some(rotate_axis(value, degrees)),
            _ => None,
        };
        if let Some(new_value) = rotated
            && let Ok(matching) = block.parse_property(key, &new_value)
        {
            parsed.push(matching);
            changed = true;
        }
    }
    if changed {
        block.apply_properties(state, &parsed)
    } else {
        state
    }
}

/// Cardinals rotate in quarters; diagonals and verticals stay.
fn rotate_facing(value: &str, degrees: f64) -> String {
    let order = ["north", "east", "south", "west"];
    let Some(start) = order.iter().position(|candidate| *candidate == value) else {
        return value.to_string();
    };
    let shift = (java_round((degrees % 360.0) / 90.0) as i32).rem_euclid(4) as usize;
    order[(start + shift) % 4].to_string()
}

/// The 0-15 `rotation` property moves in 22.5° steps.
fn rotate_rotation(value: &str, degrees: f64) -> String {
    let Ok(current) = value.parse::<i32>() else {
        return value.to_string();
    };
    let shift = java_round((degrees % 360.0) / 22.5) as i32;
    (current.rem_euclid(16) + shift).rem_euclid(16).to_string()
}

/// `x`/`z` axes swap every quarter turn; `y` never moves.
fn rotate_axis(value: &str, degrees: f64) -> String {
    if value == "y" {
        return value.to_string();
    }
    let swapped = (java_round((degrees % 360.0) / 90.0) as i32).rem_euclid(2) == 1;
    match (value, swapped) {
        (_, false) => value.to_string(),
        ("z", true) => "x".to_string(),
        _ => "z".to_string(),
    }
}

/// Re-exported for tests and future Arceon ports.
#[must_use]
pub fn sample_block(name: &str) -> Option<&'static Block> {
    Block::from_registry_key(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radian_matches_arceon_quirk() {
        assert!((get_radian(90.0) - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
    }

    #[test]
    fn facing_rotates_by_quarter_turns() {
        assert_eq!(rotate_facing("north", 90.0), "east");
        assert_eq!(rotate_facing("north", 360.0), "north");
        assert_eq!(rotate_facing("west", 90.0), "north");
        assert_eq!(rotate_facing("south", -90.0), "east");
    }

    #[test]
    fn rotation_steps_by_22_5_degrees() {
        assert_eq!(rotate_rotation("0", 90.0), "4");
        assert_eq!(rotate_rotation("15", 22.5), "0");
        assert_eq!(rotate_rotation("8", -45.0), "6");
    }

    #[test]
    fn axis_swaps_each_quarter() {
        assert_eq!(rotate_axis("x", 90.0), "z");
        assert_eq!(rotate_axis("z", 90.0), "x");
        assert_eq!(rotate_axis("x", 180.0), "x");
        assert_eq!(rotate_axis("y", 90.0), "y");
    }

    #[test]
    fn state_rotation_applies_to_stairs_facing() {
        let stairs = carrot_worldedit::states::parse_state_str(
            "minecraft:oak_stairs[facing=north,half=top]",
        )
        .expect("stairs");
        let rotated = rotate_state(stairs, 90.0);
        let text = carrot_worldedit::states::state_to_string(rotated);
        assert!(
            text.starts_with("minecraft:oak_stairs[facing=east"),
            "got {text}"
        );
    }

    #[test]
    fn line_samples_five_per_block() {
        let a = Vector3::new(0.0, 0.0, 0.0);
        let b = Vector3::new(2.0, 0.0, 0.0);
        assert_eq!(line(a, b).len(), 11);
    }
}
