//! Shared runtime state of the multiverse module.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, PoisonError, RwLock as StdRwLock},
    time::Instant,
};

use std::sync::Arc;

use pumpkin_core::server::Server;
use pumpkin_core::world::World;
use pumpkin_data::dimension::Dimension;
use pumpkin_data::game_rules::{GameRule, GameRuleValue};
use pumpkin_util::difficulty::Difficulty;

use crate::config::{GeneratorKind, MultiverseConfig, WorldDef};

/// Everything the multiverse module shares between executors, handlers and
/// the auto-unload task.
pub struct MultiverseState {
    config: StdRwLock<MultiverseConfig>,
    data_folder: StdRwLock<PathBuf>,
    /// First instant each loaded managed world was seen empty.
    empty_since: Mutex<HashMap<String, Instant>>,
    /// Abort handle of the auto-unload task, if running.
    unload_task: Mutex<Option<tokio::task::AbortHandle>>,
}

impl Default for MultiverseState {
    fn default() -> Self {
        Self::new()
    }
}

impl MultiverseState {
    /// Creates an empty state; [`MultiverseState::reload`] fills it on enable.
    #[must_use]
    pub fn new() -> Self {
        Self {
            config: StdRwLock::new(MultiverseConfig::default()),
            data_folder: StdRwLock::new(PathBuf::new()),
            empty_since: Mutex::new(HashMap::new()),
            unload_task: Mutex::new(None),
        }
    }

    /// Reloads `worlds.toml` from the data folder.
    pub fn reload(&self, data_folder: &std::path::Path) {
        let config = MultiverseConfig::load_or_seed(&data_folder.join("worlds.toml"));
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = config;
        *self
            .data_folder
            .write()
            .unwrap_or_else(PoisonError::into_inner) = data_folder.to_path_buf();
        self.clear_transient();
    }

    /// Drops in-memory-only state.
    pub fn clear_transient(&self) {
        self.empty_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }

    /// Runs a closure with the current config snapshot.
    pub fn with_config<T>(&self, f: impl FnOnce(&MultiverseConfig) -> T) -> T {
        let guard = self.config.read().unwrap_or_else(PoisonError::into_inner);
        f(&guard)
    }

    /// The configured auto-unload timeout in seconds (zero disables).
    #[must_use]
    pub fn auto_unload_secs(&self) -> u64 {
        self.with_config(|config| config.settings.auto_unload_secs)
    }

    /// Definitions of every managed world, cloned.
    #[must_use]
    pub fn world_defs(&self) -> HashMap<String, WorldDef> {
        self.with_config(|config| config.worlds.clone().into_iter().collect())
    }

    /// Creates every auto-load world that is not loaded yet and returns the
    /// worlds as they ended up in the server list.
    pub fn load_managed_worlds(&self, server: &Arc<Server>) {
        self.with_config(|config| {
            for (name, def) in &config.worlds {
                if !def.auto_load {
                    continue;
                }
                self.ensure_world_loaded(server, name, def);
            }
        });
    }

    /// Returns the loaded world by name, creating it with the configured
    /// generator when missing. Existing worlds keep their generator so
    /// persisted chunks stay authoritative.
    pub fn ensure_world_loaded(
        &self,
        server: &Arc<Server>,
        name: &str,
        def: &WorldDef,
    ) -> Arc<World> {
        if let Some(world) = loaded_world(server, name) {
            return world;
        }
        let world = server.create_world(name.to_string(), Dimension::OVERWORLD);
        let seed = server.level_info.load().world_gen_settings.seed;
        if let Some(generator) =
            crate::generator::world_generator(def.generator, Dimension::OVERWORLD, seed as u64)
        {
            world.level.set_world_gen(Arc::new(generator));
            tracing::info!(
                "carrot-multiverse: world '{name}' created ({})",
                generator_name(def.generator)
            );
        } else {
            tracing::info!("carrot-multiverse: world '{name}' created (vanilla generator)");
        }
        world
    }

    /// Applies the global settings to the shared level data: difficulty, mob
    /// spawning game rules and the main spawn position.
    pub fn apply_global_settings(&self, server: &Arc<Server>) {
        let Some(world) = server.worlds.load().first().cloned() else {
            return;
        };

        self.with_config(|config| {
            if let Ok(difficulty) = config.settings.difficulty.parse::<Difficulty>() {
                world.set_difficulty(difficulty);
            }
            if config.settings.disable_mob_spawning {
                world.set_game_rule(&GameRule::SpawnMobs, GameRuleValue::Bool(false));
                world.set_game_rule(&GameRule::SpawnMonsters, GameRuleValue::Bool(false));
            }

            let main_spawn = config
                .worlds
                .iter()
                .find(|(_, def)| def.main_spawn)
                .and_then(|(name, def)| def.spawn.map(|spawn| (name.as_str(), spawn)))
                .or_else(|| {
                    config
                        .worlds
                        .iter()
                        .find_map(|(name, def)| def.spawn.map(|spawn| (name.as_str(), spawn)))
                });
            if let Some((name, spawn)) = main_spawn {
                let current = world.level_info.load();
                let mut new_info = (**current).clone();
                new_info.spawn_x = floor_i32(spawn.x);
                new_info.spawn_y = floor_i32(spawn.y);
                new_info.spawn_z = floor_i32(spawn.z);
                new_info.spawn_yaw = spawn.yaw;
                new_info.spawn_pitch = spawn.pitch;
                world.level_info.store(Arc::new(new_info));
                tracing::info!(
                    "carrot-multiverse: global spawn set from world '{name}' at ({}, {}, {})",
                    spawn.x,
                    spawn.y,
                    spawn.z
                );
            }
        });
    }

    /// Records that a managed world is empty, returning when it was first
    /// seen empty (`None` when this is the first sighting).
    pub fn note_empty(&self, name: &str) -> Option<Instant> {
        let mut map = self
            .empty_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(since) = map.get(name) {
            return Some(*since);
        }
        map.insert(name.to_string(), Instant::now());
        None
    }

    /// Records that a managed world has players again.
    pub fn note_active(&self, name: &str) {
        self.empty_since
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(name);
    }

    /// Forgets the empty-tracking of a world (after unloading it).
    pub fn clear_empty(&self, name: &str) {
        self.note_active(name);
    }

    /// Stores the auto-unload task handle, aborting any previous one.
    pub fn set_unload_task(&self, handle: tokio::task::AbortHandle) {
        let mut guard = self
            .unload_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(old) = guard.take() {
            old.abort();
        }
        *guard = Some(handle);
    }

    /// Aborts the auto-unload task, if any.
    pub fn abort_unload_task(&self) {
        let handle = self
            .unload_task
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(handle) = handle {
            handle.abort();
        }
    }
}

/// Finds a loaded world by name.
#[must_use]
pub fn loaded_world(server: &Arc<Server>, name: &str) -> Option<Arc<World>> {
    server
        .worlds
        .load()
        .iter()
        .find(|world| world.get_world_name() == name)
        .cloned()
}

const fn generator_name(kind: GeneratorKind) -> &'static str {
    match kind {
        GeneratorKind::Vanilla => "vanilla",
        GeneratorKind::Void => "void",
        GeneratorKind::Flat => "flat",
    }
}

const fn floor_i32(value: f64) -> i32 {
    value.floor() as i32
}
