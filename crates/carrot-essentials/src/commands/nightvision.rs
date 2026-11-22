//! `/n` / `/nv`: toggles night vision, the Builders-Utilities behavior
//! (effect with `i32::MAX` duration, no ambient particles).

use std::sync::PoisonError;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, command},
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_data::effect::StatusEffect;
use pumpkin_data::potion::Effect;
use pumpkin_util::permission::PermissionDefault;

use crate::commands::feedback;

const PERMISSION: &str = "carrot:essentials.nightvision";
const DESCRIPTION: &str = "Toggles night vision.";

/// Registers `/n` and `/nv`.
pub fn register(context: &mut ModuleContext<'_>) {
    context.register_permission(
        PERMISSION,
        "Allows toggling night vision.",
        PermissionDefault::Allow,
    );
    for name in ["n", "nv"] {
        context.register_command(
            name,
            command(name, DESCRIPTION).executes(NightVisionExecutor),
            PERMISSION,
        );
    }
}

struct NightVisionExecutor;

impl CommandExecutor for NightVisionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        // The guard is a temporary of this expression, released before the
        // effect APIs take their own locks.
        let active = player
            .living_entity
            .active_effects
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key(&StatusEffect::NIGHT_VISION);

        if active {
            // remove_effect drops the entry and broadcasts the removal.
            player
                .living_entity
                .remove_effect(&StatusEffect::NIGHT_VISION);
            feedback(context, "Visão noturna desativada.".to_string());
        } else {
            player.living_entity.add_effect(Effect {
                effect_type: &StatusEffect::NIGHT_VISION,
                duration: i32::MAX,
                amplifier: 0,
                ambient: true,
                show_particles: false,
                show_icon: true,
                blend: false,
            });
            feedback(context, "Visão noturna ativada.".to_string());
        }
        Ok(1)
    }
}
