//! `carrot-chat`: parity with the reference build server's `nChat` —
//! local (default, world-scoped with a distance radius) and global chat
//! channels, the Brazilian Portuguese message corrector, anti-caps and URL
//! censorship.
//!
//! While enabled, the module owns player chat: the vanilla broadcast is
//! cancelled for every message and replaced by channel-scoped delivery.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod config;
pub mod filters;
pub mod state;

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_core::net::chat::{OutgoingChatMessage, PlayerChatMessage};
use pumpkin_core::plugin::{
    BoxFuture, EventHandler, EventPriority,
    events::{
        Cancellable, player::player_chat::PlayerChatEvent, player::player_leave::PlayerLeaveEvent,
    },
};
use pumpkin_core::server::Server;
use pumpkin_data::world::RAW;
use pumpkin_util::text::TextComponent;
use state::{Channel, ChatState};

/// The `chat` module.
pub struct Chat {
    state: Arc<ChatState>,
}

impl Chat {
    /// Creates the module with empty state; configuration loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(ChatState::new()),
        }
    }
}

impl Default for Chat {
    fn default() -> Self {
        Self::new()
    }
}

impl CarrotModule for Chat {
    fn id(&self) -> &'static str {
        "chat"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        self.state.reload(context.data_folder());
        register_channel_commands(context, &self.state);

        context.register_event(
            Arc::new(ChatHandler(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(LeaveHandler(Arc::clone(&self.state))),
            EventPriority::Normal,
            false,
        );
        Ok(())
    }

    fn on_disable(&self, _context: &mut ModuleContext<'_>) {
        self.state.clear_transient();
    }
}

/// Handles raw chat: the vanilla broadcast is always cancelled and the
/// message is re-routed through the player's focused channel.
struct ChatHandler(Arc<ChatState>);

impl EventHandler<PlayerChatEvent> for ChatHandler {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerChatEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            event.set_cancelled(true);
            let channel = self.0.channel_of(event.player.gameprofile.id);
            send_to_channel(&self.0, server, &event.player, &event.message, channel);
        })
    }
}

/// Drops leaving players' channel focus and cooldowns.
struct LeaveHandler(Arc<ChatState>);

impl EventHandler<PlayerLeaveEvent> for LeaveHandler {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a PlayerLeaveEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0.forget_player(event.player.gameprofile.id);
        })
    }
}

/// Applies the filters and delivers a message to a channel's recipients.
///
/// Blocked (spoofed) messages disappear silently: the sender's client has
/// already echoed the message locally, exactly like the reference `nChat`
/// `spoof-send` behavior.
fn send_to_channel(
    state: &ChatState,
    server: &Arc<Server>,
    player: &Arc<Player>,
    raw_message: &str,
    channel: Channel,
) {
    let message = raw_message.trim().to_string();
    if message.is_empty() {
        return;
    }

    let (format, distance, cooldown) = state.with_config(|config| {
        if channel == Channel::Global {
            (
                config.format_global.clone(),
                config.global.distance,
                config.global.cooldown_secs,
            )
        } else {
            (
                config.format_local.clone(),
                config.local.distance,
                config.local.cooldown_secs,
            )
        }
    });

    let outcome = state.with_config(|config| filters::apply_filters(&message, &config.filters));
    let final_message = match outcome {
        filters::FilterOutcome::Send(text) => text,
        filters::FilterOutcome::Block => {
            if !state.with_config(|config| config.filters.spoof_send) {
                player.send_system_message(&TextComponent::text(
                    "Sua mensagem foi bloqueada.".to_string(),
                ));
            }
            return;
        }
    };

    if let Some(wait) = state.check_cooldown(player.gameprofile.id, cooldown) {
        player.send_system_message(&TextComponent::text(format!(
            "Aguarde {:.0}s para falar novamente.",
            wait.ceil()
        )));
        return;
    }

    let decorated =
        TextComponent::chat_decorated(&format, &player.gameprofile.name, &final_message);
    let outgoing = PlayerChatMessage::system(final_message).with_unsigned_content(decorated);
    let tracked = OutgoingChatMessage::create(outgoing);
    let chat_type = (RAW + 1).into();
    let empty = TextComponent::empty();

    let recipients: Vec<Arc<Player>> = match channel {
        Channel::Global => server.get_all_players(),
        Channel::Local => {
            let world = player.world();
            let sender_pos = player.position();
            let max_sq = distance * distance;
            world
                .players
                .load()
                .iter()
                .filter(|recipient| {
                    if distance <= 0.0 {
                        return true;
                    }
                    let pos = recipient.position();
                    let dx = pos.x - sender_pos.x;
                    let dy = pos.y - sender_pos.y;
                    let dz = pos.z - sender_pos.z;
                    dx * dx + dy * dy + dz * dz <= max_sq
                })
                .cloned()
                .collect()
        }
    };

    for recipient in &recipients {
        recipient.send_chat_message(
            &tracked,
            recipient.is_text_filtering_enabled(),
            chat_type,
            &empty,
            None,
        );
    }
}

/// Focus feedback for `/l` / `/g` without a message.
struct FocusExecutor {
    state: Arc<ChatState>,
    channel: Channel,
}

impl CommandExecutor for FocusExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            context
                .source
                .send_feedback(TextComponent::text("Apenas jogadores.".to_string()), false);
            return Ok(0);
        };
        self.state.set_channel(player.gameprofile.id, self.channel);
        context.source.send_feedback(
            TextComponent::text(format!("Canal alterado para {}.", self.channel.name())),
            false,
        );
        Ok(1)
    }
}

/// `/l <message>` / `/g <message>`.
struct SendExecutor {
    state: Arc<ChatState>,
    channel: Channel,
}

impl CommandExecutor for SendExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            context
                .source
                .send_feedback(TextComponent::text("Apenas jogadores.".to_string()), false);
            return Ok(0);
        };
        let message = StringArgumentType::get(context, "message")?;
        send_to_channel(
            &self.state,
            context.source.server(),
            &player,
            message,
            self.channel,
        );
        Ok(1)
    }
}

fn register_channel_commands(context: &mut ModuleContext<'_>, state: &Arc<ChatState>) {
    use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
    use pumpkin_util::permission::PermissionDefault;

    let (local_command, global_command) =
        state.with_config(|config| (config.local.command.clone(), config.global.command.clone()));

    let channels = [
        (
            local_command,
            Channel::Local,
            "carrot:chat.channel.local",
            "Allows using the local chat channel.",
            "Talks in the local channel.",
        ),
        (
            global_command,
            Channel::Global,
            "carrot:chat.channel.global",
            "Allows using the global chat channel.",
            "Talks in the global channel.",
        ),
    ];

    for (command_name, channel, permission, permission_description, description) in channels {
        if command_name.is_empty() {
            continue;
        }
        context.register_permission(permission, permission_description, PermissionDefault::Allow);
        let focus = FocusExecutor {
            state: Arc::clone(state),
            channel,
        };
        let send = SendExecutor {
            state: Arc::clone(state),
            channel,
        };
        let node = command(command_name.clone(), description)
            .executes(focus)
            .then(argument("message", StringArgumentType::GreedyPhrase).executes(send));
        context.register_command(&command_name, node, permission);
    }
}
