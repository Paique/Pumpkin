//! Serializable locations used by homes, warps and `/back`.

use std::sync::Arc;

use pumpkin_core::{
    entity::{EntityBase, player::Player},
    server::Server,
};
use pumpkin_util::math::vector3::Vector3;
use serde::{Deserialize, Serialize};

/// A saved position: world name plus coordinates and rotation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Location {
    /// World name, as reported by `World::get_world_name`.
    pub world: String,
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
}

impl Location {
    /// Captures the player's current position and rotation.
    #[must_use]
    pub fn of(player: &Arc<Player>) -> Self {
        let entity = player.get_entity();
        let pos = player.position();
        Self {
            world: player.world().get_world_name().to_string(),
            x: pos.x,
            y: pos.y,
            z: pos.z,
            yaw: entity.yaw.load(),
            pitch: entity.pitch.load(),
        }
    }

    /// Resolves the world name against the server's loaded worlds.
    #[must_use]
    pub fn resolve_world(&self, server: &Arc<Server>) -> Option<Arc<pumpkin_core::world::World>> {
        server
            .worlds
            .load()
            .iter()
            .find(|world| world.get_world_name() == self.world)
            .cloned()
    }

    /// Teleports the player to this location, crossing worlds when needed.
    ///
    /// Returns an error message when the saved world no longer exists.
    pub fn teleport(&self, player: &Arc<Player>, server: &Arc<Server>) -> Result<(), String> {
        let Some(world) = self.resolve_world(server) else {
            return Err(format!("mundo '{}' não existe mais", self.world));
        };
        player.teleport(
            Vector3::new(self.x, self.y, self.z),
            Some(self.yaw),
            Some(self.pitch),
            world,
        );
        Ok(())
    }
}
