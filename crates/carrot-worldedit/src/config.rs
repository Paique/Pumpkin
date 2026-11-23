//! `worldedit.toml`, seeded with the reference server's FAWE settings.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Module configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldEditConfig {
    /// Item used as the selection wand.
    pub wand_item: String,
    /// Wand range in blocks when farwand mode is active.
    pub farwand_distance: f64,
    /// Maximum blocks a single edit may touch.
    pub max_blocks: u64,
    /// Edits larger than this ask for `//confirm` first.
    pub confirm_above: u64,
    /// Undo entries kept per player (on disk).
    pub history_size: usize,
    /// Days an undo file survives.
    pub history_days: u32,
    /// Positions the edit queue may diff (or one page to apply) per
    /// slice; smaller values spread a huge edit thinner in memory.
    pub queue_slice_blocks: usize,
}

impl Default for WorldEditConfig {
    fn default() -> Self {
        Self {
            wand_item: "wooden_axe".to_string(),
            farwand_distance: 512.0,
            max_blocks: 50_000_000,
            confirm_above: 5_000_000,
            history_size: 15,
            history_days: 7,
            queue_slice_blocks: 524_288,
        }
    }
}

impl WorldEditConfig {
    /// Loads the config from `worldedit.toml`, writing the seeded file
    /// when missing or merging missing fields (never fails; the module
    /// runs with defaults if the file is unreadable).
    #[must_use]
    pub fn load(data_folder: &Path) -> Self {
        let path = data_folder.join("worldedit.toml");
        std::fs::read_to_string(&path).map_or_else(
            |_| Self::seeded(&path),
            |text| {
                toml::from_str(&text).unwrap_or_else(|err| {
                    tracing::warn!("carrot worldedit: config inválida ({err}); usando padrões");
                    Self::default()
                })
            },
        )
    }

    /// Writes the default config file and returns it.
    fn seeded(path: &Path) -> Self {
        let config = Self::default();
        if let Ok(text) = toml::to_string_pretty(&config) {
            let _ = std::fs::write(path, text);
        }
        config
    }
}
