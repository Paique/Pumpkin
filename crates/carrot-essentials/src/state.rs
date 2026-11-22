//! Shared runtime state of the essentials module.
//!
//! The module struct holds one [`EssentialsState`] behind an `Arc`; command
//! executors and event handlers clone it. Persistence is write-through: every
//! home/warp mutation saves the whole (small) file before reporting success.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, PoisonError, RwLock as StdRwLock},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{location::Location, storage};

/// A pending `/tpa` request, held by the teleport target.
#[derive(Debug, Clone)]
pub struct TpaRequest {
    /// UUID of the player who asked to teleport.
    pub sender: Uuid,
    /// Name of the sender at request time (for matching `/tpaccept <name>`).
    pub sender_name: String,
    /// When the request was created; used for lazy expiry.
    pub created: Instant,
}

/// Module configuration (`essentials.toml` in the data folder).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EssentialsConfig {
    /// World name `/spawn` teleports to (reference server: `byv`). When the
    /// world is not loaded, `/spawn` falls back to the player's current world.
    #[serde(default = "default_spawn_world")]
    pub spawn_world: String,
    /// Seconds a `/tpa` request stays valid (the reference `tpa-accept-cancellation`).
    #[serde(default = "default_tpa_timeout_secs")]
    pub tpa_timeout_secs: u64,
    /// Maximum accepted `/speed` value on the `EssentialsX` scale, where 8.0
    /// matches the reference server's `max-fly-speed`/`max-walk-speed` of 0.8.
    #[serde(default = "default_max_speed")]
    pub max_speed: f32,
}

fn default_spawn_world() -> String {
    "byv".to_string()
}

const fn default_tpa_timeout_secs() -> u64 {
    120
}

const fn default_max_speed() -> f32 {
    8.0
}

impl Default for EssentialsConfig {
    fn default() -> Self {
        Self {
            spawn_world: default_spawn_world(),
            tpa_timeout_secs: default_tpa_timeout_secs(),
            max_speed: default_max_speed(),
        }
    }
}

/// Everything the essentials commands and event handlers share.
pub struct EssentialsState {
    config: StdRwLock<EssentialsConfig>,
    homes: StdRwLock<storage::HomesFile>,
    warps: StdRwLock<storage::WarpsFile>,
    back: Mutex<HashMap<Uuid, Location>>,
    tpa: Mutex<HashMap<Uuid, Vec<TpaRequest>>>,
    data_folder: StdRwLock<PathBuf>,
}

impl Default for EssentialsState {
    fn default() -> Self {
        Self::new()
    }
}

impl EssentialsState {
    /// Creates an empty state; [`EssentialsState::reload`] fills it on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: StdRwLock::new(EssentialsConfig::default()),
            homes: StdRwLock::new(storage::HomesFile::default()),
            warps: StdRwLock::new(storage::WarpsFile::default()),
            back: Mutex::new(HashMap::new()),
            tpa: Mutex::new(HashMap::new()),
            data_folder: StdRwLock::new(PathBuf::new()),
        }
    }

    /// Reloads configuration and storage from the data folder and drops all
    /// transient state. Called on every enable.
    pub fn reload(&self, data_folder: &std::path::Path) {
        let config_path = data_folder.join("essentials.toml");
        let config: EssentialsConfig = storage::load_or_empty(&config_path);
        // Materialize the file so every field is visible for editing.
        if let Err(err) = storage::save(&config, &config_path) {
            tracing::warn!(
                "carrot-essentials: failed to rewrite {}: {err}",
                config_path.display()
            );
        }
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = config;

        *self
            .data_folder
            .write()
            .unwrap_or_else(PoisonError::into_inner) = data_folder.to_path_buf();
        *self.homes.write().unwrap_or_else(PoisonError::into_inner) =
            storage::load_or_empty(&data_folder.join("homes.toml"));
        *self.warps.write().unwrap_or_else(PoisonError::into_inner) =
            storage::load_or_empty(&data_folder.join("warps.toml"));
        self.clear_transient();
    }

    /// Drops in-memory-only state (pending requests and back locations).
    pub fn clear_transient(&self) {
        self.back
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.tpa
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// The configured spawn world name.
    #[must_use]
    pub fn spawn_world(&self) -> String {
        self.config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .spawn_world
            .clone()
    }

    /// The configured `/tpa` expiry.
    #[must_use]
    pub fn tpa_timeout(&self) -> Duration {
        Duration::from_secs(
            self.config
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .tpa_timeout_secs,
        )
    }

    /// The maximum accepted `/speed` value.
    #[must_use]
    pub fn max_speed(&self) -> f32 {
        self.config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .max_speed
    }

    /// Returns a home of the given player.
    #[must_use]
    pub fn home(&self, player: Uuid, name: &str) -> Option<Location> {
        self.homes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .homes
            .get(&player.to_string())?
            .get(name)
            .cloned()
    }

    /// Lists a player's home names.
    #[must_use]
    pub fn home_names(&self, player: Uuid) -> Vec<String> {
        self.homes
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .homes
            .get(&player.to_string())
            .map(|homes| homes.keys().cloned().collect())
            .unwrap_or_default()
    }

    /// Saves (or overwrites) a home, persisting immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the homes file cannot be written.
    pub fn set_home(&self, player: Uuid, name: &str, location: Location) -> std::io::Result<()> {
        let mut homes = self.homes.write().unwrap_or_else(PoisonError::into_inner);
        homes
            .homes
            .entry(player.to_string())
            .or_default()
            .insert(name.to_string(), location);
        let path = self
            .data_folder
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .join("homes.toml");
        storage::save(&*homes, &path)
    }

    /// Deletes a home, persisting immediately. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the homes file cannot be written.
    pub fn del_home(&self, player: Uuid, name: &str) -> std::io::Result<bool> {
        let mut homes = self.homes.write().unwrap_or_else(PoisonError::into_inner);
        let uuid = player.to_string();
        let existed = homes
            .homes
            .get_mut(&uuid)
            .is_some_and(|map| map.remove(name).is_some());
        if homes
            .homes
            .get(&uuid)
            .is_some_and(std::collections::BTreeMap::is_empty)
        {
            homes.homes.remove(&uuid);
        }
        let path = self
            .data_folder
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .join("homes.toml");
        storage::save(&*homes, &path)?;
        Ok(existed)
    }

    /// Returns a warp by name.
    #[must_use]
    pub fn warp(&self, name: &str) -> Option<Location> {
        self.warps
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .warps
            .get(name)
            .cloned()
    }

    /// Lists all warp names.
    #[must_use]
    pub fn warp_names(&self) -> Vec<String> {
        self.warps
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .warps
            .keys()
            .cloned()
            .collect()
    }

    /// Saves (or overwrites) a warp, persisting immediately.
    ///
    /// # Errors
    ///
    /// Returns an error if the warps file cannot be written.
    pub fn set_warp(&self, name: &str, location: Location) -> std::io::Result<()> {
        let mut warps = self.warps.write().unwrap_or_else(PoisonError::into_inner);
        warps.warps.insert(name.to_string(), location);
        let path = self
            .data_folder
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .join("warps.toml");
        storage::save(&*warps, &path)
    }

    /// Deletes a warp, persisting immediately. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Returns an error if the warps file cannot be written.
    pub fn del_warp(&self, name: &str) -> std::io::Result<bool> {
        let mut warps = self.warps.write().unwrap_or_else(PoisonError::into_inner);
        let existed = warps.warps.remove(name).is_some();
        let path = self
            .data_folder
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .join("warps.toml");
        storage::save(&*warps, &path)?;
        Ok(existed)
    }

    /// Records the player's location for a later `/back`.
    pub fn set_back(&self, player: Uuid, location: Location) {
        self.back
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(player, location);
    }

    /// Returns the player's last recorded `/back` location.
    #[must_use]
    pub fn back(&self, player: Uuid) -> Option<Location> {
        self.back
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&player)
            .cloned()
    }

    /// Records a `/tpa` request for the target, replacing an earlier request
    /// from the same sender.
    pub fn insert_tpa(&self, target: Uuid, request: TpaRequest) {
        let timeout = self.tpa_timeout();
        let mut tpa = self.tpa.lock().unwrap_or_else(PoisonError::into_inner);
        let requests = tpa.entry(target).or_default();
        requests.retain(|existing| {
            existing.sender != request.sender && existing.created.elapsed() < timeout
        });
        requests.push(request);
    }

    /// Takes a pending request for the target: the newest one, or the one from
    /// `sender_name` when given. Expired requests are discarded lazily.
    #[must_use]
    pub fn take_tpa(&self, target: Uuid, sender_name: Option<&str>) -> Option<TpaRequest> {
        let timeout = self.tpa_timeout();
        let mut tpa = self.tpa.lock().unwrap_or_else(PoisonError::into_inner);
        let requests = tpa.get_mut(&target)?;
        requests.retain(|request| request.created.elapsed() < timeout);
        let request = if let Some(name) = sender_name {
            let index = requests
                .iter()
                .rposition(|request| request.sender_name.eq_ignore_ascii_case(name))?;
            requests.remove(index)
        } else {
            requests.pop()?
        };
        if requests.is_empty() {
            tpa.remove(&target);
        }
        Some(request)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Location;

    fn state_with_dir(tag: &str) -> (EssentialsState, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "carrot-essentials-test-{}-{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        let state = EssentialsState::new();
        state.reload(&dir);
        (state, dir)
    }

    fn location(world: &str) -> Location {
        Location {
            world: world.to_string(),
            x: 1.0,
            y: 64.0,
            z: -2.5,
            yaw: 90.0,
            pitch: 0.0,
        }
    }

    #[test]
    fn homes_persist_across_reload() {
        let (state, dir) = state_with_dir("homes");
        let player = Uuid::nil();
        state
            .set_home(player, "flat", location("Dovahkiin__"))
            .unwrap_or_else(|err| panic!("set_home: {err}"));

        let reloaded = EssentialsState::new();
        reloaded.reload(&dir);
        assert_eq!(reloaded.home(player, "flat"), Some(location("Dovahkiin__")));
        assert!(reloaded.del_home(player, "flat").unwrap_or(false));
        assert_eq!(reloaded.home(player, "flat"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn take_tpa_returns_newest_then_named() {
        let (state, dir) = state_with_dir("tpa");
        let target = Uuid::nil();
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        state.insert_tpa(
            target,
            TpaRequest {
                sender: a,
                sender_name: "alpha".to_string(),
                created: Instant::now(),
            },
        );
        state.insert_tpa(
            target,
            TpaRequest {
                sender: b,
                sender_name: "beta".to_string(),
                created: Instant::now(),
            },
        );

        let newest = state
            .take_tpa(target, None)
            .unwrap_or_else(|| panic!("newest"));
        assert_eq!(newest.sender_name, "beta");

        // Re-insert so the remaining request is alpha's again.
        state.insert_tpa(
            target,
            TpaRequest {
                sender: a,
                sender_name: "alpha".to_string(),
                created: Instant::now(),
            },
        );
        let named = state
            .take_tpa(target, Some("ALPHA"))
            .unwrap_or_else(|| panic!("named"));
        assert_eq!(named.sender, a);
        assert!(state.take_tpa(target, None).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
