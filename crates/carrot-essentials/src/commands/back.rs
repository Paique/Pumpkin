//! `/back`: returns to the location recorded before the last teleport or
//! death (see the module's event handlers in `lib.rs`).

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, command},
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::permission::PermissionDefault;

use crate::{commands::feedback, state::EssentialsState};

const PERMISSION: &str = "carrot:essentials.back";
const DESCRIPTION: &str = "Teleports to your location before the last teleport or death.";

/// Registers `/back`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    context.register_permission(
        PERMISSION,
        "Allows returning to your pre-teleport location.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "back",
        command("back", DESCRIPTION).executes(BackExecutor {
            state: Arc::clone(state),
        }),
        PERMISSION,
    );
}

struct BackExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for BackExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let Some(location) = self.state.back(player.gameprofile.id) else {
            feedback(
                context,
                "Você não tem uma localização anterior.".to_string(),
            );
            return Ok(0);
        };
        if let Err(err) = location.teleport(&player, context.source.server()) {
            feedback(context, format!("Falha ao teleportar: {err}."));
            return Ok(0);
        }
        feedback(
            context,
            "Teleportado para a sua localização anterior.".to_string(),
        );
        Ok(1)
    }
}
