//! Command registration and shared parsing helpers for the terrain
//! module. Most commands take a single free-form argument string
//! (FAWE-style) and parse tokens and `-x` flags manually.

pub mod aliases;
pub mod br;
pub mod gobrush_cmd;
pub mod sniper_cmd;

use std::collections::HashSet;
use std::sync::Arc;

use carrot_core::ModuleContext;
use carrot_worldedit::pattern::Pattern;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::entity::player::Player;
use pumpkin_data::BlockStateId;
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};
use pumpkin_util::text::TextComponent;

use crate::state::TerrainState;

/// Shared "player-only command" message.
pub(crate) const ONLY_PLAYERS: &str = "Apenas jogadores.";

/// Brush tools (`//br`, `//ab` and `//terragen`).
pub(crate) const PERM_BRUSH: &str = "carrot:terrain.brush";
/// `VoxelSniper` tool (`/b`, `/v`, `/vr`).
pub(crate) const PERM_SNIPER: &str = "carrot:terrain.sniper";
/// goBrush tool (`/gobrush`, `/gb`).
pub(crate) const PERM_GOBRUSH: &str = "carrot:terrain.gobrush";

/// Operator level two, like every worldedit permission.
pub(crate) const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers every terrain command and permission.
pub fn register_all(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    for (node, description) in [
        (PERM_BRUSH, "Allows the terrain brush tools."),
        (PERM_SNIPER, "Allows the VoxelSniper brush tool."),
        (PERM_GOBRUSH, "Allows the goBrush heightmap tool."),
    ] {
        context.register_permission(node, description, OP_TWO);
    }
    br::register(context, state);
    sniper_cmd::register(context, state);
    gobrush_cmd::register(context, state);
    aliases::register(context, state);
    crate::typereplace::register(context, state);
    crate::revolve::register(context, state);
    crate::loft::register(context, state);
    crate::eztexture::register(context, state);
}

/// The player behind the command source, if any.
pub(crate) fn player_of(context: &CommandContext) -> Option<Arc<Player>> {
    context.source.output.as_player()
}

/// Sends a plain feedback line to the command source.
pub(crate) fn reply(context: &CommandContext, message: impl Into<String>) {
    context
        .source
        .send_feedback(TextComponent::text(message.into()), false);
}

/// Splits a free-form argument string into tokens.
pub(crate) fn tokens(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .map(std::string::ToString::to_string)
        .collect()
}

/// Separates `-x` flags from positional tokens (order preserved).
pub(crate) fn split_flags(tokens: &[String]) -> (Vec<String>, HashSet<char>) {
    let mut positional = Vec::new();
    let mut flags = HashSet::new();
    for token in tokens {
        if let Some(rest) = token.strip_prefix('-')
            && !rest.is_empty()
            && rest.chars().all(char::is_alphabetic)
        {
            flags.extend(rest.chars());
        } else {
            positional.push(token.clone());
        }
    }
    (positional, flags)
}

/// Parses a bare number or errors in Portuguese.
pub(crate) fn number<T: std::str::FromStr>(token: &str, label: &str) -> Result<T, String> {
    token
        .parse::<T>()
        .map_err(|_| format!("{label} inválido: {token}"))
}

/// Parses `n` or `min,max` pairs.
pub(crate) fn parse_pair(token: &str) -> Result<(i32, i32), String> {
    if let Some((min, max)) = token.split_once(',') {
        Ok((
            min.trim()
                .parse()
                .map_err(|_| format!("par inválido: {token}"))?,
            max.trim()
                .parse()
                .map_err(|_| format!("par inválido: {token}"))?,
        ))
    } else {
        let value: i32 = token
            .parse()
            .map_err(|_| format!("par inválido: {token}"))?;
        Ok((value, value))
    }
}

/// The block state held by the player (for `hand` patterns).
pub(crate) fn held_block(player: &Player) -> Option<BlockStateId> {
    let stack = player.inventory().held_item();
    if stack.item_count == 0 {
        return None;
    }
    pumpkin_data::Block::from_item_id(stack.item.id).map(|block| block.default_state.id)
}

/// Parses a pattern token with the player's held block context.
pub(crate) fn parse_pattern(player: &Player, token: &str) -> Result<Pattern, String> {
    Pattern::parse(token, held_block(player))
}

/// The raw argument string of the `args` `GreedyPhrase` node, if given.
pub(crate) fn raw_args(context: &CommandContext) -> Option<String> {
    pumpkin_core::command::argument_types::core::string::StringArgumentType::get(context, "args")
        .ok()
        .map(std::string::ToString::to_string)
}

/// Clamps a radius to the configured maximum (with feedback when cut).
pub(crate) fn clamp_radius(state: &Arc<TerrainState>, radius: u32) -> u32 {
    let max = state.with_config(|config| config.max_brush_radius);
    radius.min(max)
}

/// Binds (or clears) the player's `//br` tool on the held item.
pub(crate) fn bind_brush(
    state: &Arc<TerrainState>,
    player: &Player,
    brush: Option<crate::brush::Brush>,
) {
    let held = player.inventory().held_item();
    let item = if held.item_count == 0 {
        String::new()
    } else {
        held.item
            .registry_key
            .trim_start_matches("minecraft:")
            .to_string()
    };
    let player_state = state.player(player.gameprofile.id);
    let mut guard = player_state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.we = brush.map(|brush| (item, brush));
}
