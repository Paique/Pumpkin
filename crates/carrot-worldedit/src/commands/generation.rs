//! Shape generation commands: `//cyl`, `//hcyl`, `//sphere`, `//hsphere`.
//!
//! The geometry mirrors `EditSession#makeCylinder`/`makeSphere` from
//! WorldEdit/FAWE: radii gain `+0.5`, cells are tested on the normalized
//! ellipse/sphere equation and hollow variants keep a cell only when a
//! neighbor step along an axis leaves the shape.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};

use crate::commands::{
    ONLY_PLAYERS, check_volume_estimate, clipboard_of, held_block, player_block_pos, player_of,
    reply, spawn_edit,
};
use crate::edit::EditRequest;
use crate::pattern::Pattern;
use crate::session::WeState;

const PERM_GENERATION: &str = "carrot:worldedit.generation";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers shape generation commands and permissions.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(
        PERM_GENERATION,
        "Allows generating cylinders and spheres.",
        OP_TWO,
    );

    for (name, hollow) in [("//cyl", false), ("//hcyl", true)] {
        let executor = CylExecutor {
            state: Arc::clone(state),
            hollow,
        };
        register_with_args(
            context,
            name,
            "Generates a cylinder.",
            executor,
            PERM_GENERATION,
        );
    }
    for (name, hollow) in [("//sphere", false), ("//hsphere", true)] {
        let executor = SphereExecutor {
            state: Arc::clone(state),
            hollow,
        };
        register_with_args(
            context,
            name,
            "Generates a sphere.",
            executor,
            PERM_GENERATION,
        );
    }
}

/// `<pattern> <args...>` shape command shape.
fn register_with_args(
    context: &mut ModuleContext<'_>,
    name: &'static str,
    description: &'static str,
    executor: impl CommandExecutor + Clone + 'static,
    permission: &str,
) {
    context.register_command(
        name,
        command(name, description)
            .executes(executor.clone())
            .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
        permission,
    );
}

/// Parsed shape arguments: pattern plus comma-separated radii.
struct ShapeArgs {
    pattern: Pattern,
    radii: [f64; 3],
    height: i32,
    hollow_flag: bool,
}

/// Parses `<pattern> <radius[,...]> [height] [-h]`.
fn parse_shape_args(
    raw: &str,
    hand: Option<BlockStateId>,
    radii_count: usize,
) -> Result<ShapeArgs, String> {
    let mut tokens = raw.split_whitespace();
    let Some(pattern_token) = tokens.next() else {
        return Err("use: <padrão> <raio[,raioZ][,raioY]> [altura] [-h]".to_string());
    };
    let pattern = Pattern::parse(pattern_token, hand)?;

    let Some(radius_token) = tokens.next() else {
        return Err("raio faltando".to_string());
    };
    let mut radii = [1.0f64; 3];
    let values: Vec<f64> = radius_token
        .split(',')
        .map(|part| {
            part.parse::<f64>()
                .map_err(|_| format!("raio inválido: {part}"))
        })
        .collect::<Result<_, _>>()?;
    if values.is_empty() || values.len() > radii_count {
        return Err(format!("use até {radii_count} raios separados por vírgula"));
    }
    for (slot, value) in values.iter().enumerate() {
        if *value < 1.0 {
            return Err("raio mínimo é 1".to_string());
        }
        radii[slot] = *value;
    }
    // Unspecified axes copy the last given radius.
    for slot in values.len()..radii_count {
        radii[slot] = radii[values.len() - 1];
    }

    let mut height = 1;
    let mut hollow_flag = false;
    for token in tokens {
        if token.eq_ignore_ascii_case("-h") {
            hollow_flag = true;
        } else if let Ok(parsed) = token.parse::<i32>() {
            height = parsed;
        } else {
            return Err(format!("argumento inválido: {token}"));
        }
    }

    Ok(ShapeArgs {
        pattern,
        radii,
        height,
        hollow_flag,
    })
}

/// Approximate shape volume as `4/3·π·a·b·c` (sphere) for limit checks.
fn ellipsoid_volume(radii: [f64; 3]) -> u64 {
    let product: f64 = radii.iter().copied().product::<f64>();
    ((4.0 / 3.0) * std::f64::consts::PI * product) as u64
}

/// Applies the pattern to computed positions and spawns the edit.
fn spawn_shape_edit(
    state: &Arc<WeState>,
    player: &Arc<pumpkin_core::entity::player::Player>,
    pattern: &Pattern,
    positions: Vec<BlockPos>,
    description: String,
) {
    let clipboard = clipboard_of(state, player.gameprofile.id);
    let desired = positions
        .into_iter()
        .filter_map(|pos| {
            pattern
                .apply(&pos, clipboard.as_deref())
                .map(|st| (pos, st))
        })
        .collect();
    spawn_edit(state, player, EditRequest::new(desired, description));
}

/// `//cyl <pattern> <radius[,<radiusZ>] [height] [-h]`.
#[derive(Clone)]
struct CylExecutor {
    state: Arc<WeState>,
    hollow: bool,
}

impl CommandExecutor for CylExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args").unwrap_or_default();
        let hand = held_block(&player);
        let args = match parse_shape_args(raw, hand, 2) {
            Ok(args) => args,
            Err(message) => {
                reply(context, message);
                return Ok(0);
            }
        };
        let cylinder_volume =
            ellipsoid_volume([args.radii[0], f64::from(args.height.abs()), args.radii[1]]);
        if let Err(message) = check_volume_estimate(&self.state, cylinder_volume) {
            reply(context, message);
            return Ok(0);
        }

        let origin = player_block_pos(&player);
        let hollow = self.hollow || args.hollow_flag;
        let positions =
            cylinder_positions(origin, args.radii[0], args.radii[1], args.height, hollow);
        let description = format!("//cyl {}", raw.trim());
        spawn_shape_edit(&self.state, &player, &args.pattern, positions, description);
        reply(context, "Aplicando edição...".to_string());
        Ok(1)
    }
}

/// `//sphere <pattern> <radius[,<radiusY>[,<radiusZ>]] [-h]`.
#[derive(Clone)]
struct SphereExecutor {
    state: Arc<WeState>,
    hollow: bool,
}

impl CommandExecutor for SphereExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args").unwrap_or_default();
        let hand = held_block(&player);
        let args = match parse_shape_args(raw, hand, 3) {
            Ok(args) => args,
            Err(message) => {
                reply(context, message);
                return Ok(0);
            }
        };
        if let Err(message) = check_volume_estimate(&self.state, ellipsoid_volume(args.radii)) {
            reply(context, message);
            return Ok(0);
        }

        let origin = player_block_pos(&player);
        let hollow = self.hollow || args.hollow_flag;
        let positions = sphere_positions(origin, args.radii, hollow);
        let description = format!("//sphere {}", raw.trim());
        spawn_shape_edit(&self.state, &player, &args.pattern, positions, description);
        reply(context, "Aplicando edição...".to_string());
        Ok(1)
    }
}

/// `WorldEdit` `makeCylinder`: `+0.5` radii, normalized ellipse test and a
/// hollow check against the next cell along each axis.
#[must_use]
pub fn cylinder_positions(
    origin: BlockPos,
    radius_x: f64,
    radius_z: f64,
    height: i32,
    hollow: bool,
) -> Vec<BlockPos> {
    let radius_x = radius_x + 0.5;
    let radius_z = radius_z + 0.5;
    let (base_y, layers) = if height < 0 {
        (origin.0.y - (-height), height.unsigned_abs() as i32)
    } else {
        (origin.0.y, height)
    };

    let inv_x = 1.0 / radius_x;
    let inv_z = 1.0 / radius_z;
    let ceil_x = radius_x.ceil() as i32;
    let ceil_z = radius_z.ceil() as i32;

    let mut positions = Vec::new();
    'outer_x: for x in 0..=ceil_x {
        let xn = f64::from(x) * inv_x;
        let next_xn = f64::from(x + 1) * inv_x;
        for z in 0..=ceil_z {
            let zn = f64::from(z) * inv_z;
            let next_zn = f64::from(z + 1) * inv_z;
            let distance = xn * xn + zn * zn;
            if distance > 1.0 {
                if z == 0 {
                    break 'outer_x;
                }
                break;
            }
            if hollow && zn * zn + next_xn * next_xn <= 1.0 && next_zn * next_zn + xn * xn <= 1.0 {
                continue;
            }
            for layer in 0..layers {
                let y = base_y + layer;
                for (dx, dz) in [(x, z), (-x, z), (x, -z), (-x, -z)] {
                    positions.push(BlockPos(Vector3::new(origin.0.x + dx, y, origin.0.z + dz)));
                }
            }
        }
    }
    let unique: std::collections::HashSet<BlockPos> = positions.into_iter().collect();
    unique.into_iter().collect()
}

/// `WorldEdit` `makeSphere`: same conventions in three axes.
#[must_use]
pub fn sphere_positions(origin: BlockPos, radii: [f64; 3], hollow: bool) -> Vec<BlockPos> {
    let [rx, ry, rz] = radii.map(|radius| radius + 0.5);
    let inv = [1.0 / rx, 1.0 / ry, 1.0 / rz];
    let ceils = [rx.ceil() as i32, ry.ceil() as i32, rz.ceil() as i32];

    let mut positions = Vec::new();
    'outer_x: for x in 0..=ceils[0] {
        let xn = f64::from(x) * inv[0];
        let dx = xn * xn;
        let next_xn = f64::from(x + 1) * inv[0];
        let next_xn_sq = next_xn * next_xn;
        'outer_z: for z in 0..=ceils[2] {
            let zn = f64::from(z) * inv[2];
            let dz = zn * zn;
            let dxz = dx + dz;
            if dxz > 1.0 {
                if z == 0 {
                    break 'outer_x;
                }
                break 'outer_z;
            }
            let next_zn = f64::from(z + 1) * inv[2];
            let next_zn_sq = next_zn * next_zn;
            for y in 0..=ceils[1] {
                let yn = f64::from(y) * inv[1];
                let dy = yn * yn;
                let dxyz = dxz + dy;
                if dxyz > 1.0 {
                    if y == 0 {
                        if z == 0 {
                            break 'outer_x;
                        }
                        break 'outer_z;
                    }
                    break;
                }
                let next_yn = f64::from(y + 1) * inv[1];
                let next_yn_sq = next_yn * next_yn;
                if hollow
                    && next_xn_sq + dy + dz <= 1.0
                    && next_yn_sq + dxz <= 1.0
                    && next_zn_sq + dx + dy <= 1.0
                {
                    continue;
                }
                for (dxs, dys, dzs) in [
                    (x, y, z),
                    (-x, y, z),
                    (x, -y, z),
                    (-x, -y, z),
                    (x, y, -z),
                    (-x, y, -z),
                    (x, -y, -z),
                    (-x, -y, -z),
                ] {
                    positions.push(BlockPos(Vector3::new(
                        origin.0.x + dxs,
                        origin.0.y + dys,
                        origin.0.z + dzs,
                    )));
                }
            }
        }
    }
    let unique: std::collections::HashSet<BlockPos> = positions.into_iter().collect();
    unique.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORIGIN: BlockPos = BlockPos(Vector3::new(0, 64, 0));

    #[test]
    fn cylinder_radius_one_is_three_by_three() {
        let positions = cylinder_positions(ORIGIN, 1.0, 1.0, 1, false);
        // radius 1 -> +0.5 = 1.5 -> 3x3 disc.
        assert_eq!(positions.len(), 9);
    }

    #[test]
    fn hollow_cylinder_keeps_shell_only() {
        let filled = cylinder_positions(ORIGIN, 3.0, 3.0, 1, false);
        let hollow = cylinder_positions(ORIGIN, 3.0, 3.0, 1, true);
        assert!(hollow.len() < filled.len());
        assert!(hollow.contains(&BlockPos(Vector3::new(0, 64, 3))));
        assert!(!hollow.contains(&BlockPos(Vector3::new(0, 64, 0))));
    }

    #[test]
    fn negative_height_builds_downwards() {
        let positions = cylinder_positions(ORIGIN, 1.0, 1.0, -2, false);
        assert!(positions.contains(&BlockPos(Vector3::new(0, 63, 0))));
        assert!(positions.contains(&BlockPos(Vector3::new(0, 62, 0))));
    }

    #[test]
    fn sphere_radius_one_has_core() {
        let positions = sphere_positions(ORIGIN, [1.0; 3], false);
        assert!(positions.contains(&ORIGIN));
        // 3x3x3 minus edge-midpoints that fall outside the +0.5 sphere.
        assert!(positions.len() > 9 && positions.len() <= 27);
    }

    #[test]
    fn hollow_sphere_drops_core() {
        let hollow = sphere_positions(ORIGIN, [2.0; 3], true);
        assert!(!hollow.contains(&ORIGIN));
    }
}
