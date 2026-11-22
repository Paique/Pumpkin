//! `/mv` (management) and `/goto` (teleport) commands.
//!
//! `/goto <world>` mirrors the reference server's `MultiWorld` teleport
//! command: it loads the world on demand and lands on the declared spawn.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::{
    argument_builder::{ArgumentBuilder, argument, command},
    argument_types::core::string::StringArgumentType,
    context::command_context::CommandContext,
    node::{CommandExecutor, CommandExecutorResult},
};
use pumpkin_core::entity::EntityBase;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::permission::PermissionDefault;
use pumpkin_util::text::TextComponent;

use crate::{
    config::GeneratorKind,
    state::{self, MultiverseState},
};

const ADMIN_PERMISSION: &str = "carrot:multiverse.admin";
const GOTO_PERMISSION: &str = "carrot:multiverse.goto";

/// Registers `/mv` and `/goto`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<MultiverseState>) {
    context.register_permission(
        ADMIN_PERMISSION,
        "Allows managing carrot multiverse worlds.",
        PermissionDefault::Op(pumpkin_util::PermissionLvl::Two),
    );
    context.register_permission(
        GOTO_PERMISSION,
        "Allows teleporting between worlds.",
        PermissionDefault::Allow,
    );
    context.register_command(
        "mv",
        command("mv", "Manages the multiverse worlds.")
            .executes(ListExecutor {
                state: Arc::clone(state),
            })
            .then(
                argument("action", StringArgumentType::SingleWord)
                    .executes(ActionExecutor {
                        state: Arc::clone(state),
                    })
                    .then(argument("world", StringArgumentType::SingleWord).executes(
                        WorldActionExecutor {
                            state: Arc::clone(state),
                        },
                    )),
            ),
        ADMIN_PERMISSION,
    );
    context.register_command(
        "goto",
        command("goto", "Teleports to a world, loading it when needed.").then(
            argument("world", StringArgumentType::SingleWord).executes(GotoExecutor {
                state: Arc::clone(state),
            }),
        ),
        GOTO_PERMISSION,
    );
}

fn feedback(context: &CommandContext, message: String) {
    context
        .source
        .send_feedback(TextComponent::text(message), false);
}

struct ListExecutor {
    state: Arc<MultiverseState>,
}

impl CommandExecutor for ListExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let server = context.source.server();
        let mut lines = Vec::new();
        self.state.with_config(|config| {
            for (name, def) in &config.worlds {
                let loaded = state::loaded_world(server, name).is_some();
                let generator = match def.generator {
                    GeneratorKind::Vanilla => "vanilla",
                    GeneratorKind::Void => "void",
                    GeneratorKind::Flat => "flat",
                };
                lines.push(format!(
                    "{name} [{generator}] {}{}",
                    if loaded { "carregado" } else { "descarregado" },
                    if def.auto_load {
                        ""
                    } else {
                        " (sem auto-load)"
                    },
                ));
            }
        });
        if lines.is_empty() {
            feedback(context, "Nenhum mundo declarado.".to_string());
            return Ok(0);
        }
        feedback(
            context,
            format!("Mundos ({}):\n{}", lines.len(), lines.join("\n")),
        );
        Ok(lines.len() as i32)
    }
}

struct ActionExecutor {
    state: Arc<MultiverseState>,
}

impl CommandExecutor for ActionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let action = StringArgumentType::get(context, "action")?;
        if action == "list" {
            ListExecutor {
                state: Arc::clone(&self.state),
            }
            .execute(context)
        } else {
            feedback(
                context,
                format!("Ação desconhecida '{action}'. Use list, info, load ou unload."),
            );
            Ok(0)
        }
    }
}

struct WorldActionExecutor {
    state: Arc<MultiverseState>,
}

impl CommandExecutor for WorldActionExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let action = StringArgumentType::get(context, "action")?;
        let name = StringArgumentType::get(context, "world")?;
        let server = context.source.server();

        match action {
            "info" => {
                let Some(def) = self.state.world_defs().remove(name) else {
                    feedback(context, format!("Mundo '{name}' não declarado."));
                    return Ok(0);
                };
                let loaded = state::loaded_world(server, name).is_some();
                feedback(
                    context,
                    format!(
                        "{name}: {}carregado, gerador {}, auto-load {}, spawn {}",
                        if loaded { "" } else { "não " },
                        match def.generator {
                            GeneratorKind::Vanilla => "vanilla",
                            GeneratorKind::Void => "void",
                            GeneratorKind::Flat => "flat",
                        },
                        def.auto_load,
                        if def.spawn.is_some() {
                            "definido"
                        } else {
                            "não definido"
                        },
                    ),
                );
                Ok(1)
            }
            "load" => {
                let Some(def) = self.state.world_defs().remove(name) else {
                    feedback(context, format!("Mundo '{name}' não declarado."));
                    return Ok(0);
                };
                self.state.ensure_world_loaded(server, name, &def);
                feedback(context, format!("Mundo '{name}' carregado."));
                Ok(1)
            }
            "unload" => {
                if !self.state.world_defs().contains_key(name) {
                    feedback(context, format!("Mundo '{name}' não declarado."));
                    return Ok(0);
                }
                feedback(context, format!("Descarregando '{name}'..."));
                let server = Arc::clone(server);
                let name = name.to_string();
                tokio::spawn(async move {
                    match server.unload_world(&name).await {
                        Ok(()) => tracing::info!("carrot-multiverse: world '{name}' unloaded"),
                        Err(err) => tracing::warn!("carrot-multiverse: {err}"),
                    }
                });
                Ok(1)
            }
            _ => {
                feedback(
                    context,
                    format!("Ação desconhecida '{action}'. Use list, info, load ou unload."),
                );
                Ok(0)
            }
        }
    }
}

struct GotoExecutor {
    state: Arc<MultiverseState>,
}

impl CommandExecutor for GotoExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = context.source.output.as_player() else {
            feedback(
                context,
                "Apenas jogadores podem usar este comando.".to_string(),
            );
            return Ok(0);
        };
        let name = StringArgumentType::get(context, "world")?;
        let server = context.source.server();
        let Some(def) = self.state.world_defs().remove(name) else {
            feedback(context, format!("Mundo '{name}' não declarado."));
            return Ok(0);
        };
        let world = self.state.ensure_world_loaded(server, name, &def);
        if world.get_world_name() != name {
            feedback(context, format!("Mundo '{name}' não pôde ser carregado."));
            return Ok(0);
        }

        let level = world.level_info.load();
        let (position, yaw, pitch) = match def.spawn {
            Some(spawn) => (
                Vector3::new(spawn.x, spawn.y, spawn.z),
                Some(spawn.yaw),
                Some(spawn.pitch),
            ),
            None => (
                Vector3::new(
                    f64::from(level.spawn_x) + 0.5,
                    f64::from(level.spawn_y),
                    f64::from(level.spawn_z) + 0.5,
                ),
                Some(level.spawn_yaw),
                Some(level.spawn_pitch),
            ),
        };
        drop(level);
        player.teleport(position, yaw, pitch, world);
        feedback(context, format!("Teleportado para '{name}'."));
        Ok(1)
    }
}
