//! Block state string parsing and formatting, including the legacy
//! numeric ids with data values the reference server still used.

use pumpkin_data::{Block, BlockStateId};

/// Legacy `id:data` pairs observed in the reference server logs, mapped to
/// modern block names (the `data` selects the variant).
const LEGACY_IDS: &[(i32, &str, &[&str])] = &[
    (35, "wool", WOOL_COLORS),
    (95, "stained_glass", WOOL_COLORS),
    (159, "terracotta", WOOL_COLORS),
    (160, "stained_glass_pane", WOOL_COLORS),
    (171, "carpet", WOOL_COLORS),
    (251, "concrete", WOOL_COLORS),
    (252, "concrete_powder", WOOL_COLORS),
    (5, "planks", PLANK_KINDS),
    (17, "log", LOG_KINDS),
    (162, "log2", LOG2_KINDS),
];

const WOOL_COLORS: &[&str] = &[
    "white",
    "orange",
    "magenta",
    "light_blue",
    "yellow",
    "lime",
    "pink",
    "gray",
    "light_gray",
    "cyan",
    "purple",
    "blue",
    "brown",
    "green",
    "red",
    "black",
];

const PLANK_KINDS: &[&str] = &["oak", "spruce", "birch", "jungle", "acacia", "dark_oak"];

const LOG_KINDS: &[&str] = &["oak", "spruce", "birch", "jungle"];

const LOG2_KINDS: &[&str] = &["acacia", "dark_oak"];

/// Parses `minecraft:name[key=value,...]` into a block state id.
///
/// Also accepts bare `0` (air), full state strings and legacy `id:data`
/// pairs from the reference server. `hand` resolution happens in the
/// pattern parser instead.
///
/// # Errors
///
/// Returns a Portuguese error message describing the first problem found.
pub fn parse_state_str(input: &str) -> Result<BlockStateId, String> {
    let input = input.trim();
    if input.is_empty() {
        return Err("bloco vazio".to_string());
    }
    if input == "0" || input.eq_ignore_ascii_case("air") {
        return Ok(BlockStateId::AIR);
    }

    // Legacy `id:data` form.
    if let Some((id, data)) = split_legacy(input) {
        return resolve_legacy(id, data);
    }

    let (name, props) = split_properties(input)?;
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let Some(block) = Block::from_registry_key(name) else {
        return Err(format!("bloco desconhecido: {name}"));
    };

    let mut state_id = block.default_state.id;
    if !props.is_empty() {
        let mut parsed = Vec::new();
        for pair in props {
            let Some((key, value)) = pair.split_once('=') else {
                return Err(format!("propriedade sem valor: {pair}"));
            };
            match block.parse_property(key, value) {
                Ok(property) => parsed.push(property),
                Err(_) => return Err(format!("propriedade inválida: {key}={value}")),
            }
        }
        state_id = block.apply_properties(state_id, &parsed);
    }
    Ok(state_id)
}

/// Formats a state id back into `minecraft:name[key=value,...]`, omitting
/// properties when the state is the block's default.
#[must_use]
pub fn state_to_string(state_id: BlockStateId) -> String {
    let block = state_id.to_block();
    if state_id == block.default_state.id {
        return format!("minecraft:{}", block.name);
    }
    let mut text = format!("minecraft:{}[", block.name);
    if let Some(properties) = block.properties(state_id) {
        for property in properties.to_props() {
            text.push_str(property.key_str());
            text.push('=');
            text.push_str(property.value_str());
            text.push(',');
        }
    }
    if text.ends_with(',') {
        text.pop();
    }
    text.push(']');
    text
}

/// `true` for both vanilla air states (used by mask defaults).
#[must_use]
pub const fn is_air(state_id: BlockStateId) -> bool {
    state_id.to_state().is_air()
}

fn split_legacy(input: &str) -> Option<(i32, i32)> {
    let (id, data) = input.split_once(':')?;
    if !id.bytes().all(|b| b.is_ascii_digit()) || !data.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((id.parse().ok()?, data.parse().ok()?))
}

fn resolve_legacy(id: i32, data: i32) -> Result<BlockStateId, String> {
    let Some((_, name, variants)) = LEGACY_IDS.iter().find(|(legacy, _, _)| *legacy == id) else {
        return Err(format!("id legado desconhecido: {id}"));
    };
    let Some(kind) = variants.get(data.unsigned_abs() as usize) else {
        return Err(format!("dados {data} inválidos para id {id}"));
    };
    let full = format!("{kind}_{name}");
    parse_state_str(&full)
}

/// Splits `name[k=v,...]` into the name and property pairs.
fn split_properties(input: &str) -> Result<(&str, Vec<&str>), String> {
    let Some(open) = input.find('[') else {
        return Ok((input, Vec::new()));
    };
    let Some(close) = input.rfind(']') else {
        return Err(format!("']' faltando em: {input}"));
    };
    if close < open || close != input.len() - 1 {
        return Err(format!("estado malformado: {input}"));
    }
    let name = &input[..open];
    let body = &input[open + 1..close];
    let props = body.split(',').filter(|part| !part.is_empty()).collect();
    Ok((name, props))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_block() {
        let state = parse_state_str("stone").expect("stone exists");
        assert_eq!(state_to_string(state), "minecraft:stone");
    }

    #[test]
    fn parses_namespaced_block_with_properties() {
        let state = parse_state_str("minecraft:oak_stairs[facing=north,half=top]")
            .expect("valid stairs state");
        let text = state_to_string(state);
        assert!(text.starts_with("minecraft:oak_stairs[facing=north,half=top"));
    }

    #[test]
    fn parses_air_shortcuts() {
        assert_eq!(parse_state_str("0"), Ok(BlockStateId::AIR));
        assert_eq!(parse_state_str("air"), Ok(BlockStateId::AIR));
    }

    #[test]
    fn parses_legacy_wool() {
        let state = parse_state_str("35:1").expect("legacy wool");
        assert_eq!(state_to_string(state), "minecraft:orange_wool");
    }

    #[test]
    fn parses_legacy_planks() {
        let state = parse_state_str("5:1").expect("legacy planks");
        assert_eq!(state_to_string(state), "minecraft:spruce_planks");
    }

    #[test]
    fn rejects_unknown_block() {
        assert!(parse_state_str("nao_existe").is_err());
    }

    #[test]
    fn rejects_bad_property() {
        assert!(parse_state_str("stone[cor=rosa]").is_err());
    }

    #[test]
    fn rejects_unknown_legacy() {
        assert!(parse_state_str("9999:0").is_err());
        assert!(parse_state_str("35:99").is_err());
    }

    #[test]
    fn round_trips_non_default_state() {
        let state = parse_state_str("oak_log[axis=y]").expect("log state");
        let text = state_to_string(state);
        assert_eq!(parse_state_str(&text), Ok(state));
    }
}
