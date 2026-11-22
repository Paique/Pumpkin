//! The `/carrot` admin command: module and group management.

use pumpkin_core::{
    command::argument_builder::{ArgumentBuilder, argument, command, literal},
    command::argument_types::core::string::StringArgumentType,
    command::context::command_context::CommandContext,
    command::node::{CommandExecutor, CommandExecutorResult},
    plugin::Context,
};
use pumpkin_util::text::TextComponent;

const CARROT_ADMIN: &str = "carrot:admin";
const DESCRIPTION: &str = "Manage carrot server modules and groups.";

/// Registers the `/carrot` command on the root context.
///
/// The command is infrastructure (not a module): it stays available even when
/// every module is disabled, so a disabled module can always be re-enabled.
pub(crate) fn register_carrot_command(context: &Context) {
    context.register_command(
        command("carrot", DESCRIPTION)
            .then(
                literal("module")
                    .then(literal("list").executes(ModuleListExecutor))
                    .then(
                        literal("enable").then(
                            argument("id", StringArgumentType::SingleWord)
                                .executes(ModuleEnableExecutor),
                        ),
                    )
                    .then(
                        literal("disable").then(
                            argument("id", StringArgumentType::SingleWord)
                                .executes(ModuleDisableExecutor),
                        ),
                    ),
            )
            .then(literal("groups").then(literal("reload").executes(GroupsReloadExecutor))),
        CARROT_ADMIN,
    );
}

fn feedback(context: &CommandContext, message: String) {
    context
        .source
        .send_feedback(TextComponent::text(message), false);
}

struct ModuleListExecutor;

impl CommandExecutor for ModuleListExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(carrot) = crate::global() else {
            return Ok(0);
        };
        let states = carrot.module_states();
        if states.is_empty() {
            feedback(context, "No carrot modules registered.".to_string());
            return Ok(0);
        }
        let count = states.len() as i32;
        let lines: Vec<String> = states
            .iter()
            .map(|state| {
                format!(
                    " - {}: {}",
                    state.id,
                    if state.enabled { "enabled" } else { "disabled" }
                )
            })
            .collect();
        feedback(context, format!("Carrot modules:\n{}", lines.join("\n")));
        Ok(count)
    }
}

struct ModuleEnableExecutor;

impl CommandExecutor for ModuleEnableExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(carrot) = crate::global() else {
            return Ok(0);
        };
        let id = StringArgumentType::get(context, "id")?;
        match carrot.enable_module(id) {
            Ok(()) => {
                feedback(context, format!("Module '{id}' enabled."));
                Ok(1)
            }
            Err(err) => {
                feedback(context, format!("Failed to enable module '{id}': {err}"));
                Ok(0)
            }
        }
    }
}

struct ModuleDisableExecutor;

impl CommandExecutor for ModuleDisableExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(carrot) = crate::global() else {
            return Ok(0);
        };
        let id = StringArgumentType::get(context, "id")?;
        match carrot.disable_module(id) {
            Ok(()) => {
                feedback(context, format!("Module '{id}' disabled."));
                Ok(1)
            }
            Err(err) => {
                feedback(context, format!("Failed to disable module '{id}': {err}"));
                Ok(0)
            }
        }
    }
}

struct GroupsReloadExecutor;

impl CommandExecutor for GroupsReloadExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(carrot) = crate::global() else {
            return Ok(0);
        };
        match carrot.reload_groups() {
            Ok(applied) => {
                feedback(
                    context,
                    format!("Groups reloaded; re-applied to {applied} online player(s)."),
                );
                Ok(1)
            }
            Err(err) => {
                feedback(context, format!("Failed to reload groups: {err}"));
                Ok(0)
            }
        }
    }
}
