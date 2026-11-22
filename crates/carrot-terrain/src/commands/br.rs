//! `//br` (WorldEdit-style brushes), `//ab` (Arceon cube/boulder) and
//! `//terragen` (column noise terrain over the selection).

use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command, literal};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::CommandExecutor;

use crate::brush::Brush;
use crate::commands::{
    ONLY_PLAYERS, PERM_BRUSH, bind_brush, clamp_radius, number, parse_pair, parse_pattern,
    player_of, raw_args, reply, split_flags, tokens,
};
use crate::state::TerrainState;

/// Sentinel pair for "reuse size".
const SENTINEL: (i32, i32) = (10_000, 10_000);

/// Registers `//br`, `//ab`, `//terragen` and `//tgen`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    let mut root = command(
        "//br",
        "Brushes de terreno (esfera, cilindro, smooth, erode, blend, blob, shatter, boulder).",
    )
    .then(literal("none").executes(BrExecutor {
        state: Arc::clone(state),
        kind: "none",
    }));
    for kind in [
        "sphere",
        "cylinder",
        "smooth",
        "erode",
        "blendball",
        "blob",
        "shatter",
        "boulder",
    ] {
        let executor = BrExecutor {
            state: Arc::clone(state),
            kind,
        };
        root = root.then(
            literal(if kind == "blob" { "rock" } else { kind })
                .executes(executor.clone())
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
        );
    }
    // `blob` shares the rock parser.
    let executor = BrExecutor {
        state: Arc::clone(state),
        kind: "blob",
    };
    root = root.then(
        literal("blob")
            .executes(executor.clone())
            .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
    );
    context.register_command("//br", root, PERM_BRUSH);

    let cube = CubeExecutor {
        state: Arc::clone(state),
    };
    let boulder = BoulderExecutor {
        state: Arc::clone(state),
    };
    let ab = command("//ab", "Brushes Arceon (cube, boulder).")
        .then(
            literal("cube")
                .executes(cube.clone())
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(cube)),
        )
        .then(
            literal("boulder")
                .executes(boulder.clone())
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(boulder)),
        );
    context.register_command("//ab", ab, PERM_BRUSH);

    let executor = TerragenExecutor {
        state: Arc::clone(state),
    };
    for name in ["//terragen", "//tgen"] {
        let terragen = command(
            name,
            "Gera terreno por ruído na seleção (tipos: simplex, fractal, billowy, ridged, turbulence, electric, euclidean, manhattan, natural, poly).",
        )
        .executes(executor.clone())
        .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor.clone()));
        context.register_command(name, terragen, PERM_BRUSH);
    }
}

/// `//br <kind> [args]` — binds the brush to the held item.
#[derive(Clone)]
struct BrExecutor {
    state: Arc<TerrainState>,
    kind: &'static str,
}

impl CommandExecutor for BrExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        match parse_brush(&self.state, &player, self.kind, &raw) {
            Ok(Some(brush)) => {
                bind_brush(&self.state, &player, Some(brush.clone()));
                reply(
                    context,
                    format!("Brush \"{}\" vinculado ao item na mão.", brush.label()),
                );
                Ok(1)
            }
            Ok(None) => {
                bind_brush(&self.state, &player, None);
                reply(context, "Brush desvinculado.".to_string());
                Ok(1)
            }
            Err(message) => {
                reply(context, format!("{}: {message}", self.kind));
                Ok(0)
            }
        }
    }
}

/// Parses one `//br` brush from free-form args.
#[allow(clippy::too_many_lines)] // one arm per brush kind
pub(crate) fn parse_brush(
    state: &Arc<TerrainState>,
    player: &pumpkin_core::entity::player::Player,
    kind: &str,
    raw: &str,
) -> Result<Option<Brush>, String> {
    let all = tokens(raw);
    let (args, flags) = split_flags(&all);
    let brush = match kind {
        "help" => {
            return Err(
                "use //br sphere|cylinder|smooth|erode|blendball|rock|shatter|boulder|none"
                    .to_string(),
            );
        }
        "none" => return Ok(None),
        "sphere" => Brush::Sphere {
            pattern: parse_pattern(player, args.first().map_or("stone", String::as_str))?,
            radius: clamp_radius(
                state,
                args.get(1).map_or(Ok(2), |value| number(value, "raio"))?,
            ),
            hollow: flags.contains(&'h'),
        },
        "cylinder" => Brush::Cylinder {
            pattern: parse_pattern(player, args.first().map_or("stone", String::as_str))?,
            radius: clamp_radius(
                state,
                args.get(1).map_or(Ok(2), |value| number(value, "raio"))?,
            ),
            height: args.get(2).map_or(Ok(1), |value| number(value, "altura"))?,
            hollow: flags.contains(&'h'),
        },
        "smooth" => Brush::Smooth {
            radius: clamp_radius(
                state,
                args.first().map_or(Ok(2), |value| number(value, "raio"))?,
            ),
            iterations: args
                .get(1)
                .map_or(Ok(4), |value| number(value, "iterações"))?,
            mask: match args.get(2) {
                Some(mask) => Some(carrot_worldedit::mask::Mask::parse(
                    mask,
                    crate::commands::held_block(player),
                )?),
                None => None,
            },
        },
        "erode" => Brush::Erode {
            radius: clamp_radius(
                state,
                args.first().map_or(Ok(5), |value| number(value, "raio"))?,
            ),
            erode_faces: args
                .get(1)
                .map_or(Ok(2), |value| number(value, "erodeFaces"))?,
            erode_rec: args
                .get(2)
                .map_or(Ok(1), |value| number(value, "erodeRec"))?,
            fill_faces: args
                .get(3)
                .map_or(Ok(5), |value| number(value, "fillFaces"))?,
            fill_rec: args
                .get(4)
                .map_or(Ok(1), |value| number(value, "fillRec"))?,
        },
        "blendball" => Brush::BlendBall {
            radius: clamp_radius(
                state,
                args.first().map_or(Ok(5), |value| number(value, "raio"))?,
            ),
            min_freq_diff: args
                .get(1)
                .map_or(Ok(1), |value| number(value, "minFreqDiff"))?,
            only_air: flags.contains(&'a'),
        },
        "blob" => {
            let pattern = parse_pattern(player, args.first().map_or("stone", String::as_str))?;
            let radii_raw = args.get(1).map_or("10", String::as_str);
            let (rx, ry, rz) = parse_radius_vector(radii_raw)?;
            let max = rx.max(ry).max(rz).max(f64::EPSILON);
            Brush::Blob {
                pattern,
                radius: clamp_radius(state, max as u32),
                radii: [rx / max, ry / max, rz / max],
                sphericity: args
                    .get(2)
                    .map_or(Ok(100.0), |value| number::<f64>(value, "roundness"))?
                    / 100.0,
                frequency: args
                    .get(3)
                    .map_or(Ok(30.0), |value| number::<f64>(value, "frequência"))?
                    / 100.0,
                amplitude: args
                    .get(4)
                    .map_or(Ok(50.0), |value| number::<f64>(value, "amplitude"))?
                    / 100.0,
            }
        }
        "shatter" => Brush::Shatter {
            pattern: parse_pattern(player, args.first().map_or("stone", String::as_str))?,
            radius: clamp_radius(
                state,
                args.get(1).map_or(Ok(10), |value| number(value, "raio"))?,
            ),
            count: args
                .get(2)
                .map_or(Ok(10), |value| number(value, "linhas"))?,
        },
        "boulder" => parse_boulder(player, &args, &flags)?,
        other => return Err(format!("brush desconhecido: {other}")),
    };
    Ok(Some(brush))
}

/// `boulder <pattern> [size] [smoothness] [height] [width] -h -f -a`
/// (shared by `//br boulder` and `//ab boulder`).
pub(crate) fn parse_boulder(
    player: &pumpkin_core::entity::player::Player,
    args: &[String],
    flags: &std::collections::HashSet<char>,
) -> Result<Brush, String> {
    Ok(Brush::Boulder {
        pattern: parse_pattern(player, args.first().map_or("stone", String::as_str))?,
        size: args.get(1).map_or(Ok((8, 8)), |value| parse_pair(value))?,
        smoothness: args
            .get(2)
            .map_or(Ok(3), |value| number(value, "smoothness"))?,
        height: args
            .get(3)
            .map_or(Ok(SENTINEL), |value| parse_pair(value))?,
        width: args
            .get(4)
            .map_or(Ok(SENTINEL), |value| parse_pair(value))?,
        hollow: flags.contains(&'h'),
        falling: flags.contains(&'f'),
        angled: flags.contains(&'a'),
    })
}

/// Parses `10` or `10,5,8` radius vectors.
fn parse_radius_vector(token: &str) -> Result<(f64, f64, f64), String> {
    let parts: Vec<&str> = token.split(',').collect();
    match parts.len() {
        1 => {
            let value: f64 = parts[0]
                .parse()
                .map_err(|_| format!("raio inválido: {token}"))?;
            Ok((value, value, value))
        }
        3 => {
            let parse = |part: &str| -> Result<f64, String> {
                part.parse().map_err(|_| format!("raio inválido: {token}"))
            };
            Ok((parse(parts[0])?, parse(parts[1])?, parse(parts[2])?))
        }
        _ => Err(format!("raio inválido: {token} (use N ou N,N,N)")),
    }
}

/// `//ab cube [args]` — Arceon cube syntax.
#[derive(Clone)]
struct CubeExecutor {
    state: Arc<TerrainState>,
}

impl CommandExecutor for CubeExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        let all = tokens(&raw);
        let (args, flags) = split_flags(&all);
        match build_cube(&player, &args, &flags) {
            Ok((brush, warning)) => {
                if let Some(warning) = warning {
                    reply(context, warning);
                }
                bind_brush(&self.state, &player, Some(brush.clone()));
                reply(
                    context,
                    format!("Brush \"{}\" vinculado ao item na mão.", brush.label()),
                );
                Ok(1)
            }
            Err(message) => {
                reply(context, format!("cube: {message}"));
                Ok(0)
            }
        }
    }
}

/// Builds a cube brush (Arceon decrements the size pair by one and
/// demands snapAngle divide 360).
pub(crate) fn build_cube(
    player: &pumpkin_core::entity::player::Player,
    args: &[String],
    flags: &std::collections::HashSet<char>,
) -> Result<(Brush, Option<String>), String> {
    let size = args.get(1).map_or(Ok((8, 8)), |value| parse_pair(value))?;
    let requested_snap = args
        .get(2)
        .map_or(Ok(45), |value| number::<i32>(value, "snapAngle"))?;
    let (snap, warning) = if requested_snap <= 0 || 360 % requested_snap != 0 {
        (
            1,
            Some("snapAngle precisa ser divisor de 360; usando 1.".to_string()),
        )
    } else {
        (requested_snap, None)
    };
    let brush = Brush::Cube {
        pattern: parse_pattern(player, args.first().map_or("stone", String::as_str))?,
        size: (size.0 - 1, size.1 - 1),
        snap_angle: snap,
        height: args
            .get(3)
            .map_or(Ok(SENTINEL), |value| parse_pair(value))?,
        width: args
            .get(4)
            .map_or(Ok(SENTINEL), |value| parse_pair(value))?,
        hollow: flags.contains(&'h'),
        angled: flags.contains(&'a'),
        centered: flags.contains(&'c'),
    };
    Ok((brush, warning))
}

/// `//ab boulder [args]` — Arceon boulder syntax (full pairs).
#[derive(Clone)]
struct BoulderExecutor {
    state: Arc<TerrainState>,
}

impl CommandExecutor for BoulderExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        let all = tokens(&raw);
        let (args, flags) = split_flags(&all);
        match parse_boulder(&player, &args, &flags) {
            Ok(brush) => {
                bind_brush(&self.state, &player, Some(brush.clone()));
                reply(
                    context,
                    format!("Brush \"{}\" vinculado ao item na mão.", brush.label()),
                );
                Ok(1)
            }
            Err(message) => {
                reply(context, format!("boulder: {message}"));
                Ok(0)
            }
        }
    }
}

/// `//terragen [args]`.
#[derive(Clone)]
struct TerragenExecutor {
    state: Arc<TerrainState>,
}

impl CommandExecutor for TerragenExecutor {
    fn execute(
        &self,
        context: &CommandContext,
    ) -> pumpkin_core::command::node::CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = raw_args(context).unwrap_or_default();
        let all = tokens(&raw);
        let (args, flags) = split_flags(&all);

        let Some(noise_type) = args
            .first()
            .and_then(|token| crate::terragen::NoiseType::parse(token))
        else {
            reply(
                context,
                format!(
                    "Use //terragen <tipo> — tipos: {}",
                    crate::terragen::NoiseType::names()
                ),
            );
            return Ok(0);
        };
        let params = build_terragen(&player, &args, &flags, noise_type);
        let params = match params {
            Ok(params) => params,
            Err(message) => {
                reply(context, format!("terragen: {message}"));
                return Ok(0);
            }
        };
        reply(context, "Gerando terreno...".to_string());
        let state = Arc::clone(&self.state);
        let player = Arc::clone(&player);
        tokio::spawn(async move {
            crate::terragen::run(&state, &player, params).await;
        });
        Ok(1)
    }
}

/// Builds terragen parameters from free-form tokens.
fn build_terragen(
    player: &pumpkin_core::entity::player::Player,
    args: &[String],
    flags: &std::collections::HashSet<char>,
    noise_type: crate::terragen::NoiseType,
) -> Result<crate::terragen::TerragenParams, String> {
    Ok(crate::terragen::TerragenParams {
        noise_type,
        pattern: parse_pattern(player, args.get(1).map_or("stone", String::as_str))?,
        height: args
            .get(2)
            .map_or(Ok(20.0), |value| number(value, "altura"))?,
        zoom: args.get(3).map_or(Ok(1.0), |value| number(value, "zoom"))?,
        wave: args.get(4).map_or(Ok(0.0), |value| number(value, "wave"))?,
        add_mode: flags.contains(&'e'),
        extra_octaves: flags.contains(&'n'),
        cell_values: flags.contains(&'r'),
        snow: flags.contains(&'s'),
        vignette: flags.contains(&'v'),
    })
}
