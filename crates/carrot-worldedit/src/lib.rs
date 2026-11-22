//! `carrot-worldedit`: parity subset of the reference build server's
//! FAWE — cuboid selection with the wooden axe wand, region edits with
//! patterns and masks, clipboard with rotate/flip/stack/move, Sponge
//! `.schem` interoperability and on-disk undo/redo history.
//!
//! Heavy edits run on tokio tasks with chunk loads awaited asynchronously
//! and block-update batches flushed periodically, so ticks are never
//! blocked.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
// Mutex guards are scoped with `{ ... }` blocks so they never live across
// an await (keeps command futures `Send`); the trailing-semicolon style
// lint disagrees with that pattern, so it is silenced crate-wide.
#![allow(clippy::semicolon_outside_block)]

pub mod clipboard;
pub mod commands;
pub mod config;
pub mod edit;
pub mod history;
pub mod mask;
pub mod pattern;
pub mod region;
pub mod schematic;
pub mod session;
pub mod states;

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::plugin::{
    BoxFuture, Cancellable, EventHandler, EventPriority,
    events::player::{PlayerInteractEvent, player_interact_event::InteractAction},
};
use pumpkin_core::server::Server;
use pumpkin_util::text::TextComponent;
use session::WeState;

/// The `worldedit` module.
pub struct WorldEdit {
    state: Arc<WeState>,
}

impl WorldEdit {
    /// Creates the module with empty state; configuration loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Arc::new(WeState::new()),
        }
    }

    /// The shared module state, for dependent modules (terrain brushes
    /// reuse the edit engine, masks and history).
    #[must_use]
    pub const fn state(&self) -> &Arc<WeState> {
        &self.state
    }
}

impl Default for WorldEdit {
    fn default() -> Self {
        Self::new()
    }
}

impl CarrotModule for WorldEdit {
    fn id(&self) -> &'static str {
        "worldedit"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        self.state.configure(context.data_folder());
        commands::register_all(context, &self.state);
        context.register_event(
            Arc::new(WandHandler(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        Ok(())
    }

    fn on_disable(&self, _context: &mut ModuleContext<'_>) {
        self.state.clear_transient();
    }
}

/// Wand selection: left click sets pos1, right click sets pos2. Every
/// interaction with the wand item is cancelled so the axe never edits
/// blocks by accident, exactly like FAWE.
struct WandHandler(Arc<WeState>);

impl EventHandler<PlayerInteractEvent> for WandHandler {
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let wand_item = self
                .0
                .with_config(|config| config.wand_item.clone())
                .trim_start_matches("minecraft:")
                .to_string();
            let held = event.player.inventory().held_item();
            if held.item_count == 0 || held.item.registry_key != wand_item {
                return;
            }

            // The wand always swallows the interaction.
            event.set_cancelled(true);

            let (corner, is_click) = match event.action {
                InteractAction::LeftClickBlock | InteractAction::LeftClickAir => (0, true),
                InteractAction::RightClickBlock | InteractAction::RightClickAir => (1, true),
            };

            let farwand = {
                let session = self.0.session(event.player.gameprofile.id);
                let guard = session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.farwand
            };

            let position = event.clicked_pos.or_else(|| {
                if !farwand {
                    return None;
                }
                let distance = self.0.with_config(|config| config.farwand_distance);
                let world = event.player.world();
                event.player.get_target_block(&world, distance)
            });
            let Some(position) = position else {
                if is_click {
                    event.player.send_system_message(&TextComponent::text(
                        "Nenhum bloco no alvo.".to_string(),
                    ));
                }
                return;
            };

            {
                let session = self.0.session(event.player.gameprofile.id);
                let mut guard = session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let entry = guard.selection.get_or_insert([position; 2]);
                entry[corner] = position;
            }
            let label = if corner == 0 {
                "Posição 1"
            } else {
                "Posição 2"
            };
            event
                .player
                .send_system_message(&TextComponent::text(format!(
                    "{label} definida em {} {} {}.",
                    position.0.x, position.0.y, position.0.z
                )));
        })
    }
}
