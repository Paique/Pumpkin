//! `terrain.toml`, seeded with the reference server's brush limits
//! (FAWE brush radius 100, goBrush maximums from its config).

use std::path::Path;

use serde::{Deserialize, Serialize};

/// Module configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerrainConfig {
    /// Maximum radius a `//br` or `/b` brush may use.
    pub max_brush_radius: u32,
    /// Maximum goBrush size (diameter).
    pub gobrush_max_size: i32,
    /// Maximum goBrush intensity.
    pub gobrush_max_intensity: i32,
    /// Default goBrush brush file.
    pub gobrush_default_brush: String,
    /// Default goBrush size.
    pub gobrush_default_size: i32,
    /// Default goBrush intensity.
    pub gobrush_default_intensity: i32,
    /// Default goBrush direction (`true` paints up).
    pub gobrush_default_up: bool,
    /// Default goBrush 3D mode.
    pub gobrush_default_3d: bool,
    /// Default goBrush flat mode.
    pub gobrush_default_flat: bool,
    /// Default goBrush auto rotation.
    pub gobrush_default_auto_rotation: bool,
}

impl Default for TerrainConfig {
    fn default() -> Self {
        Self {
            max_brush_radius: 100,
            gobrush_max_size: 305,
            gobrush_max_intensity: 10,
            gobrush_default_brush: "cliff.png".to_string(),
            gobrush_default_size: 15,
            gobrush_default_intensity: 4,
            gobrush_default_up: true,
            gobrush_default_3d: false,
            gobrush_default_flat: false,
            gobrush_default_auto_rotation: true,
        }
    }
}

impl TerrainConfig {
    /// Loads the config from `terrain.toml`, writing the seeded file when
    /// missing (never fails; the module runs with defaults if unreadable).
    #[must_use]
    pub fn load(data_folder: &Path) -> Self {
        let path = data_folder.join("terrain.toml");
        std::fs::read_to_string(&path).map_or_else(
            |_| Self::seeded(&path),
            |text| {
                toml::from_str(&text).unwrap_or_else(|err| {
                    tracing::warn!("carrot terrain: config inválida ({err}); usando padrões");
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
