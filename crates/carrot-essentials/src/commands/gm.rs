//! `/gm`: EssentialsX-style gamemode alias (`c`, `s`, `a`, `sp` or numbers).

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, argument, command},
    argument_types::core::string::StringArgumentType,
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_util::permission::PermissionDefault;
use pumpkin_util::text::TextComponent;
use pumpkin_util::{GameMode, PermissionLvl};

use crate::commands::feedback;

const PERMISSION: &str = "carrot:essentials.gamemode";
const DESCRIPTION: &str = "Changes gamemode (alias: c/s/a/sp or 0-3).";

/// Registers `/gm <mode> [player]`.
pub fn register(context: &mut ModuleContext<'_>) {
    context.register_permission(
        PERMISSION,
        "Allows changing gamemodes.",
        PermissionDefault::Op(PermissionLvl::Two),
    );
    context.register_command(
        "gm",
        command("gm", DESCRIPTION).then(
            argument("mode", StringArgumentType::SingleWord)
                .executes(GmExecutor {
                    needs_target: false,
                })
                .then(
                    argument("target", StringArgumentType::SingleWord)
                        .executes(GmExecutor { needs_target: true }),
                ),
        ),
        PERMISSION,
    );
}

/// Maps an `EssentialsX` mode alias to a gamemode.
fn parse_gamemode(input: &str) -> Option<GameMode> {
    match input.to_ascii_lowercase().as_str() {
        "0" | "s" | "survival" => Some(GameMode::Survival),
        "1" | "c" | "creative" => Some(GameMode::Creative),
        "2" | "a" | "adventure" => Some(GameMode::Adventure),
        "3" | "sp" | "spectator" => Some(GameMode::Spectator),
        _ => None,
    }
}

const fn mode_name(gamemode: GameMode) -> &'static str {
    match gamemode {
        GameMode::Survival => "survival",
        GameMode::Creative => "creative",
        GameMode::Adventure => "adventure",
        GameMode::Spectator => "spectator",
    }
}

struct GmExecutor {
    needs_target: bool,
}

impl CommandExecutor for GmExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let mode_input = StringArgumentType::get(context, "mode")?;
        let Some(gamemode) = parse_gamemode(mode_input) else {
            feedback(
                context,
                "Modo inválido: use c, s, a, sp (ou 0-3).".to_string(),
            );
            return Ok(0);
        };

        let target = if self.needs_target {
            let name = StringArgumentType::get(context, "target")?;
            let Some(player) = context.source.server().get_player_by_name(name) else {
                feedback(context, format!("Jogador '{name}' não encontrado."));
                return Ok(0);
            };
            player
        } else {
            let Some(player) = context.source.output.as_player() else {
                feedback(
                    context,
                    "Apenas jogadores podem usar este comando.".to_string(),
                );
                return Ok(0);
            };
            player
        };

        let is_self = context
            .source
            .output
            .as_player()
            .is_some_and(|sender| sender.gameprofile.id == target.gameprofile.id);
        target.set_gamemode(gamemode);
        let mode = mode_name(gamemode);
        if is_self {
            feedback(
                context,
                format!("Seu modo de jogo foi alterado para {mode}."),
            );
        } else {
            target.send_system_message(&TextComponent::text(format!(
                "Seu modo de jogo foi alterado para {mode}."
            )));
            feedback(
                context,
                format!(
                    "Modo de jogo de {} alterado para {mode}.",
                    target.gameprofile.name
                ),
            );
        }
        Ok(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_aliases_and_numbers() {
        assert_eq!(parse_gamemode("c"), Some(GameMode::Creative));
        assert_eq!(parse_gamemode("SP"), Some(GameMode::Spectator));
        assert_eq!(parse_gamemode("0"), Some(GameMode::Survival));
        assert_eq!(parse_gamemode("adventure"), Some(GameMode::Adventure));
        assert_eq!(parse_gamemode("flight"), None);
    }
}
