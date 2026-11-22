//! `carrot/groups.toml` handling: permission groups with inheritance.
//!
//! Groups are the carrot equivalent of the `default`/`builder` `LuckPerms`
//! layout of the reference build server. Permissions listed in a group are
//! applied to a player's attachment when they join (and re-applied on
//! `carrot groups reload`). A permission entry prefixed with `-` denies the
//! node instead of granting it.

use std::{collections::BTreeMap, fs, io, path::Path};

use serde::{Deserialize, Serialize};
use tracing::error;

/// A single permission group.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupDef {
    /// Groups whose permissions are inherited by this group.
    #[serde(default)]
    pub inherits: Vec<String>,
    /// Permission nodes granted by this group. Prefix with `-` to deny.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Player names (case-sensitive) belonging to this group.
    #[serde(default)]
    pub players: Vec<String>,
}

/// All permission groups, keyed by group name.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GroupsConfig {
    /// Group definitions.
    #[serde(default)]
    pub groups: BTreeMap<String, GroupDef>,
}

impl GroupsConfig {
    /// Builds the default group layout, seeded from the reference build
    /// server's `LuckPerms` data (`default` + `builder` with its builders).
    #[must_use]
    pub fn seed() -> Self {
        let mut groups = BTreeMap::new();
        groups.insert(
            "default".to_string(),
            GroupDef {
                players: vec![
                    "Kyphy".to_string(),
                    "sickabliat".to_string(),
                    "DrakooPlays".to_string(),
                ],
                ..GroupDef::default()
            },
        );
        groups.insert(
            "builder".to_string(),
            GroupDef {
                inherits: vec!["default".to_string()],
                permissions: vec![
                    "carrot:buildermode".to_string(),
                    "carrot:essentials.gamemode".to_string(),
                    "carrot:essentials.setwarp".to_string(),
                    "carrot:essentials.delwarp".to_string(),
                    "axiom.default".to_string(),
                ],
                players: vec![
                    "sh4wsz".to_string(),
                    "oJotaaa".to_string(),
                    "DoTrO_NoL".to_string(),
                    "ManoDitro".to_string(),
                    "Mitsudomoe".to_string(),
                    "sickabliat".to_string(),
                    "Dovahkiin__".to_string(),
                    "levyxsw".to_string(),
                    "CHACRAVAU".to_string(),
                    "Frostz9x".to_string(),
                    "_HyZaCk_".to_string(),
                    "spreitail".to_string(),
                    "CreeperBUILDS".to_string(),
                    "Assuiya".to_string(),
                    "tufy1249".to_string(),
                    "FormigTeen".to_string(),
                    "JNPbuilds".to_string(),
                    "Kyphy".to_string(),
                ],
            },
        );
        Self { groups }
    }

    /// Reads the groups file. When it does not exist yet, the seeded default
    /// is returned and written to disk so it can be edited.
    #[must_use]
    pub fn load(path: &Path) -> Self {
        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let seed = Self::seed();
                if let Err(write_err) = seed.save(path) {
                    error!(
                        "carrot: failed to write default {}: {write_err}",
                        path.display()
                    );
                }
                return seed;
            }
            Err(err) => {
                error!(
                    "carrot: failed to read {}: {err}; using defaults",
                    path.display()
                );
                return Self::seed();
            }
        };
        match toml::from_str(&raw) {
            Ok(config) => config,
            Err(err) => {
                error!(
                    "carrot: failed to parse {}: {err}; using defaults",
                    path.display()
                );
                Self::seed()
            }
        }
    }

    /// Persists the groups file, creating parent directories as needed.
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

    /// Returns the names of the groups a player belongs to.
    #[must_use]
    pub fn groups_of(&self, player_name: &str) -> Vec<String> {
        self.groups
            .iter()
            .filter(|(_, def)| def.players.iter().any(|p| p == player_name))
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Resolves every permission node that applies to a player, following
    /// group inheritance (with cycle protection). Later groups win over
    /// earlier ones; an explicit deny (`-node`) always overrides a grant of
    /// the same node from another group.
    #[must_use]
    pub fn permissions_of(&self, player_name: &str) -> Vec<(String, bool)> {
        let mut resolved: BTreeMap<String, bool> = BTreeMap::new();
        let mut visited: Vec<String> = Vec::new();
        for group in self.groups_of(player_name) {
            self.collect_group_permissions(&group, &mut resolved, &mut visited);
        }
        resolved.into_iter().collect()
    }

    fn collect_group_permissions(
        &self,
        group: &str,
        resolved: &mut BTreeMap<String, bool>,
        visited: &mut Vec<String>,
    ) {
        if visited.iter().any(|v| v == group) {
            return;
        }
        visited.push(group.to_string());
        let Some(def) = self.groups.get(group) else {
            return;
        };
        for inherited in &def.inherits {
            self.collect_group_permissions(inherited, resolved, visited);
        }
        for entry in &def.permissions {
            if let Some(node) = entry.strip_prefix('-') {
                resolved.insert(node.to_string(), false);
            } else {
                resolved.insert(entry.clone(), true);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GroupsConfig {
        toml::from_str(
            r#"
            [groups.default]
            permissions = ["carrot:basic"]

            [groups.builder]
            inherits = ["default"]
            permissions = ["carrot:buildermode", "-carrot:basic"]
            players = ["oJotaaa"]

            [groups.loop_a]
            inherits = ["loop_b"]
            permissions = ["carrot:a"]
            players = ["loopy"]

            [groups.loop_b]
            inherits = ["loop_a"]
            permissions = ["carrot:b"]
            "#,
        )
        .unwrap_or_else(|err| panic!("fixture should parse: {err}"))
    }

    #[test]
    fn resolves_inheritance_and_deny() {
        let perms = config().permissions_of("oJotaaa");
        assert!(perms.contains(&("carrot:buildermode".to_string(), true)));
        assert!(perms.contains(&("carrot:basic".to_string(), false)));
    }

    #[test]
    fn unknown_player_has_no_permissions() {
        assert!(config().permissions_of("nobody").is_empty());
    }

    #[test]
    fn inheritance_cycles_terminate() {
        let perms = config().permissions_of("loopy");
        assert!(perms.contains(&("carrot:a".to_string(), true)));
        assert!(perms.contains(&("carrot:b".to_string(), true)));
    }
}
