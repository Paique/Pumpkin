//! `/speed`: sets fly or walk speed on the `EssentialsX` scale (1 = 0.1).

use std::sync::{Arc, PoisonError};

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, argument, command},
    argument_types::core::{float::FloatArgumentType, string::StringArgumentType},
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::permission::PermissionDefault;

use crate::{commands::feedback, state::EssentialsState};

const PERMISSION: &str = "carrot:essentials.speed";
const DESCRIPTION: &str = "Sets your fly or walk speed (EssentialsX scale, 1-8).";
/// One `EssentialsX` speed unit equals this much client ability speed.
const SPEED_SCALE: f32 = 10.0;

/// Registers `/speed <value> [walk|fly]`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    context.register_permission(
        PERMISSION,
        "Allows changing your own movement speed.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "speed",
        command("speed", DESCRIPTION).then(
            argument("value", FloatArgumentType::with_min(0.0))
                .executes(SpeedExecutor {
                    state: Arc::clone(state),
                })
                .then(
                    argument("mode", StringArgumentType::SingleWord).executes(SpeedExecutor {
                        state: Arc::clone(state),
                    }),
                ),
        ),
        PERMISSION,
    );
}

struct SpeedExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for SpeedExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let value = FloatArgumentType::get(context, "value")?;
        let max = self.state.max_speed();
        if value < 0.0 || value > max {
            feedback(context, format!("A velocidade deve estar entre 0 e {max}."));
            return Ok(0);
        }

        let mode_arg = StringArgumentType::get(context, "mode").ok();
        let flying = player
            .abilities
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .flying;
        let apply_fly = match mode_arg {
            Some(mode) if mode.eq_ignore_ascii_case("fly") => true,
            Some(mode) if mode.eq_ignore_ascii_case("walk") => false,
            Some(_) => {
                feedback(context, "Modo inválido: use 'fly' ou 'walk'.".to_string());
                return Ok(0);
            }
            // `EssentialsX` defaults to whichever movement is active.
            None => flying,
        };

        let factor = value / SPEED_SCALE;
        let mut abilities = player
            .abilities
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if apply_fly {
            abilities.fly_speed = factor;
        } else {
            abilities.walk_speed = factor;
        }
        drop(abilities);
        player.send_abilities_update();
        let kind = if apply_fly { "voo" } else { "caminhada" };
        feedback(
            context,
            format!("Velocidade de {kind} definida para {value}."),
        );
        Ok(1)
    }
}
