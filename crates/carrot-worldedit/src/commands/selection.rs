//! Selection commands: wand, positions, info, expand/contract/shift and
//! farwand — including the pure `DoTrO` aliases.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::coordinates::block_pos::BlockPosArgumentType;
use pumpkin_core::command::argument_types::core::integer::IntegerArgumentType;
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_data::item::Item;
use pumpkin_data::item_stack::ItemStack;
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};

use crate::commands::{
    NO_SELECTION, ONLY_PLAYERS, check_volume, look_direction, player_block_pos, player_of, reply,
    selection_of,
};
use crate::region::{Cuboid, Direction};
use crate::session::WeState;

const PERM_WAND: &str = "carrot:worldedit.wand";
const PERM_POS: &str = "carrot:worldedit.selection.pos";
const PERM_INFO: &str = "carrot:worldedit.selection.sel";
const PERM_RESIZE: &str = "carrot:worldedit.selection.resize";
const PERM_FARWAND: &str = "carrot:worldedit.farwand";

const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers selection commands and permissions.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(PERM_WAND, "Allows getting the selection wand.", OP_TWO);
    context.register_permission(PERM_POS, "Allows setting selection corners.", OP_TWO);
    context.register_permission(PERM_INFO, "Allows inspecting the selection.", OP_TWO);
    context.register_permission(PERM_RESIZE, "Allows resizing the selection.", OP_TWO);
    context.register_permission(
        PERM_FARWAND,
        "Allows toggling long-range wand selection.",
        OP_TWO,
    );

    context.register_command(
        "//wand",
        command("//wand", "Gives the selection wand.").executes(WandExecutor {
            state: Arc::clone(state),
        }),
        PERM_WAND,
    );

    for (name, corner) in [
        ("//pos1", Corner::First),
        ("//pos2", Corner::Second),
        ("//1", Corner::First),
        ("//2", Corner::Second),
    ] {
        let executor = SetCornerExecutor {
            state: Arc::clone(state),
            corner,
        };
        let node = command(name, "Sets a selection corner.")
            .executes(executor.clone())
            .then(argument("pos", BlockPosArgumentType).executes(executor));
        context.register_command(name, node, PERM_POS);
    }

    context.register_command(
        "//sel",
        command("//sel", "Shows the current selection.").executes(SelectionInfoExecutor {
            state: Arc::clone(state),
        }),
        PERM_INFO,
    );

    for name in ["//expand", "//exp"] {
        register_resize(context, state, name, ResizeMode::Expand);
    }
    for name in ["//contract", "//co"] {
        register_resize(context, state, name, ResizeMode::Contract);
    }

    let shift = ShiftExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//shift",
        command("//shift", "Moves the selection.")
            .executes(shift.clone())
            .then(argument("amount", IntegerArgumentType::any()).executes(shift)),
        PERM_RESIZE,
    );

    context.register_command(
        "//vert",
        command("//vert", "Expands the selection vertically.").executes(VerticalExecutor {
            state: Arc::clone(state),
        }),
        PERM_RESIZE,
    );

    let farwand = FarwandExecutor {
        state: Arc::clone(state),
    };
    for name in ["//farwand", "//fw"] {
        context.register_command(
            name,
            command(name, "Toggles long-range wand selection.").executes(farwand.clone()),
            PERM_FARWAND,
        );
    }
}

fn register_resize(
    context: &mut ModuleContext<'_>,
    state: &Arc<WeState>,
    name: &str,
    mode: ResizeMode,
) {
    let executor = ResizeExecutor {
        state: Arc::clone(state),
        mode,
    };
    let node = command(name.to_string(), "Grows or shrinks the selection.")
        .executes(executor.clone())
        .then(
            argument("amount", IntegerArgumentType::any())
                .executes(executor.clone())
                .then(argument("direction", StringArgumentType::SingleWord).executes(executor)),
        );
    context.register_command(name, node, PERM_RESIZE);
}

/// Which corner a position command sets.
#[derive(Clone, Copy)]
enum Corner {
    First,
    Second,
}

/// `//wand`: hands out the configured wand item.
struct WandExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for WandExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let wand_item = self.state.with_config(|config| config.wand_item.clone());
        let Some(item) = Item::from_registry_key(&wand_item) else {
            reply(
                context,
                format!("Item de wand inválido na config: {wand_item}"),
            );
            return Ok(0);
        };
        let mut stack = ItemStack::new(1, item);
        player.inventory().insert_stack_anywhere(&mut stack);
        reply(
            context,
            format!("Wand entregue ({wand_item}). Esquerdo: pos1, direito: pos2."),
        );
        Ok(1)
    }
}

/// `//pos1` / `//pos2` / `//1` / `//2` with an optional position.
#[derive(Clone)]
struct SetCornerExecutor {
    state: Arc<WeState>,
    corner: Corner,
}

impl CommandExecutor for SetCornerExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let pos = BlockPosArgumentType::get_block_pos(context, "pos")
            .unwrap_or_else(|_| player_block_pos(&player));

        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = guard.selection.get_or_insert([pos; 2]);
        let slot = match self.corner {
            Corner::First => &mut entry[0],
            Corner::Second => &mut entry[1],
        };
        *slot = pos;
        drop(guard);

        let label = match self.corner {
            Corner::First => "Posição 1",
            Corner::Second => "Posição 2",
        };
        reply(
            context,
            format!("{label} definida em {} {} {}.", pos.0.x, pos.0.y, pos.0.z),
        );
        Ok(1)
    }
}

/// `//sel`: describes the current selection.
#[derive(Clone)]
struct SelectionInfoExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for SelectionInfoExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        match selection_of(&self.state, player.gameprofile.id) {
            Ok(region) => {
                let size = region.size();
                let volume = region.volume().unwrap_or_default();
                reply(
                    context,
                    format!(
                        "Seleção cuboide: {volume} blocos ({}x{}x{}).",
                        size.x, size.y, size.z
                    ),
                );
                Ok(1)
            }
            Err(message) => {
                reply(context, message);
                Ok(0)
            }
        }
    }
}

/// What `//expand`/`//contract` do.
#[derive(Clone, Copy)]
enum ResizeMode {
    Expand,
    Contract,
}

/// `//expand <amount> [direction]` / `//contract <amount> [direction]`,
/// defaulting amount to 1 and direction to where the player looks.
#[derive(Clone)]
struct ResizeExecutor {
    state: Arc<WeState>,
    mode: ResizeMode,
}

impl CommandExecutor for ResizeExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let amount = IntegerArgumentType::get(context, "amount").unwrap_or(1);
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

        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(selection) = guard.selection else {
            drop(guard);
            reply(context, NO_SELECTION);
            return Ok(0);
        };
        let region = Cuboid::new(selection[0], selection[1]);
        let resized = match self.mode {
            ResizeMode::Expand => region.expand(direction, amount),
            ResizeMode::Contract => region.contract(direction, amount),
        };
        if let Err(message) = check_volume(&self.state, &resized) {
            drop(guard);
            reply(context, message);
            return Ok(0);
        }
        guard.selection = Some([resized.min(), resized.max()]);
        drop(guard);

        let volume = resized.volume().unwrap_or_default();
        reply(context, format!("Seleção ajustada: {volume} blocos."));
        Ok(1)
    }
}

/// `//shift <amount>` (direction follows the player's look).
#[derive(Clone)]
struct ShiftExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for ShiftExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let amount = IntegerArgumentType::get(context, "amount").unwrap_or(1);
        let direction = look_direction(&player);
        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(selection) = guard.selection else {
            drop(guard);
            reply(context, NO_SELECTION);
            return Ok(0);
        };
        let region = Cuboid::new(selection[0], selection[1]).shift(direction, amount);
        guard.selection = Some([region.min(), region.max()]);
        drop(guard);
        reply(context, "Seleção movida.".to_string());
        Ok(1)
    }
}

/// `//vert`.
#[derive(Clone)]
struct VerticalExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for VerticalExecutor {
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
        let world = player.world();
        let vertical = region.expand_vertical(world.get_bottom_y(), world.get_top_y());
        if let Err(message) = check_volume(&self.state, &vertical) {
            reply(context, message);
            return Ok(0);
        }
        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.selection = Some([vertical.min(), vertical.max()]);
        drop(guard);
        let volume = vertical.volume().unwrap_or_default();
        reply(
            context,
            format!("Seleção expandida na vertical: {volume} blocos."),
        );
        Ok(1)
    }
}

/// `//farwand` / `//fw`.
#[derive(Clone)]
struct FarwandExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for FarwandExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let session = self.state.session(player.gameprofile.id);
        let mut guard = session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard.farwand = !guard.farwand;
        let enabled = guard.farwand;
        drop(guard);
        reply(
            context,
            if enabled {
                "Farwand ativado: a wand seleciona até onde você mira.".to_string()
            } else {
                "Farwand desativado.".to_string()
            },
        );
        Ok(1)
    }
}
