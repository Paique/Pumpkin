//! Homes and warps persisted as TOML files in the module data folder.
//!
//! Both files are small by design (a whitelist-sized build server), so every
//! mutation writes the whole file back while holding the state lock.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};
use tracing::error;

use crate::location::Location;

/// Homes of every player, keyed by player UUID string then home name.
///
/// UUIDs are stored as strings because TOML table keys are strings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomesFile {
    /// Homes keyed by player UUID string, then home name.
    #[serde(default)]
    pub homes: BTreeMap<String, BTreeMap<String, Location>>,
}

/// Warps keyed by name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WarpsFile {
    /// Warps keyed by name.
    #[serde(default)]
    pub warps: BTreeMap<String, Location>,
}

/// Reads a TOML file, falling back to an empty default when missing or
/// unreadable. Parse failures are logged and never abort the module.
#[must_use]
pub fn load_or_empty<T>(path: &Path) -> T
where
    T: Default + for<'de> Deserialize<'de>,
{
    let raw = match fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return T::default(),
        Err(err) => {
            error!(
                "carrot-essentials: failed to read {}: {err}; starting empty",
                path.display()
            );
            return T::default();
        }
    };
    match toml::from_str(&raw) {
        Ok(value) => value,
        Err(err) => {
            error!(
                "carrot-essentials: failed to parse {}: {err}; starting empty",
                path.display()
            );
            T::default()
        }
    }
}

/// Persists a TOML file, creating parent directories as needed.
///
/// # Errors
///
/// Returns an error if the file cannot be serialized or written.
pub fn save<T>(value: &T, path: &Path) -> io::Result<()>
where
    T: Serialize,
{
    let raw = toml::to_string_pretty(value)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location() -> Location {
        Location {
            world: "Dovahkiin__".to_string(),
            x: -401.3,
            y: 145.8,
            z: -307.3,
            yaw: 179.8,
            pitch: 23.1,
        }
    }

    #[test]
    fn homes_round_trip_through_toml() {
        let mut homes = HomesFile::default();
        homes
            .homes
            .entry("550e8400-e29b-41d4-a716-446655440000".to_string())
            .or_default()
            .insert("flat".to_string(), location());
        let raw = toml::to_string_pretty(&homes).unwrap_or_default();
        let parsed: HomesFile = toml::from_str(&raw).unwrap_or_else(|err| panic!("reparse: {err}"));
        assert_eq!(
            parsed.homes["550e8400-e29b-41d4-a716-446655440000"]["flat"],
            location()
        );
    }

    #[test]
    fn warps_round_trip_through_toml() {
        let mut warps = WarpsFile::default();
        warps.warps.insert("mundoj".to_string(), location());
        let raw = toml::to_string_pretty(&warps).unwrap_or_default();
        let parsed: WarpsFile = toml::from_str(&raw).unwrap_or_else(|err| panic!("reparse: {err}"));
        assert_eq!(parsed.warps["mundoj"], location());
    }
}
