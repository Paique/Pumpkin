//! `carrot/carrot.toml` handling: per-module enabled state.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};
use tracing::error;

/// Root configuration of the carrot module system.
///
/// Stored at `<exec_dir>/carrot/carrot.toml`. Modules missing from the map
/// fall back to each module's own default enabled state, which the manager
/// materializes into the file on startup so every module is visible there.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CarrotConfig {
    /// Per-module enabled state keyed by module id.
    #[serde(default)]
    pub modules: BTreeMap<String, bool>,
}

impl CarrotConfig {
    /// Reads the configuration file, falling back to defaults when the file is
    /// missing or unreadable. Parse failures are logged and never abort the
    /// server, matching how `pumpkin.toml` behaves.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Self::default(),
            Err(err) => {
                error!(
                    "carrot: failed to read {}: {err}; using defaults",
                    path.display()
                );
                return Self::default();
            }
        };
        match toml::from_str(&raw) {
            Ok(config) => config,
            Err(err) => {
                error!(
                    "carrot: failed to parse {}: {err}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Persists the configuration, creating parent directories as needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be serialized or written.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        let raw = toml::to_string_pretty(self)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, raw)
    }
}
