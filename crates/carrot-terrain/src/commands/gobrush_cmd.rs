//! `/gobrush` (alias `/gb`) — the goBrush heightmap tool command.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::CommandExecutor;
use pumpkin_util::text::TextComponent;

use crate::commands::{ONLY_PLAYERS, PERM_GOBRUSH, player_of, raw_args, reply, tokens};
use crate::state::TerrainState;

/// Registers `/gobrush` and `/gb`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    let executor = GoBrushExecutor {
        state: Arc::clone(state),
    };
    for name in ["gobrush", "gb"] {
        let builder = command(name, "Brush de heightmap goBrush (flint).")
            .executes(executor.clone())
            .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor.clone()));
        context.register_command(name, builder, PERM_GOBRUSH);
    }
}

/// `/gb [subcommand]` — size, intensity, brush, toggle and modes.
#[derive(Clone)]
struct GoBrushExecutor {
    state: Arc<TerrainState>,
}

#[allow(clippy::too_many_lines)] // one parser per subcommand
impl CommandExecutor for GoBrushExecutor {
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

        let player_state = self.state.player(player.gameprofile.id);
        let mut guard = player_state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let gobrush = &mut guard.gobrush;

        if args.is_empty() {
            player.send_system_message(&TextComponent::text(gobrush.status()));
            player.send_system_message(&TextComponent::text(
                "/gb size|s N · intensity|i N · brush|b nome · toggle|t · up|down · flat · 3d · rotate · list"
                    .to_string(),
            ));
            return Ok(1);
        }

        let (max_size, max_intensity) = self
            .state
            .with_config(|config| (config.gobrush_max_size, config.gobrush_max_intensity));
        match args[0].to_ascii_lowercase().as_str() {
            "size" | "s" => {
                let Some(value) = args.get(1).and_then(|token| token.parse::<i32>().ok()) else {
                    reply(context, "/gb size <número>".to_string());
                    return Ok(0);
                };
                let mut size = value;
                if size < 5 {
                    size = 5;
                }
                if size > max_size {
                    size = max_size;
                }
                if size % 2 == 0 {
                    size += 1;
                }
                gobrush.size = size;
                reply(context, format!("Tamanho: {size}"));
            }
            "intensity" | "i" => {
                let Some(value) = args.get(1).and_then(|token| token.parse::<i32>().ok()) else {
                    reply(context, "/gb intensity <número>".to_string());
                    return Ok(0);
                };
                let intensity = value.clamp(1, max_intensity);
                gobrush.intensity = intensity;
                reply(context, format!("Intensidade: {intensity}"));
            }
            "brush" | "b" => {
                let Some(name) = args.get(1) else {
                    reply(context, "/gb brush <nome> (/gb list)".to_string());
                    return Ok(0);
                };
                if self.state.heightmap(name).is_some() {
                    let file = if name.to_ascii_lowercase().ends_with(".png") {
                        name.clone()
                    } else {
                        format!("{name}.png")
                    };
                    gobrush.brush = file;
                    reply(context, format!("Brush: {}", gobrush.brush));
                } else {
                    reply(context, format!("Brush não encontrado: {name} (/gb list)"));
                    return Ok(0);
                }
            }
            "toggle" | "t" => {
                gobrush.enabled = !gobrush.enabled;
                reply(
                    context,
                    if gobrush.enabled {
                        "goBrush ativado (flint)."
                    } else {
                        "goBrush desativado."
                    },
                );
            }
            "up" => {
                gobrush.direction_up = true;
                reply(context, "Direção: cima.".to_string());
            }
            "down" => {
                gobrush.direction_up = false;
                reply(context, "Direção: baixo.".to_string());
            }
            "flat" => {
                gobrush.flat_mode = !gobrush.flat_mode;
                reply(
                    context,
                    format!(
                        "Modo plano: {}",
                        if gobrush.flat_mode { "on" } else { "off" }
                    ),
                );
            }
            "3d" => {
                gobrush.mode_3d = !gobrush.mode_3d;
                reply(
                    context,
                    format!("Modo 3D: {}", if gobrush.mode_3d { "on" } else { "off" }),
                );
            }
            "rotate" => {
                gobrush.auto_rotation = !gobrush.auto_rotation;
                reply(
                    context,
                    format!(
                        "Auto-rotação: {}",
                        if gobrush.auto_rotation { "on" } else { "off" }
                    ),
                );
            }
            "list" => {
                let names = self.state.heightmap_names().join(", ");
                reply(context, format!("Brushes: {names}"));
            }
            "info" => {
                player.send_system_message(&TextComponent::text(gobrush.status()));
            }
            other => {
                reply(context, format!("Subcomando desconhecido: {other}"));
                return Ok(0);
            }
        }
        Ok(1)
    }
}
