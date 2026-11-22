//! `//loft`: multi-frame bezier lofts (Arceon port).
//!
//! Builders mark frames and points on plain paper (left click = new
//! frame, right click = point) and generate the swept surface with
//! `//loft set <pattern> [count] [-p -o -c -d]`, including the slab
//! top/bottom/double detail pass.

use std::collections::HashMap;
use std::sync::Arc;

use carrot_core::ModuleContext;
use carrot_worldedit::edit::{EditRequest, ensure_chunks, execute};
use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::states::is_air;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::{Axis, Vector3};
use pumpkin_util::text::TextComponent;
use uuid::Uuid;

use crate::commands::{ONLY_PLAYERS, PERM_BRUSH, held_block, player_of, reply};
use crate::state::TerrainState;

/// A loft in progress: one or more frames of marked points.
#[derive(Default)]
pub struct LoftSelection {
    frames: Vec<Vec<Vector3<f64>>>,
}

impl LoftSelection {
    /// The last frame, creating an empty one when needed.
    fn last_frame_mut(&mut self) -> &mut Vec<Vector3<f64>> {
        if self.frames.is_empty() {
            self.frames.push(Vec::new());
        }
        let last = self.frames.len() - 1;
        &mut self.frames[last]
    }

    /// Adds a point to the last frame; `false` when the exact position
    /// is already the frame's last point.
    fn add_point(&mut self, point: Vector3<f64>) -> bool {
        let frame = self.last_frame_mut();
        if frame.last().is_some_and(|last| *last == point) {
            return false;
        }
        frame.push(point);
        true
    }

    /// Starts a new frame.
    fn new_frame(&mut self) {
        self.frames.push(Vec::new());
    }

    /// Removes the last point; drops the frame when it empties (unless
    /// it is the only one).
    fn remove_last(&mut self) -> bool {
        while let Some(frame) = self.frames.last_mut() {
            frame.pop();
            if !frame.is_empty() {
                return true;
            }
            if self.frames.len() == 1 {
                return true;
            }
            self.frames.pop();
        }
        false
    }
}

/// The per-player loft state of the module.
#[derive(Default)]
pub struct LoftState {
    /// Public to the module's own state reset on disable.
    pub selections: HashMap<Uuid, LoftSelection>,
}

impl LoftState {
    /// The player's selection, created on first use.
    pub fn selection(&mut self, uuid: Uuid) -> &mut LoftSelection {
        self.selections.entry(uuid).or_default()
    }
}

/// Adds a point from a paper right-click (`point` at the clicked block
/// or the player's feet).
pub fn paper_point(state: &TerrainState, player: &Arc<Player>, clicked: Option<BlockPos>) {
    let point = clicked.map_or_else(
        || {
            let position = player.position();
            Vector3::new(position.x, position.y, position.z)
        },
        |block| {
            Vector3::new(
                f64::from(block.0.x) + 0.5,
                f64::from(block.0.y) + 0.5,
                f64::from(block.0.z) + 0.5,
            )
        },
    );
    let mut lofts = state
        .lofts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let selection = lofts.selection(player.gameprofile.id);
    if !selection.add_point(point) {
        drop(lofts);
        player.send_system_message(&TextComponent::text(
            "Loft> não é possível adicionar a mesma posição".to_string(),
        ));
        return;
    }
    let frames = selection.frames.len();
    let points = selection.frames.last().map_or(0, Vec::len);
    drop(lofts);
    player.send_system_message(&TextComponent::text(format!(
        "Loft> ponto #{points} no frame #{frames} em {} {} {}",
        point.x.floor() as i32,
        point.y.floor() as i32,
        point.z.floor() as i32
    )));
}

/// Starts a new frame from a paper left-click.
pub fn paper_frame(state: &TerrainState, player: &Arc<Player>, clicked: Option<BlockPos>) {
    let point = clicked.map_or_else(
        || {
            let position = player.position();
            Vector3::new(position.x, position.y, position.z)
        },
        |block| {
            Vector3::new(
                f64::from(block.0.x) + 0.5,
                f64::from(block.0.y) + 0.5,
                f64::from(block.0.z) + 0.5,
            )
        },
    );
    let mut lofts = state
        .lofts
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let selection = lofts.selection(player.gameprofile.id);
    selection.new_frame();
    selection.add_point(point);
    let frames = selection.frames.len();
    drop(lofts);
    player.send_system_message(&TextComponent::text(format!(
        "Loft> frame #{frames} iniciado em {} {} {}",
        point.x.floor() as i32,
        point.y.floor() as i32,
        point.z.floor() as i32
    )));
}

/// Registers `//loft`.
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<TerrainState>) {
    let executor = LoftExecutor {
        state: Arc::clone(state),
    };
    context.register_command(
        "//loft",
        command("//loft", "Multi-frame bezier lofts.")
            .executes(executor.clone())
            .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
        PERM_BRUSH,
    );
}

/// `//loft set|frame|point|clear|remove ...`.
#[derive(Clone)]
struct LoftExecutor {
    state: Arc<TerrainState>,
}

impl CommandExecutor for LoftExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args").unwrap_or_default();
        let mut tokens = raw.split_whitespace();
        let rest = tokens.next().map_or(String::new(), |head| {
            raw[head.len()..].trim_start().to_string()
        });
        match raw.split_whitespace().next().unwrap_or_default() {
            "set" | "s" => {
                self.set(context, &player, &rest);
                Ok(1)
            }
            "frame" | "f" => {
                paper_frame(&self.state, &player, None);
                Ok(1)
            }
            "point" | "pos" | "p" => {
                paper_point(&self.state, &player, None);
                Ok(1)
            }
            "clear" | "c" => {
                let mut lofts = self
                    .state
                    .lofts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                lofts.selection(player.gameprofile.id).frames.clear();
                drop(lofts);
                reply(context, "Loft> frames limpos.".to_string());
                Ok(1)
            }
            "remove" | "r" => {
                let mut lofts = self
                    .state
                    .lofts
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let removed = lofts.selection(player.gameprofile.id).remove_last();
                drop(lofts);
                reply(
                    context,
                    if removed {
                        "Loft> último ponto removido.".to_string()
                    } else {
                        "Loft> sem pontos para remover.".to_string()
                    },
                );
                Ok(1)
            }
            _ => {
                reply(
                    context,
                    "use: //loft set <padrão> [quantidade] [-p] [-o] [-c] [-d] · frame · point · clear · remove"
                        .to_string(),
                );
                Ok(0)
            }
        }
    }
}

impl LoftExecutor {
    /// `//loft set <pattern> [count] [-p poly] [-o outline] [-c connect] [-d stackdown]`.
    fn set(&self, context: &CommandContext, player: &Arc<Player>, args: &str) {
        let mut tokens = args.split_whitespace();
        let Some(pattern_token) = tokens.next() else {
            reply(
                context,
                "use: //loft set <padrão> [quantidade] [-p] [-o] [-c] [-d]".to_string(),
            );
            return;
        };
        let hand = held_block(player);
        let pattern = match Pattern::parse(pattern_token, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return;
            }
        };
        let mut count: i32 = -1;
        let mut poly = false;
        let mut outline = false;
        let mut connect = false;
        let mut stackdown = false;
        for token in tokens {
            match token {
                "-p" => poly = true,
                "-o" => outline = true,
                "-c" => connect = true,
                "-d" => stackdown = true,
                other => {
                    let Ok(value) = other.parse::<i32>() else {
                        reply(context, format!("argumento inválido: {other}"));
                        return;
                    };
                    count = value;
                }
            }
        }

        let selection = {
            let mut lofts = self
                .state
                .lofts
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            lofts.selection(player.gameprofile.id).frames.clone()
        };
        if selection.len() <= 1 {
            reply(
                context,
                "Loft> são necessários pelo menos 2 frames.".to_string(),
            );
            return;
        }

        let state = Arc::clone(&self.state);
        let player = Arc::clone(player);
        tokio::spawn(async move {
            place(
                &state, &player, &selection, &pattern, count, poly, outline, connect, stackdown,
            )
            .await;
        });
        reply(context, "Gerando loft...".to_string());
    }
}

/// A cubic bezier composite through knots, the Arceon `BezierSpline`
/// with the classic tridiagonal control-point solve.
struct BezierSpline {
    /// `[p0, p1, p2, p3]` control points per segment.
    segments: Vec<[Vector3<f64>; 4]>,
    length: f64,
}

impl BezierSpline {
    fn new(knots: &[Vector3<f64>]) -> Self {
        let mut segments: Vec<[Vector3<f64>; 4]> = knots
            .windows(2)
            .map(|pair| [pair[0], Vector3::default(), Vector3::default(), pair[1]])
            .collect();
        if !segments.is_empty() {
            Self::solve_control_points(&mut segments, knots);
        }
        let length = segments.iter().map(segment_length).sum();
        Self { segments, length }
    }

    /// The Thomas-algorithm solve from `BezierSpline#calculateControlPoints`.
    fn solve_control_points(segments: &mut [[Vector3<f64>; 4]], knots: &[Vector3<f64>]) {
        let n = knots.len() - 1;
        if n == 1 {
            let midpoint = (segments[0][0] + segments[0][3]) * 0.5;
            segments[0][1] = midpoint;
            segments[0][2] = midpoint;
            return;
        }
        let mut b = vec![0.0f64; n];
        let mut r = vec![[0.0f64; 3]; n];
        b[0] = 2.0;
        r[0] = axis_values(knots[0] + knots[1] * 2.0);
        for i in 1..n - 1 {
            b[i] = 4.0;
            r[i] = axis_values(knots[i] * 4.0 + knots[i + 1] * 2.0);
        }
        b[n - 1] = 7.0;
        r[n - 1] = axis_values(knots[n - 1] * 8.0 + knots[n]);

        // Elimination: `a` is 1 on middle rows and 2 on the last, `c`
        // is 1 everywhere (zero on the last, never read).
        for i in 1..n {
            let a = if i == n - 1 { 2.0 } else { 1.0 };
            let m = a / b[i - 1];
            b[i] -= m;
            for axis_index in 0..3 {
                r[i][axis_index] -= m * r[i - 1][axis_index];
            }
        }

        let mut p1 = vec![[0.0f64; 3]; n];
        for axis_index in 0..3 {
            p1[n - 1][axis_index] = r[n - 1][axis_index] / b[n - 1];
        }
        for i in (0..n - 1).rev() {
            for axis_index in 0..3 {
                p1[i][axis_index] = (r[i][axis_index] - p1[i + 1][axis_index]) / b[i];
            }
        }
        for i in 0..n {
            segments[i][1] = Vector3::new(p1[i][0], p1[i][1], p1[i][2]);
        }
        for i in 0..n - 1 {
            segments[i][2] = knots[i + 1] * 2.0 - segments[i + 1][1];
        }
        segments[n - 1][2] = (knots[n] + segments[n - 1][1]) * 0.5;

        // Arceon keeps fully-flat axes exactly flat through the solve.
        for axis in [Axis::X, Axis::Y, Axis::Z] {
            let flat = knots[0].get_axis(axis);
            if knots.iter().all(|knot| knot.get_axis(axis) == flat) {
                for segment in segments.iter_mut() {
                    for control in segment.iter_mut() {
                        control.set_axis(axis, flat);
                    }
                }
            }
        }
    }

    /// Point at parameter `f` in segment units (`0..=segments.len()`).
    fn point(&self, f: f64) -> Vector3<f64> {
        if self.segments.is_empty() {
            return Vector3::default();
        }
        if f >= self.segments.len() as f64 {
            let last = self.segments.len() - 1;
            return segment_point(&self.segments[last], 1.0);
        }
        let index = f.floor() as usize;
        segment_point(&self.segments[index], f - f.floor())
    }
}

/// The three axis components of a vector.
const fn axis_values(value: Vector3<f64>) -> [f64; 3] {
    [value.x, value.y, value.z]
}

/// Evaluates one cubic bezier segment at `t`.
fn segment_point(segment: &[Vector3<f64>; 4], t: f64) -> Vector3<f64> {
    let u = 1.0 - t;
    segment[0] * u.powi(3)
        + segment[1] * (3.0 * u * u * t)
        + segment[2] * (3.0 * u * t * t)
        + segment[3] * t.powi(3)
}

/// Arceon segment length: 20 samples at 0.05 step.
fn segment_length(segment: &[Vector3<f64>; 4]) -> f64 {
    let mut length = 0.0;
    let mut current = segment[0];
    for step in 1..20 {
        let next = segment_point(segment, f64::from(step) * 0.05);
        length += (next - current).length();
        current = next;
    }
    length
}

/// A polyline spline (Arceon `LowPolySpline`).
struct LowPolySpline {
    points: Vec<Vector3<f64>>,
    length: f64,
}

impl LowPolySpline {
    fn new(points: &[Vector3<f64>]) -> Self {
        let length = points
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).length())
            .sum();
        Self {
            points: points.to_vec(),
            length,
        }
    }

    /// Point at parameter `t` in segment units; clamps at the ends.
    fn point(&self, t: f64) -> Vector3<f64> {
        if t < 0.0 {
            return self.points[0];
        }
        let last = self.points.len() - 1;
        if t >= last as f64 {
            return self.points[last];
        }
        let index = t.floor() as usize;
        let rest = t % 1.0;
        self.points[index] + (self.points[index + 1] - self.points[index]) * rest
    }
}

/// Resamples a frame to exactly `amount` points along its curve
/// (Arceon `Math3D.calcPoints`).
fn calc_points(points: &[Vector3<f64>], amount: usize, smooth: bool) -> Vec<Vector3<f64>> {
    if amount == 0 || amount == points.len() || points.len() < 2 {
        return points.to_vec();
    }
    if points.len() == 1 {
        return vec![points[0]; amount.max(1)];
    }
    let scale = f64::from(u32::try_from(amount - 1).unwrap_or(1));
    if smooth {
        let spline = BezierSpline::new(points);
        (0..amount)
            .map(|step| spline.point(spline.segments.len() as f64 * step as f64 / scale))
            .collect()
    } else {
        let spline = LowPolySpline::new(points);
        let segments = (spline.points.len() - 1) as f64;
        (0..amount)
            .map(|step| spline.point(segments * step as f64 / scale))
            .collect()
    }
}

/// Half-slab detail kinds for the `locations` pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlabPosition {
    Top,
    Bottom,
    Full,
}

/// Generates the loft surface, the `LoftSelection#place` port.
// Literal port of Arceon's place(); the flag set mirrors the command.
#[allow(
    clippy::too_many_lines,
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments
)]
async fn place(
    state: &Arc<TerrainState>,
    player: &Arc<Player>,
    frames: &[Vec<Vector3<f64>>],
    pattern: &Pattern,
    count: i32,
    poly: bool,
    outline: bool,
    connect: bool,
    stackdown: bool,
) {
    let world = player.world();
    if let Some((min, max)) = frames_bounds(frames) {
        let corners = vec![
            BlockPos::new(
                min.x.floor() as i32,
                min.y.floor() as i32 - 1,
                min.z.floor() as i32,
            ),
            BlockPos::new(
                max.x.floor() as i32,
                max.y.floor() as i32 + 1,
                max.z.floor() as i32,
            ),
        ];
        ensure_chunks(&world, &corners, |pos: &BlockPos| pos).await;
    }

    // Slab detail: single-block slab patterns get the top/bottom/double
    // treatment (Arceon checks the pattern for "slab" in the name).
    let slab = match pattern {
        Pattern::Single(slab) if slab.to_block().name.contains("slab") => Some(*slab),
        _ => None,
    };
    let detailed = slab.is_some();
    let slab_variant = |kind: &str| -> Option<BlockStateId> {
        slab.and_then(|base| {
            let block = base.to_block();
            block
                .parse_property("type", kind)
                .map(|property| block.apply_properties(base, &[property]))
                .ok()
        })
    };
    let bottom_block = slab_variant("bottom");
    let top_block = slab_variant("top");
    let full_block = slab_variant("double");

    let max_points = frames.iter().map(Vec::len).max().unwrap_or(0).max(2);
    let frame_count = frames.len();
    let mut placed: Vec<(BlockPos, BlockStateId)> = Vec::new();
    let mut locations: HashMap<BlockPos, SlabPosition> = HashMap::new();

    if poly {
        place_low_poly(
            frames,
            pattern,
            detailed,
            count,
            connect,
            outline,
            max_points,
            frame_count,
            &mut placed,
            &mut locations,
        );
    } else {
        place_bezier(
            frames,
            pattern,
            detailed,
            count,
            connect,
            outline,
            max_points,
            frame_count,
            &mut placed,
            &mut locations,
        );
    }

    emit_loft(
        state,
        player,
        &world,
        &mut placed,
        &locations,
        bottom_block,
        top_block,
        full_block,
        stackdown,
        pattern,
    )
    .await;
}

/// The smooth bezier surface pass (Arceon `place` non-poly branch).
#[allow(
    clippy::too_many_lines,
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments
)]
fn place_bezier(
    frames: &[Vec<Vector3<f64>>],
    pattern: &Pattern,
    detailed: bool,
    count: i32,
    connect: bool,
    outline: bool,
    max_points: usize,
    frame_count: usize,
    placed: &mut Vec<(BlockPos, BlockStateId)>,
    locations: &mut HashMap<BlockPos, SlabPosition>,
) {
    // Frames resampled to the same point count (bezier smoothing), with
    // the connect ring duplicating the first frames at the end.
    let mut connect_frames: Vec<Vec<Vector3<f64>>> = Vec::new();
    if connect && frame_count > 2 {
        connect_frames.push(calc_points(&frames[frame_count - 1], max_points, true));
    }
    for frame in frames {
        connect_frames.push(calc_points(frame, max_points, true));
    }
    if connect && frame_count > 2 {
        connect_frames.push(calc_points(&frames[0], max_points, true));
        connect_frames.push(calc_points(&frames[1], max_points, true));
    }

    // Longitudinal splines: one per point index across the frames.
    let mut splines = Vec::new();
    let mut max_size = f64::MIN;
    for index in 0..max_points {
        let curve: Vec<Vector3<f64>> = connect_frames
            .iter()
            .map(|frame| frame[index.min(frame.len() - 1)])
            .collect();
        let spline = BezierSpline::new(&curve);
        max_size = max_size.max(spline.length);
        splines.push(spline);
    }

    let (stations, outline_only) = station_count(count, max_size);

    if outline {
        // Outline = the two edge splines plus cross curves through the
        // spline start and end knots.
        let outline_curves: Vec<Vec<Vector3<f64>>> = vec![
            splines.iter().map(|s| s.point(0.0)).collect(),
            splines
                .iter()
                .map(|s| s.point(frame_count as f64 - 1.0))
                .collect(),
        ];
        for curve in &outline_curves {
            if curve.len() < 2 {
                continue;
            }
            let spline = BezierSpline::new(curve);
            let samples = (spline.length * 3.0 + 1.0) as i32;
            for step in 0..samples {
                sample_loft(
                    pattern,
                    detailed,
                    placed,
                    locations,
                    spline
                        .point(f64::from(step) / f64::from(samples) * spline.segments.len() as f64),
                );
            }
        }
        for index in [0, splines.len() - 1] {
            let spline = &splines[index];
            let samples = (spline.length * 3.0 + 1.0) as i32;
            for step in 0..samples {
                sample_loft(
                    pattern,
                    detailed,
                    placed,
                    locations,
                    spline
                        .point(f64::from(step) / f64::from(samples) * spline.segments.len() as f64),
                );
            }
        }
    }

    if outline_only && outline {
        return;
    }
    for station in 0..=stations {
        let t = f64::from(station) / f64::from(stations) * connect_span(connect, frame_count);
        let curve: Vec<Vector3<f64>> = splines.iter().map(|s| s.point(t)).collect();
        if curve.len() < 2 {
            continue;
        }
        let cross = BezierSpline::new(&curve);
        let samples = (cross.length * 3.0 + 1.0) as i32;
        for step in 0..samples {
            sample_loft(
                pattern,
                detailed,
                placed,
                locations,
                cross.point(f64::from(step) / f64::from(samples) * cross.segments.len() as f64),
            );
        }
    }
}

/// The low-poly linear surface pass (Arceon `place` `-p` branch).
#[allow(
    clippy::too_many_lines,
    clippy::fn_params_excessive_bools,
    clippy::too_many_arguments
)]
fn place_low_poly(
    frames: &[Vec<Vector3<f64>>],
    pattern: &Pattern,
    detailed: bool,
    count: i32,
    connect: bool,
    outline: bool,
    max_points: usize,
    frame_count: usize,
    placed: &mut Vec<(BlockPos, BlockStateId)>,
    locations: &mut HashMap<BlockPos, SlabPosition>,
) {
    let mut frames_resampled: Vec<Vec<Vector3<f64>>> = frames
        .iter()
        .map(|frame| calc_points(frame, max_points, false))
        .collect();
    if connect && frame_count > 2 {
        frames_resampled.push(calc_points(&frames[0], max_points, true));
    }
    let mut splines = Vec::new();
    let mut max_size = f64::MIN;
    for index in 0..max_points {
        let curve: Vec<Vector3<f64>> = frames_resampled
            .iter()
            .map(|frame| frame[index.min(frame.len() - 1)])
            .collect();
        let spline = LowPolySpline::new(&curve);
        max_size = max_size.max(spline.length);
        splines.push(spline);
    }

    let (stations, outline_only) = station_count(count, max_size);

    if outline {
        for index in [0, splines.len() - 1] {
            let samples = (splines[index].length * 3.0 + 1.0) as i32;
            let segments = (splines[index].points.len() - 1) as f64;
            for step in 0..samples {
                sample_loft(
                    pattern,
                    detailed,
                    placed,
                    locations,
                    splines[index].point(f64::from(step) / f64::from(samples) * segments),
                );
            }
        }
        for t in [0.0, (frame_count - 1) as f64] {
            let curve: Vec<Vector3<f64>> = splines.iter().map(|s| s.point(t)).collect();
            let cross = LowPolySpline::new(&curve);
            let samples = (cross.length * 3.0 + 1.0) as i32;
            let segments = (cross.points.len() - 1) as f64;
            for step in 0..samples {
                sample_loft(
                    pattern,
                    detailed,
                    placed,
                    locations,
                    cross.point(f64::from(step) / f64::from(samples) * segments),
                );
            }
        }
    }

    if outline_only && outline {
        return;
    }
    for station in 0..=stations {
        let t = f64::from(station) / f64::from(stations) * connect_span(connect, frame_count);
        let curve: Vec<Vector3<f64>> = splines.iter().map(|s| s.point(t)).collect();
        let cross = LowPolySpline::new(&curve);
        let samples = (cross.length * 3.0 + 1.0) as i32;
        let segments = (cross.points.len() - 1) as f64;
        for step in 0..samples {
            sample_loft(
                pattern,
                detailed,
                placed,
                locations,
                cross.point(f64::from(step) / f64::from(samples) * segments),
            );
        }
    }
}

/// Station count and the outline-only flag: `-1` derives stations from
/// the longest spline (`length * 5 + 1`, Arceon auto mode).
fn station_count(count: i32, max_size: f64) -> (i32, bool) {
    if count < 0 {
        ((max_size * 5.0 + 1.0) as i32, true)
    } else {
        (count, false)
    }
}

/// Spline parameter span: the connect ring adds one extra frame.
const fn connect_span(connect: bool, frame_count: usize) -> f64 {
    if connect && frame_count > 2 {
        frame_count as f64
    } else {
        (frame_count - 1) as f64
    }
}

/// One surface sample: slabs go through the detail map, plain patterns
/// place directly.
fn sample_loft(
    pattern: &Pattern,
    detailed: bool,
    placed: &mut Vec<(BlockPos, BlockStateId)>,
    locations: &mut HashMap<BlockPos, SlabPosition>,
    point: Vector3<f64>,
) {
    if detailed {
        record_slab(locations, point);
    } else if let Some(after) = pattern.apply(&to_block_pos(point), None) {
        placed.push((to_block_pos(point), after));
    }
}

/// Records a slab sample into the top/bottom/full map (upper half of a
/// cell claims `Top`, lower half `Bottom`, both merge to `Full`).
fn record_slab(locations: &mut HashMap<BlockPos, SlabPosition>, point: Vector3<f64>) {
    let is_top = if point.y < 0.0 {
        point.y % 1.0 > -0.5
    } else {
        point.y % 1.0 >= 0.5
    };
    let cell = to_block_pos(point);
    let entry = locations.entry(cell).or_insert(if is_top {
        SlabPosition::Top
    } else {
        SlabPosition::Bottom
    });
    if *entry == SlabPosition::Bottom && is_top || *entry == SlabPosition::Top && !is_top {
        *entry = SlabPosition::Full;
    }
}

/// Floors a sample onto its block position.
const fn to_block_pos(point: Vector3<f64>) -> BlockPos {
    BlockPos::new(
        point.x.floor() as i32,
        point.y.floor() as i32,
        point.z.floor() as i32,
    )
}

/// Turns slab samples into blocks, applies stackdown and runs the edit.
#[allow(clippy::too_many_arguments)] // literal port tail of Arceon's place()
async fn emit_loft(
    state: &Arc<TerrainState>,
    player: &Arc<Player>,
    world: &Arc<World>,
    placed: &mut Vec<(BlockPos, BlockStateId)>,
    locations: &HashMap<BlockPos, SlabPosition>,
    bottom: Option<BlockStateId>,
    top: Option<BlockStateId>,
    full: Option<BlockStateId>,
    stackdown: bool,
    pattern: &Pattern,
) {
    let mut extra: Vec<(BlockPos, BlockStateId)> = Vec::new();
    let stack_fill = full
        .or(bottom)
        .or_else(|| pattern.apply(&BlockPos::new(0, 0, 0), None));
    for (cell, position) in locations {
        let target = match position {
            SlabPosition::Full => full.or(bottom),
            SlabPosition::Top => top.or(bottom),
            SlabPosition::Bottom => bottom,
        };
        if let Some(target) = target {
            extra.push((*cell, target));
        }
        if stackdown && let Some(fill) = stack_fill {
            let stop = highest_terrain(world, cell.0.x, cell.0.z, cell.0.y - 1);
            let mut y = cell.0.y - 1;
            while y > stop {
                extra.push((BlockPos::new(cell.0.x, y, cell.0.z), fill));
                y -= 1;
            }
        }
    }
    if stackdown {
        for (pos, after) in placed.iter() {
            let stop = highest_terrain(world, pos.0.x, pos.0.z, pos.0.y - 1);
            let mut y = pos.0.y - 1;
            while y > stop {
                extra.push((BlockPos::new(pos.0.x, y, pos.0.z), *after));
                y -= 1;
            }
        }
    }
    placed.extend(extra);

    let description = "//loft set".to_string();
    execute(
        &state.we,
        player,
        EditRequest::new(std::mem::take(placed), description),
    )
    .await;
}

/// The highest motion-blocking block at `x/z` at or below `from`
/// (`-1` when the column is empty, like an empty scan result).
fn highest_terrain(world: &World, x: i32, z: i32, from: i32) -> i32 {
    let bottom = world.get_bottom_y();
    let mut y = from;
    while y >= bottom {
        let state = world.get_block_state_id(&BlockPos::new(x, y, z));
        if !is_air(state) && state.to_state().is_solid() {
            return y;
        }
        y -= 1;
    }
    bottom - 1
}

/// Overall bounds of every frame point.
fn frames_bounds(frames: &[Vec<Vector3<f64>>]) -> Option<(Vector3<f64>, Vector3<f64>)> {
    let mut lofts = frames.iter().flatten();
    let first = *lofts.next()?;
    let mut min = first;
    let mut max = first;
    for point in lofts {
        min = Vector3::new(min.x.min(point.x), min.y.min(point.y), min.z.min(point.z));
        max = Vector3::new(max.x.max(point.x), max.y.max(point.y), max.z.max(point.z));
    }
    Some((min, max))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bezier_hits_knots() {
        let knots = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(4.0, 2.0, 0.0),
            Vector3::new(8.0, 0.0, 0.0),
        ];
        let spline = BezierSpline::new(&knots);
        let start = spline.point(0.0);
        let end = spline.point(2.0);
        assert_eq!(start, knots[0]);
        assert_eq!(end, knots[2]);
        // The curve passes above the straight chord midpoint.
        let middle = spline.point(1.0);
        assert!(middle.y > 1.0, "middle.y = {}", middle.y);
    }

    #[test]
    fn flat_axis_stays_flat() {
        let knots = vec![
            Vector3::new(0.0, 5.0, 0.0),
            Vector3::new(0.0, 6.0, 3.0),
            Vector3::new(0.0, 5.0, 6.0),
        ];
        let spline = BezierSpline::new(&knots);
        for step in 0..=10 {
            let point = spline.point(f64::from(step) / 10.0 * 2.0);
            assert!((point.x - 0.0).abs() < 1e-9);
        }
    }

    #[test]
    fn low_poly_interpolates_linearly() {
        let points = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 4.0),
        ];
        let spline = LowPolySpline::new(&points);
        let middle = spline.point(1.5);
        assert_eq!(middle, Vector3::new(2.0, 0.0, 2.0));
    }

    #[test]
    fn calc_points_resamples_count() {
        let points = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(4.0, 0.0, 0.0),
            Vector3::new(9.0, 0.0, 0.0),
        ];
        assert_eq!(calc_points(&points, 7, true).len(), 7);
        assert_eq!(calc_points(&points, 7, false).len(), 7);
        let smooth = calc_points(&points, 7, true);
        assert_eq!(smooth.first().copied(), Some(points[0]));
        assert_eq!(smooth.last().copied(), Some(points[2]));
    }

    #[test]
    fn slab_record_merges_halves() {
        let mut locations = HashMap::new();
        record_slab(&mut locations, Vector3::new(0.5, 5.7, 0.5));
        assert_eq!(locations[&BlockPos::new(0, 5, 0)], SlabPosition::Top);
        record_slab(&mut locations, Vector3::new(0.5, 5.2, 0.5));
        assert_eq!(locations[&BlockPos::new(0, 5, 0)], SlabPosition::Full);
    }
}
