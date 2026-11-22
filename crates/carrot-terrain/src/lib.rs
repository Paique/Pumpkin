//! `carrot-terrain`: the terrain module — WorldEdit/VoxelSniper brush
//! suite (`//br`, `/b`), Arceon cube/boulder brushes (`//ab`), goBrush
//! heightmaps (`/gb`), Arceon noise masks (`#arcangle`, `#vor`, ...) and
//! `//terragen`. Brush execution rides the worldedit edit engine, so
//! masks, `//confirm` and undo/redo history apply to every stroke.
//!
//! Interaction never blocks the tick thread: the handler only snapshots
//! per-player state and spawns the brush onto tokio.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]
// Mutex guards are scoped with `{ ... }` blocks so they never live across
// an await (keeps futures `Send`); the trailing-semicolon style lint
// disagrees with that pattern, so it is silenced crate-wide.
#![allow(clippy::semicolon_outside_block)]

pub mod arceon_brushes;
pub mod brush;
pub mod commands;
pub mod config;
pub mod fawe_brushes;
pub mod gobrush;
pub mod masks;
pub mod noise;
pub mod sniper;
pub mod state;
pub mod terragen;
pub mod util;

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use carrot_worldedit::session::WeState;
use pumpkin_core::plugin::{
    BoxFuture, Cancellable, EventHandler, EventPriority,
    events::player::{PlayerInteractEvent, player_interact_event::InteractAction},
};
use pumpkin_core::server::Server;
use pumpkin_util::text::TextComponent;
use state::TerrainState;

/// The `terrain` module.
pub struct Terrain {
    state: Arc<TerrainState>,
}

impl Terrain {
    /// Creates the module around the worldedit module state.
    #[must_use]
    pub fn new(we: Arc<WeState>) -> Self {
        Self {
            state: Arc::new(TerrainState::new(we)),
        }
    }
}

impl CarrotModule for Terrain {
    fn id(&self) -> &'static str {
        "terrain"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        self.state.configure(context.data_folder());
        masks::register();
        commands::register_all(context, &self.state);
        context.register_event(
            Arc::new(BrushHandler(Arc::clone(&self.state))),
            EventPriority::Normal,
            true,
        );
        Ok(())
    }

    fn on_disable(&self, _context: &mut ModuleContext<'_>) {
        self.state.clear_transient();
    }
}

/// Brush interaction: the `//br` bound item paints, arrow/gunpowder run
/// the `VoxelSniper` tool and flint runs goBrush. The handler cancels
/// every interaction it consumes, like `FAWE`/`FAVS`/goBrush do.
struct BrushHandler(Arc<TerrainState>);

impl EventHandler<PlayerInteractEvent> for BrushHandler {
    #[allow(clippy::too_many_lines)] // three tools, one handler
    fn handle_blocking<'a>(
        &'a self,
        _server: &'a Arc<Server>,
        event: &'a mut PlayerInteractEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            let held = event.player.inventory().held_item();
            if held.item_count == 0 {
                return;
            }
            let item = held
                .item
                .registry_key
                .trim_start_matches("minecraft:")
                .to_string();
            let right_click = matches!(
                event.action,
                InteractAction::RightClickBlock | InteractAction::RightClickAir
            );
            let left_click = matches!(
                event.action,
                InteractAction::LeftClickBlock | InteractAction::LeftClickAir
            );

            // 1. The //br tool bound to this item.
            let bound = {
                let player = self.0.player(event.player.gameprofile.id);
                let guard = player
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard
                    .we
                    .as_ref()
                    .map(|(item, brush)| (item.clone(), brush.clone()))
            };
            if let Some((bound_item, brush)) = bound
                && bound_item == item
            {
                event.set_cancelled(true);
                let Some(target) = brush_target(event, 256.0) else {
                    return;
                };
                let state = Arc::clone(&self.0);
                let player = Arc::clone(&event.player);
                tokio::spawn(async move {
                    brush.run(&state, &player, target).await;
                });
                return;
            }

            // 2. The VoxelSniper tool on arrow/gunpowder.
            if item == "arrow" || item == "gunpowder" {
                let has_brush = {
                    let player = self.0.player(event.player.gameprofile.id);
                    let guard = player
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    !matches!(guard.sniper.brush, sniper::SniperBrush::None)
                };
                if has_brush {
                    event.set_cancelled(true);
                    let Some(target) = brush_target(event, 256.0) else {
                        return;
                    };
                    let action = if item == "arrow" {
                        sniper::SnipeAction::Arrow
                    } else {
                        sniper::SnipeAction::Gunpowder
                    };
                    let state = Arc::clone(&self.0);
                    let player = Arc::clone(&event.player);
                    tokio::spawn(async move {
                        sniper::snipe(&state, &player, action, target).await;
                    });
                    return;
                }
            }

            // 3. goBrush on flint: right click paints, left click toggles.
            if item == "flint" {
                event.set_cancelled(true);
                let player_state = self.0.player(event.player.gameprofile.id);
                if left_click {
                    let (message, enabled) = {
                        let mut guard = player_state
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        guard.gobrush.enabled = !guard.gobrush.enabled;
                        (guard.gobrush.status(), guard.gobrush.enabled)
                    };
                    event
                        .player
                        .send_system_message(&TextComponent::text(format!(
                            "{} — {}",
                            if enabled { "ativado" } else { "desativado" },
                            message
                        )));
                    return;
                }
                if !right_click {
                    return;
                }
                let enabled = {
                    let guard = player_state
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.gobrush.enabled
                };
                if !enabled {
                    event.player.send_system_message(&TextComponent::text(
                        "goBrush desativado (clique esquerdo ou /gb toggle).".to_string(),
                    ));
                    return;
                }
                let Some(target) = brush_target(event, 200.0) else {
                    return;
                };
                let state = Arc::clone(&self.0);
                let player = Arc::clone(&event.player);
                tokio::spawn(async move {
                    gobrush::paint(&state, &player, target).await;
                });
            }
        })
    }
}

/// The block a brush stroke centers on: the clicked block, or the
/// crosshair ray target for clicks in the air.
fn brush_target(
    event: &PlayerInteractEvent,
    distance: f64,
) -> Option<pumpkin_util::math::position::BlockPos> {
    event.clicked_pos.or_else(|| {
        let world = event.player.world();
        event.player.get_target_block(&world, distance)
    })
}
