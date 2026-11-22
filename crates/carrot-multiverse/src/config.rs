//! Module configuration (`worlds.toml` in the data folder).
//!
//! The seed reproduces the reference build server's `MultiWorld` layout
//! (`plugins/MultiWorld/world.json`): twelve managed worlds, six of them
//! auto-loaded, all peaceful with building physics disabled.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Generator used when a managed world is created.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GeneratorKind {
    /// Vanilla noise terrain.
    Vanilla,
    /// Empty chunks (reference `VoidWorldGenerator`).
    Void,
    /// Fixed layers with a grass surface at `y = 63` (reference
    /// `FlatWorldGenerator`; the `flat` warp sits on `y = 64`).
    Flat,
}

/// A declared world spawn position.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    #[serde(default)]
    pub yaw: f32,
    #[serde(default)]
    pub pitch: f32,
}

/// Per-world building flags, mirroring the reference server where every
/// managed world disables physics, explosions, portals, weather changes and
/// leaf decay.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldFlags {
    /// Allow block physics (falling sand, fluid spread neighbors).
    pub block_physics: bool,
    /// Allow explosions to destroy blocks.
    pub explosions: bool,
    /// Allow portal travel from this world.
    pub portals: bool,
    /// Allow weather changes.
    pub weather_changes: bool,
    /// Allow natural leaf decay.
    pub leaves_decay: bool,
}

/// A managed world declaration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WorldDef {
    /// Generator applied when the world is first created.
    pub generator: GeneratorKind,
    /// Create the world on module enable (reference: `auto-load-enabled`).
    pub auto_load: bool,
    /// Declared spawn, used by `/goto` and `/mv info`.
    pub spawn: Option<SpawnPoint>,
    /// When true, this world's declared spawn also becomes the server-wide
    /// spawn (reference: `EssentialsXSpawn` pointing into `byv`). The
    /// reference position is applied to the shared level data, so `/spawn`
    /// and respawns land there.
    pub main_spawn: bool,
    pub flags: WorldFlags,
}

impl Default for WorldDef {
    fn default() -> Self {
        Self {
            generator: GeneratorKind::Vanilla,
            auto_load: true,
            spawn: None,
            main_spawn: false,
            flags: WorldFlags::default(),
        }
    }
}

/// Global settings applied on enable.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Difficulty applied to the shared level data (reference: every world
    /// `PEACEFUL`). The Pumpkin difficulty is server-wide by design.
    pub difficulty: String,
    /// Disables all natural mob spawning (`spawn_mobs`/`spawn_monsters` game
    /// rules; reference: `spawn-animals`/`spawn-monsters` false everywhere).
    pub disable_mob_spawning: bool,
    /// Seconds a managed world stays empty before being unloaded
    /// (reference: 1800). Zero disables auto-unloading.
    pub auto_unload_secs: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            difficulty: "peaceful".to_string(),
            disable_mob_spawning: true,
            auto_unload_secs: 1800,
        }
    }
}

/// `worlds.toml` contents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MultiverseConfig {
    pub settings: Settings,
    /// Managed worlds keyed by world name.
    pub worlds: BTreeMap<String, WorldDef>,
}

impl MultiverseConfig {
    /// Builds the default layout from the reference server: six auto-load
    /// worlds plus the six on-demand ones, all with the reference build
    /// flags. `byv` carries the real `EssentialsXSpawn` position.
    #[must_use]
    pub fn seed() -> Self {
        fn def(generator: GeneratorKind) -> WorldDef {
            WorldDef {
                generator,
                ..WorldDef::default()
            }
        }

        let mut worlds = BTreeMap::new();
        worlds.insert("spawn".to_string(), def(GeneratorKind::Vanilla));
        worlds.insert("MundoJ".to_string(), def(GeneratorKind::Void));
        worlds.insert("screw_water".to_string(), def(GeneratorKind::Void));
        worlds.insert("screw_poision".to_string(), def(GeneratorKind::Void));
        worlds.insert("Dovahkiin__".to_string(), def(GeneratorKind::Flat));

        // `byv` carries the real `EssentialsXSpawn` position and becomes the
        // server-wide spawn.
        worlds.insert(
            "byv".to_string(),
            WorldDef {
                generator: GeneratorKind::Flat,
                spawn: Some(SpawnPoint {
                    x: -5483.04,
                    y: 133.74,
                    z: 3114.63,
                    yaw: -54.30,
                    pitch: 36.00,
                }),
                main_spawn: true,
                ..WorldDef::default()
            },
        );

        // Worlds present on the reference server but not auto-loaded; several
        // imported warps point into them, so `/mv load` can bring them back.
        for name in [
            "screw-final",
            "screwspawn",
            "screw-spawn-and-rock",
            "RichBoatWorld",
            "dova2",
            "Dova",
        ] {
            worlds.insert(
                name.to_string(),
                WorldDef {
                    generator: GeneratorKind::Void,
                    auto_load: false,
                    ..WorldDef::default()
                },
            );
        }

        Self {
            settings: Settings::default(),
            worlds,
        }
    }

    /// Loads `worlds.toml`, seeding it when absent and materializing the
    /// full file for editing. Never fails: parse errors keep the seed.
    #[must_use]
    pub fn load_or_seed(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_else(|err| {
                tracing::warn!(
                    "carrot-multiverse: failed to parse {}: {err}; using seed",
                    path.display()
                );
                Self::seed()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let seeded = Self::seed();
                if let Err(save_err) =
                    std::fs::write(path, toml::to_string_pretty(&seeded).unwrap_or_default())
                {
                    tracing::warn!(
                        "carrot-multiverse: failed to write {}: {save_err}",
                        path.display()
                    );
                }
                seeded
            }
            Err(err) => {
                tracing::warn!(
                    "carrot-multiverse: failed to read {}: {err}; using seed",
                    path.display()
                );
                Self::seed()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_round_trips_through_toml() {
        let seed = MultiverseConfig::seed();
        let raw = toml::to_string_pretty(&seed).expect("serialize");
        let back: MultiverseConfig = toml::from_str(&raw).expect("deserialize");
        assert_eq!(back.settings.auto_unload_secs, 1800);
        assert!(!back.settings.difficulty.is_empty());
        let auto_loads: Vec<&String> = back
            .worlds
            .iter()
            .filter(|(_, def)| def.auto_load)
            .map(|(name, _)| name)
            .collect();
        assert_eq!(auto_loads.len(), 6, "six auto-load worlds");
        for expected in [
            "spawn",
            "MundoJ",
            "screw_water",
            "screw_poision",
            "Dovahkiin__",
            "byv",
        ] {
            assert!(back.worlds.contains_key(expected), "missing {expected}");
        }
        let byv = &back.worlds["byv"];
        assert!(byv.main_spawn);
        assert_eq!(byv.generator, GeneratorKind::Flat);
        assert!(byv.spawn.is_some());
        assert_eq!(back.worlds["screw_water"].generator, GeneratorKind::Void);
        assert_eq!(back.worlds["spawn"].generator, GeneratorKind::Vanilla);
        assert!(!back.worlds["screw-final"].auto_load);
    }

    #[test]
    fn defaults_match_reference_flags() {
        let def = WorldDef::default();
        assert!(!def.flags.block_physics);
        assert!(!def.flags.explosions);
        assert!(!def.flags.portals);
        assert!(!def.flags.weather_changes);
        assert!(!def.flags.leaves_decay);
    }
}
