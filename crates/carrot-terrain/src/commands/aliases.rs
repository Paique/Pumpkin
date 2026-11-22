//! The builder shortcuts from the reference server's
//! `worldedit_abreviacoes.sk` (`DoTrO`), each mapped to its brush or
//! tool state; signatures match the `Skript` commands they replace.

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::CommandExecutor;

use crate::brush::Brush;
use crate::commands::{
    PERM_BRUSH, PERM_GOBRUSH, PERM_SNIPER, bind_brush, clamp_radius, number, parse_pair,
    parse_pattern, player_of, raw_args, reply, split_flags, tokens,
};
use crate::sniper::{ErodeParams, SniperBrush};
use crate::state::TerrainState;

const SENTINEL: (i32, i32) = (10_000, 10_000);

/// Which shortcut a command runs.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Alias {
    /// `/brb <pattern> <size> <smoothness> <height>` — boulder.
    Brb,
    /// `/brblob <pattern> <radius> <roundness> <frequency> <amplitude>`.
    Brblob,
    /// `/brshatter <radius> <count>` — stone cracks.
    Brshatter,
    /// `/abcube <pattern> [size] [snap] [height] [width]` + own flags.
    Abcube,
    /// `/abcubeh` — cube with `-h` forced.
    Abcubeh,
    /// `/abb <pattern> <size> <smoothness> <height> <width>`.
    Abb,
    /// `/abba` — boulder with `-a` forced.
    Abba,
    /// `/abac <pattern> <size> <snap>` — cube with `-a -c`.
    Abac,
    /// `/bang` — goBrush cliff preset, enabled.
    Bang,
    /// `/bmelt` — sniper erode melt.
    Bmelt,
    /// `/bbb` — sniper blend ball.
    Bbb,
    /// `/bbd` — sniper blend disc.
    Bbd,
    /// `/befill` — sniper erode-blend fill.
    Befill,
    /// `/brandom` — sniper ball with random radius 1..100.
    Brandom,
}

/// Registers every shortcut with the permission of the tool it drives.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    let entries: [(&str, Alias, &str); 14] = [
        ("brb", Alias::Brb, PERM_BRUSH),
        ("brblob", Alias::Brblob, PERM_BRUSH),
        ("brshatter", Alias::Brshatter, PERM_BRUSH),
        ("abcube", Alias::Abcube, PERM_BRUSH),
        ("abcubeh", Alias::Abcubeh, PERM_BRUSH),
        ("abb", Alias::Abb, PERM_BRUSH),
        ("abba", Alias::Abba, PERM_BRUSH),
        ("abac", Alias::Abac, PERM_BRUSH),
        ("bang", Alias::Bang, PERM_GOBRUSH),
        ("bmelt", Alias::Bmelt, PERM_SNIPER),
        ("bbb", Alias::Bbb, PERM_SNIPER),
        ("bbd", Alias::Bbd, PERM_SNIPER),
        ("befill", Alias::Befill, PERM_SNIPER),
        ("brandom", Alias::Brandom, PERM_SNIPER),
    ];
    for (name, alias, permission) in entries {
        let executor = AliasExecutor {
            state: Arc::clone(state),
            alias,
        };
        let builder = command(name, "Atalho de brush do servidor de referência.")
            .executes(executor.clone())
            .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor));
        context.register_command(name, builder, permission);
    }
}

#[derive(Clone)]
struct AliasExecutor {
    state: Arc<TerrainState>,
    alias: Alias,
}

#[allow(clippy::unreachable)] // exhaustive alias arms above make it dead
impl CommandExecutor for AliasExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, "Apenas jogadores.");
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        let all = tokens(&raw);
        let (args, mut flags) = split_flags(&all);

        // Sniper and goBrush shortcuts set tool state directly.
        match self.alias {
            Alias::Bang => {
                let player_state = self.state.player(player.gameprofile.id);
                let mut guard = player_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.gobrush.brush = "cliff.png".to_string();
                guard.gobrush.enabled = true;
                drop(guard);
                reply(context, "goBrush cliff.png ativado (flint).");
                return Ok(1);
            }
            Alias::Bmelt | Alias::Befill | Alias::Bbb | Alias::Bbd | Alias::Brandom => {
                let brush = match self.alias {
                    Alias::Bmelt => SniperBrush::Erode(ErodeParams::MELT),
                    Alias::Befill => SniperBrush::ErodeBlend(ErodeParams::FILL),
                    Alias::Bbb => SniperBrush::BlendBall,
                    Alias::Bbd => SniperBrush::BlendDisc,
                    Alias::Brandom => SniperBrush::RandomBall { min: 1, max: 100 },
                    // The guard above matched every other alias.
                    Alias::Bang
                    | Alias::Brb
                    | Alias::Brblob
                    | Alias::Brshatter
                    | Alias::Abcube
                    | Alias::Abcubeh
                    | Alias::Abb
                    | Alias::Abba
                    | Alias::Abac => unreachable!("alias already matched"),
                };
                let player_state = self.state.player(player.gameprofile.id);
                let mut guard = player_state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                guard.sniper.brush = brush;
                drop(guard);
                reply(
                    context,
                    format!(
                        "Sniper selecionado: {}",
                        match self.alias {
                            Alias::Bmelt => "erode melt",
                            Alias::Befill => "erode_blend fill",
                            Alias::Bbb => "blend_ball",
                            Alias::Bbd => "blend_disc",
                            _ => "random ball (1-100)",
                        }
                    ),
                );
                return Ok(1);
            }
            _ => {}
        }

        // Brush shortcuts bind a //br-style brush to the held item.
        let built: Result<Brush, String> = match self.alias {
            Alias::Brb | Alias::Abb | Alias::Abba => {
                if self.alias == Alias::Abba {
                    flags.insert('a');
                }
                let with_width = self.alias != Alias::Brb;
                build_boulder_alias(&player, &args, &flags, with_width)
            }
            Alias::Brblob => build_blob_alias(&self.state, &player, &args),
            Alias::Brshatter => build_shatter_alias(&self.state, &player, &args),
            Alias::Abcube | Alias::Abcubeh | Alias::Abac => {
                if self.alias == Alias::Abcubeh {
                    flags.insert('h');
                }
                if self.alias == Alias::Abac {
                    flags.insert('a');
                    flags.insert('c');
                }
                crate::commands::br::build_cube(&player, &args, &flags).map(|(brush, _)| brush)
            }
            // Non-brush aliases returned earlier.
            Alias::Bang
            | Alias::Bmelt
            | Alias::Befill
            | Alias::Bbb
            | Alias::Bbd
            | Alias::Brandom => unreachable!("non-brush aliases returned earlier"),
        };

        match built {
            Ok(brush) => {
                bind_brush(&self.state, &player, Some(brush.clone()));
                reply(
                    context,
                    format!("Brush \"{}\" vinculado ao item na mão.", brush.label()),
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

/// `/brb <pattern> <size> <smoothness> [height] [width]` (and `/abb`
/// variants, which also read a width pair).
fn build_boulder_alias(
    player: &pumpkin_core::entity::player::Player,
    args: &[String],
    flags: &std::collections::HashSet<char>,
    with_width: bool,
) -> Result<Brush, String> {
    let height = args
        .get(3)
        .map_or(Ok(SENTINEL), |value| parse_pair(value))?;
    let width = if with_width {
        args.get(4)
            .map_or(Ok(SENTINEL), |value| parse_pair(value))?
    } else {
        SENTINEL
    };
    Ok(Brush::Boulder {
        pattern: parse_pattern(player, args.first().map_or("stone", String::as_str))?,
        size: args.get(1).map_or(Ok((8, 8)), |value| parse_pair(value))?,
        smoothness: args
            .get(2)
            .map_or(Ok(3), |value| number::<i32>(value, "smoothness"))?,
        height,
        width,
        hollow: flags.contains(&'h'),
        falling: flags.contains(&'f'),
        angled: flags.contains(&'a'),
    })
}

/// `/brblob <pattern> <radius> <roundness> <frequency> <amplitude>`.
fn build_blob_alias(
    state: &Arc<TerrainState>,
    player: &pumpkin_core::entity::player::Player,
    args: &[String],
) -> Result<Brush, String> {
    if args.len() < 5 {
        return Err("/brblob <padrão> <raio> <roundness> <frequência> <amplitude>".to_string());
    }
    let (rx, ry, rz) = parse_vector(&args[1])?;
    let max = rx.max(ry).max(rz).max(f64::EPSILON);
    Ok(Brush::Blob {
        pattern: parse_pattern(player, &args[0])?,
        radius: clamp_radius(state, max as u32),
        radii: [rx / max, ry / max, rz / max],
        sphericity: number::<f64>(&args[2], "roundness")? / 100.0,
        frequency: number::<f64>(&args[3], "frequência")? / 100.0,
        amplitude: number::<f64>(&args[4], "amplitude")? / 100.0,
    })
}

/// `/brshatter <radius> <count>` (stone cracks).
fn build_shatter_alias(
    state: &Arc<TerrainState>,
    player: &pumpkin_core::entity::player::Player,
    args: &[String],
) -> Result<Brush, String> {
    Ok(Brush::Shatter {
        pattern: parse_pattern(player, "stone")?,
        radius: clamp_radius(
            state,
            args.first().map_or(Ok(10), |value| number(value, "raio"))?,
        ),
        count: args
            .get(1)
            .map_or(Ok(10), |value| number(value, "linhas"))?,
    })
}

/// Parses `N` or `N,N,N`.
fn parse_vector(token: &str) -> Result<(f64, f64, f64), String> {
    let parts: Vec<&str> = token.split(',').collect();
    let parse = |part: &str| -> Result<f64, String> {
        part.parse().map_err(|_| format!("raio inválido: {token}"))
    };
    match parts.len() {
        1 => {
            let value = parse(parts[0])?;
            Ok((value, value, value))
        }
        3 => Ok((parse(parts[0])?, parse(parts[1])?, parse(parts[2])?)),
        _ => Err(format!("raio inválido: {token}")),
    }
}
