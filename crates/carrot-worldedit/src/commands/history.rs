//! History commands: `//undo` / `//redo` (plus the `//u` alias).

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::integer::IntegerArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};
use pumpkin_util::text::TextComponent;

use crate::commands::{ONLY_PLAYERS, player_of, reply};
use crate::edit::replay_entries;
use crate::session::WeState;

const PERM_HISTORY: &str = "carrot:worldedit.history";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers history commands and permissions.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(PERM_HISTORY, "Allows undoing and redoing edits.", OP_TWO);

    for (name, undo) in [("//undo", true), ("//redo", false), ("//u", true)] {
        let executor = HistoryExecutor {
            state: Arc::clone(state),
            undo,
        };
        let node = command(name, "Undoes or redoes edits.")
            .executes(executor.clone())
            .then(argument("times", IntegerArgumentType::any()).executes(executor));
        context.register_command(name, node, PERM_HISTORY);
    }
}

/// `//undo [times]` / `//redo [times]`.
#[derive(Clone)]
struct HistoryExecutor {
    state: Arc<WeState>,
    undo: bool,
}

impl CommandExecutor for HistoryExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let times = IntegerArgumentType::get(context, "times")
            .unwrap_or(1)
            .clamp(1, 100);

        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        let undo = self.undo;
        tokio::spawn(async move {
            let world = player.world();
            let session = state.session(player.gameprofile.id);
            let mut applied = 0usize;
            for _ in 0..times {
                let entries = {
                    let mut guard = session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if undo {
                        guard.history.pop_undo()
                    } else {
                        guard.history.pop_redo()
                    }
                };
                let Some(entries) = entries else {
                    break;
                };
                if entries.is_empty() {
                    continue;
                }
                replay_entries(&world, &entries, undo).await;
                applied += 1;
            }
            let label = if undo { "Desfeitas" } else { "Refeitas" };
            player.send_system_message(&TextComponent::text(format!(
                "{label} {applied} edição(ões)."
            )));
        });
        reply(
            context,
            if self.undo {
                "Desfazendo..."
            } else {
                "Refazendo..."
            }
            .to_string(),
        );
        Ok(1)
    }
}
