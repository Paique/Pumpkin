//! WorldEdit-style edit patterns: a single state, weighted random states
//! or the session clipboard sampled by absolute position.

use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;

use crate::{clipboard::Clipboard, states::parse_state_str};

/// What to place at each edited position.
#[derive(Clone, Debug)]
pub enum Pattern {
    /// One fixed state.
    Single(BlockStateId),
    /// Weighted random states (`50%stone,50%dirt`).
    Random(Vec<(u32, BlockStateId)>),
    /// The session clipboard indexed by absolute position (`#clipboard`).
    Clipboard,
}

impl Pattern {
    /// Parses a pattern string.
    ///
    /// `hand` is the state of the block held by the caller (resolved by
    /// the command layer before parsing) so `hand`/`h` terms work.
    ///
    /// # Errors
    ///
    /// Returns a Portuguese error message for the first invalid term.
    pub fn parse(input: &str, hand: Option<BlockStateId>) -> Result<Self, String> {
        let terms = split_terms(input);
        if terms.is_empty() {
            return Err("padrão vazio".to_string());
        }
        if terms.len() == 1 {
            return Ok(match parse_term(&terms[0], hand)? {
                Term::Single(state) | Term::Weighted(_, state) => Self::Single(state),
                Term::Clipboard => Self::Clipboard,
            });
        }

        let mut choices = Vec::with_capacity(terms.len());
        for term in terms {
            match parse_term(&term, hand)? {
                Term::Single(state) => choices.push((1, state)),
                Term::Weighted(weight, state) => choices.push((weight, state)),
                Term::Clipboard => {
                    return Err("#clipboard não pode entrar em listas".to_string());
                }
            }
        }
        Ok(Self::Random(choices))
    }

    /// The state this pattern produces at `pos`, or `None` to leave the
    /// position untouched (out-of-clipboard).
    #[must_use]
    pub fn apply(&self, pos: &BlockPos, clipboard: Option<&Clipboard>) -> Option<BlockStateId> {
        match self {
            Self::Single(state) => Some(*state),
            Self::Random(choices) => {
                let total: u32 = choices.iter().map(|(weight, _)| weight).sum();
                if total == 0 {
                    return choices.first().map(|(_, state)| *state);
                }
                let mut pick = random_below(total);
                for (weight, state) in choices {
                    if pick < *weight {
                        return Some(*state);
                    }
                    pick -= *weight;
                }
                choices.last().map(|(_, state)| *state)
            }
            Self::Clipboard => {
                let clipboard = clipboard?;
                clipboard.state_at_absolute(pos)
            }
        }
    }
}

/// One parsed pattern term (before composition into [`Pattern`]).
enum Term {
    /// A bare block state.
    Single(BlockStateId),
    /// A weighted state (`50%stone`).
    Weighted(u32, BlockStateId),
    /// `#clipboard`.
    Clipboard,
}

fn parse_term(term: &str, hand: Option<BlockStateId>) -> Result<Term, String> {
    let term = term.trim();
    if term.eq_ignore_ascii_case("#clipboard") {
        return Ok(Term::Clipboard);
    }
    if term.eq_ignore_ascii_case("hand") || term.eq_ignore_ascii_case("h") {
        return Ok(Term::Single(
            hand.ok_or_else(|| "nada de bloco na mão".to_string())?,
        ));
    }
    if let Some((weight, rest)) = term.split_once('%') {
        let weight: u32 = weight
            .parse()
            .map_err(|_| format!("peso inválido: {weight}"))?;
        let state = parse_state_str(rest)?;
        return Ok(Term::Weighted(weight, state));
    }
    Ok(Term::Single(parse_state_str(term)?))
}

/// Splits a pattern on top-level commas (commas inside `[...]` stay).
fn split_terms(input: &str) -> Vec<String> {
    let mut terms = Vec::new();
    let mut depth = 0usize;
    let mut current = String::new();
    for ch in input.chars() {
        match ch {
            '[' => {
                depth += 1;
                current.push(ch);
            }
            ']' => {
                depth = depth.saturating_sub(1);
                current.push(ch);
            }
            ',' if depth == 0 => {
                terms.push(std::mem::take(&mut current));
            }
            _ => current.push(ch),
        }
    }
    if !current.trim().is_empty() {
        terms.push(current);
    }
    terms
        .into_iter()
        .map(|term| term.trim().to_string())
        .filter(|term| !term.is_empty())
        .collect()
}

/// Uniform `u32` below `bound` from a fast per-thread xorshift; patterns
/// are sampled millions of times per edit, so the crate avoids pulling a
/// full rng dependency here.
fn random_below(bound: u32) -> u32 {
    thread_local! {
        static STATE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    }
    STATE.with(|state| {
        let mut value = state.get();
        if value == 0 {
            value = seed_from_clock();
        }
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        state.set(value);
        ((value >> 32) as u32) % bound.max(1)
    })
}

fn seed_from_clock() -> u64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0x2545_F491_4F6C_DD1D, |duration| duration.as_nanos() as u64);
    nanos | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_state() {
        assert!(matches!(
            Pattern::parse("stone", None),
            Ok(Pattern::Single(_))
        ));
    }

    #[test]
    fn parses_hand() {
        let stone = parse_state_str("stone").expect("stone");
        assert!(matches!(
            Pattern::parse("hand", Some(stone)),
            Ok(Pattern::Single(state)) if state == stone
        ));
    }

    #[test]
    fn rejects_hand_without_context() {
        assert!(Pattern::parse("hand", None).is_err());
    }

    #[test]
    fn parses_weighted_list() {
        let pattern = Pattern::parse("50%stone,50%dirt", None).expect("valid list");
        let Pattern::Random(choices) = pattern else {
            panic!("expected random pattern");
        };
        assert_eq!(choices.len(), 2);
        assert_eq!(choices[0].0, 50);
        assert_eq!(choices[1].0, 50);
    }

    #[test]
    fn keeps_property_commas_together() {
        let pattern = Pattern::parse("oak_stairs[facing=north,half=top],stone", None);
        assert!(pattern.is_ok());
    }

    #[test]
    fn parses_clipboard_pattern() {
        assert!(matches!(
            Pattern::parse("#clipboard", None),
            Ok(Pattern::Clipboard)
        ));
    }

    #[test]
    fn random_pattern_only_picks_members() {
        let pattern = Pattern::parse("100%stone,1%dirt", None).expect("valid");
        let pos = BlockPos(pumpkin_util::math::vector3::Vector3::new(0, 0, 0));
        for _ in 0..64 {
            let state = pattern.apply(&pos, None).expect("always yields a state");
            let text = crate::states::state_to_string(state);
            assert!(text == "minecraft:stone" || text == "minecraft:dirt");
        }
    }

    #[test]
    fn clipboard_pattern_outside_region_is_none() {
        let pos = BlockPos(pumpkin_util::math::vector3::Vector3::new(99, 99, 99));
        assert_eq!(Pattern::Clipboard.apply(&pos, None), None);
    }
}
