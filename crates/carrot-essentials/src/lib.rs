//! `carrot-essentials`: parity with the `EssentialsX` setup of the reference
//! build server (locale `pt_BR`, teleport delay 0).
//!
//! Commands: `/fly`, `/speed`, `/gm`, `/tpa` + `/tpaccept` + `/tpdeny`,
//! `/home` + `/sethome` + `/delhome`, `/warp` + `/setwarp` + `/delwarp`,
//! `/back` and `/spawn`. `/back` locations are recorded by the event handlers
//! below on every teleport, world change and death, like `EssentialsX`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod commands;
pub mod location;
pub mod state;
pub mod storage;

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::{
    entity::EntityBase,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        events::{
            entity::PlayerDeathEvent,
            player::{PlayerChangeWorldEvent, PlayerTeleportEvent},
        },
    },
    server::Server,
};
use state::EssentialsState;

/// The `essentials` module.
pub struct Essentials {
    state: Arc<EssentialsState>,
}

impl Essentials {
    /// Creates the module with empty state; storage loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(EssentialsState::new()),
        }
    }
}

impl Default for Essentials {
    fn default() -> Self {
        Self::new()
    }
}

impl CarrotModule for Essentials {
    fn id(&self) -> &'static str {
        "essentials"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        self.state.reload(context.data_folder());
        commands::register_all(context, &self.state);
        // Back tracking runs blocking so the recorded position is read before
        // the teleport moves the player.
        context.register_event(
            Arc::new(BackTeleport(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(BackWorldChange(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(BackDeath(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        Ok(())
    }

    fn on_disable(&self, _context: &mut ModuleContext<'_>) {
        // Homes and warps persist write-through; only transient state drops.
        self.state.clear_transient();
    }
}

/// Records the pre-teleport position on same-world teleports.
struct BackTeleport(Arc<EssentialsState>);

impl EventHandler<PlayerTeleportEvent> for BackTeleport {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a PlayerTeleportEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let entity = event.player.get_entity();
            self.0.set_back(
                event.player.gameprofile.id,
                location::Location {
                    world: event.player.world().get_world_name().to_string(),
                    x: event.from.x,
                    y: event.from.y,
                    z: event.from.z,
                    yaw: entity.yaw.load(),
                    pitch: entity.pitch.load(),
                },
            );
        })
    }
}

/// Records the departure position on cross-world teleports (and respawns into
/// another dimension, where the position is still the death spot).
struct BackWorldChange(Arc<EssentialsState>);

impl EventHandler<PlayerChangeWorldEvent> for BackWorldChange {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a PlayerChangeWorldEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let entity = event.player.get_entity();
            let pos = event.player.position();
            self.0.set_back(
                event.player.gameprofile.id,
                location::Location {
                    world: event.previous_world.get_world_name().to_string(),
                    x: pos.x,
                    y: pos.y,
                    z: pos.z,
                    yaw: entity.yaw.load(),
                    pitch: entity.pitch.load(),
                },
            );
        })
    }
}

/// Records the death position.
struct BackDeath(Arc<EssentialsState>);

impl EventHandler<PlayerDeathEvent> for BackDeath {
    fn handle<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a PlayerDeathEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            self.0.set_back(
                event.player.gameprofile.id,
                location::Location::of(&event.player),
            );
        })
    }
}
