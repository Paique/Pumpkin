//! `carrot-multiverse`: parity with the reference build server's `MultiWorld`
//! setup — declared worlds with void/flat generators, peaceful difficulty,
//! no natural mob spawning, blocked portals, disabled building physics and
//! idle-world auto-unloading.
//!
//! Managed worlds are recreated on every enable (the vanilla boot loop only
//! reloads the primary world) and their chunks persist on disk, so a world
//! that already exists keeps whatever was built in it.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod commands;
pub mod config;
pub mod generator;
pub mod handlers;
pub mod state;

use std::sync::Arc;
use std::time::Duration;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::plugin::EventPriority;
use pumpkin_core::server::Server;
use state::MultiverseState;

/// The `multiverse` module.
pub struct Multiverse {
    state: Arc<MultiverseState>,
}

impl Multiverse {
    /// Creates the module with empty state; configuration loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(MultiverseState::new()),
        }
    }
}

impl Default for Multiverse {
    fn default() -> Self {
        Self::new()
    }
}

impl CarrotModule for Multiverse {
    fn id(&self) -> &'static str {
        "multiverse"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        self.state.reload(context.data_folder());

        let server = Arc::clone(context.server());
        self.state.load_managed_worlds(&server);
        self.state.apply_global_settings(&server);

        commands::register(context, &self.state);

        let state = Arc::clone(&self.state);
        context.register_event(
            Arc::new(handlers::PhysicsHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(handlers::EntityExplosionHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(handlers::BlockExplosionHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(handlers::PortalHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(handlers::WeatherHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(handlers::ThunderHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );
        context.register_event(
            Arc::new(handlers::LeavesDecayHandler(Arc::clone(&state))),
            EventPriority::Normal,
            true,
        );

        // Auto-unload sweeps empty managed worlds; the task dies with the
        // module and is aborted on disable.
        let task = tokio::spawn(auto_unload_task(server, state));
        self.state.set_unload_task(task.abort_handle());

        Ok(())
    }

    fn on_disable(&self, _context: &mut ModuleContext<'_>) {
        // Worlds stay loaded: chunks are persistent data and players may be
        // inside them. Only transient bookkeeping is dropped; global settings
        // (difficulty, game rules, spawn) remain until changed again.
        self.state.abort_unload_task();
        self.state.clear_transient();
    }
}

/// Unloads managed worlds that have been empty longer than the configured
/// timeout (reference: 1800 seconds).
async fn auto_unload_task(server: Arc<Server>, state: Arc<MultiverseState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        let timeout_secs = state.auto_unload_secs();
        if timeout_secs == 0 {
            continue;
        }
        let managed = state.world_defs();
        let loaded: Vec<_> = server.worlds.load().iter().cloned().collect();
        let mut to_unload = Vec::new();
        for world in &loaded {
            let name = world.get_world_name().to_string();
            if !managed.contains_key(&name) || !world.players.load().is_empty() {
                state.note_active(&name);
                continue;
            }
            let first_empty = state.note_empty(&name);
            if first_empty.is_some_and(|since| since.elapsed().as_secs() >= timeout_secs) {
                to_unload.push(name);
            }
        }
        for name in to_unload {
            match server.unload_world(&name).await {
                Ok(()) => tracing::info!("carrot-multiverse: auto-unloaded world '{name}'"),
                Err(err) => tracing::debug!("carrot-multiverse: auto-unload '{name}': {err}"),
            }
            state.clear_empty(&name);
        }
    }
}
