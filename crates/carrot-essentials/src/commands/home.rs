//! `/home`, `/sethome` and `/delhome`: per-player named homes.
//!
//! `/home` and `/sethome` without arguments use the default home name "home",
//! like `EssentialsX`.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, argument, command},
    argument_types::core::string::StringArgumentType,
    context::command_context::CommandContext,
    errors::command_syntax_error::CommandSyntaxError,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::permission::PermissionDefault;

use crate::{commands::feedback, location::Location, state::EssentialsState};
const HOME_PERMISSION: &str = "carrot:essentials.home";
const SETHOME_PERMISSION: &str = "carrot:essentials.sethome";
const DELHOME_PERMISSION: &str = "carrot:essentials.delhome";
const DEFAULT_HOME: &str = "home";

/// Registers `/home`, `/sethome` and `/delhome`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    context.register_permission(
        HOME_PERMISSION,
        "Allows teleporting to your homes.",
        PermissionDefault::Allow,
    );
    context.register_permission(
        SETHOME_PERMISSION,
        "Allows setting your homes.",
        PermissionDefault::Allow,
    );
    context.register_permission(
        DELHOME_PERMISSION,
        "Allows deleting your homes.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "home",
        command("home", "Teleports to one of your homes.")
            .executes(HomeExecutor {
                state: Arc::clone(state),
                named: false,
            })
            .then(
                argument("name", StringArgumentType::SingleWord).executes(HomeExecutor {
                    state: Arc::clone(state),
                    named: true,
                }),
            ),
        HOME_PERMISSION,
    );
    context.register_command(
        "sethome",
        command("sethome", "Sets one of your homes.")
            .executes(SetHomeExecutor {
                state: Arc::clone(state),
                named: false,
            })
            .then(
                argument("name", StringArgumentType::SingleWord).executes(SetHomeExecutor {
                    state: Arc::clone(state),
                    named: true,
                }),
            ),
        SETHOME_PERMISSION,
    );
    context.register_command(
        "delhome",
        command("delhome", "Deletes one of your homes.")
            .executes(DelHomeExecutor {
                state: Arc::clone(state),
                named: false,
            })
            .then(
                argument("name", StringArgumentType::SingleWord).executes(DelHomeExecutor {
                    state: Arc::clone(state),
                    named: true,
                }),
            ),
        DELHOME_PERMISSION,
    );
}

fn home_name(context: &CommandContext, named: bool) -> Result<String, CommandSyntaxError> {
    if named {
        Ok(StringArgumentType::get(context, "name")?.to_string())
    } else {
        Ok(DEFAULT_HOME.to_string())
    }
}

struct HomeExecutor {
    state: Arc<EssentialsState>,
    named: bool,
}

impl CommandExecutor for HomeExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let name = home_name(context, self.named)?;
        let Some(location) = self.state.home(player.gameprofile.id, &name) else {
            feedback(context, format!("Home '{name}' não encontrada."));
            return Ok(0);
        };
        if let Err(err) = location.teleport(&player, context.source.server()) {
            feedback(context, format!("Falha ao teleportar: {err}."));
            return Ok(0);
        }
        feedback(context, format!("Teleportado para a home '{name}'."));
        Ok(1)
    }
}

struct SetHomeExecutor {
    state: Arc<EssentialsState>,
    named: bool,
}

impl CommandExecutor for SetHomeExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let name = home_name(context, self.named)?;
        match self
            .state
            .set_home(player.gameprofile.id, &name, Location::of(&player))
        {
            Ok(()) => {
                feedback(context, format!("Home '{name}' definida."));
                Ok(1)
            }
            Err(err) => {
                feedback(context, format!("Falha ao salvar a home: {err}."));
                Ok(0)
            }
        }
    }
}

struct DelHomeExecutor {
    state: Arc<EssentialsState>,
    named: bool,
}

impl CommandExecutor for DelHomeExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let name = home_name(context, self.named)?;
        match self.state.del_home(player.gameprofile.id, &name) {
            Ok(true) => {
                feedback(context, format!("Home '{name}' removida."));
                Ok(1)
            }
            Ok(false) => {
                feedback(context, format!("Home '{name}' não encontrada."));
                Ok(0)
            }
            Err(err) => {
                feedback(context, format!("Falha ao salvar as homes: {err}."));
                Ok(0)
            }
        }
    }
}
