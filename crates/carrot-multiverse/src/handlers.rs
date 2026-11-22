//! Event handlers enforcing the per-world build flags.
//!
//! Weather and leaf decay events carry the world, so those flags are exact.
//! Block physics, explosion and portal events do not, so those flags are
//! applied whenever any managed world requests them — on the reference
//! server every managed world disables all of them, which makes the
//! behavior identical.

use std::sync::Arc;

use pumpkin_core::plugin::{
    BoxFuture, EventHandler,
    events::{
        Cancellable, block::block_explode::BlockExplodeEvent,
        block::block_physics::BlockPhysicsEvent, block::leaves_decay::LeavesDecayEvent,
        entity::entity_explode::EntityExplodeEvent, entity::entity_portal::EntityPortalEvent,
        world::weather_change::ThunderChangeEvent, world::weather_change::WeatherChangeEvent,
    },
};
use pumpkin_core::server::Server;
use pumpkin_core::world::World;

use crate::config::WorldFlags;
use crate::state::MultiverseState;

/// True when any managed world allows the given flag.
fn any_world_allows(state: &MultiverseState, allow: impl Fn(&WorldFlags) -> bool) -> bool {
    state.with_config(|config| config.worlds.values().any(|def| allow(&def.flags)))
}

/// Cancels block physics unless any managed world allows them.
pub struct PhysicsHandler(pub Arc<MultiverseState>);

impl EventHandler<BlockPhysicsEvent> for PhysicsHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockPhysicsEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !any_world_allows(&self.0, |flags| flags.block_physics) {
                event.set_cancelled(true);
            }
        })
    }
}

/// Cancels entity explosions (TNT, creepers) unless allowed.
pub struct EntityExplosionHandler(pub Arc<MultiverseState>);

impl EventHandler<EntityExplodeEvent> for EntityExplosionHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityExplodeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !any_world_allows(&self.0, |flags| flags.explosions) {
                event.set_cancelled(true);
            }
        })
    }
}

/// Cancels block explosions (beds in the nether and friends) unless allowed.
pub struct BlockExplosionHandler(pub Arc<MultiverseState>);

impl EventHandler<BlockExplodeEvent> for BlockExplosionHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut BlockExplodeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !any_world_allows(&self.0, |flags| flags.explosions) {
                event.set_cancelled(true);
            }
        })
    }
}

/// Cancels portal travel unless allowed (reference: every world has
/// `nether-portal-accessible`/`end-portal-accessible` false).
pub struct PortalHandler(pub Arc<MultiverseState>);

impl EventHandler<EntityPortalEvent> for PortalHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut EntityPortalEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if !any_world_allows(&self.0, |flags| flags.portals) {
                event.set_cancelled(true);
            }
        })
    }
}

/// Whether the event's world is managed and disables the given flag.
fn managed_world_disallows(
    state: &MultiverseState,
    world: &World,
    allow: impl Fn(&WorldFlags) -> bool,
) -> bool {
    let name = world.get_world_name().to_string();
    state.with_config(|config| {
        config
            .worlds
            .get(&name)
            .is_some_and(|def| !allow(&def.flags))
    })
}

/// Cancels weather changes in managed worlds that disabled them.
pub struct WeatherHandler(pub Arc<MultiverseState>);

impl EventHandler<WeatherChangeEvent> for WeatherHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut WeatherChangeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if managed_world_disallows(&self.0, &event.world, |flags| flags.weather_changes) {
                event.set_cancelled(true);
            }
        })
    }
}

/// Cancels thunder changes in managed worlds that disabled weather.
pub struct ThunderHandler(pub Arc<MultiverseState>);

impl EventHandler<ThunderChangeEvent> for ThunderHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut ThunderChangeEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if managed_world_disallows(&self.0, &event.world, |flags| flags.weather_changes) {
                event.set_cancelled(true);
            }
        })
    }
}

/// Cancels natural leaf decay in managed worlds that disabled it.
pub struct LeavesDecayHandler(pub Arc<MultiverseState>);

impl EventHandler<LeavesDecayEvent> for LeavesDecayHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut LeavesDecayEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if managed_world_disallows(&self.0, &event.world, |flags| flags.leaves_decay) {
                event.set_cancelled(true);
            }
        })
    }
}
