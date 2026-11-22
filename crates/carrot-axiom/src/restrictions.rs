//! `axiom:restrictions` payload: the capability set derived from the
//! player's permissions (mirror of upstream `Restrictions.java` and
//! `AxiomPaper.calculateRestrictions`).
//!
//! Wire layout:
//! `VarInt nAllowed` + `Utf8` names · `VarInt nDenied` + names ·
//! `Int infiniteReachLimit` · `VarInt nBounds` + packed-long `min`,`max`
//! pairs. `axiom.default` serializes as the internal name `axiom.*`;
//! hierarchy is deduplicated (a node is omitted when its enum parent is in
//! the same set) exactly like the reference implementation.

use std::sync::Arc;

use pumpkin_core::{entity::player::Player, server::Server};
use pumpkin_util::permission::PermissionManager;
use uuid::Uuid;

use crate::{
    config::AxiomConfig,
    permissions::{self, AxiomPerm, ALL_NODE, DEFAULT_NODE, PERMISSIONS, wire_name},
    wire::{self, Bounds, Writer},
};

/// The restrictions announced to one player.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restrictions {
    /// Internal node names allowed (before wire-name translation).
    allowed: Vec<&'static str>,
    /// Internal node names denied.
    denied: Vec<&'static str>,
    /// Infinite reach block limit (config).
    pub infinite_reach_limit: i32,
    /// Region bounds (empty until a region integration exists).
    pub bounds: Vec<Bounds>,
}

impl Restrictions {
    /// The unrestricted set used for ops and `axiom.all` holders.
    #[must_use]
    pub fn all(infinite_reach_limit: i32) -> Self {
        Self {
            allowed: vec![ALL_NODE],
            denied: Vec::new(),
            infinite_reach_limit,
            bounds: Vec::new(),
        }
    }

    /// Computes a player's restrictions from explicit permission
    /// attachments. Resolution mirrors `LuckPerms` semantics: the node's own
    /// value wins, then its enum ancestors, then `axiom.default` for
    /// recommended-set nodes; registry defaults and op status are handled by
    /// the caller's short-circuit.
    #[must_use]
    pub fn calculate(server: &Arc<Server>, player: &Arc<Player>, config: &AxiomConfig) -> Self {
        let uuid = player.gameprofile.id;
        let manager = &server.permission_manager;

        let mut allowed = Vec::new();
        let mut denied = Vec::new();
        for perm in PERMISSIONS.iter().filter(|perm| perm.node != ALL_NODE) {
            match resolve(manager, uuid, perm) {
                Some(true) => allowed.push(perm.node),
                Some(false) => denied.push(perm.node),
                None => {}
            }
        }

        let allowed = dedupe(&allowed);
        let denied = dedupe(&denied);
        Self {
            allowed,
            denied,
            infinite_reach_limit: config.infinite_reach_limit,
            bounds: Vec::new(),
        }
    }

    /// Encodes the `axiom:restrictions` payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut writer = Writer::new();
        writer.write_var_int(self.allowed.len() as i32);
        for node in &self.allowed {
            writer.write_string(wire_name(node));
        }
        writer.write_var_int(self.denied.len() as i32);
        for node in &self.denied {
            writer.write_string(wire_name(node));
        }
        writer.write_i32_be(self.infinite_reach_limit);
        writer.write_var_int(self.bounds.len() as i32);
        for bound in &self.bounds {
            writer.write_i64_be(wire::pack_block_pos(bound.min.0, bound.min.1, bound.min.2));
            writer.write_i64_be(wire::pack_block_pos(bound.max.0, bound.max.1, bound.max.2));
        }
        writer.into_bytes()
    }
}

/// Resolves a node to an explicit tri-state, walking enum ancestors and the
/// `axiom.default` recommendation root.
fn resolve(manager: &PermissionManager, uuid: Uuid, perm: &'static AxiomPerm) -> Option<bool> {
    if let Some(value) = manager.has_permission_set(&uuid, perm.node) {
        return Some(value);
    }
    let mut parent = perm.parent;
    while let Some(node) = parent {
        if let Some(value) = manager.has_permission_set(&uuid, node) {
            return Some(value);
        }
        parent = permissions::find(node).and_then(|entry| entry.parent);
    }
    if perm.default_group && perm.node != DEFAULT_NODE {
        return manager.has_permission_set(&uuid, DEFAULT_NODE);
    }
    None
}

/// Drops nodes whose enum parent is present in the same set.
fn dedupe(nodes: &[&'static str]) -> Vec<&'static str> {
    nodes
        .iter()
        .filter(|node| {
            permissions::find(node)
                .and_then(|perm| perm.parent)
                .is_none_or(|parent| !nodes.contains(&parent))
        })
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_restrictions_encode() {
        let restrictions = Restrictions::all(256);
        let bytes = restrictions.encode();
        // VarInt 1 + Utf8 "axiom.all" + VarInt 0 + Int 256 BE + VarInt 0.
        let expected: Vec<u8> = [
            &[1u8, 9][..],
            b"axiom.all",
            &[0u8],
            &256i32.to_be_bytes(),
            &[0u8],
        ]
        .concat();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn default_group_serializes_as_axiom_wildcard() {
        let restrictions = Restrictions {
            allowed: vec![permissions::DEFAULT_NODE],
            denied: Vec::new(),
            infinite_reach_limit: 256,
            bounds: Vec::new(),
        };
        let bytes = restrictions.encode();
        assert_eq!(&bytes[..2], &[1, 7]); // VarInt 1 + len 7
        assert_eq!(&bytes[2..9], b"axiom.*");
    }
}
