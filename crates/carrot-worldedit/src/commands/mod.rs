//! Command registration and shared helpers.

pub mod clipboard;
pub mod generation;
pub mod history;
pub mod mask;
pub mod navigation;
pub mod region;
pub mod schem;
pub mod selection;
pub mod utility;

use std::sync::Arc;

use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::entity::player::Player;
use pumpkin_data::{Block, BlockStateId};
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::text::TextComponent;
use uuid::Uuid;

use crate::clipboard::Clipboard;
use crate::edit::{EditRequest, execute};
use crate::region::{Cuboid, Direction};
use crate::session::WeState;

/// Shared "no selection yet" error message.
pub(crate) const NO_SELECTION: &str = "Faça uma seleção primeiro (wand, //pos1 e //pos2).";

/// Shared "player-only command" message.
pub(crate) const ONLY_PLAYERS: &str = "Apenas jogadores.";

/// Registers every worldedit command and permission.
pub fn register_all(context: &mut carrot_core::ModuleContext<'_>, state: &Arc<WeState>) {
    selection::register(context, state);
    region::register(context, state);
    clipboard::register(context, state);
    history::register(context, state);
    schem::register(context, state);
    mask::register(context, state);
    generation::register(context, state);
    utility::register(context, state);
    navigation::register(context, state);
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

/// Sends a plain message directly to a player (async feedback).
pub(crate) fn tell(player: &Player, message: impl Into<String>) {
    player.send_system_message(&TextComponent::text(message.into()));
}

/// The block the player holds, resolved to a state (for `hand`).
pub(crate) fn held_block(player: &Player) -> Option<BlockStateId> {
    let stack = player.inventory().held_item();
    if stack.item_count == 0 {
        return None;
    }
    Block::from_item_id(stack.item.id).map(|block| block.default_state.id)
}

/// The block the player stands in (`WorldEdit` placement position).
pub(crate) fn player_block_pos(player: &Player) -> BlockPos {
    let position = player.position();
    BlockPos(pumpkin_util::math::vector3::Vector3::new(
        position.x.floor() as i32,
        position.y.floor() as i32,
        position.z.floor() as i32,
    ))
}

/// The horizontal direction the player faces.
pub(crate) fn look_direction(player: &Player) -> Direction {
    Direction::from_yaw(player.living_entity.entity.yaw.load())
}

/// The player's cuboid selection or a Portuguese error message.
pub(crate) fn selection_of(state: &WeState, uuid: Uuid) -> Result<Cuboid, String> {
    let session = state.session(uuid);
    let guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .selection
        .map(|selection| Cuboid::new(selection[0], selection[1]))
        .ok_or_else(|| NO_SELECTION.to_string())
}

/// Rejects regions larger than the configured limit.
pub(crate) fn check_volume(state: &WeState, region: &Cuboid) -> Result<(), String> {
    let max = state.with_config(|config| config.max_blocks);
    match region.volume() {
        Some(volume) if volume <= max => Ok(()),
        Some(volume) => Err(format!(
            "Região grande demais: {volume} blocos (limite {max})."
        )),
        None => Err("Região grande demais.".to_string()),
    }
}

/// Rejects estimated shape volumes above the configured limit.
pub(crate) fn check_volume_estimate(state: &WeState, estimate: u64) -> Result<(), String> {
    let max = state.with_config(|config| config.max_blocks);
    if estimate <= max {
        Ok(())
    } else {
        Err(format!(
            "Forma grande demais: ~{estimate} blocos (limite {max})."
        ))
    }
}

/// Reads the session clipboard for `#clipboard` patterns.
pub(crate) fn clipboard_of(state: &WeState, uuid: Uuid) -> Option<Arc<Clipboard>> {
    let session = state.session(uuid);
    let guard = session
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.clipboard.clone()
}

/// Spawns an edit request onto the tokio runtime (never blocks the
/// command executor or the tick loop).
pub(crate) fn spawn_edit(state: &Arc<WeState>, player: &Arc<Player>, request: EditRequest) {
    let state = Arc::clone(state);
    let player = Arc::clone(player);
    tokio::spawn(async move {
        execute(&state, &player, request).await;
    });
}
