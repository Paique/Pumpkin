//! Shared terrain module state: configuration, the worldedit engine
//! handle, per-player brush state and the goBrush heightmap registry.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::session::WeState;
use uuid::Uuid;

use crate::config::TerrainConfig;
use crate::gobrush::GoBrushPlayer;
use crate::sniper::SniperState;

/// Per-player brush state (one struct so a single lock covers it all).
pub struct PlayerBrushes {
    /// The `//br` tool: the item it is bound to and the brush settings.
    pub we: Option<(String, crate::brush::Brush)>,
    /// The VoxelSniper-style `/b` tool (arrow + gunpowder).
    pub sniper: SniperState,
    /// The goBrush heightmap tool (flint).
    pub gobrush: GoBrushPlayer,
}

impl PlayerBrushes {
    /// Fresh state from the module configuration.
    #[must_use]
    pub fn new(config: &TerrainConfig) -> Self {
        Self {
            we: None,
            sniper: SniperState::new(),
            gobrush: GoBrushPlayer::new(config),
        }
    }
}

/// Module-wide state behind `Arc`.
pub struct TerrainState {
    config: RwLock<TerrainConfig>,
    /// The worldedit module state (edit engine, masks, history).
    pub we: Arc<WeState>,
    players: Mutex<HashMap<Uuid, Arc<Mutex<PlayerBrushes>>>>,
    /// Loaded goBrush heightmaps by file name.
    heightmaps: RwLock<HashMap<String, Arc<crate::gobrush::HeightMap>>>,
    /// Per-player loft selections (`//loft`).
    pub lofts: Mutex<crate::loft::LoftState>,
    /// Named texturing palettes (`//ezpalette`).
    pub palettes: crate::eztexture::PaletteStore,
    /// Where heightmap PNGs live (`carrot/gobrush/brushes`).
    heightmaps_dir: RwLock<PathBuf>,
}

impl TerrainState {
    /// Creates the state around the worldedit module state.
    #[must_use]
    pub fn new(we: Arc<WeState>) -> Self {
        Self {
            config: RwLock::new(TerrainConfig::default()),
            we,
            players: Mutex::new(HashMap::new()),
            heightmaps: RwLock::new(HashMap::new()),
            heightmaps_dir: RwLock::new(PathBuf::new()),
            lofts: Mutex::new(crate::loft::LoftState::default()),
            palettes: crate::eztexture::PaletteStore::default(),
        }
    }

    /// Loads the configuration and seeds the heightmap folder on enable.
    pub fn configure(&self, data_folder: &std::path::Path) {
        let config = TerrainConfig::load(data_folder);
        self.config
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&config);

        let carrot_root = data_folder.parent().and_then(|modules| modules.parent());
        let dir = carrot_root.map_or_else(
            || data_folder.join("brushes"),
            |carrot_root| carrot_root.join("gobrush").join("brushes"),
        );
        crate::gobrush::seed_default_brushes(&dir);
        self.palettes.configure(carrot_root.map_or_else(
            || data_folder.join("ezpalettes"),
            |carrot_root| carrot_root.join("ezpalettes"),
        ));
        *self
            .heightmaps_dir
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = dir;
        self.heightmaps
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    /// Drops in-memory player state on disable.
    pub fn clear_transient(&self) {
        self.players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.lofts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .selections
            .clear();
    }

    /// Runs `f` with a snapshot of the configuration.
    pub fn with_config<T>(&self, f: impl FnOnce(&TerrainConfig) -> T) -> T {
        let config = self
            .config
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        f(&config)
    }

    /// The per-player brush state, created on first use.
    pub fn player(&self, uuid: Uuid) -> Arc<Mutex<PlayerBrushes>> {
        let mut players = self
            .players
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        players
            .entry(uuid)
            .or_insert_with(|| {
                let config = self.with_config(std::clone::Clone::clone);
                Arc::new(Mutex::new(PlayerBrushes::new(&config)))
            })
            .clone()
    }

    /// The heightmap folder path.
    #[must_use]
    pub fn heightmaps_dir(&self) -> PathBuf {
        self.heightmaps_dir
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Loads (and caches) a heightmap by file name, accepting names with
    /// or without the `.png` extension.
    #[must_use]
    pub fn heightmap(&self, name: &str) -> Option<Arc<crate::gobrush::HeightMap>> {
        let file = if name.to_ascii_lowercase().ends_with(".png") {
            name.to_string()
        } else {
            format!("{name}.png")
        };
        if let Some(map) = self
            .heightmaps
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&file)
        {
            return Some(Arc::clone(map));
        }
        let loaded = crate::gobrush::HeightMap::load(&self.heightmaps_dir().join(&file))?;
        let map = Arc::new(loaded);
        self.heightmaps
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(file, Arc::clone(&map));
        Some(map)
    }

    /// The names of every heightmap available (loaded from disk).
    #[must_use]
    pub fn heightmap_names(&self) -> Vec<String> {
        crate::gobrush::list_brushes(&self.heightmaps_dir())
    }
}

/// Snapshots what a brush needs from the player's worldedit session so
/// the async brush run never holds a session lock.
#[derive(Clone)]
pub struct WeSnapshot {
    /// The session clipboard, for `#clipboard` patterns.
    pub clipboard: Option<Arc<carrot_worldedit::clipboard::Clipboard>>,
}

impl WeSnapshot {
    /// Captures the player's session clipboard.
    #[must_use]
    pub fn capture(we: &WeState, uuid: Uuid) -> Self {
        let session = we.session(uuid);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Self {
            clipboard: guard.clipboard.clone(),
        }
    }

    /// Applies a pattern at a position (None leaves the block alone).
    #[must_use]
    pub fn apply_pattern(
        &self,
        pattern: &Pattern,
        pos: &pumpkin_util::math::position::BlockPos,
    ) -> Option<pumpkin_data::BlockStateId> {
        pattern.apply(pos, self.clipboard.as_deref())
    }
}
