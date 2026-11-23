//! `carrot-axiom`: server-side implementation of the Axiom client-mod
//! protocol (<https://axiom.moulberry.com/>).
//!
//! Axiom runs its whole editing engine on the client; the server negotiates
//! the handshake over plugin messaging, applies what the client sends, and
//! answers a small set of player/world controls.
//!
//! Phase AX-0 delivered the
//! handshake (`axiom:hello` → `axiom:enable`, restrictions, goodbye,
//! 20-tick probe). Phase AX-1 adds block editing: the `axiom:tunnel`
//! multiplexer (fragment reassembly, `ZSTD`, `DoS` guards), `axiom:set_block`
//! batches, `axiom:set_buffer` section buffers (runtime palettes with
//! `ZSTD`-dictionary block entities) and `axiom:tick_blocks`, applied with
//! chunk leases, backpressure and per-player dispatch budgets.
//!
//! Reference: local `AxiomPaperPlugin` fork (Axiom 6.x, API version 10).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod apply;
pub mod blocks;
pub mod buffer;
pub mod channels;
pub mod config;
pub mod handshake;
pub mod packets;
pub mod permissions;
pub mod restrictions;
pub mod state;
pub mod tunnel;
pub mod wire;

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::{
    entity::player::Player,
    net::DisconnectReason,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        events::player::{
            player_custom_payload::PlayerCustomPayloadEvent, player_leave::PlayerLeaveEvent,
        },
    },
    server::Server,
};
use pumpkin_util::permission::PermissionLvl;
use pumpkin_util::text::TextComponent;
use state::AxiomState;
use uuid::Uuid;

/// The `axiom` module.
pub struct Axiom {
    state: Arc<AxiomState>,
}

impl Axiom {
    /// Creates the module with empty state; configuration loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(AxiomState::new()),
        }
    }
}

impl Default for Axiom {
    fn default() -> Self {
        Self::new()
    }
}

impl CarrotModule for Axiom {
    fn id(&self) -> &'static str {
        "axiom"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        self.state.reload(context.data_folder());
        permissions::register(context);

        context.register_event(
            Arc::new(PayloadHandler(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(LeaveHandler(Arc::clone(&self.state))),
            EventPriority::Normal,
            false,
        );

        // The probe starts handshakes for players already online when the
        // module is (re)enabled at runtime.
        let server = Arc::clone(context.server());
        let task = tokio::spawn(handshake::probe_task(server, Arc::clone(&self.state)));
        self.state.set_probe_task(task.abort_handle());
        Ok(())
    }

    fn on_disable(&self, context: &mut ModuleContext<'_>) {
        // Say goodbye to active clients: a minimal `false` enable payload
        // turns the editor off on their side. Sends are async, so they are
        // spawned; the module state is cleared synchronously afterwards.
        let server = Arc::clone(context.server());
        for player in server.get_all_players() {
            if self
                .state
                .with_player(player.gameprofile.id, |entry| entry.active)
            {
                server.spawn_task(async move {
                    if let Some(java) = player.as_java() {
                        java.send_custom_payload(channels::ENABLE, &[0]).await;
                    }
                });
            }
        }
        self.state.clear();
    }
}

/// Whether the player holds operator level (upstream: ops implicitly hold
/// every Axiom permission).
pub(crate) fn is_op(player: &Player) -> bool {
    matches!(
        player.permission_lvl.load(),
        PermissionLvl::One | PermissionLvl::Two | PermissionLvl::Three | PermissionLvl::Four
    )
}

/// The handshake gate: op or `axiom.use` (resolved through children and
/// wildcards, so `axiom.default` group grants qualify).
pub(crate) fn has_use(server: &Arc<Server>, player: &Arc<Player>) -> bool {
    is_op(player)
        || server.permission_manager.has_permission(
            &player.gameprofile.id,
            permissions::USE_NODE,
            player.permission_lvl.load(),
        )
}

/// Disconnects a player with a message (protocol-level kick).
pub(crate) fn kick(player: &Arc<Player>, message: &str) {
    player.kick(
        DisconnectReason::Unknown,
        &TextComponent::text(message.to_string()),
    );
}

/// Dispatches one decoded serverbound payload to its channel handler.
///
/// Errors kick the player (the upstream dedicated-channel wrapper kicks on
/// any handler failure; the tunnel path kicks on protocol violations).
async fn dispatch_channel(
    state: &Arc<AxiomState>,
    server: &Arc<Server>,
    player: &Arc<Player>,
    channel: &str,
    payload: &[u8],
) {
    match channel {
        channels::HELLO => {
            handshake::handle_hello(state, server, player, payload).await;
        }
        channels::SET_BLOCK => {
            if let Err(reason) = packets::handle_set_block(state, server, player, payload).await {
                kick(player, &reason);
            }
        }
        channels::SET_BUFFER => {
            if let Err(reason) = packets::handle_set_buffer(state, server, player, payload).await {
                kick(player, &reason);
            }
        }
        channels::TICK_BLOCKS => {
            if let Err(reason) = packets::handle_tick_blocks(state, server, player, payload).await {
                kick(player, &reason);
            }
        }
        // Not a channel this phase registered serverbound (upstream:
        // "Sent packet that was unregistered").
        _ => kick(player, "Axiom: Sent packet that was unregistered"),
    }
    // Handlers verify full payload consumption themselves (upstream's
    // "Failed to fully read" invariant) and report it via their errors.
}

/// Handles one `axiom:tunnel` payload: reassembly, decoding and dispatch
/// (upstream `handleTunnelBuffer`/`handleTunnelPacket`).
async fn handle_tunnel(state: &Arc<AxiomState>, server: &Arc<Server>, player: &Arc<Player>, data: &[u8]) {
    let uuid = player.gameprofile.id;
    if !state.with_player(uuid, |player| player.active) {
        return;
    }
    let config = state.config();
    let action = state.with_player(uuid, |entry| {
        tunnel::process_fragment(&config, &mut entry.tunnel, data)
    });
    let tunnel::TunnelAction::Dispatch(bytes) = action else {
        if let tunnel::TunnelAction::Violation(reason) = action {
            kick(player, reason);
        }
        return;
    };

    match tunnel::decode_packet(&config, &bytes) {
        Ok((channel, payload)) => {
            // Nested tunnels are rejected even though the channel is
            // advertised (upstream registers a null handler for it).
            if channel == channels::TUNNEL {
                kick(player, "Axiom: Sent packet that was unregistered");
                return;
            }
            if !tunnel::dispatchable(&channel) {
                kick(player, "Axiom: Sent packet that was unregistered");
                return;
            }
            dispatch_channel(state, server, player, &channel, &payload).await;
        }
        Err(reason) => kick(player, &reason),
    }
}

/// Dispatches Axiom plugin messages: the handshake and, since AX-1, the
/// tunnel multiplexer and the dedicated editing channels.
struct PayloadHandler(Arc<AxiomState>);

impl EventHandler<PlayerCustomPayloadEvent> for PayloadHandler {
    fn handle_blocking<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a mut PlayerCustomPayloadEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            match event.channel.as_str() {
                channels::HELLO => {
                    handshake::handle_hello(&self.0, server, &event.player, &event.data).await;
                }
                channels::TUNNEL => {
                    handle_tunnel(&self.0, server, &event.player, &event.data).await;
                }
                channels::SET_BLOCK | channels::SET_BUFFER | channels::TICK_BLOCKS => {
                    let channel = event.channel.clone();
                    if self.0.with_player(event.player.gameprofile.id, |entry| entry.active) {
                        dispatch_channel(
                            &self.0,
                            server,
                            &event.player,
                            &channel,
                            &event.data,
                        )
                        .await;
                    }
                }
                other if other.starts_with("axiom:") => {
                    tracing::debug!("carrot-axiom: ignoring channel {other} (later phase)");
                }
                _ => {}
            }
        })
    }
}

/// Drops leaving players' handshake state ("player disconnected" also
/// cancels any in-flight negotiation by construction).
struct LeaveHandler(Arc<AxiomState>);

impl EventHandler<PlayerLeaveEvent> for LeaveHandler {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a PlayerLeaveEvent,
    ) -> BoxFuture<'a, ()> {
        let uuid: Uuid = event.player.gameprofile.id;
        Box::pin(async move {
            self.0.forget_player(uuid);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_table_is_consistent() {
        for perm in permissions::PERMISSIONS {
            if let Some(parent) = perm.parent {
                let entry = permissions::find(parent)
                    .unwrap_or_else(|| panic!("unknown parent {parent}"));
                // Parents must exist and never form a cycle through roots.
                assert!(entry.parent.is_none_or(|grand| grand != perm.node));
            }
        }
        // Wire names are unique (only axiom.default maps to axiom.*).
        let mut names: Vec<&str> = permissions::PERMISSIONS
            .iter()
            .map(|perm| permissions::wire_name(perm.node))
            .collect();
        names.sort_unstable();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count);
    }
}
