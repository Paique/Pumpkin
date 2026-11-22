//! Module configuration (`chat.toml` in the data folder).
//!
//! Defaults reproduce the reference server's `nChat` setup: a local channel
//! (default, 50-block radius, 5s delay) and a global channel (`/g`, 10s
//! delay), with the Brazilian Portuguese message corrector, anti-caps and
//! URL censorship.

use serde::{Deserialize, Serialize};

/// A public chat channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelConfig {
    /// Command that sends to (or focuses) the channel.
    pub command: String,
    /// Reception radius in blocks; zero means the whole world.
    pub distance: f64,
    /// Seconds between messages in this channel.
    pub cooldown_secs: f64,
}

impl Default for ChannelConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            distance: 0.0,
            cooldown_secs: 0.0,
        }
    }
}

/// Message filters, seeded from the reference `nChat` configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FiltersConfig {
    /// Word replacements in `from1,from2->to` form.
    pub replacers: Vec<String>,
    /// Capitalizes the message and appends a trailing period.
    pub fix_message: bool,
    /// Rewrites shouty messages to lowercase past the allowed percentage.
    pub anti_caps: bool,
    /// Minimum message length for the anti-caps check.
    pub anti_caps_min_length: usize,
    /// Maximum percentage of uppercase letters allowed.
    pub anti_caps_max_percent: u32,
    /// Words kept as written by the anti-caps filter.
    pub anti_caps_skip: Vec<String>,
    /// Blocks messages containing unauthorized URLs.
    pub censor_urls: bool,
    /// When blocking, pretend the message was sent (the sender's client
    /// already echoed it locally, so nobody sees a block notice).
    pub spoof_send: bool,
    /// URL domains that are allowed in chat.
    pub allowed_domains: Vec<String>,
    /// Blocked message fragments, matched with non-alphanumeric characters
    /// concatenated away (bypass protection).
    pub blocked_content: Vec<String>,
}

impl Default for FiltersConfig {
    fn default() -> Self {
        Self {
            replacers: default_replacers(),
            fix_message: true,
            anti_caps: true,
            anti_caps_min_length: 4,
            anti_caps_max_percent: 25,
            anti_caps_skip: vec!["VIP".to_string(), "VIP+".to_string(), "Master".to_string()],
            censor_urls: true,
            spoof_send: true,
            allowed_domains: default_allowed_domains(),
            blocked_content: vec!["ip:".to_string()],
        }
    }
}

fn default_replacers() -> Vec<String> {
    [
        "vc,voce,vose,ce->você",
        "nois->nós",
        "nao->não",
        "tmb,tbm->também",
        "viajem->viagem",
        "viagar->viajar",
        "encima,emcima->em cima",
        ":christmas:,:natal:->☃",
        ":star:,:estrela:->★",
        ":music:,:musica:->♪",
        ":virus:->☣",
        ":urss:->☭",
        "<3,s2,S2->❤",
        ":smile:->◕‿◕",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

pub(crate) fn default_allowed_domains() -> Vec<String> {
    [
        "youtube.com",
        "youtu.be",
        "discord.com",
        "discord.gg",
        "nickuc.com",
        "hastebin.com",
        "imgur.com",
        "prntscr.com",
        "prnt.scr",
    ]
    .iter()
    .map(ToString::to_string)
    .collect()
}

/// `chat.toml` contents.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ChatConfig {
    pub local: ChannelConfig,
    pub global: ChannelConfig,
    pub filters: FiltersConfig,
    /// Chat format with `{DISPLAYNAME}`/`{MESSAGE}` placeholders and legacy
    /// `&` color codes (reference `nChat`: yellow local, gray global).
    pub format_local: String,
    pub format_global: String,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            local: ChannelConfig {
                command: "l".to_string(),
                distance: 50.0,
                cooldown_secs: 5.0,
            },
            global: ChannelConfig {
                command: "g".to_string(),
                distance: 0.0,
                cooldown_secs: 10.0,
            },
            filters: FiltersConfig::default(),
            format_local: "&e[L] &f{DISPLAYNAME}&e: {MESSAGE}".to_string(),
            format_global: "&7[G] &f{DISPLAYNAME}&7: {MESSAGE}".to_string(),
        }
    }
}

impl ChatConfig {
    /// Loads `chat.toml`, seeding it when absent and materializing the full
    /// file for editing. Never fails: parse errors keep the defaults.
    #[must_use]
    pub fn load_or_seed(path: &std::path::Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => toml::from_str(&raw).unwrap_or_else(|err| {
                tracing::warn!(
                    "carrot-chat: failed to parse {}: {err}; using defaults",
                    path.display()
                );
                Self::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let seeded = Self::default();
                if let Err(save_err) =
                    std::fs::write(path, toml::to_string_pretty(&seeded).unwrap_or_default())
                {
                    tracing::warn!(
                        "carrot-chat: failed to write {}: {save_err}",
                        path.display()
                    );
                }
                seeded
            }
            Err(err) => {
                tracing::warn!(
                    "carrot-chat: failed to read {}: {err}; using defaults",
                    path.display()
                );
                Self::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_through_toml() {
        let config = ChatConfig::default();
        let raw = toml::to_string_pretty(&config).expect("serialize");
        let back: ChatConfig = toml::from_str(&raw).expect("deserialize");
        assert_eq!(back.local.command, "l");
        assert_eq!(back.local.distance, 50.0);
        assert_eq!(back.global.cooldown_secs, 10.0);
        assert!(back.filters.replacers.contains(&"nois->nós".to_string()));
        assert!(
            back.filters
                .allowed_domains
                .contains(&"discord.gg".to_string())
        );
        assert!(back.format_local.contains("{DISPLAYNAME}"));
    }

    #[test]
    fn partial_file_falls_back_to_defaults() {
        let raw = "[global]\ncommand = \"global\"\n";
        let parsed: ChatConfig = toml::from_str(raw).expect("parse");
        // `local` falls back to the struct default, not to empty values.
        assert_eq!(parsed.local.command, "l");
        assert_eq!(parsed.global.command, "global");
    }
}
