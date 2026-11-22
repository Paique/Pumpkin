//! `/tpa`, `/tpaccept` and `/tpdeny`: instant teleport requests.
//!
//! Parity with the reference server: no teleport warmup (delay 0) and
//! requests expire after the configured timeout (`tpa-accept-cancellation`).

use std::sync::Arc;
use std::time::Instant;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, argument, command},
    argument_types::core::string::StringArgumentType,
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::permission::PermissionDefault;
use pumpkin_util::text::TextComponent;

use crate::{
    commands::feedback,
    location::Location,
    state::{EssentialsState, TpaRequest},
};

const TPA_PERMISSION: &str = "carrot:essentials.tpa";
const TPACCEPT_PERMISSION: &str = "carrot:essentials.tpaccept";
const TPDENY_PERMISSION: &str = "carrot:essentials.tpdeny";

/// Registers `/tpa`, `/tpaccept` and `/tpdeny`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<EssentialsState>) {
    context.register_permission(
        TPA_PERMISSION,
        "Allows asking another player to teleport to them.",
        PermissionDefault::Allow,
    );
    context.register_permission(
        TPACCEPT_PERMISSION,
        "Allows accepting teleport requests.",
        PermissionDefault::Allow,
    );
    context.register_permission(
        TPDENY_PERMISSION,
        "Allows denying teleport requests.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "tpa",
        command("tpa", "Asks a player to let you teleport to them.").then(
            argument("target", StringArgumentType::SingleWord).executes(TpaExecutor {
                state: Arc::clone(state),
            }),
        ),
        TPA_PERMISSION,
    );
    context.register_command(
        "tpaccept",
        command("tpaccept", "Accepts a pending teleport request.")
            .executes(TpacceptExecutor {
                state: Arc::clone(state),
            })
            .then(
                argument("sender", StringArgumentType::SingleWord).executes(TpacceptExecutor {
                    state: Arc::clone(state),
                }),
            ),
        TPACCEPT_PERMISSION,
    );
    context.register_command(
        "tpdeny",
        command("tpdeny", "Denies a pending teleport request.")
            .executes(TpdenyExecutor {
                state: Arc::clone(state),
            })
            .then(
                argument("sender", StringArgumentType::SingleWord).executes(TpdenyExecutor {
                    state: Arc::clone(state),
                }),
            ),
        TPDENY_PERMISSION,
    );
}

fn notify(player: &Arc<pumpkin_core::entity::player::Player>, message: String) {
    player.send_system_message(&TextComponent::text(message));
}

struct TpaExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for TpaExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(sender) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let target_name = StringArgumentType::get(context, "target")?;
        if target_name.eq_ignore_ascii_case(&sender.gameprofile.name) {
            feedback(
                context,
                "Você não pode pedir teleporte para si mesmo.".to_string(),
            );
            return Ok(0);
        }
        let Some(target) = context.source.server().get_player_by_name(target_name) else {
            feedback(context, format!("Jogador '{target_name}' não encontrado."));
            return Ok(0);
        };

        self.state.insert_tpa(
            target.gameprofile.id,
            TpaRequest {
                sender: sender.gameprofile.id,
                sender_name: sender.gameprofile.name.clone(),
                created: Instant::now(),
            },
        );
        notify(
            &target,
            format!("{} quer teleportar até você.", sender.gameprofile.name),
        );
        notify(
            &target,
            "Use /tpaccept para aceitar ou /tpdeny para recusar.".to_string(),
        );
        feedback(
            context,
            format!(
                "Pedido de teleporte enviado para {}.",
                target.gameprofile.name
            ),
        );
        Ok(1)
    }
}

struct TpacceptExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for TpacceptExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(target) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let sender_name = StringArgumentType::get(context, "sender").ok();
        let Some(request) = self.state.take_tpa(target.gameprofile.id, sender_name) else {
            feedback(
                context,
                "Você não tem pedidos de teleporte pendentes.".to_string(),
            );
            return Ok(0);
        };
        let Some(sender) = context
            .source
            .server()
            .get_player_by_name(&request.sender_name)
        else {
            feedback(
                context,
                format!("{} saiu do servidor.", request.sender_name),
            );
            return Ok(0);
        };

        // Delay 0 on the reference server: teleport immediately.
        let location = Location::of(&target);
        if let Err(err) = location.teleport(&sender, context.source.server()) {
            feedback(context, format!("Falha ao teleportar: {err}."));
            return Ok(0);
        }
        notify(&sender, "Seu pedido de teleporte foi aceito.".to_string());
        feedback(
            context,
            format!("Pedido de {} aceito.", request.sender_name),
        );
        Ok(1)
    }
}

struct TpdenyExecutor {
    state: Arc<EssentialsState>,
}

impl CommandExecutor for TpdenyExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(target) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let sender_name = StringArgumentType::get(context, "sender").ok();
        let Some(request) = self.state.take_tpa(target.gameprofile.id, sender_name) else {
            feedback(
                context,
                "Você não tem pedidos de teleporte pendentes.".to_string(),
            );
            return Ok(0);
        };
        if let Some(sender) = context
            .source
            .server()
            .get_player_by_name(&request.sender_name)
        {
            notify(&sender, "Seu pedido de teleporte foi recusado.".to_string());
        }
        feedback(
            context,
            format!("Pedido de {} recusado.", request.sender_name),
        );
        Ok(1)
    }
}
