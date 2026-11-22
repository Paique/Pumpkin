//! `/fly`: toggles flight for the executing player.

use std::sync::PoisonError;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, command},
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::permission::PermissionDefault;

use crate::commands::feedback;

const PERMISSION: &str = "carrot:essentials.fly";
const DESCRIPTION: &str = "Toggles flight for yourself.";

/// Registers `/fly`.
pub fn register(context: &mut ModuleContext<'_>) {
    context.register_permission(
        PERMISSION,
        "Allows toggling your own flight.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "fly",
        command("fly", DESCRIPTION).executes(FlyExecutor),
        PERMISSION,
    );
}

struct FlyExecutor;

impl CommandExecutor for FlyExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let mut abilities = player
            .abilities
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        abilities.allow_flying = !abilities.allow_flying;
        abilities.flying = abilities.allow_flying;
        let enabled = abilities.allow_flying;
        drop(abilities);
        player.send_abilities_update();
        let message = if enabled {
            "Você pode voar agora."
        } else {
            "Você não pode mais voar."
        };
        feedback(context, message.to_string());
        Ok(1)
    }
}
