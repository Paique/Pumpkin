//! `carrot-axiom`: server-side implementation of the Axiom client-mod
//! protocol (<https://axiom.moulberry.com/>).
//!
//! Axiom runs its whole editing engine on the client; the server negotiates
//! the handshake over plugin messaging, applies what the client sends, and
//! answers a small set of player/world controls. This crate is phase AX-0:
//! a functional handshake (`axiom:hello` → `axiom:enable`, restrictions,
//! goodbye, 20-tick probe) that removes the client's "Axiom is not
//! installed on this server" warning and opens the editor toolbar. Editing
//! channels (tunnel, buffers, controls) land in later phases.
//!
//! Reference: local `AxiomPaperPlugin` fork (Axiom 6.x, API version 10).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod channels;
pub mod config;
pub mod handshake;
pub mod permissions;
pub mod restrictions;
pub mod state;
pub mod wire;

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        events::player::{
            player_custom_payload::PlayerCustomPayloadEvent, player_leave::PlayerLeaveEvent,
        },
    },
    server::Server,
};
use pumpkin_util::permission::PermissionLvl;
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

/// Dispatches Axiom plugin messages. AX-0 only speaks `axiom:hello`; any
/// other Axiom channel is dropped with a debug note until its phase lands.
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
                other if other.starts_with("axiom:") => {
                    tracing::debug!("carrot-axiom: ignoring channel {other} (not in AX-0)");
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
