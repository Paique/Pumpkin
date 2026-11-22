//! `/warp`, `/setwarp` and `/delwarp`: server-wide named warps.
//!
//! The reference server runs `EssentialsX` with `per-warp-permission` disabled,
//! so any player may use every warp; managing warps stays admin-only.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, argument, command},
    argument_types::core::string::StringArgumentType,
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::PermissionLvl;
use pumpkin_util::permission::PermissionDefault;

use crate::{commands::feedback, location::Location, state::EssentialsState};

const WARP_PERMISSION: &str = "carrot:essentials.warp";
const SETWARP_PERMISSION: &str = "carrot:essentials.setwarp";
const DELWARP_PERMISSION: &str = "carrot:essentials.delwarp";

/// Registers `/warp`, `/setwarp` and `/delwarp`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    context.register_permission(
        WARP_PERMISSION,
        "Allows using warps.",
        PermissionDefault::Allow,
    );
    context.register_permission(
        SETWARP_PERMISSION,
        "Allows creating and overwriting warps.",
        PermissionDefault::Op(PermissionLvl::Two),
    );
    context.register_permission(
        DELWARP_PERMISSION,
        "Allows deleting warps.",
        PermissionDefault::Op(PermissionLvl::Two),
    );
    context.register_command(
        "warp",
        command("warp", "Teleports to a warp, or lists all warps.")
            .executes(WarpListExecutor {
                state: Arc::clone(state),
            })
            .then(
                argument("name", StringArgumentType::SingleWord).executes(WarpExecutor {
                    state: Arc::clone(state),
                }),
            ),
        WARP_PERMISSION,
    );
    context.register_command(
        "setwarp",
        command("setwarp", "Sets a warp at your position.").then(
            argument("name", StringArgumentType::SingleWord).executes(SetWarpExecutor {
                state: Arc::clone(state),
            }),
        ),
        SETWARP_PERMISSION,
    );
    context.register_command(
        "delwarp",
        command("delwarp", "Deletes a warp.").then(
            argument("name", StringArgumentType::SingleWord).executes(DelWarpExecutor {
                state: Arc::clone(state),
            }),
        ),
        DELWARP_PERMISSION,
    );
}

struct WarpListExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for WarpListExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let names = self.state.warp_names();
        if names.is_empty() {
            feedback(context, "Nenhuma warp definida.".to_string());
            return Ok(0);
        }
        feedback(context, format!("Warps: {}", names.join(", ")));
        Ok(names.len() as i32)
    }
}

struct WarpExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for WarpExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let name = StringArgumentType::get(context, "name")?;
        let Some(location) = self.state.warp(name) else {
            feedback(context, format!("Warp '{name}' não encontrada."));
            return Ok(0);
        };
        if let Err(err) = location.teleport(&player, context.source.server()) {
            feedback(context, format!("Falha ao teleportar: {err}."));
            return Ok(0);
        }
        feedback(context, format!("Teleportado para a warp '{name}'."));
        Ok(1)
    }
}

struct SetWarpExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for SetWarpExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let name = StringArgumentType::get(context, "name")?;
        match self.state.set_warp(name, Location::of(&player)) {
            Ok(()) => {
                feedback(context, format!("Warp '{name}' definida."));
                Ok(1)
            }
            Err(err) => {
                feedback(context, format!("Falha ao salvar a warp: {err}."));
                Ok(0)
            }
        }
    }
}

struct DelWarpExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for DelWarpExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let name = StringArgumentType::get(context, "name")?;
        match self.state.del_warp(name) {
            Ok(true) => {
                feedback(context, format!("Warp '{name}' removida."));
                Ok(1)
            }
            Ok(false) => {
                feedback(context, format!("Warp '{name}' não encontrada."));
                Ok(0)
            }
            Err(err) => {
                feedback(context, format!("Falha ao salvar as warps: {err}."));
                Ok(0)
            }
        }
    }
}
