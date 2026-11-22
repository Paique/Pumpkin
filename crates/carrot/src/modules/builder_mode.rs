//! `builder_mode`: creative gamemode by permission, on join and respawn.
//!
//! Parity with the reference build server's `criativo.sk` Skript: players
//! holding `carrot:buildermode` are switched to creative mode when they join
//! or respawn; everyone else is switched back to survival.

use std::sync::Arc;

use carrot_core::{CarrotError, CarrotModule, ModuleContext};
use pumpkin_core::{
    entity::player::Player,
    plugin::{
        BoxFuture, EventHandler, EventPriority,
        events::player::{PlayerJoinEvent, PlayerRespawnEvent},
    },
    server::Server,
};
use pumpkin_util::GameMode;
use pumpkin_util::permission::PermissionDefault;
use pumpkin_util::text::TextComponent;

/// Permission node that grants builder mode.
pub const PERMISSION: &str = "carrot:buildermode";

/// The `builder_mode` module.
pub struct BuilderMode;

impl CarrotModule for BuilderMode {
    fn id(&self) -> &'static str {
        "builder_mode"
    }

    fn on_enable(&self, context: &mut ModuleContext<'_>) -> Result<(), CarrotError> {
        context.register_permission(
            PERMISSION,
            "Switches the holder to creative mode on join and respawn.",
            PermissionDefault::Deny,
        );
        // Blocking so these run after the (also blocking, earlier-registered)
        // group handler has applied group permissions.
        context.register_event(Arc::new(JoinSwitch), EventPriority::Normal, true);
        context.register_event(Arc::new(RespawnSwitch), EventPriority::Normal, true);
        Ok(())
    }

    fn on_disable(&self, _context: &mut ModuleContext<'_>) {}
}

fn apply_mode(server: &Arc<Server>, player: &Arc<Player>) {
    let op_level = player.permission_lvl.load();
    let is_builder =
        server
            .permission_manager
            .has_permission(&player.gameprofile.id, PERMISSION, op_level);
    let mode = if is_builder {
        GameMode::Creative
    } else {
        GameMode::Survival
    };
    if player.set_gamemode(mode) {
        let mode_name = if is_builder { "creative" } else { "survival" };
        player.send_system_message(&TextComponent::text(format!(
            "carrot: builder mode applied ({mode_name})"
        )));
    }
}

struct JoinSwitch;

impl EventHandler<PlayerJoinEvent> for JoinSwitch {
    fn handle<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a PlayerJoinEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            apply_mode(server, &event.player);
        })
    }
}

struct RespawnSwitch;

impl EventHandler<PlayerRespawnEvent> for RespawnSwitch {
    fn handle<'a>(
        &'a self,
        server: &'a Arc<Server>,
        event: &'a PlayerRespawnEvent,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            apply_mode(server, &event.player);
        })
    }
}
