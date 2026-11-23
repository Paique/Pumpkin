//! Region edit commands: `//set`, `//replace`, `//walls`, `//outline`.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};

use crate::commands::{
    NO_SELECTION, ONLY_PLAYERS, check_volume, clipboard_of, held_block, player_of, reply,
    selection_of, spawn_edit,
};
use crate::edit::EditRequest;
use crate::mask::Mask;
use crate::pattern::Pattern;
use crate::region::RegionKind;
use crate::session::WeState;

const PERM_REGION: &str = "carrot:worldedit.region";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers region edit commands and permissions.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(PERM_REGION, "Allows editing the selected region.", OP_TWO);

    // `//s` is the Builders-Utilities shorthand for `//set`.
    for name in ["//set", "//s"] {
        let set = SetExecutor {
            state: Arc::clone(state),
        };
        context.register_command(
            name,
            command(name, "Fills the selection with a pattern.")
                .then(argument("pattern", StringArgumentType::GreedyPhrase).executes(set)),
            PERM_REGION,
        );
    }

    // `//replace` takes an optional source mask before the pattern; the
    // aliases cover the shorthands the builders actually type.
    for name in ["//replace", "//rep", "//r", "//re"] {
        let executor = ReplaceExecutor {
            state: Arc::clone(state),
        };
        let node = command(name, "Replaces blocks in the selection.")
            .then(argument("to", StringArgumentType::GreedyPhrase).executes(executor.clone()))
            .then(
                argument("from", StringArgumentType::SingleWord)
                    .then(argument("to", StringArgumentType::GreedyPhrase).executes(executor)),
            );
        context.register_command(name, node, PERM_REGION);
    }

    for (name, kind) in [
        ("//walls", RegionKind::Walls),
        ("//outline", RegionKind::Outline),
    ] {
        let executor = ShapeExecutor {
            state: Arc::clone(state),
            kind,
        };
        context.register_command(
            name,
            command(name, "Builds the region shell with a pattern.")
                .then(argument("pattern", StringArgumentType::GreedyPhrase).executes(executor)),
            PERM_REGION,
        );
    }
}

/// `//set <pattern>`.
struct SetExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for SetExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "pattern")?;
        let hand = held_block(&player);
        let pattern = match Pattern::parse(raw, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return Ok(0);
            }
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

        let description = format!("//set {raw}");
        let clipboard = clipboard_of(&self.state, player.gameprofile.id);
        spawn_edit(
            &self.state,
            &player,
            EditRequest::new(
                pattern.apply_all(region.iterate(), clipboard.as_deref()),
                description,
            ),
        );
        reply(context, "Aplicando edição...".to_string());
        Ok(1)
    }
}

/// `//replace [from] <to>`.
#[derive(Clone)]
struct ReplaceExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for ReplaceExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let from_raw = StringArgumentType::get(context, "from").ok();
        let to_raw = StringArgumentType::get(context, "to")?;
        let hand = held_block(&player);

        let from_mask = match from_raw {
            Some(raw) => match Mask::parse(raw, hand) {
                Ok(mask) => mask,
                Err(message) => {
                    reply(context, format!("Máscara inválida: {message}"));
                    return Ok(0);
                }
            },
            // Without a source mask `//replace` keeps air untouched.
            None => Mask::NonAir,
        };
        let pattern = match Pattern::parse(to_raw, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return Ok(0);
            }
        };

        let Ok(region) = selection_of(&self.state, player.gameprofile.id) else {
            reply(context, NO_SELECTION);
            return Ok(0);
        };
        if let Err(message) = check_volume(&self.state, &region) {
            reply(context, message);
            return Ok(0);
        }

        let description = from_raw.map_or_else(
            || format!("//replace {to_raw}"),
            |raw| format!("//replace {raw} {to_raw}"),
        );
        let clipboard = clipboard_of(&self.state, player.gameprofile.id);
        let mut request = EditRequest::new(
            pattern.apply_all(region.iterate(), clipboard.as_deref()),
            description,
        );
        request.mask = Some(from_mask);
        spawn_edit(&self.state, &player, request);
        reply(context, "Aplicando edição...".to_string());
        Ok(1)
    }
}

/// `//walls <pattern>` / `//outline <pattern>`.
struct ShapeExecutor {
    state: Arc<WeState>,
    kind: RegionKind,
}

impl CommandExecutor for ShapeExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "pattern")?;
        let hand = held_block(&player);
        let pattern = match Pattern::parse(raw, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return Ok(0);
            }
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

        let positions = region.positions_of_kind(self.kind);
        let name = match self.kind {
            RegionKind::Walls => "walls",
            RegionKind::Outline => "outline",
            RegionKind::All => "set",
        };
        let description = format!("//{name} {raw}");
        let clipboard = clipboard_of(&self.state, player.gameprofile.id);
        spawn_edit(
            &self.state,
            &player,
            EditRequest::new(
                pattern.apply_all(positions, clipboard.as_deref()),
                description,
            ),
        );
        reply(context, "Aplicando edição...".to_string());
        Ok(1)
    }
}
