//! Clipboard commands: copy/cut/paste/rotate/flip/stack/move plus the
//! `DoTrO` shorthand aliases and rotate-paste macros.

use std::collections::HashMap;
use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::integer::IntegerArgumentType;
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};

use crate::clipboard::{Clipboard, FlipAxis};
use crate::commands::{
    ONLY_PLAYERS, check_volume, look_direction, player_block_pos, player_of, reply, selection_of,
};
use crate::edit::{EditRequest, execute, read_states};
use crate::region::{Cuboid, Direction};
use crate::session::WeState;
use crate::states::is_air;

const PERM_CLIPBOARD: &str = "carrot:worldedit.clipboard";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers clipboard commands and permissions.
// Flat registration table; extracting helpers would obscure the alias map.
#[allow(clippy::too_many_lines)]
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(PERM_CLIPBOARD, "Allows using the clipboard.", OP_TWO);

    context.register_command(
        "//copy",
        command("//copy", "Copies the selection.").executes(CopyExecutor {
            state: Arc::clone(state),
            cut: false,
        }),
        PERM_CLIPBOARD,
    );
    context.register_command(
        "//c",
        command("//c", "Copies the selection.").executes(CopyExecutor {
            state: Arc::clone(state),
            cut: false,
        }),
        PERM_CLIPBOARD,
    );
    context.register_command(
        "//cut",
        command("//cut", "Copies and clears the selection.").executes(CopyExecutor {
            state: Arc::clone(state),
            cut: true,
        }),
        PERM_CLIPBOARD,
    );

    // FAWE paste aliases (`//p`, `//pa`) plus the Builders-Utilities
    // shorthand aliases the builders actually type (`//c`, `//f`, `//pa`).
    register_paste(context, state, "//paste");
    register_paste(context, state, "//p");
    register_paste(context, state, "//pa");
    register_rotate(context, state, "//rotate");
    {
        let name = "//rt";
        register_rotate(context, state, name);
    }
    register_flip(context, state, "//flip");
    register_flip(context, state, "//f");

    let stack = StackExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//stack",
        command("//stack", "Repeats the selection along a direction.")
            .executes(stack.clone())
            .then(
                argument("count", IntegerArgumentType::any())
                    .executes(stack.clone())
                    .then(argument("direction", StringArgumentType::SingleWord).executes(stack)),
            ),
        PERM_CLIPBOARD,
    );
    let mv = MoveExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//move",
        command("//move", "Moves the selection contents.")
            .executes(mv.clone())
            .then(
                argument("count", IntegerArgumentType::any())
                    .executes(mv.clone())
                    .then(argument("direction", StringArgumentType::SingleWord).executes(mv)),
            ),
        PERM_CLIPBOARD,
    );

    // DoTrO pure aliases.
    let stack_alias = StackExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//st",
        command("//st", "Repeats the selection along a direction.")
            .executes(stack_alias.clone())
            .then(
                argument("count", IntegerArgumentType::any())
                    .executes(stack_alias.clone())
                    .then(
                        argument("direction", StringArgumentType::SingleWord).executes(stack_alias),
                    ),
            ),
        PERM_CLIPBOARD,
    );
    let move_alias = MoveExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//m",
        command("//m", "Moves the selection contents.")
            .executes(move_alias.clone())
            .then(
                argument("count", IntegerArgumentType::any())
                    .executes(move_alias.clone())
                    .then(
                        argument("direction", StringArgumentType::SingleWord).executes(move_alias),
                    ),
            ),
        PERM_CLIPBOARD,
    );

    // DoTrO macros (flip+paste and rotate+paste cycles).
    for (name, kind) in MACROS {
        let executor = MacroExecutor {
            state: Arc::clone(state),
            kind: *kind,
        };
        context.register_command(
            name,
            command(*name, "Runs a combined clipboard macro.").executes(executor),
            PERM_CLIPBOARD,
        );
    }
}

/// The `DoTrO` macros, exactly as configured in `dotro-worldedit.yml`.
const MACROS: &[(&str, MacroKind)] = &[
    ("//fp", MacroKind::Flip { skip_air: false }),
    ("//fp-a", MacroKind::Flip { skip_air: true }),
    (
        "//rt90x4",
        MacroKind::Rotate {
            copies: 4,
            skip_air: false,
        },
    ),
    (
        "//rt90x4-a",
        MacroKind::Rotate {
            copies: 4,
            skip_air: true,
        },
    ),
    (
        "//crt90x4",
        MacroKind::CopyRotate {
            copies: 4,
            skip_air: false,
        },
    ),
    (
        "//crt90x4-a",
        MacroKind::CopyRotate {
            copies: 4,
            skip_air: true,
        },
    ),
];

fn register_paste(context: &mut ModuleContext<'_>, state: &Arc<WeState>, name: &str) {
    let plain = PasteExecutor {
        state: Arc::clone(state),
    };
    let flagged = PasteExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        name,
        command(name.to_string(), "Pastes the clipboard.")
            .executes(plain)
            .then(argument("flags", StringArgumentType::GreedyPhrase).executes(flagged)),
        PERM_CLIPBOARD,
    );
}

fn register_rotate(context: &mut ModuleContext<'_>, state: &Arc<WeState>, name: &str) {
    let executor = RotateExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        name,
        command(name.to_string(), "Rotates the clipboard.")
            .then(argument("angle", IntegerArgumentType::any()).executes(executor)),
        PERM_CLIPBOARD,
    );
}

fn register_flip(context: &mut ModuleContext<'_>, state: &Arc<WeState>, name: &str) {
    let executor = FlipExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        name,
        command(name.to_string(), "Mirrors the clipboard.")
            .executes(executor.clone())
            .then(argument("direction", StringArgumentType::SingleWord).executes(executor)),
        PERM_CLIPBOARD,
    );
}

/// `//copy` and `//cut`.
struct CopyExecutor {
    state: Arc<WeState>,
    cut: bool,
}

impl CommandExecutor for CopyExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let region = match selection_of(&self.state, player.gameprofile.id) {
            Ok(region) => region,
            Err(message) => {
                reply(context, message);
                return Ok(0);
            }
        };
        if let Err(message) = check_volume(&self.state, &region) {
            reply(context, message);
            return Ok(0);
        }

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        let cut = self.cut;
        tokio::spawn(async move {
            let world = player.world();
            let origin = player_block_pos(&player);
            let clipboard = Arc::new(Clipboard::from_world(&world, &region, origin));
            let volume = clipboard.volume();
            {
                let session = state.session(player.gameprofile.id);
                let mut guard = session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.clipboard = Some(clipboard);
            }
            crate::commands::tell(&player, format!("Copiados {volume} blocos."));

            if cut {
                let positions = region.iterate();
                let air = BlockStateId::AIR;
                let desired = positions.into_iter().map(|pos| (pos, air)).collect();
                let request = EditRequest::new(desired, "//cut".to_string());
                execute(&state, &player, request).await;
            }
        });
        reply(context, "Copiando...".to_string());
        Ok(1)
    }
}

/// `//paste [-a] [-o] [-s]`: `-a` skips air, `-o` pastes at the copy
/// origin, `-s` re-selects the pasted area.
#[derive(Clone)]
struct PasteExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for PasteExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let flags = StringArgumentType::get(context, "flags").unwrap_or_default();
        let (skip_air, at_origin, select) = parse_paste_flags(flags);

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        tokio::spawn(async move {
            match build_paste_request(&state, &player, skip_air, at_origin) {
                Ok(request) => {
                    if select {
                        let session = state.session(player.gameprofile.id);
                        let mut guard = session
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let clipboard = guard.clipboard.clone();
                        if let Some(clipboard) = clipboard {
                            let anchor = paste_anchor(&player, &clipboard, at_origin);
                            let region = Cuboid::new(
                                anchor,
                                BlockPos(Vector3::new(
                                    anchor.0.x + clipboard.size().x - 1,
                                    anchor.0.y + clipboard.size().y - 1,
                                    anchor.0.z + clipboard.size().z - 1,
                                )),
                            );
                            guard.selection = Some([region.min(), region.max()]);
                        }
                    }
                    execute(&state, &player, request).await;
                }
                Err(message) => crate::commands::tell(&player, message),
            }
        });
        reply(context, "Colando...".to_string());
        Ok(1)
    }
}

/// `//rotate <angle>` (multiples of 90).
#[derive(Clone)]
struct RotateExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for RotateExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let angle = IntegerArgumentType::get(context, "angle")?;
        if angle % 90 != 0 {
            reply(context, "O ângulo precisa ser múltiplo de 90.".to_string());
            return Ok(0);
        }
        let steps = ((angle / 90) % 4).rem_euclid(4) as u8;

        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(clipboard) = guard.clipboard.clone() else {
            drop(guard);
            reply(context, "Nada copiado ainda (use //copy).".to_string());
            return Ok(0);
        };
        guard.clipboard = Some(Arc::new(clipboard.rotated(steps)));
        drop(guard);
        reply(context, format!("Clipboard rotacionada em {angle} graus."));
        Ok(1)
    }
}

/// `//flip [direction]`.
#[derive(Clone)]
struct FlipExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for FlipExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let axis = match StringArgumentType::get(context, "direction") {
            Ok(keyword) => {
                if let Some(axis) = flip_axis(keyword) {
                    axis
                } else {
                    reply(context, format!("Direção desconhecida: {keyword}"));
                    return Ok(0);
                }
            }
            Err(_) => flip_axis_of_look(look_direction(&player)),
        };

        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(clipboard) = guard.clipboard.clone() else {
            drop(guard);
            reply(context, "Nada copiado ainda (use //copy).".to_string());
            return Ok(0);
        };
        guard.clipboard = Some(Arc::new(clipboard.flipped(axis)));
        drop(guard);
        reply(context, "Clipboard espelhada.".to_string());
        Ok(1)
    }
}

/// `//stack <count> [direction]`.
#[derive(Clone)]
struct StackExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for StackExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let count = IntegerArgumentType::get(context, "count")
            .unwrap_or(1)
            .clamp(1, 100);
        let direction = match StringArgumentType::get(context, "direction") {
            Ok(keyword) => {
                if let Some(direction) = Direction::parse(keyword) {
                    direction
                } else {
                    reply(context, format!("Direção desconhecida: {keyword}"));
                    return Ok(0);
                }
            }
            Err(_) => look_direction(&player),
        };

        let region = match selection_of(&self.state, player.gameprofile.id) {
            Ok(region) => region,
            Err(message) => {
                reply(context, message);
                return Ok(0);
            }
        };
        let span = span_along(&region, direction);
        let stacked = region.shift(direction, span.saturating_mul(count));
        if let Err(message) = check_volume(&self.state, &stacked) {
            reply(context, message);
            return Ok(0);
        }

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        tokio::spawn(async move {
            let world = player.world();
            let positions = region.iterate();
            let states = read_states(&world, &positions);
            let mut desired = Vec::with_capacity(positions.len() * count as usize);
            for copy in 1..=count {
                let offset = direction.offset() * (span * copy);
                for (index, pos) in positions.iter().enumerate() {
                    desired.push((
                        BlockPos(Vector3::new(
                            pos.0.x + offset.x,
                            pos.0.y + offset.y,
                            pos.0.z + offset.z,
                        )),
                        states[index],
                    ));
                }
            }
            let request = EditRequest::new(desired, format!("//stack {count}"));
            execute(&state, &player, request).await;
        });
        reply(context, "Repetindo seleção...".to_string());
        Ok(1)
    }
}

/// `//move <count> [direction]`.
#[derive(Clone)]
struct MoveExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for MoveExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let count = IntegerArgumentType::get(context, "count").unwrap_or(1);
        let direction = match StringArgumentType::get(context, "direction") {
            Ok(keyword) => {
                if let Some(direction) = Direction::parse(keyword) {
                    direction
                } else {
                    reply(context, format!("Direção desconhecida: {keyword}"));
                    return Ok(0);
                }
            }
            Err(_) => look_direction(&player),
        };

        let region = match selection_of(&self.state, player.gameprofile.id) {
            Ok(region) => region,
            Err(message) => {
                reply(context, message);
                return Ok(0);
            }
        };
        let moved = region.shift(direction, count);
        if let Err(message) = check_volume(&self.state, &moved) {
            reply(context, message);
            return Ok(0);
        }

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        tokio::spawn(async move {
            let world = player.world();
            let positions = region.iterate();
            let states = read_states(&world, &positions);
            let delta = direction.offset() * count;

            let mut desired = Vec::with_capacity(positions.len() * 2);
            for (index, pos) in positions.iter().enumerate() {
                let target = BlockPos(Vector3::new(
                    pos.0.x + delta.x,
                    pos.0.y + delta.y,
                    pos.0.z + delta.z,
                ));
                desired.push((target, states[index]));
            }
            // Clear the parts of the original region the move vacated.
            for pos in &positions {
                if !moved.contains(pos) {
                    desired.push((*pos, BlockStateId::AIR));
                }
            }

            {
                let session = state.session(player.gameprofile.id);
                let mut guard = session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.selection = Some([moved.min(), moved.max()]);
            }
            let request = EditRequest::new(desired, format!("//move {count}"));
            execute(&state, &player, request).await;
        });
        reply(context, "Movendo seleção...".to_string());
        Ok(1)
    }
}

/// Which `DoTrO` macro to run.
#[derive(Clone, Copy)]
enum MacroKind {
    /// `//flip` + `//paste`.
    Flip { skip_air: bool },
    /// `//rotate 90` + `//paste`, repeated `copies` times.
    Rotate { copies: u8, skip_air: bool },
    /// `//copy` first, then the rotate+paste cycle.
    CopyRotate { copies: u8, skip_air: bool },
}

/// `DoTrO` macros: flip+paste and rotate+paste in one command.
struct MacroExecutor {
    state: Arc<WeState>,
    kind: MacroKind,
}

impl CommandExecutor for MacroExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        let kind = self.kind;
        tokio::spawn(run_macro(state, player, kind));
        reply(context, "Executando macro...".to_string());
        Ok(1)
    }
}

/// Runs a `DoTrO` macro end to end (guards stay inside blocks so the
/// future stays `Send` across the awaits).
async fn run_macro(state: Arc<WeState>, player: Arc<Player>, kind: MacroKind) {
    match kind {
        MacroKind::CopyRotate { copies, skip_air } => {
            let region = match selection_of(&state, player.gameprofile.id) {
                Ok(region) => region,
                Err(message) => {
                    crate::commands::tell(&player, message);
                    return;
                }
            };
            let world = player.world();
            let origin = player_block_pos(&player);
            let clipboard = Clipboard::from_world(&world, &region, origin);
            {
                let session = state.session(player.gameprofile.id);
                let mut guard = session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.clipboard = Some(Arc::new(clipboard));
            }
            run_rotate_paste(&state, &player, copies, skip_air).await;
        }
        MacroKind::Flip { skip_air } => {
            let axis = flip_axis_of_look(look_direction(&player));
            {
                let session = state.session(player.gameprofile.id);
                let mut guard = session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let flipped = guard
                    .clipboard
                    .as_ref()
                    .map(|cb| Arc::new(cb.flipped(axis)));
                if let Some(flipped) = flipped {
                    guard.clipboard = Some(flipped);
                } else {
                    crate::commands::tell(&player, "Nada copiado ainda (use //copy).".to_string());
                    return;
                }
            }
            match build_paste_request(&state, &player, skip_air, false) {
                Ok(request) => execute(&state, &player, request).await,
                Err(message) => crate::commands::tell(&player, message),
            }
        }
        MacroKind::Rotate { copies, skip_air } => {
            run_rotate_paste(&state, &player, copies, skip_air).await;
        }
    }
}

/// Runs `copies` rotate-90-then-paste cycles on the session clipboard.
async fn run_rotate_paste(state: &Arc<WeState>, player: &Arc<Player>, copies: u8, skip_air: bool) {
    for _ in 0..copies {
        {
            let session = state.session(player.gameprofile.id);
            let mut guard = session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let Some(clipboard) = guard.clipboard.clone() else {
                drop(guard);
                crate::commands::tell(player, "Nada copiado ainda (use //copy).".to_string());
                return;
            };
            guard.clipboard = Some(Arc::new(clipboard.rotated(1)));
        }
        match build_paste_request(state, player, skip_air, false) {
            Ok(request) => execute(state, player, request).await,
            Err(message) => {
                crate::commands::tell(player, message);
                return;
            }
        }
    }
}

/// Builds a paste edit request from the session clipboard.
fn build_paste_request(
    state: &Arc<WeState>,
    player: &Arc<Player>,
    skip_air: bool,
    at_origin: bool,
) -> Result<EditRequest, String> {
    let uuid = player.gameprofile.id;
    let clipboard = {
        let session = state.session(uuid);
        let guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.clipboard.clone()
    };
    let Some(clipboard) = clipboard else {
        return Err("Nada copiado ainda (use //copy).".to_string());
    };

    let max = state.with_config(|config| config.max_blocks);
    if clipboard.volume() as u64 > max {
        return Err(format!("Clipboard grande demais (limite {max})."));
    }

    let anchor = paste_anchor(player, &clipboard, at_origin);
    let origin = clipboard.origin();
    let mut desired = Vec::with_capacity(clipboard.volume());
    let mut block_entities = HashMap::new();
    for (pos, state_id) in clipboard.iter_blocks() {
        if skip_air && is_air(state_id) {
            continue;
        }
        let target = BlockPos(Vector3::new(
            anchor.0.x + (pos.0.x - origin.0.x),
            anchor.0.y + (pos.0.y - origin.0.y),
            anchor.0.z + (pos.0.z - origin.0.z),
        ));
        desired.push((target, state_id));
    }
    for (index, nbt) in clipboard.block_entities() {
        let Some(pos) = clipboard.position_of_index(*index) else {
            continue;
        };
        if skip_air && clipboard.block_at_index(*index).is_some_and(is_air) {
            continue;
        }
        let target = BlockPos(Vector3::new(
            anchor.0.x + (pos.0.x - origin.0.x),
            anchor.0.y + (pos.0.y - origin.0.y),
            anchor.0.z + (pos.0.z - origin.0.z),
        ));
        block_entities.insert(target, nbt.clone());
    }

    let mut request = EditRequest::new(desired, "//paste".to_string());
    request.block_entities = block_entities;
    Ok(request)
}

fn paste_anchor(player: &Arc<Player>, clipboard: &Clipboard, at_origin: bool) -> BlockPos {
    if at_origin {
        clipboard.origin()
    } else {
        player_block_pos(player)
    }
}

fn parse_paste_flags(flags: &str) -> (bool, bool, bool) {
    let mut skip_air = false;
    let mut at_origin = false;
    let mut select = false;
    for flag in flags.split_whitespace() {
        match flag {
            "-a" => skip_air = true,
            "-o" => at_origin = true,
            "-s" => select = true,
            _ => {}
        }
    }
    (skip_air, at_origin, select)
}

fn flip_axis(keyword: &str) -> Option<FlipAxis> {
    match Direction::parse(keyword)? {
        Direction::North | Direction::South => Some(FlipAxis::Z),
        Direction::East | Direction::West => Some(FlipAxis::X),
        Direction::Up | Direction::Down => None,
    }
}

const fn flip_axis_of_look(direction: Direction) -> FlipAxis {
    match direction {
        Direction::North | Direction::South => FlipAxis::Z,
        Direction::East | Direction::West | Direction::Up | Direction::Down => FlipAxis::X,
    }
}

const fn span_along(region: &Cuboid, direction: Direction) -> i32 {
    let size = region.size();
    match direction {
        Direction::North | Direction::South => size.z,
        Direction::East | Direction::West => size.x,
        Direction::Up | Direction::Down => size.y,
    }
}
