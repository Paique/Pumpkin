//! Schematic commands: `//schem` / `//schematic` load/save/list/delete on
//! the shared `carrot/schematics` folder (Sponge `.schem`).

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_util::permission::{PermissionDefault, PermissionLvl};

use crate::commands::{ONLY_PLAYERS, player_of, reply};
use crate::schematic;
use crate::session::WeState;

const PERM_SCHEM: &str = "carrot:worldedit.schematic";
const OP_TWO: PermissionDefault = PermissionDefault::Op(PermissionLvl::Two);

/// Registers schematic commands and permissions.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    context.register_permission(PERM_SCHEM, "Allows managing schematics.", OP_TWO);

    for name in ["//schem", "//schematic"] {
        let node =
            command(name, "Schematic management.")
                .then(literal("save").then(
                    argument("name", StringArgumentType::SingleWord).executes(SaveExecutor {
                        state: Arc::clone(state),
                    }),
                ))
                .then(literal("load").then(
                    argument("name", StringArgumentType::SingleWord).executes(LoadExecutor {
                        state: Arc::clone(state),
                    }),
                ))
                .then(literal("list").executes(ListExecutor {
                    state: Arc::clone(state),
                }))
                .then(literal("delete").then(
                    argument("name", StringArgumentType::SingleWord).executes(DeleteExecutor {
                        state: Arc::clone(state),
                    }),
                ));
        context.register_command(name, node, PERM_SCHEM);
    }
}

/// `//schem save <name>`.
struct SaveExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for SaveExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "name")?;
        let Some(name) = sanitize_name(raw) else {
            reply(context, "Nome inválido.".to_string());
            return Ok(0);
        };

        let clipboard = {
            let session = self.state.session(player.gameprofile.id);
            let guard = session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.clipboard.clone()
        };
        let Some(clipboard) = clipboard else {
            reply(context, "Nada copiado ainda (use //copy).".to_string());
            return Ok(0);
        };

        let dir = self.state.schematics_dir();
        let path = dir.join(format!("{name}.schem"));
        match std::fs::create_dir_all(&dir)
            .map_err(|err| err.to_string())
            .and_then(|()| schematic::write_schematic(&clipboard))
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(|err| err.to_string()))
        {
            Ok(()) => {
                reply(
                    context,
                    format!(
                        "Salvo em {} ({}, MC {}).",
                        path.display(),
                        clipboard.volume(),
                        schematic::minecraft_version()
                    ),
                );
                Ok(1)
            }
            Err(message) => {
                reply(context, format!("Falha ao salvar: {message}"));
                Ok(0)
            }
        }
    }
}

/// `//schem load <name>`.
struct LoadExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for LoadExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "name")?;
        let Some(name) = sanitize_name(raw) else {
            reply(context, "Nome inválido.".to_string());
            return Ok(0);
        };

        let max = self.state.with_config(|config| config.max_blocks);
        let path = resolve_schematic(&self.state.schematics_dir(), &name);
        let Some(path) = path else {
            reply(context, format!("Schematic {name} não encontrada."));
            return Ok(0);
        };

        match std::fs::read(&path) {
            Ok(bytes) => match schematic::read_schematic(&bytes, max) {
                Ok(clipboard) => {
                    let origin = clipboard.origin();
                    let volume = clipboard.volume();
                    let session = self.state.session(player.gameprofile.id);
                    let mut guard = session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    guard.clipboard = Some(Arc::new(clipboard));
                    drop(guard);
                    reply(
                        context,
                        format!(
                            "{name} carregada: {volume} blocos (âncora {} {} {}).",
                            origin.0.x, origin.0.y, origin.0.z
                        ),
                    );
                    Ok(1)
                }
                Err(message) => {
                    reply(context, format!("Falha ao ler {name}: {message}"));
                    Ok(0)
                }
            },
            Err(err) => {
                reply(context, format!("Falha ao abrir {name}: {err}"));
                Ok(0)
            }
        }
    }
}

/// `//schem list`.
struct ListExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for ListExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let dir = self.state.schematics_dir();
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let path = entry.path();
                        let is_schem = path.extension().is_some_and(|ext| {
                            ext.eq_ignore_ascii_case("schem")
                                || ext.eq_ignore_ascii_case("schematic")
                        });
                        is_schem.then(|| {
                            path.file_stem().map_or_else(String::new, |stem| {
                                stem.to_string_lossy().into_owned()
                            })
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if names.is_empty() {
            reply(context, format!("Nenhuma schematic em {}.", dir.display()));
            return Ok(0);
        }
        names.sort();
        names.dedup();
        reply(
            context,
            format!("Schematics ({}): {}", names.len(), names.join(", ")),
        );
        Ok(1)
    }
}

/// `//schem delete <name>`.
struct DeleteExecutor {
    state: Arc<WeState>,
}

impl CommandExecutor for DeleteExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let raw = StringArgumentType::get(context, "name")?;
        let Some(name) = sanitize_name(raw) else {
            reply(context, "Nome inválido.".to_string());
            return Ok(0);
        };
        let Some(path) = resolve_schematic(&self.state.schematics_dir(), &name) else {
            reply(context, format!("Schematic {name} não encontrada."));
            return Ok(0);
        };
        match std::fs::remove_file(&path) {
            Ok(()) => {
                reply(context, format!("{name} apagada."));
                Ok(1)
            }
            Err(err) => {
                reply(context, format!("Falha ao apagar: {err}"));
                Ok(0)
            }
        }
    }
}

/// Accepts `[a-z0-9._-]+` names only (no traversal).
fn sanitize_name(raw: &str) -> Option<String> {
    let name = raw.to_ascii_lowercase();
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        && !name.starts_with('.')
        && !name.contains("..");
    valid.then_some(name)
}

/// Finds `name.schem` or `name.schematic` in the schematics folder.
fn resolve_schematic(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let schem = dir.join(format!("{name}.schem"));
    if schem.is_file() {
        return Some(schem);
    }
    let legacy = dir.join(format!("{name}.schematic"));
    legacy.is_file().then_some(legacy)
}
