//! The disallowed-blocks filter (mirror of `DisallowedBlocks.java`).
//!
//! Upstream rewrites the player's block-state registry so every state on
//! the configured list collides with the void-air sentinel: a buffer or
//! `set_block` carrying a disallowed state simply never applies it. The
//! Rust port keeps one global resolved deny set: states resolve to `None`
//! in [`crate::buffer::filter_state`] and in the `set_block` parser, which
//! is observationally identical (the state is skipped, the edit continues).
//!
//! Entries are block identifiers such as `minecraft:command_block`; entries
//! with properties (e.g. `minecraft:water[level=0]`) are not supported and
//! are ignored with a warning, matching upstream's best-effort parse.

use std::collections::HashSet;
use std::sync::OnceLock;

use pumpkin_data::{Block, BlockId};

/// The resolved deny set (`None` while unset = empty).
static DISALLOWED: OnceLock<HashSet<BlockId>> = OnceLock::new();

/// Resolves the configured deny list. Idempotent on the shared set; the
/// first call wins (the module is enabled once per process).
pub fn configure(disallowed: &[String]) {
    let mut set = HashSet::new();
    for entry in disallowed {
        let name = entry.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(block) = Block::from_name(name) {
            set.insert(block.id);
        } else {
            tracing::warn!("carrot-axiom: unknown disallowed block {entry:?}, ignoring");
        }
    }
    let _ = DISALLOWED.set(set);
}

/// Whether `block` is on the deny list.
#[must_use]
pub fn is_disallowed(block: BlockId) -> bool {
    DISALLOWED.get().is_some_and(|set| set.contains(&block))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unlisted_blocks_are_allowed() {
        assert!(!is_disallowed(Block::STONE.id));
    }

    #[test]
    fn from_name_resolves_namespaced_entries() {
        let set: HashSet<BlockId> = ["minecraft:command_block", "bedrock"]
            .iter()
            .filter_map(|entry| Block::from_name(entry).map(|block| block.id))
            .collect();
        assert!(set.contains(&Block::COMMAND_BLOCK.id));
        assert!(set.contains(&Block::BEDROCK.id));
        assert!(!set.contains(&Block::STONE.id));
    }
}
