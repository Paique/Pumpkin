//! The `//br` brush set shared by the WorldEdit-style and Arceon-style
//! commands: parameters, labels and async execution through the
//! worldedit edit engine (masks, confirm and history included).

use std::collections::HashMap;
use std::sync::Arc;

use carrot_worldedit::mask::Mask;
use carrot_worldedit::pattern::Pattern;
use pumpkin_core::entity::player::Player;
use pumpkin_util::math::position::BlockPos;

use crate::state::TerrainState;

/// One bound `//br` brush.
#[derive(Clone)]
pub enum Brush {
    /// Solid or hollow sphere.
    Sphere {
        pattern: Pattern,
        radius: u32,
        hollow: bool,
    },
    /// Vertical cylinder rising from the target.
    Cylinder {
        pattern: Pattern,
        radius: u32,
        height: i32,
        hollow: bool,
    },
    /// Gaussian heightmap smoothing (`WorldEdit` convolution).
    Smooth {
        radius: u32,
        iterations: u32,
        mask: Option<Mask>,
    },
    /// FAWE erosion (face-count erosion + fill passes).
    Erode {
        radius: u32,
        erode_faces: u32,
        erode_rec: u32,
        fill_faces: u32,
        fill_rec: u32,
    },
    /// Block-type voting blend inside the sphere.
    BlendBall {
        radius: u32,
        min_freq_diff: u32,
        only_air: bool,
    },
    /// FAWE rock/blob: distorted ellipsoid.
    Blob {
        pattern: Pattern,
        radius: u32,
        radii: [f64; 3],
        sphericity: f64,
        frequency: f64,
        amplitude: f64,
    },
    /// FAWE shatter: competing surface frontiers painting cracks.
    Shatter {
        pattern: Pattern,
        radius: u32,
        count: u32,
    },
    /// Arceon boulder (icosphere rock).
    Boulder {
        pattern: Pattern,
        size: (i32, i32),
        smoothness: i32,
        height: (i32, i32),
        width: (i32, i32),
        hollow: bool,
        falling: bool,
        angled: bool,
    },
    /// Arceon cube (rotated, hollow-able box).
    Cube {
        pattern: Pattern,
        size: (i32, i32),
        snap_angle: i32,
        height: (i32, i32),
        width: (i32, i32),
        hollow: bool,
        angled: bool,
        centered: bool,
    },
}

impl Brush {
    /// The largest horizontal reach of this brush (for clamps).
    #[must_use]
    pub fn radius(&self) -> u32 {
        match *self {
            Self::Sphere { radius, .. }
            | Self::Cylinder { radius, .. }
            | Self::Smooth { radius, .. }
            | Self::Erode { radius, .. }
            | Self::BlendBall { radius, .. }
            | Self::Blob { radius, .. }
            | Self::Shatter { radius, .. } => radius,
            Self::Boulder {
                size,
                height,
                width,
                ..
            }
            | Self::Cube {
                size,
                height,
                width,
                ..
            } => size.1.max(height.1).max(width.1).unsigned_abs(),
        }
    }

    /// Short description used in edit feedback.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Sphere { radius, hollow, .. } => format!(
                "Brush esfera r={radius}{}",
                if *hollow { " (oco)" } else { "" }
            ),
            Self::Cylinder {
                radius,
                height,
                hollow,
                ..
            } => format!(
                "Brush cilindro r={radius} h={height}{}",
                if *hollow { " (oco)" } else { "" }
            ),
            Self::Smooth {
                radius, iterations, ..
            } => format!("Brush smooth r={radius} x{iterations}"),
            Self::Erode {
                radius,
                erode_faces,
                erode_rec,
                fill_faces,
                fill_rec,
            } => format!(
                "Brush erode r={radius} ({erode_faces},{erode_rec},{fill_faces},{fill_rec})"
            ),
            Self::BlendBall { radius, .. } => format!("Brush blendball r={radius}"),
            Self::Blob { radius, .. } => format!("Brush blob r={radius}"),
            Self::Shatter { radius, count, .. } => {
                format!("Brush shatter r={radius} n={count}")
            }
            Self::Boulder { size, .. } => format!("Brush boulder size={}", size.1),
            Self::Cube { size, .. } => format!("Brush cube size={}", size.1),
        }
    }

    /// Runs the brush at `target`, applying the result through the
    /// worldedit engine (global mask, `//confirm`, history, feedback).
    #[allow(clippy::too_many_lines)] // dispatch table over every brush
    pub async fn run(&self, terrain: &Arc<TerrainState>, player: &Arc<Player>, target: BlockPos) {
        let world = player.world();
        let snapshot = crate::state::WeSnapshot::capture(&terrain.we, player.gameprofile.id);
        let yaw = f64::from(player.living_entity.entity.yaw.load());
        let pitch = f64::from(player.living_entity.entity.pitch.load());
        let description = self.label();
        let desired = match self {
            Self::Sphere {
                pattern,
                radius,
                hollow,
            } => crate::fawe_brushes::sphere(&snapshot, pattern, target, *radius, *hollow),
            Self::Cylinder {
                pattern,
                radius,
                height,
                hollow,
            } => {
                crate::fawe_brushes::cylinder(&snapshot, pattern, target, *radius, *height, *hollow)
            }
            Self::Smooth {
                radius,
                iterations,
                mask,
            } => {
                crate::fawe_brushes::smooth(
                    terrain,
                    &world,
                    target,
                    *radius,
                    *iterations,
                    mask.as_ref(),
                )
                .await
            }
            Self::Erode {
                radius,
                erode_faces,
                erode_rec,
                fill_faces,
                fill_rec,
            } => {
                crate::fawe_brushes::erode(
                    terrain,
                    &world,
                    target,
                    *radius,
                    *erode_faces,
                    *erode_rec,
                    *fill_faces,
                    *fill_rec,
                    false,
                )
                .await
            }
            Self::BlendBall {
                radius,
                min_freq_diff,
                only_air,
            } => {
                crate::fawe_brushes::blend_ball(
                    terrain,
                    &world,
                    target,
                    *radius,
                    *min_freq_diff,
                    *only_air,
                )
                .await
            }
            Self::Blob {
                pattern,
                radius,
                radii,
                sphericity,
                frequency,
                amplitude,
            } => crate::fawe_brushes::blob(
                &snapshot,
                pattern,
                target,
                *radius,
                *radii,
                *sphericity,
                *frequency,
                *amplitude,
            ),
            Self::Shatter {
                pattern,
                radius,
                count,
            } => {
                crate::fawe_brushes::shatter(
                    &snapshot, terrain, &world, pattern, target, *radius, *count,
                )
                .await
            }
            Self::Boulder {
                pattern,
                size,
                smoothness,
                height,
                width,
                hollow,
                falling,
                angled,
            } => {
                crate::arceon_brushes::boulder(
                    terrain,
                    &snapshot,
                    &world,
                    pattern,
                    target,
                    yaw,
                    pitch,
                    *size,
                    *smoothness,
                    *height,
                    *width,
                    *hollow,
                    *falling,
                    *angled,
                )
                .await
            }
            Self::Cube {
                pattern,
                size,
                snap_angle,
                height,
                width,
                hollow,
                angled,
                centered,
            } => crate::arceon_brushes::cube(
                &snapshot,
                pattern,
                target,
                yaw,
                pitch,
                *size,
                *snap_angle,
                *height,
                *width,
                *hollow,
                *angled,
                *centered,
            ),
        };

        let request = carrot_worldedit::edit::EditRequest {
            desired,
            mask: None,
            block_entities: HashMap::new(),
            description,
        };
        carrot_worldedit::edit::execute(&terrain.we, player, request).await;
    }
}
