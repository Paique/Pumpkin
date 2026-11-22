//! Utility region commands: `//fill`, `//center`, `//line` and `//curve`.

use std::collections::VecDeque;
use std::sync::Arc;

use carrot_core::ModuleContext;
use pumpkin_core::command::argument_builder::{ArgumentBuilder, argument, command};
use pumpkin_core::command::argument_types::core::string::StringArgumentType;
use pumpkin_core::command::context::command_context::CommandContext;
use pumpkin_core::command::node::{CommandExecutor, CommandExecutorResult};
use pumpkin_core::entity::player::Player;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;

use crate::commands::{
    NO_SELECTION, ONLY_PLAYERS, check_volume_estimate, clipboard_of, held_block, player_block_pos,
    player_of, reply, selection_of, spawn_edit,
};
use crate::edit::{EditRequest, ensure_chunks};
use crate::pattern::Pattern;
use crate::region::Direction;
use crate::session::WeState;
use crate::states::is_air;

const PERM_REGION: &str = "carrot:worldedit.region";

/// Registers the utility commands and their permission node (shared with
/// the region permission).
pub fn register(context: &mut ModuleContext<'_>, state: &Arc<WeState>) {
    for name in ["//fill", "//center", "//line", "//curve"] {
        let executor = UtilityExecutor {
            state: Arc::clone(state),
            kind: match name {
                "//fill" => Kind::Fill,
                "//center" => Kind::Center,
                "//line" => Kind::Line,
                _ => Kind::Curve,
            },
        };
        context.register_command(
            name,
            command(name, "Region utility command.")
                .executes(executor.clone())
                .then(argument("args", StringArgumentType::GreedyPhrase).executes(executor)),
            PERM_REGION,
        );
    }
}

/// Which utility command an executor runs.
#[derive(Clone, Copy)]
enum Kind {
    Fill,
    Center,
    Line,
    Curve,
}

/// One utility command executor for every literal (the context does not
/// expose which literal fired).
#[derive(Clone)]
struct UtilityExecutor {
    state: Arc<WeState>,
    kind: Kind,
}

impl CommandExecutor for UtilityExecutor {
    fn execute(&self, context: &CommandContext) -> CommandExecutorResult {
        let Some(player) = player_of(context) else {
            reply(context, ONLY_PLAYERS);
            return Ok(0);
        };
        let raw = StringArgumentType::get(context, "args").unwrap_or_default();
        let hand = held_block(&player);

        match self.kind {
            Kind::Fill => self.fill(context, &player, raw, hand),
            Kind::Center => self.center(context, &player, raw, hand),
            Kind::Line | Kind::Curve => self.line(context, &player, raw, hand),
        }
        Ok(1)
    }
}

impl UtilityExecutor {
    /// `//fill <pattern> <radius> [depth] [direction]` — breadth-first
    /// flood of connected air below/around the player, `WorldEdit's`
    /// `DownwardVisitor` semantics.
    fn fill(
        &self,
        context: &CommandContext,
        player: &Arc<Player>,
        raw: &str,
        hand: Option<BlockStateId>,
    ) {
        let mut tokens = raw.split_whitespace();
        let Some(pattern_token) = tokens.next() else {
            reply(
                context,
                "use: //fill <padrão> <raio> [profundidade] [direção]".to_string(),
            );
            return;
        };
        let pattern = match Pattern::parse(pattern_token, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return;
            }
        };
        let Some(radius) = tokens.next().map(str::parse::<f64>) else {
            reply(context, "raio faltando".to_string());
            return;
        };
        let Ok(radius) = radius else {
            reply(context, "raio inválido".to_string());
            return;
        };
        let radius = radius.max(1.0);
        let depth = tokens
            .next()
            .map_or(Ok(1), str::parse::<i32>)
            .unwrap_or(1)
            .max(1);
        let direction = tokens
            .next()
            .map_or(Some(Direction::Down), Direction::parse)
            .unwrap_or(Direction::Down);

        let estimate = ((4.0 / 3.0) * std::f64::consts::PI * radius * radius * radius) as u64;
        if let Err(message) = check_volume_estimate(&self.state, estimate) {
            reply(context, message);
            return;
        }

        let origin = player_block_pos(player);
        let state = Arc::clone(&self.state);
        let player = Arc::clone(player);
        let description = format!("//fill {}", raw.trim());
        tokio::spawn(async move {
            let world = player.world();
            let cells = flood_fill(&world, origin, radius, depth, direction).await;
            let clipboard = clipboard_of(&state, player.gameprofile.id);
            let desired = cells
                .into_iter()
                .filter_map(|pos| {
                    pattern
                        .apply(&pos, clipboard.as_deref())
                        .map(|after| (pos, after))
                })
                .collect();
            crate::edit::execute(&state, &player, EditRequest::new(desired, description)).await;
        });
        reply(context, "Preenchendo...".to_string());
    }

    /// `//center <pattern>` — the center block(s) of the selection
    /// (a 2x2x2 area for even-sized regions).
    fn center(
        &self,
        context: &CommandContext,
        player: &Arc<Player>,
        raw: &str,
        hand: Option<BlockStateId>,
    ) {
        let Some(pattern_token) = raw.split_whitespace().next() else {
            reply(context, "use: //center <padrão>".to_string());
            return;
        };
        let pattern = match Pattern::parse(pattern_token, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return;
            }
        };
        let Ok(region) = selection_of(&self.state, player.gameprofile.id) else {
            reply(context, NO_SELECTION);
            return;
        };

        // WorldEdit fills from floor(center) to roundHalfUp(center) per
        // axis: one block for odd sizes, two for even.
        let min = region.min().0;
        let max = region.max().0;
        let mut positions = Vec::new();
        for (low, high) in [(min.x, max.x), (min.y, max.y), (min.z, max.z)] {
            let center = (low + high) as f64 / 2.0;
            let hi = center.round() as i32;
            let lo = center as i32;
            positions.push((lo, hi));
        }
        let mut cells = Vec::new();
        for x in positions[0].0..=positions[0].1 {
            for y in positions[1].0..=positions[1].1 {
                for z in positions[2].0..=positions[2].1 {
                    cells.push(BlockPos(Vector3::new(x, y, z)));
                }
            }
        }

        let clipboard = clipboard_of(&self.state, player.gameprofile.id);
        let desired = cells
            .into_iter()
            .filter_map(|pos| {
                pattern
                    .apply(&pos, clipboard.as_deref())
                    .map(|after| (pos, after))
            })
            .collect();
        let description = format!("//center {}", raw.trim());
        spawn_edit(&self.state, player, EditRequest::new(desired, description));
        reply(context, "Aplicando edição...".to_string());
    }

    /// `//line`/`//curve <pattern> [thickness] [-h]` — with the cuboid
    /// two-point selection both draw the segment between the corners
    /// (`WorldEdit` `drawLine`).
    fn line(
        &self,
        context: &CommandContext,
        player: &Arc<Player>,
        raw: &str,
        hand: Option<BlockStateId>,
    ) {
        let mut tokens = raw.split_whitespace();
        let Some(pattern_token) = tokens.next() else {
            reply(context, "use: //line <padrão> [espessura] [-h]".to_string());
            return;
        };
        let pattern = match Pattern::parse(pattern_token, hand) {
            Ok(pattern) => pattern,
            Err(message) => {
                reply(context, format!("Padrão inválido: {message}"));
                return;
            }
        };
        let mut thickness = 0.0f64;
        let mut hollow = false;
        for token in tokens {
            if token.eq_ignore_ascii_case("-h") {
                hollow = true;
            } else if let Ok(parsed) = token.parse::<f64>() {
                thickness = parsed;
            } else {
                reply(context, format!("argumento inválido: {token}"));
                return;
            }
        }
        if thickness < 0.0 {
            reply(context, "espessura deve ser >= 0".to_string());
            return;
        }

        let Ok(region) = selection_of(&self.state, player.gameprofile.id) else {
            reply(context, NO_SELECTION);
            return;
        };

        let cells = line_cells(region.min(), region.max(), thickness, hollow);
        if let Err(message) = check_volume_estimate(&self.state, cells.len() as u64) {
            reply(context, message);
            return;
        }
        let clipboard = clipboard_of(&self.state, player.gameprofile.id);
        let desired = cells
            .into_iter()
            .filter_map(|pos| {
                pattern
                    .apply(&pos, clipboard.as_deref())
                    .map(|after| (pos, after))
            })
            .collect();
        let name = match self.kind {
            Kind::Curve => "curve",
            _ => "line",
        };
        let description = format!("//{name} {}", raw.trim());
        spawn_edit(&self.state, player, EditRequest::new(desired, description));
        reply(context, "Aplicando edição...".to_string());
    }
}

/// `WorldEdit` `fillXZ`/`fillDirection`: flood connected air inside the
/// ellipsoid of `radius` around the origin, never visiting against
/// `direction`.
async fn flood_fill(
    world: &Arc<pumpkin_core::world::World>,
    origin: BlockPos,
    radius: f64,
    depth: i32,
    direction: Direction,
) -> Vec<BlockPos> {
    let radius_sq = radius * radius;
    let bounds = CuboidBounds::around(origin, radius, depth);
    // Load every chunk the ellipsoid can touch before reading states:
    // unloaded chunks read as air and would poison the flood.
    let corners = vec![
        BlockPos(Vector3::new(bounds.min_x, bounds.min_y, bounds.min_z)),
        BlockPos(Vector3::new(bounds.max_x, bounds.max_y, bounds.max_z)),
    ];
    ensure_chunks(world, &corners, |pos| pos).await;

    // The opposite of the fill direction is never visited (down = the
    // classic DownwardVisitor).
    let excluded = match direction {
        Direction::Down => Some(Direction::Up),
        Direction::Up => Some(Direction::Down),
        Direction::North => Some(Direction::South),
        Direction::South => Some(Direction::North),
        Direction::West => Some(Direction::East),
        Direction::East => Some(Direction::West),
    };

    let mut visited = std::collections::HashSet::new();
    let mut filled = Vec::new();
    let mut queue = VecDeque::new();
    if is_air(world.get_block_state_id(&origin)) {
        queue.push_back(origin);
        visited.insert(origin);
    }

    let steps = [
        (Direction::North.offset(), Direction::North),
        (Direction::South.offset(), Direction::South),
        (Direction::West.offset(), Direction::West),
        (Direction::East.offset(), Direction::East),
        (Direction::Up.offset(), Direction::Up),
        (Direction::Down.offset(), Direction::Down),
    ];
    while let Some(pos) = queue.pop_front() {
        filled.push(pos);
        for (offset, step) in steps {
            if Some(step) == excluded {
                continue;
            }
            let next = BlockPos(Vector3::new(
                pos.0.x + offset.x,
                pos.0.y + offset.y,
                pos.0.z + offset.z,
            ));
            if visited.contains(&next)
                || !bounds.contains(next.0.x, next.0.y, next.0.z)
                || !within_ellipsoid(origin, next, radius_sq)
            {
                continue;
            }
            visited.insert(next);
            if is_air(world.get_block_state_id(&next)) {
                queue.push_back(next);
            }
        }
    }
    filled
}

/// Integer bounds of the fill region: the ellipsoid cut at `depth` along
/// the fill direction.
struct CuboidBounds {
    min_x: i32,
    max_x: i32,
    min_y: i32,
    max_y: i32,
    min_z: i32,
    max_z: i32,
}

impl CuboidBounds {
    const fn around(origin: BlockPos, radius: f64, depth: i32) -> Self {
        let reach = radius.ceil() as i32;
        let (min_y, max_y) = (origin.0.y - depth + 1, origin.0.y);
        Self {
            min_x: origin.0.x - reach,
            max_x: origin.0.x + reach,
            min_y,
            max_y,
            min_z: origin.0.z - reach,
            max_z: origin.0.z + reach,
        }
    }

    const fn contains(&self, x: i32, y: i32, z: i32) -> bool {
        x >= self.min_x
            && x <= self.max_x
            && y >= self.min_y
            && y <= self.max_y
            && z >= self.min_z
            && z <= self.max_z
    }
}

/// `(dx² + dy² + dz²) <= radius²` around the origin.
fn within_ellipsoid(origin: BlockPos, pos: BlockPos, radius_sq: f64) -> bool {
    let dx = f64::from(pos.0.x - origin.0.x);
    let dy = f64::from(pos.0.y - origin.0.y);
    let dz = f64::from(pos.0.z - origin.0.z);
    dx * dx + dy * dy + dz * dz <= radius_sq
}

/// `WorldEdit` `drawLine`: dominant-axis rasterization, then balloon by
/// `thickness` (union of discs) and hollowing for `-h`.
#[must_use]
pub fn line_cells(pos1: BlockPos, pos2: BlockPos, thickness: f64, hollow: bool) -> Vec<BlockPos> {
    let mut line = rasterize_line(pos1, pos2);
    if thickness > 0.0 {
        line = balloon(&line, thickness);
    }
    if hollow && !line.is_empty() {
        let set: std::collections::HashSet<BlockPos> = line.iter().copied().collect();
        line.retain(|pos| {
            NEIGHBORS_6.iter().any(|offset| {
                let neighbor = BlockPos(Vector3::new(
                    pos.0.x + offset.x,
                    pos.0.y + offset.y,
                    pos.0.z + offset.z,
                ));
                !set.contains(&neighbor)
            })
        });
    }
    line
}

const NEIGHBORS_6: [Vector3<i32>; 6] = [
    Vector3::new(1, 0, 0),
    Vector3::new(-1, 0, 0),
    Vector3::new(0, 1, 0),
    Vector3::new(0, -1, 0),
    Vector3::new(0, 0, 1),
    Vector3::new(0, 0, -1),
];

/// Dominant-axis line rasterization with rounded minor coordinates,
/// exactly like `EditSession#drawLine`.
fn rasterize_line(pos1: BlockPos, pos2: BlockPos) -> Vec<BlockPos> {
    let (x1, y1, z1) = (pos1.0.x, pos1.0.y, pos1.0.z);
    let (x2, y2, z2) = (pos2.0.x, pos2.0.y, pos2.0.z);
    let dx = (x2 - x1).unsigned_abs() as i32;
    let dy = (y2 - y1).unsigned_abs() as i32;
    let dz = (z2 - z1).unsigned_abs() as i32;
    let mut cells = Vec::new();
    if dx + dy + dz == 0 {
        cells.push(pos1);
        return cells;
    }

    let sign = |a: i32, b: i32| i32::from(b > a) - i32::from(a > b);
    let mut push = |x: i32, y: i32, z: i32| {
        cells.push(BlockPos(Vector3::new(x, y, z)));
    };
    let d_max = dx.max(dy).max(dz);
    if d_max == dx {
        for step in 0..=dx {
            let x = x1 + step * sign(x1, x2);
            let y = y1 + ((step * dy) as f64 / dx as f64).round() as i32 * sign(y1, y2);
            let z = z1 + ((step * dz) as f64 / dx as f64).round() as i32 * sign(z1, z2);
            push(x, y, z);
        }
    } else if d_max == dy {
        for step in 0..=dy {
            let y = y1 + step * sign(y1, y2);
            let x = x1 + ((step * dx) as f64 / dy as f64).round() as i32 * sign(x1, x2);
            let z = z1 + ((step * dz) as f64 / dy as f64).round() as i32 * sign(z1, z2);
            push(x, y, z);
        }
    } else {
        for step in 0..=dz {
            let z = z1 + step * sign(z1, z2);
            let y = y1 + ((step * dy) as f64 / dz as f64).round() as i32 * sign(y1, y2);
            let x = x1 + ((step * dx) as f64 / dz as f64).round() as i32 * sign(x1, x2);
            push(x, y, z);
        }
    }
    cells
}

/// Union of `radius`-spheres around each cell (`WorldEdit` `getBallooned`).
fn balloon(cells: &[BlockPos], radius: f64) -> Vec<BlockPos> {
    let reach = radius.ceil() as i32;
    let radius_sq = (radius + 0.5) * (radius + 0.5);
    let mut bounds_min = Vector3::new(i32::MAX, i32::MAX, i32::MAX);
    let mut bounds_max = Vector3::new(i32::MIN, i32::MIN, i32::MIN);
    for pos in cells {
        bounds_min = Vector3::new(
            bounds_min.x.min(pos.0.x - reach),
            bounds_min.y.min(pos.0.y - reach),
            bounds_min.z.min(pos.0.z - reach),
        );
        bounds_max = Vector3::new(
            bounds_max.x.max(pos.0.x + reach),
            bounds_max.y.max(pos.0.y + reach),
            bounds_max.z.max(pos.0.z + reach),
        );
    }

    let mut result = Vec::new();
    for x in bounds_min.x..=bounds_max.x {
        for y in bounds_min.y..=bounds_max.y {
            for z in bounds_min.z..=bounds_max.z {
                let pos = BlockPos(Vector3::new(x, y, z));
                if cells.iter().any(|cell| {
                    let dx = f64::from(pos.0.x - cell.0.x);
                    let dy = f64::from(pos.0.y - cell.0.y);
                    let dz = f64::from(pos.0.z - cell.0.z);
                    dx * dx + dy * dy + dz * dz <= radius_sq
                }) {
                    result.push(pos);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: BlockPos = BlockPos(Vector3::new(0, 0, 0));
    const B: BlockPos = BlockPos(Vector3::new(4, 0, 0));

    #[test]
    fn line_rasterizes_straight_segment() {
        let cells = rasterize_line(A, B);
        assert_eq!(cells.len(), 5);
        assert!(cells.contains(&BlockPos(Vector3::new(2, 0, 0))));
    }

    #[test]
    fn diagonal_line_steps_every_block() {
        let cells = rasterize_line(A, BlockPos(Vector3::new(3, 3, 0)));
        assert_eq!(cells.len(), 4);
        assert!(cells.contains(&BlockPos(Vector3::new(1, 1, 0))));
    }

    #[test]
    fn thickness_balloons_line() {
        let plain = line_cells(A, B, 0.0, false);
        let thick = line_cells(A, B, 2.0, false);
        assert!(thick.len() > plain.len());
    }

    #[test]
    fn hollow_thick_line_drops_core() {
        let cells = line_cells(A, B, 3.0, true);
        assert!(!cells.is_empty());
        // The middle of the tube is not part of the shell.
        assert!(!cells.contains(&BlockPos(Vector3::new(2, 0, 0))));
    }

    #[test]
    fn degenerate_line_is_single_block() {
        assert_eq!(rasterize_line(A, A).len(), 1);
    }
}
