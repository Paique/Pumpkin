//! Arceon brush algorithms ported from the decompiled 0.5.0 source.
//!
//! `//ab cube` and `//ab boulder` / `//br boulder`, keeping its `Math3D`
//! quirks: `getRadian` reads degrees as a fraction of a full turn, lines
//! sample five points per block and the boulder icosphere displaces
//! vertices per axis while refining.

use std::sync::Arc;

use carrot_worldedit::edit::ensure_chunks;
use carrot_worldedit::pattern::Pattern;
use carrot_worldedit::states::is_air;
use pumpkin_core::world::World;
use pumpkin_data::BlockStateId;
use pumpkin_util::math::position::BlockPos;

use rand::RngExt;

use crate::state::{TerrainState, WeSnapshot};

/// Sentinel that means "use `size`" for the height/width pairs.
const SENTINEL: i32 = 10_000;

/// Reference `Math3D.getRadian`: degrees as a fraction of a full turn.
fn get_radian(deg: f64) -> f64 {
    deg / 360.0 * std::f64::consts::TAU
}

/// Reference `Math3D.line(a, b)`: five samples per block plus the end.
fn line(a: [f64; 3], b: [f64; 3]) -> Vec<[f64; 3]> {
    let count = (distance(a, b) * 5.0) as i32;
    line_n(a, b, count.max(1))
}

/// Reference `Math3D.line(a, b, n)`: `n` segments plus the end point.
fn line_n(a: [f64; 3], b: [f64; 3], n: i32) -> Vec<[f64; 3]> {
    let n = n.max(1);
    let mut points = Vec::with_capacity(n as usize + 1);
    let delta = [
        (b[0] - a[0]) / f64::from(n),
        (b[1] - a[1]) / f64::from(n),
        (b[2] - a[2]) / f64::from(n),
    ];
    for step in 0..n {
        points.push([
            a[0] + delta[0] * f64::from(step),
            a[1] + delta[1] * f64::from(step),
            a[2] + delta[2] * f64::from(step),
        ]);
    }
    points.push(b);
    points
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    let dz = b[2] - a[2];
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Reference `randBetween` (float): the bound includes the `+1`, so it
/// can overshoot `max` by up to one unit.
fn rand_between_f(rng: &mut impl rand::RngExt, min: f64, max: f64) -> f64 {
    if (min - max).abs() < f64::EPSILON {
        return min;
    }
    let base = if min < max { min } else { max };
    base + rng.random::<f64>() * ((max - min).abs() + 1.0)
}

/// Reference `randIntBetween` (inclusive on both ends).
fn rand_int_between(rng: &mut impl rand::RngExt, min: i32, max: i32) -> i32 {
    if min == max {
        return min;
    }
    let base = if min < max { min } else { max };
    base + rng.random_range(0..=((max - min).abs()))
}

fn rotate_y(point: [f64; 3], rad: f64) -> [f64; 3] {
    let (sin, cos) = rad.sin_cos();
    [
        point[0] * cos + point[2] * sin,
        point[1],
        -point[0] * sin + point[2] * cos,
    ]
}

fn push_desired(
    desired: &mut Vec<(BlockPos, BlockStateId)>,
    snapshot: &WeSnapshot,
    pattern: &Pattern,
    point: [f64; 3],
) {
    let pos = BlockPos::containing(point[0], point[1], point[2]);
    if let Some(state) = snapshot.apply_pattern(pattern, &pos) {
        desired.push((pos, state));
    }
}

/// `//ab cube` — rotated box built from four-point faces.
#[allow(clippy::too_many_arguments)]
#[must_use]
#[allow(clippy::too_many_lines)] // literal port of Arceon's CubeBrush
pub fn cube(
    snapshot: &WeSnapshot,
    pattern: &Pattern,
    target: BlockPos,
    yaw: f64,
    pitch: f64,
    size: (i32, i32),
    snap_angle: i32,
    height: (i32, i32),
    width: (i32, i32),
    hollow: bool,
    angled: bool,
    centered: bool,
) -> Vec<(BlockPos, BlockStateId)> {
    let mut rng = rand::rng();
    let origin = [
        f64::from(target.0.x) + 0.5,
        f64::from(target.0.y),
        f64::from(target.0.z) + 0.5,
    ];

    let mut rot = ((360.0 - yaw + 90.0) % 360.0) as i32;
    let mut pitch_deg = 180.0 - pitch;
    let half_l = rand_between_f(&mut rng, f64::from(size.0), f64::from(size.1)) / 2.0;
    let half_h = if height.0 == SENTINEL {
        half_l
    } else {
        rand_between_f(&mut rng, f64::from(height.0), f64::from(height.1)) / 2.0
    };
    let half_w = if width.0 == SENTINEL {
        half_l
    } else {
        rand_between_f(&mut rng, f64::from(width.0), f64::from(width.1)) / 2.0
    };
    let y_off = if centered { 0.0 } else { half_h + 1.0 };

    let snap = f64::from(snap_angle.max(1));
    rot = ((f64::from(rot) / snap).round() * snap) as i32;
    pitch_deg = (pitch_deg / snap).round() * snap;

    let rot_rad = get_radian(f64::from(rot));
    let pitch_rad = get_radian(pitch_deg);

    let corner = |long: f64, h: f64, wide: f64| -> [f64; 3] {
        let mut point = [long, h, wide];
        if angled {
            // Reference quirk: cosine negated in the pitch rotation.
            let cos = -pitch_rad.cos();
            let sin = pitch_rad.sin();
            point = [
                point[0] * cos - point[1] * sin,
                point[1] * cos + point[0] * sin,
                point[2],
            ];
        }
        let mut turned = rotate_y(point, rot_rad);
        turned[1] += y_off;
        turned
    };

    let top1 = corner(half_l, half_h, half_w);
    let top2 = corner(half_l, half_h, -half_w);
    let top3 = corner(-half_l, half_h, -half_w);
    let top4 = corner(-half_l, half_h, half_w);
    let bottom1 = corner(half_l, -half_h, half_w);
    let bottom2 = corner(half_l, -half_h, -half_w);
    let bottom3 = corner(-half_l, -half_h, -half_w);
    let bottom4 = corner(-half_l, -half_h, half_w);

    let mut desired = Vec::new();
    let set_face = |desired: &mut Vec<(BlockPos, BlockStateId)>,
                    a: [f64; 3],
                    b: [f64; 3],
                    c: [f64; 3],
                    d: [f64; 3]| {
        let n = (distance(a, b) * 5.0) as i32;
        let tops = line_n(a, b, n.max(1));
        let bots = line_n(c, d, n.max(1));
        for (top, bottom) in tops.iter().zip(&bots) {
            for point in line(*top, *bottom) {
                push_desired(
                    desired,
                    snapshot,
                    pattern,
                    [
                        point[0] + origin[0],
                        point[1] + origin[1],
                        point[2] + origin[2],
                    ],
                );
            }
        }
    };

    if hollow {
        set_face(&mut desired, top1, top2, bottom1, bottom2);
        set_face(&mut desired, top2, top3, bottom2, bottom3);
        set_face(&mut desired, top3, top4, bottom3, bottom4);
        set_face(&mut desired, top4, top1, bottom4, bottom1);
        set_face(&mut desired, top1, top2, top4, top3);
        set_face(&mut desired, bottom1, bottom2, bottom4, bottom3);
    } else {
        for point in line(bottom1, top1) {
            let delta = [
                point[0] - bottom1[0],
                point[1] - bottom1[1],
                point[2] - bottom1[2],
            ];
            set_face(
                &mut desired,
                point,
                [
                    bottom2[0] + delta[0],
                    bottom2[1] + delta[1],
                    bottom2[2] + delta[2],
                ],
                [
                    bottom4[0] + delta[0],
                    bottom4[1] + delta[1],
                    bottom4[2] + delta[2],
                ],
                [
                    bottom3[0] + delta[0],
                    bottom3[1] + delta[1],
                    bottom3[2] + delta[2],
                ],
            );
        }
    }
    desired
}

/// The boulder's icosphere mesh: an icosahedron with per-axis vertex
/// jitter, refined by midpoint subdivision with random displacement.
struct IcoSphere {
    triangles: Vec<([f64; 3], [f64; 3], [f64; 3])>,
}

/// The classic icosahedron face topology.
const FACES: [(usize, usize, usize); 20] = [
    (0, 11, 5),
    (0, 5, 1),
    (0, 1, 7),
    (0, 7, 10),
    (0, 10, 11),
    (1, 5, 9),
    (5, 11, 4),
    (11, 10, 2),
    (10, 7, 6),
    (7, 1, 8),
    (3, 9, 4),
    (3, 4, 2),
    (3, 2, 6),
    (3, 6, 8),
    (3, 8, 9),
    (4, 9, 5),
    (2, 4, 11),
    (6, 2, 10),
    (8, 6, 7),
    (9, 8, 1),
];

impl IcoSphere {
    /// `iterations` is the smoothness (clamped to 0..=5 upstream).
    fn new(iterations: i32) -> Self {
        let mut rng = rand::rng();
        let phi = 1.0f64.midpoint(5.0f64.sqrt());
        let raw: Vec<[f64; 3]> = [
            (-1.0, phi, 0.0),
            (1.0, phi, 0.0),
            (-1.0, -phi, 0.0),
            (1.0, -phi, 0.0),
            (0.0, -1.0, phi),
            (0.0, 1.0, phi),
            (0.0, -1.0, -phi),
            (0.0, 1.0, -phi),
            (phi, 0.0, -1.0),
            (phi, 0.0, 1.0),
            (-phi, 0.0, -1.0),
            (-phi, 0.0, 1.0),
        ]
        .into_iter()
        .map(|(x, y, z)| [x / phi, y / phi, z / phi])
        .collect();
        let spin = rng.random::<f64>() * std::f64::consts::TAU;
        let mut base = Vec::with_capacity(raw.len());
        for vertex in raw {
            // Per-axis jitter in [0.75, 1.25] and a random initial spin.
            let jittered = [
                vertex[0] * (rng.random::<f64>() / 2.0 + 0.75),
                vertex[1] * (rng.random::<f64>() / 2.0 + 0.75),
                vertex[2] * (rng.random::<f64>() / 2.0 + 0.75),
            ];
            base.push(rotate_y(jittered, spin));
        }

        let mut triangles: Vec<([f64; 3], [f64; 3], [f64; 3])> = FACES
            .iter()
            .map(|(a, b, c)| (base[*a], base[*b], base[*c]))
            .collect();

        for round in 1..=iterations.clamp(0, 5) {
            let denom = f64::from(2 + round * round);
            let mut displace = |point: [f64; 3]| -> [f64; 3] {
                let mut factor = || rng.random::<f64>() / denom + (1.0 - 1.0 / denom / 2.0);
                [
                    point[0] * factor(),
                    point[1] * factor(),
                    point[2] * factor(),
                ]
            };
            let mut mid = |a: [f64; 3], b: [f64; 3]| {
                displace([
                    a[0].midpoint(b[0]),
                    a[1].midpoint(b[1]),
                    a[2].midpoint(b[2]),
                ])
            };
            let mut next = Vec::with_capacity(triangles.len() * 4);
            for (a, b, c) in &triangles {
                let ab = mid(*a, *b);
                let bc = mid(*b, *c);
                let ca = mid(*c, *a);
                next.push((*a, ab, ca));
                next.push((ab, *b, bc));
                next.push((ca, bc, *c));
                next.push((ab, bc, ca));
            }
            triangles = next;
        }
        Self { triangles }
    }
}

/// Scales an icosphere vertex onto the boulder, optionally pitched.
#[allow(clippy::too_many_arguments)]
fn place_vertex(
    point: [f64; 3],
    origin: [f64; 3],
    v: i32,
    h: i32,
    mult: f64,
    rot: f64,
    pitch_rad: f64,
    angled: bool,
) -> [f64; 3] {
    let mut located = [
        origin[0] + point[0] * f64::from(v) / 2.0,
        origin[1] + point[1] * f64::from(h) / 2.0,
        origin[2] + point[2] * f64::from(v) / 2.0 * mult,
    ];
    if angled {
        let (sin, cos) = pitch_rad.sin_cos();
        located = [
            located[0] * cos - located[1] * sin,
            located[1] * cos + located[0] * sin,
            located[2],
        ];
    }
    rotate_y(located, rot)
}

/// `//ab boulder` — icosphere rock with optional hollow shell, angled
/// rotation or falling columns.
#[allow(clippy::too_many_arguments)]
#[allow(clippy::too_many_lines)] // literal port of Arceon's BoulderBrush
pub async fn boulder(
    terrain: &Arc<TerrainState>,
    snapshot: &WeSnapshot,
    world: &Arc<World>,
    pattern: &Pattern,
    target: BlockPos,
    yaw: f64,
    pitch: f64,
    size: (i32, i32),
    smoothness: i32,
    height: (i32, i32),
    width: (i32, i32),
    hollow: bool,
    falling: bool,
    angled: bool,
) -> Vec<(BlockPos, BlockStateId)> {
    let _ = terrain;
    // Random draws happen before any await: ThreadRng is not Send.
    let (v, h, w) = {
        let mut rng = rand::rng();
        let v = rand_int_between(&mut rng, size.0, size.1).max(1);
        let h = if height.0 == SENTINEL {
            v
        } else {
            rand_int_between(&mut rng, height.0, height.1).max(1)
        };
        let w = if width.0 == SENTINEL {
            v
        } else {
            rand_int_between(&mut rng, width.0, width.1).max(1)
        };
        (v, h, w)
    };
    let mult = f64::from(w) / f64::from(v);

    let origin = [
        f64::from(target.0.x) + 0.5,
        f64::from(target.0.y),
        f64::from(target.0.z) + 0.5,
    ];
    let rot = get_radian((yaw - 90.0) % 360.0);
    let pitch_rad = get_radian(pitch);

    // Load everything the shell and the fills can touch.
    let reach = v.max(h).max(w) + 2;
    let mut region = Vec::new();
    for x in -reach..=reach {
        for z in -reach..=reach {
            region.push(BlockPos::new(target.0.x + x, target.0.y, target.0.z + z));
        }
    }
    ensure_chunks(world, &region, |pos: &BlockPos| pos).await;

    let ico = IcoSphere::new(smoothness);
    let mut shell: Vec<[f64; 3]> = Vec::new();
    for (p1, p2, p3) in &ico.triangles {
        let l1 = place_vertex(*p1, origin, v, h, mult, rot, pitch_rad, angled);
        let l2 = place_vertex(*p2, origin, v, h, mult, rot, pitch_rad, angled);
        let l3 = place_vertex(*p3, origin, v, h, mult, rot, pitch_rad, angled);
        let n = (distance(l2, l1) * 2.0) as i32 + 1;
        for step in 0..=n {
            let t = f64::from(step) / f64::from(n);
            let edge = [
                l2[0] + (l1[0] - l2[0]) * t,
                l2[1] + (l1[1] - l2[1]) * t,
                l2[2] + (l1[2] - l2[2]) * t,
            ];
            for point in line_n(edge, l3, n * 2) {
                shell.push(point);
            }
        }
    }

    let mut desired = Vec::new();
    if !hollow && !falling {
        for point in &shell {
            push_desired(&mut desired, snapshot, pattern, *point);
        }
        for vertex in &shell {
            let dist = distance(*vertex, origin) as i32;
            let edge = [vertex[0] + 0.5, vertex[1] + 0.5, vertex[2] + 0.5];
            let mid = [
                edge[0].midpoint(origin[0]),
                edge[1].midpoint(origin[1]),
                edge[2].midpoint(origin[2]),
            ];
            for point in line_n(edge, mid, dist * 3) {
                push_desired(&mut desired, snapshot, pattern, point);
            }
            for point in line_n(mid, origin, dist) {
                push_desired(&mut desired, snapshot, pattern, point);
            }
        }
    } else if falling {
        let top = target.0.y + h / 2;
        for vertex in &shell {
            let x = vertex[0].floor() as i32;
            let z = vertex[2].floor() as i32;
            let mut ground = world.get_bottom_y();
            for y in (world.get_bottom_y()..=top).rev() {
                let state = world.get_block_state_id(&BlockPos::new(x, y, z));
                if !is_air(state) {
                    ground = y;
                    break;
                }
            }
            let start = vertex[1].floor() as i32;
            let mut depth = 1;
            while start - depth > ground {
                push_desired(
                    &mut desired,
                    snapshot,
                    pattern,
                    [vertex[0], f64::from(start - depth), vertex[2]],
                );
                depth += 1;
            }
        }
    } else {
        for point in &shell {
            push_desired(&mut desired, snapshot, pattern, *point);
        }
    }
    desired
}
