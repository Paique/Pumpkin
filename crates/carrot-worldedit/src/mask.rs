//! Block masks: which existing states an edit may touch.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;

use crate::states::{is_air, parse_state_str};

/// A mask that needs world context (neighbors, absolute position) to
/// decide, such as the terrain masks contributed by `carrot-terrain`.
pub trait WorldMask: Send + Sync {
    /// `true` when the position passes this mask.
    fn test(&self, world: &World, pos: &BlockPos) -> bool;
}

/// Factory signature for `#name[arg][...]` mask terms registered by other
/// crates (returns a Portuguese error message on bad input).
pub type WorldMaskFactory =
    Arc<dyn Fn(&[&str]) -> Result<Arc<dyn WorldMask>, String> + Send + Sync>;

/// Registry of `#name` mask factories, populated by dependent modules.
fn world_mask_registry() -> &'static Mutex<HashMap<String, WorldMaskFactory>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, WorldMaskFactory>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers a `#name[arg][...]` mask factory. Registering an existing
/// name replaces it (module re-enable refreshes its own entries).
pub fn register_world_mask(name: &str, factory: WorldMaskFactory) {
    world_mask_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(name.to_string(), factory);
}

/// Parses a `#name[arg][arg]...` term through the registry.
fn parse_world_mask(term: &str) -> Option<Result<Mask, String>> {
    let rest = term.strip_prefix('#')?;
    let (name, tail) = match rest.split_once('[') {
        Some((name, tail)) => (name, tail),
        None => (rest, ""),
    };
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let factory = world_mask_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(name)
        .cloned()?;
    let mut args: Vec<&str> = Vec::new();
    let mut leftovers = tail;
    while let Some((arg, rest)) = leftovers.split_once(']') {
        args.push(arg.trim());
        leftovers = match rest.strip_prefix('[') {
            Some(next) => next,
            None => break,
        };
    }
    if tail.is_empty() {
        args.clear();
    }
    Some((factory)(&args).map(Mask::World))
}

/// Restricts edits by the state currently at each position.
#[derive(Clone)]
pub enum Mask {
    /// Every position matches.
    All,
    /// Only non-air positions match (the implicit default of `//replace`).
    NonAir,
    /// Matches the listed states. Each entry keeps whether it was written
    /// with explicit properties (`stone[facing=north]` matches only that
    /// state; `stone` matches every state of the stone block).
    States(Vec<(BlockStateId, bool)>),
    /// A registered `#name` mask needing world context.
    World(Arc<dyn WorldMask>),
}

impl Mask {
    /// Parses a mask string (`stone,dirt`, `0`, `*`, `hand`, ...).
    ///
    /// # Errors
    ///
    /// Returns a Portuguese error message for the first invalid term.
    pub fn parse(input: &str, hand: Option<BlockStateId>) -> Result<Self, String> {
        let input = input.trim();
        if input.is_empty() {
            return Err("máscara vazia".to_string());
        }
        if input == "*" {
            return Ok(Self::All);
        }
        let mut entries = Vec::new();
        for term in input.split(',') {
            let term = term.trim();
            if term.is_empty() {
                continue;
            }
            if term.eq_ignore_ascii_case("hand") || term.eq_ignore_ascii_case("h") {
                let state = hand.ok_or_else(|| "nada de bloco na mão".to_string())?;
                entries.push((state, true));
                continue;
            }
            if let Some(result) = parse_world_mask(term) {
                return result;
            }
            let exact = term.contains('[');
            let state = parse_state_str(term)?;
            entries.push((state, exact));
        }
        if entries.is_empty() {
            return Err("máscara vazia".to_string());
        }
        Ok(Self::States(entries))
    }

    /// `true` when `state_id` passes this mask.
    #[must_use]
    // World masks look like `All` here: they decide from world context
    // (see `test_at`), so the identical arm is intentional.
    #[allow(clippy::match_same_arms)]
    pub fn test(&self, state_id: BlockStateId) -> bool {
        match self {
            Self::All => true,
            Self::NonAir => !is_air(state_id),
            Self::States(entries) => entries.iter().any(|(wanted, exact)| {
                if *exact {
                    *wanted == state_id
                } else {
                    wanted.to_block_id() == state_id.to_block_id()
                }
            }),
            Self::World(_) => true,
        }
    }

    /// `true` when the position passes this mask with world context.
    ///
    /// World masks decide from the world itself; every other mask
    /// degrades to testing the state at the position.
    #[must_use]
    pub fn test_at(&self, world: &World, pos: &BlockPos) -> bool {
        match self {
            Self::World(mask) => mask.test(world, pos),
            other => other.test(world.get_block_state_id(pos)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_matches_everything() {
        assert!(
            Mask::parse("*", None)
                .expect("valid")
                .test(BlockStateId::AIR)
        );
    }

    #[test]
    fn air_mask_matches_only_air() {
        let mask = Mask::parse("0", None).expect("valid");
        assert!(mask.test(BlockStateId::AIR));
        let stone = parse_state_str("stone").expect("stone");
        assert!(!mask.test(stone));
    }

    #[test]
    fn bare_block_matches_all_states_of_block() {
        let mask = Mask::parse("oak_stairs", None).expect("valid");
        let facing = parse_state_str("oak_stairs[facing=north]").expect("stairs state");
        assert!(mask.test(facing));
    }

    #[test]
    fn stateful_term_matches_only_that_state() {
        let mask = Mask::parse("oak_stairs[facing=north]", None).expect("valid");
        let facing = parse_state_str("oak_stairs[facing=north]").expect("north stairs");
        let other = parse_state_str("oak_stairs[facing=south]").expect("south stairs");
        assert!(mask.test(facing));
        assert!(!mask.test(other));
    }

    #[test]
    fn non_air_default() {
        assert!(Mask::NonAir.test(parse_state_str("stone").expect("stone")));
        assert!(!Mask::NonAir.test(BlockStateId::AIR));
    }

    #[test]
    fn hand_mask_uses_held_state() {
        let stone = parse_state_str("stone").expect("stone");
        let mask = Mask::parse("hand", Some(stone)).expect("valid");
        assert!(mask.test(stone));
    }
}
