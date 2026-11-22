//! The `VoxelSniper` `/b` tool command plus `/v` and `/vr`.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::CommandExecutor;

use crate::commands::{
    ONLY_PLAYERS, PERM_SNIPER, clamp_radius, number, parse_pattern, player_of, raw_args, reply,
    tokens,
};
use crate::sniper::{ErodeParams, SniperBrush, SniperState};
use crate::state::TerrainState;

/// Registers `/b`, `/v` and `/vr`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    let executor = SniperExecutor {
        state: Arc::clone(state),
    };
    let brush = command("b", "Tool de brush VoxelSniper (arrow/gunpowder).")
        .executes(executor.clone())
        .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor));
    context.register_command("b", brush, PERM_SNIPER);

    let executor = VoxelExecutor {
        state: Arc::clone(state),
        replace: false,
    };
    context.register_command(
        "v",
        command("v", "Padrão do brush sniper (/v).").executes(executor),
        PERM_SNIPER,
    );
    let executor = VoxelExecutor {
        state: Arc::clone(state),
        replace: true,
    };
    context.register_command(
        "vr",
        command("vr", "Restrição de substituição do sniper (/vr).").executes(executor),
        PERM_SNIPER,
    );
}

/// `/b [args]` — brush selection and size.
#[derive(Clone)]
struct SniperExecutor {
    state: Arc<TerrainState>,
}

#[allow(clippy::too_many_lines)] // one parser per brush alias
impl CommandExecutor for SniperExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        let args = tokens(&raw);
        if args.is_empty() {
            let status = {
                let player_state = self.state.player(player.gameprofile.id);
                let guard = player_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let sniper: &SniperState = &guard.sniper;
                format!(
                    "Sniper: {} | raio {} | padrão {} | substituir {}",
                    sniper.brush.label(),
                    sniper.size,
                    carrot_worldedit::states::state_to_string(
                        sniper
                            .pattern
                            .apply(&pumpkin_util::math::position::BlockPos::new(0, 0, 0), None)
                            .unwrap_or(pumpkin_data::BlockStateId::AIR),
                    ),
                    sniper
                        .replace
                        .as_ref()
                        .map_or("todos".to_string(), |_| "restrito".to_string()),
                )
            };
            reply(context, status);
            return Ok(1);
        }

        let first = args[0].to_ascii_lowercase();
        let update =
            |brush: SniperBrush, extra: Box<dyn FnOnce(&mut SniperState) + Send>| -> String {
                let player_state = self.state.player(player.gameprofile.id);
                let mut guard = player_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.sniper.brush = brush;
                extra(&mut guard.sniper);
                guard.sniper.brush.label().to_string()
            };

        // A bare number sets the size.
        if let Ok(size) = first.parse::<u32>() {
            let size = clamp_radius(&self.state, size.max(1));
            let player_state = self.state.player(player.gameprofile.id);
            let mut guard = player_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.sniper.size = size;
            reply(context, format!("Raio do sniper definido: {size}"));
            return Ok(1);
        }

        let label = match first.as_str() {
            "ball" | "b" => {
                let true_circle = args.get(1).is_some_and(|value| value == "true");
                update(
                    SniperBrush::Ball,
                    Box::new(move |sniper| sniper.true_circle = true_circle),
                )
            }
            "cylinder" | "c" => {
                let height = args
                    .get(1)
                    .map_or(1, |value| number(value, "altura").unwrap_or(1));
                let center = args
                    .get(2)
                    .map_or(0, |value| number(value, "centro").unwrap_or(0));
                let true_circle = args.get(3).is_some_and(|value| value == "true");
                update(
                    SniperBrush::Cylinder,
                    Box::new(move |sniper| {
                        sniper.voxel_height = height;
                        sniper.cylinder_center = center;
                        sniper.true_circle = true_circle;
                    }),
                )
            }
            "disc" | "d" => {
                let true_circle = args.get(1).is_some_and(|value| value == "true");
                update(
                    SniperBrush::Disc,
                    Box::new(move |sniper| sniper.true_circle = true_circle),
                )
            }
            "erode" | "e" => {
                let params = parse_erode(&args[1..]);
                update(SniperBrush::Erode(params), Box::new(|_| {}))
            }
            "erodeblend" | "eb" => {
                let params = parse_erode(&args[1..]);
                update(SniperBrush::ErodeBlend(params), Box::new(|_| {}))
            }
            "blendball" | "blend_ball" | "bb" => {
                let water = args.iter().any(|value| value == "water");
                update(
                    SniperBrush::BlendBall,
                    Box::new(move |sniper| sniper.water_excluded = water),
                )
            }
            "blenddisc" | "blend_disc" | "bd" => {
                let water = args.iter().any(|value| value == "water");
                update(
                    SniperBrush::BlendDisc,
                    Box::new(move |sniper| sniper.water_excluded = water),
                )
            }
            other => {
                reply(
                    context,
                    format!(
                        "Brush desconhecido: {other} (ball, cylinder, disc, erode, bb, bd, eb ou um número)."
                    ),
                );
                return Ok(0);
            }
        };
        reply(context, format!("Sniper selecionado: {label}"));
        Ok(1)
    }
}

/// `preset` name or four explicit numbers (defaults to melt).
fn parse_erode(args: &[String]) -> ErodeParams {
    if let Some(name) = args.first()
        && let Some(params) = ErodeParams::preset(name)
    {
        return params;
    }
    let pick = |index: usize, default: u32| -> u32 {
        args.get(index)
            .and_then(|value| value.parse().ok())
            .unwrap_or(default)
    };
    ErodeParams {
        erode_faces: pick(0, 2),
        erode_rec: pick(1, 1),
        fill_faces: pick(2, 5),
        fill_rec: pick(3, 1),
    }
}

/// `/v [pattern]` and `/vr [pattern]` — without arguments both pick the
/// targeted block, like FAVS.
#[derive(Clone)]
struct VoxelExecutor {
    state: Arc<TerrainState>,
    replace: bool,
}

impl CommandExecutor for VoxelExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        let world = player.world();
        let target = player.get_target_block(&world, 256.0);
        let label = if self.replace { "/vr" } else { "/v" };

        if raw.trim().is_empty() {
            let Some(target) = target else {
                reply(context, "Nenhum bloco no alvo.".to_string());
                return Ok(0);
            };
            let state = world.get_block_state_id(&target);
            let player_state = self.state.player(player.gameprofile.id);
            let mut guard = player_state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.replace {
                guard.sniper.replace =
                    Some(carrot_worldedit::mask::Mask::States(vec![(state, true)]));
            } else {
                guard.sniper.pattern = carrot_worldedit::pattern::Pattern::Single(state);
            }
            reply(
                context,
                format!(
                    "{label} definido para {}.",
                    carrot_worldedit::states::state_to_string(state)
                ),
            );
            return Ok(1);
        }

        if self.replace {
            match carrot_worldedit::mask::Mask::parse(&raw, crate::commands::held_block(&player)) {
                Ok(mask) => {
                    let player_state = self.state.player(player.gameprofile.id);
                    let mut guard = player_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.sniper.replace = Some(mask);
                    reply(context, format!("{label} definido: {raw}"));
                    Ok(1)
                }
                Err(message) => {
                    reply(context, format!("{label} inválido: {message}"));
                    Ok(0)
                }
            }
        } else {
            match parse_pattern(&player, &raw) {
                Ok(pattern) => {
                    let player_state = self.state.player(player.gameprofile.id);
                    let mut guard = player_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.sniper.pattern = pattern;
                    reply(context, format!("{label} definido: {raw}"));
                    Ok(1)
                }
                Err(message) => {
                    reply(context, format!("{label} inválido: {message}"));
                    Ok(0)
                }
            }
        }
    }
}
