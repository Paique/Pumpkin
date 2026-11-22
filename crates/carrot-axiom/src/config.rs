//! Module configuration (`axiom.toml` in the data folder).
//!
//! Defaults mirror the upstream `AxiomConfig.java`/`config.yml` values
//! (Axiom 6.x): tunnel fragments stay under the vanilla 32 KiB custom-payload
//! cap, reassembled packets cap at 2 MiB, and backpressure/watchdog guards
//! match the reference server. Only the handshake subset is consumed in
//! AX-0; the remaining fields configure later phases and are wired when
//! those phases land.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// How to react when the client's Axiom API or data version does not match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VersionResolution {
    /// Send a goodbye/chat warning but let the player stay.
    Warn,
    /// Disconnect the player.
    Kick,
    /// Log only and continue the handshake.
    Ignore,
}

/// `axiom.toml` root.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AxiomConfig {
    /// Maximum size of a single tunnel fragment (must stay under the 32 767
    /// custom-payload limit).
    pub tunnel_split_size: i32,
    /// Maximum size of a reassembled tunnel packet.
    pub maximum_tunnel_packet_size: i32,
    /// Maximum decoded block-buffer packet size.
    pub max_block_buffer_packet_size: i32,
    /// Infinite reach limit announced in restrictions.
    pub infinite_reach_limit: i32,
    /// Block-buffer sends per second per player (token bucket).
    pub block_buffer_rate_limit: u32,
    /// Backpressure: pending sections before edits stall.
    pub max_pending_sections_per_player: u32,
    /// Backpressure: pending sections before a protocol kick.
    pub max_pending_sections_per_player_hard: u32,
    /// Watchdog that force-completes stuck edit units.
    pub unit_watchdog_seconds: u32,
    /// Whether annotation packets are accepted.
    pub allow_annotations: bool,
    /// Whether blueprint upload/request is enabled.
    pub blueprint_sharing: bool,
    /// Reaction to a data/protocol version mismatch.
    pub incompatible_data_version_resolution: VersionResolution,
    /// Reaction to an unsupported Axiom API version.
    pub unsupported_axiom_version_resolution: VersionResolution,
}

impl Default for AxiomConfig {
    fn default() -> Self {
        Self {
            tunnel_split_size: 31_000,
            maximum_tunnel_packet_size: 2_097_152,
            max_block_buffer_packet_size: 2_097_152,
            infinite_reach_limit: 256,
            block_buffer_rate_limit: 1024,
            max_pending_sections_per_player: 8192,
            max_pending_sections_per_player_hard: 32_768,
            unit_watchdog_seconds: 30,
            allow_annotations: false,
            blueprint_sharing: false,
            incompatible_data_version_resolution: VersionResolution::Warn,
            unsupported_axiom_version_resolution: VersionResolution::Warn,
        }
    }
}

impl AxiomConfig {
    /// Reads the module configuration, seeding the file with defaults when
    /// it does not exist yet. Parse failures fall back to defaults without
    /// aborting the enable.
    #[must_use]
    pub fn load_or_seed(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_else(|err| {
                tracing::error!(
                    "carrot-axiom: failed to parse {}: {err}; using defaults",
                    path.display()
                );
                Self::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let config = Self::default();
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                match toml::to_string_pretty(&config)
                    .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))
                    .and_then(|raw| std::fs::write(path, raw))
                {
                    Ok(()) => config,
                    Err(write_err) => {
                        tracing::error!(
                            "carrot-axiom: failed to write default {}: {write_err}",
                            path.display()
                        );
                        config
                    }
                }
            }
            Err(err) => {
                tracing::error!(
                    "carrot-axiom: failed to read {}: {err}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }
}
