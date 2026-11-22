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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WarpsFile {
    /// Warps keyed by name.
    #[serde(default)]
    pub warps: BTreeMap<String, Location>,
}

const REFERENCE_WARPS: &[(&str, &str, f64, f64, f64, f32, f32)] = &[
    (
        "blazebedwars",
        "Dovahkiin__",
        -401.30,
        145.83,
        -307.29,
        179.78,
        23.10,
    ),
    (
        "blazecorrida",
        "Dovahkiin__",
        -147.29,
        145.68,
        -4.80,
        -156.68,
        29.40,
    ),
    (
        "blazegladiador",
        "Dovahkiin__",
        -280.65,
        181.87,
        -184.24,
        -141.08,
        36.15,
    ),
    (
        "blazeroyale",
        "Dovahkiin__",
        -652.14,
        181.87,
        -255.07,
        142.42,
        25.80,
    ),
    (
        "blazeskywars",
        "Dovahkiin__",
        -381.89,
        133.24,
        14.23,
        -159.07,
        26.55,
    ),
    (
        "blazetnt",
        "Dovahkiin__",
        -524.01,
        155.13,
        -145.32,
        133.87,
        37.95,
    ),
    ("flat", "Dovahkiin__", 48.0, 64.0, 32.0, 0.0, 0.0),
    (
        "ilhadeouro",
        "MundoJ",
        -2059.09,
        85.51,
        -3443.45,
        79.50,
        40.35,
    ),
    ("j", "MundoJ", -1184.44, 77.82, -606.19, 159.87, 20.50),
    ("mundoj", "MundoJ", -1175.95, 64.0, -670.32, -62.32, 49.20),
    (
        "kakirm",
        "screw-spawn-and-rock",
        17.55,
        94.01,
        -14.59,
        -60.30,
        33.90,
    ),
    (
        "poisoncave",
        "screw_poision",
        -38.26,
        97.54,
        109.29,
        -76.08,
        16.20,
    ),
    ("predios", "Dova", -156.06, -20.0, -66.51, 178.35, 21.15),
    ("rocha", "screw-final", 49.79, 110.80, 1290.49, 28.49, 34.84),
    (
        "rockisle",
        "screw-final",
        155.78,
        158.99,
        1412.94,
        45.13,
        29.40,
    ),
    ("screw", "screw-final", 18.82, 144.52, -16.44, 179.73, 40.41),
    (
        "spawna",
        "screw-spawn-and-rock",
        22.81,
        138.11,
        150.03,
        -0.45,
        11.10,
    ),
    (
        "waterisland",
        "screw_water",
        1134.99,
        122.26,
        55.96,
        -23.10,
        29.70,
    ),
];

impl WarpsFile {
    /// The warps imported from the reference server's `EssentialsX` data
    /// (`plugins/Essentials/warps/*.yml` in the archive). Seeded once when
    /// no `warps.toml` exists yet; warps pointing at worlds that are not
    /// loaded report a clear error until those worlds come up.
    #[must_use]
    pub fn seed() -> Self {
        Self {
            warps: REFERENCE_WARPS
                .iter()
                .map(|(name, world, x, y, z, yaw, pitch)| {
                    (
                        (*name).to_string(),
                        Location {
                            world: (*world).to_string(),
                            x: *x,
                            y: *y,
                            z: *z,
                            yaw: *yaw,
                            pitch: *pitch,
                        },
                    )
                })
                .collect(),
        }
    }
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

    #[test]
    fn warps_seed_has_the_reference_layout() {
        let seeded = WarpsFile::seed();
        assert_eq!(seeded.warps.len(), 18);
        // One warp per reference file, including the six that point at
        // worlds which are not auto-loaded.
        for name in [
            "blazebedwars",
            "flat",
            "ilhadeouro",
            "kakirm",
            "poisoncave",
            "predios",
            "rocha",
            "spawna",
            "waterisland",
        ] {
            assert!(seeded.warps.contains_key(name), "missing warp {name}");
        }
        assert_eq!(seeded.warps["flat"].y, 64.0);
        assert_eq!(seeded.warps["waterisland"].world, "screw_water");
        // The whole seed survives a TOML round trip.
        let raw = toml::to_string_pretty(&seeded).unwrap_or_default();
        let parsed: WarpsFile = toml::from_str(&raw).unwrap_or_else(|err| panic!("reparse: {err}"));
        assert_eq!(parsed, seeded);
    }
}
