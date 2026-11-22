//! Per-player handshake state and module-wide shared state (the Rust
//! equivalent of upstream `AxiomPlayerState`).

use std::{
    collections::HashMap,
    path::Path,
    sync::{Mutex, RwLock},
};

use uuid::Uuid;

use crate::{config::AxiomConfig, restrictions::Restrictions};

/// Handshake state for one player, created on join and dropped on quit.
#[derive(Debug, Default, Clone)]
pub struct PlayerState {
    /// The client completed the handshake and the editor is active.
    pub active: bool,
    /// Outstanding handshake id (0 = none pending).
    pub pending_handshake_id: i64,
    /// Whether the no-permission goodbye was already sent.
    pub sent_disable_reason: bool,
    /// Last restrictions payload sent (resent only when it changes).
    pub restrictions: Option<Restrictions>,
    /// Data version reported by the client handshake.
    pub client_data_version: Option<i32>,
}

/// Module state shared between handlers and the probe task.
pub struct AxiomState {
    config: RwLock<AxiomConfig>,
    players: RwLock<HashMap<Uuid, PlayerState>>,
    enable_payload: RwLock<Vec<u8>>,
    probe_task: Mutex<Option<tokio::task::AbortHandle>>,
}

impl AxiomState {
    /// Creates empty state; configuration loads on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: RwLock::new(AxiomConfig::default()),
            players: RwLock::new(HashMap::new()),
            enable_payload: RwLock::new(Vec::new()),
            probe_task: Mutex::new(None),
        }
    }

    /// Reloads the configuration and rebuilds the cached `axiom:enable`
    /// payload. The payload is rebuilt so later phases can extend the
    /// supported-serverbound channel list from configuration.
    pub fn reload(&self, data_folder: &Path) {
        let config = AxiomConfig::load_or_seed(&data_folder.join("axiom.toml"));
        let enable = crate::handshake::build_enable_payload(&config);
        *self
            .config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
        *self
            .enable_payload
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = enable;
    }

    /// Returns a snapshot of the configuration.
    #[must_use]
    pub fn config(&self) -> AxiomConfig {
        self.config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Returns a copy of the cached `axiom:enable` payload.
    #[must_use]
    pub fn enable_payload(&self) -> Vec<u8> {
        self.enable_payload
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Runs `update` against one player's state; the player is created on
    /// first touch. Returns the update's value.
    pub fn with_player<T>(
        &self,
        uuid: Uuid,
        update: impl FnOnce(&mut PlayerState) -> T,
    ) -> T {
        let mut players = self
            .players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        update(players.entry(uuid).or_default())
    }

    /// Removes a player's state (quit).
    pub fn forget_player(&self, uuid: Uuid) {
        self.players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&uuid);
    }

    /// Drops all player state (module disable).
    pub fn clear(&self) {
        self.players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        let handle = self
            .probe_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }

    /// Stores the probe task handle so disable can abort it.
    pub fn set_probe_task(&self, handle: tokio::task::AbortHandle) {
        *self
            .probe_task
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(handle);
    }
}

impl Default for AxiomState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_state_defaults_to_inactive() {
        let state = AxiomState::new();
        let pending = state.with_player(Uuid::new_v4(), |player| {
            let pending = player.pending_handshake_id;
            player.pending_handshake_id = 42;
            pending
        });
        assert_eq!(pending, 0);
        let pending = state.with_player(Uuid::new_v4(), |player| player.pending_handshake_id);
        assert_eq!(pending, 0);
    }
}
