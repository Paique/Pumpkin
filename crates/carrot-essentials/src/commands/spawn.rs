//! `/spawn`: teleport to the configured global spawn world (reference
//! server: `byv`), falling back to the player's current world when that world
//! is not loaded.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, command},
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_core::entity::EntityBase;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::PermissionDefault;

use crate::{commands::feedback, state::EssentialsState};

const PERMISSION: &str = "carrot:essentials.spawn";
const DESCRIPTION: &str = "Teleports to the global spawn.";

/// Registers `/spawn`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    context.register_permission(
        PERMISSION,
        "Allows teleporting to the global spawn.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "spawn",
        command("spawn", DESCRIPTION).executes(SpawnExecutor {
            state: Arc::clone(state),
        }),
        PERMISSION,
    );
}

struct SpawnExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for SpawnExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let server = context.source.server();
        let configured = self.state.spawn_world();
        let world = server
            .worlds
            .load()
            .iter()
            .find(|world| world.get_world_name() == configured)
            .cloned()
            .unwrap_or_else(|| player.world());

        let level = world.level_info.load();
        // Block coordinates: center the player on the spawn block.
        let position = Vector3::new(
            f64::from(level.spawn_x) + 0.5,
            f64::from(level.spawn_y),
            f64::from(level.spawn_z) + 0.5,
        );
        player.teleport(
            position,
            Some(level.spawn_yaw),
            Some(level.spawn_pitch),
            world,
        );
        feedback(context, "Teleportado ao spawn.".to_string());
        Ok(1)
    }
}
