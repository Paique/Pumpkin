//! Global mask and confirmation commands.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};

use crate::commands::{ONLY_PLAYERS, held_block, player_of, reply};
use crate::edit::confirm_pending;
use crate::mask::Mask;
use crate::session::WeState;

const PERM_MASK: &str = "carrot:worldedit.mask";
const PERM_CONFIRM: &str = "carrot:worldedit.confirm";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers `//gmask` and `//confirm`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(PERM_MASK, "Allows setting the session-wide mask.", OP_TWO);
    context.register_permission(PERM_CONFIRM, "Allows confirming large edits.", OP_TWO);

    let executor = GmaskExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//gmask",
        command("//gmask", "Sets or clears the session mask.")
            .executes(executor.clone())
            .then(argument("mask", StringArgumentType::GreedyPhrase).executes(executor)),
        PERM_MASK,
    );

    context.register_command(
        "//confirm",
        command("//confirm", "Applies a pending large edit.").executes(ConfirmExecutor {
            state: Arc::clone(state),
        }),
        PERM_CONFIRM,
    );
}

/// `//gmask [mask]` — without arguments the mask is cleared.
#[derive(Clone)]
struct GmaskExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for GmaskExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Ok(raw) = StringArgumentType::get(context, "mask") {
            match Mask::parse(raw, held_block(&player)) {
                Ok(mask) => {
                    guard.gmask = Some(mask);
                    drop(guard);
                    reply(context, format!("Máscara global definida: {raw}"));
                    Ok(1)
                }
                Err(message) => {
                    drop(guard);
                    reply(context, format!("Máscara inválida: {message}"));
                    Ok(0)
                }
            }
        } else {
            guard.gmask = None;
            drop(guard);
            reply(context, "Máscara global limpa.".to_string());
            Ok(1)
        }
    }
}

/// `//confirm`.
struct ConfirmExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for ConfirmExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        tokio::spawn(async move {
            confirm_pending(&state, &player).await;
        });
        reply(context, "Confirmando...".to_string());
        Ok(1)
    }
}
