//! Arceon terrain masks registered into the worldedit mask parser.
//!
//! `#arcangle`, `#vor`, `#noise`, `#fractal`, `#turbulence`, `#crack`
//! and `#electric`, ported from the decompiled reference
//! (`AngleMask`, `VoronoiMask`, `NoiseMask`, `FractalMask`,
//! `TurbulenceMask`, `CrackMask`, `ElectricMask`).

use std::sync::Arc;

use carrot_worldedit::mask::{WorldMask, register_world_mask};
use pumpkin_core::world::World;
use pumpkin_util::math::position::BlockPos;

use crate::noise::{CellDistance, CellReturn, Cellular, Fractal, FractalKind, Simplex};
use crate::util;

/// Registers every terrain mask factory (idempotent; re-registration
/// replaces, so module re-enable refreshes closures).
#[allow(clippy::too_many_lines)] // one closure per mask
pub fn register() {
    register_world_mask(
        "arcangle",
        Arc::new(|args| {
            let (min, max) = match args {
                [min, max, ..] => (parse_deg(min, "min")?, parse_deg(max, "max")?),
                _ => return Err("#arcangle[min][max] exige dois ângulos".to_string()),
            };
            let mut size = 1;
            let mut overlay = false;
            for flag in args.iter().skip(2) {
                match *flag {
                    "-o" => overlay = true,
                    other => {
                        size = other
                            .parse()
                            .map_err(|_| format!("tamanho inválido: {other}"))?;
                        if size != 1 && size != 2 {
                            return Err("size deve ser 1 ou 2".to_string());
                        }
                    }
                }
            }
            Ok(Arc::new(AngleMask {
                min,
                max,
                size,
                overlay,
            }) as Arc<dyn WorldMask>)
        }),
    );

    let voronoi: carrot_worldedit::mask::WorldMaskFactory =
        Arc::new(|args: &[&str]| -> Result<Arc<dyn WorldMask>, String> {
            let mut zoom = 1.0f64;
            let mut jitter = 70i32;
            if let Some(value) = args.first() {
                zoom = value
                    .parse()
                    .map_err(|_| format!("zoom inválido: {value}"))?;
            }
            if let Some(value) = args.get(1) {
                jitter = value
                    .parse()
                    .map_err(|_| format!("jitter inválido: {value}"))?;
            }
            Ok(Arc::new(VoronoiMask {
                zoom,
                jitter,
                seed: util::random_seed(),
            }))
        });
    register_world_mask("vor", Arc::clone(&voronoi));
    register_world_mask("voronoi", voronoi);

    register_world_mask(
        "noise",
        Arc::new(|args| noise_mask(args).map(|mask| Arc::new(mask) as Arc<dyn WorldMask>)),
    );

    let fractal: carrot_worldedit::mask::WorldMaskFactory =
        Arc::new(|args: &[&str]| -> Result<Arc<dyn WorldMask>, String> {
            let (zoom, coverage) = zoom_coverage(args)?;
            Ok(Arc::new(FractalMask {
                zoom,
                coverage,
                seed: util::random_seed(),
            }))
        });
    register_world_mask("fractal", Arc::clone(&fractal));
    register_world_mask("frac", fractal);

    let turbulence: carrot_worldedit::mask::WorldMaskFactory =
        Arc::new(|args: &[&str]| -> Result<Arc<dyn WorldMask>, String> {
            let (zoom, coverage) = zoom_coverage(args)?;
            Ok(Arc::new(TurbulenceMask {
                zoom,
                coverage,
                seed: util::random_seed(),
            }))
        });
    register_world_mask("turbulence", Arc::clone(&turbulence));
    register_world_mask("turb", turbulence);

    register_world_mask(
        "crack",
        Arc::new(|args| {
            let (zoom, coverage) = zoom_coverage(args)?;
            let jitter = match args.get(2) {
                Some(value) => value
                    .parse::<i32>()
                    .map_err(|_| format!("jitter inválido: {value}"))?,
                None => 70,
            };
            Ok(Arc::new(CrackMask {
                zoom,
                coverage,
                jitter,
                seed: util::random_seed(),
            }) as Arc<dyn WorldMask>)
        }),
    );

    let electric: carrot_worldedit::mask::WorldMaskFactory =
        Arc::new(|args: &[&str]| -> Result<Arc<dyn WorldMask>, String> {
            let (zoom, coverage) = zoom_coverage(args)?;
            Ok(Arc::new(ElectricMask {
                zoom,
                coverage,
                seed: util::random_seed(),
            }))
        });
    register_world_mask("electric", Arc::clone(&electric));
    register_world_mask("elec", electric);
}

fn parse_deg(value: &str, label: &str) -> Result<f64, String> {
    value
        .parse::<f64>()
        .map_err(|_| format!("ângulo {label} inválido: {value}"))
}

fn zoom_coverage(args: &[&str]) -> Result<(f64, f64), String> {
    let mut zoom = 1.0;
    let mut coverage = 50.0;
    if let Some(value) = args.first() {
        zoom = value
            .parse()
            .map_err(|_| format!("zoom inválido: {value}"))?;
    }
    if let Some(value) = args.get(1) {
        coverage = value
            .parse()
            .map_err(|_| format!("cobertura inválida: {value}"))?;
    }
    Ok((zoom, coverage))
}

/// The reference noise threshold: coverage 50 -> 0.5, 0 -> 0.2, 100 -> 0.8.
fn threshold(coverage: f64) -> f64 {
    (100.0 - coverage) / 100.0 * 0.6 + 0.2
}

/// `#arcangle[min][max][size][-o]` — normal-based surface angle band.
struct AngleMask {
    min: f64,
    max: f64,
    size: i32,
    overlay: bool,
}

/// The six cardinal offsets.
const CARDINAL: [(i32, i32, i32); 6] = [
    (1, 0, 0),
    (-1, 0, 0),
    (0, 1, 0),
    (0, -1, 0),
    (0, 0, 1),
    (0, 0, -1),
];

impl WorldMask for AngleMask {
    fn test(&self, world: &World, pos: &BlockPos) -> bool {
        if util::is_air_at(world, pos) {
            return false;
        }
        let mut exposed = false;
        for (dx, dy, dz) in CARDINAL {
            let neighbor = BlockPos::new(pos.0.x + dx, pos.0.y + dy, pos.0.z + dz);
            if util::is_air_at(world, &neighbor) {
                exposed = true;
                break;
            }
        }
        if !exposed {
            return false;
        }

        let mut sum = (0.0f64, 0.0f64, 0.0f64);
        let mut count = 0;
        for ox in -self.size..=self.size {
            for oy in -self.size..=self.size {
                for oz in -self.size..=self.size {
                    let sample = BlockPos::new(pos.0.x + ox, pos.0.y + oy, pos.0.z + oz);
                    if util::is_air_at(world, &sample) {
                        sum.0 += f64::from(ox);
                        sum.1 += f64::from(oy);
                        sum.2 += f64::from(oz);
                        count += 1;
                    }
                }
            }
        }
        if count == 0 {
            return false;
        }
        let avg = (
            sum.0 / f64::from(count),
            sum.1 / f64::from(count),
            sum.2 / f64::from(count),
        );
        let length = (avg.0 * avg.0 + avg.1 * avg.1 + avg.2 * avg.2).sqrt();
        let (nx, ny, nz) = if length.is_nan() || length < f64::EPSILON {
            (0.0, 0.0, 0.0)
        } else {
            (avg.0 / length, avg.1 / length, avg.2 / length)
        };
        let angle = 1.0 - ny;
        let _ = (nx, nz);
        let mut ok = self.min / 90.0 <= angle && angle <= self.max / 90.0;
        if self.overlay {
            let above = BlockPos::new(pos.0.x, pos.0.y + 1, pos.0.z);
            ok = ok && util::is_air_at(world, &above);
        }
        ok
    }
}

/// `#vor[zoom][jitter]` — cellular web of cell borders (one seed drawn
/// when the mask is parsed, like the reference).
struct VoronoiMask {
    zoom: f64,
    jitter: i32,
    seed: i64,
}

impl WorldMask for VoronoiMask {
    fn test(&self, _world: &World, pos: &BlockPos) -> bool {
        let mut cellular = Cellular::new(self.seed as i32);
        cellular.frequency = self.zoom * 0.05;
        cellular.distance = CellDistance::Euclidean;
        cellular.return_type = CellReturn::Distance2Div;
        cellular.jitter = 0.3 + f64::from(self.jitter) / 100.0;
        let modifier = (self.zoom.sqrt() * -0.3).clamp(-0.4, -0.15);
        cellular.noise(f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z)) >= modifier
    }
}

/// `#noise[seed][type][coverage][scale][frequency]`.
enum NoiseKind {
    Simplex,
    SimplexFractal,
    White,
    Value,
    Cellular,
}

struct NoiseMask {
    kind: NoiseKind,
    coverage: f64,
    scale: f64,
    frequency: f64,
    octaves: u32,
    seed: i64,
}

fn noise_mask(args: &[&str]) -> Result<NoiseMask, String> {
    let mut seed: i64 = util::random_seed();
    let mut kind = NoiseKind::Simplex;
    let mut coverage = 50.0;
    let mut scale = 1.0;
    let mut frequency = 0.01;
    let mut octaves = 3;
    if let Some(value) = args.first()
        && let Ok(parsed) = value.parse::<i64>()
    {
        seed = parsed;
    }
    if let Some(kind_name) = args.get(1) {
        kind = match kind_name.to_ascii_lowercase().as_str() {
            "simplex" => NoiseKind::Simplex,
            "simplexfractal" => NoiseKind::SimplexFractal,
            "whitenoise" | "white" => NoiseKind::White,
            "value" => NoiseKind::Value,
            "cellular" => NoiseKind::Cellular,
            other => return Err(format!("tipo de ruído desconhecido: {other}")),
        };
    }
    if let Some(value) = args.get(2) {
        coverage = value
            .parse()
            .map_err(|_| format!("cobertura inválida: {value}"))?;
    }
    if let Some(value) = args.get(3) {
        scale = value
            .parse()
            .map_err(|_| format!("escala inválida: {value}"))?;
    }
    if let Some(value) = args.get(4) {
        frequency = value
            .parse()
            .map_err(|_| format!("frequência inválida: {value}"))?;
    }
    if let Some(value) = args.get(5) {
        octaves = value
            .parse()
            .map_err(|_| format!("octaves inválidas: {value}"))?;
    }
    Ok(NoiseMask {
        kind,
        coverage,
        scale,
        frequency,
        octaves,
        seed,
    })
}

impl WorldMask for NoiseMask {
    fn test(&self, _world: &World, pos: &BlockPos) -> bool {
        let (x, y, z) = (
            f64::from(pos.0.x) * self.scale,
            f64::from(pos.0.y) * self.scale,
            f64::from(pos.0.z) * self.scale,
        );
        let sample = match self.kind {
            NoiseKind::Simplex => Simplex::new(self.seed).noise3(
                x * self.frequency,
                y * self.frequency,
                z * self.frequency,
            ),
            NoiseKind::SimplexFractal => Fractal::new(self.seed, self.octaves, FractalKind::Fbm)
                .noise3(x * self.frequency, y * self.frequency, z * self.frequency),
            NoiseKind::White => crate::noise::white_noise(
                self.seed as i32,
                pos.0.x as i64,
                pos.0.y as i64,
                pos.0.z as i64,
            ),
            NoiseKind::Value => crate::noise::value_noise(
                self.seed as i32,
                x * self.frequency,
                y * self.frequency,
                z * self.frequency,
            ),
            NoiseKind::Cellular => {
                let mut cellular = Cellular::new(self.seed as i32);
                cellular.frequency = self.frequency;
                cellular.noise(x, y, z)
            }
        };
        let normalized = sample.midpoint(1.0);
        normalized >= threshold(self.coverage)
    }
}

/// `#fractal[zoom][coverage]` — 5-octave FBM coverage.
struct FractalMask {
    zoom: f64,
    coverage: f64,
    seed: i64,
}

impl WorldMask for FractalMask {
    fn test(&self, _world: &World, pos: &BlockPos) -> bool {
        let fractal = Fractal::new(self.seed, 5, FractalKind::Fbm);
        let frequency = self.zoom * 0.05;
        let sample = fractal.noise2(
            f64::from(pos.0.x) * frequency,
            f64::from(pos.0.z) * frequency,
        );
        sample.midpoint(1.0) >= threshold(self.coverage)
    }
}

/// `#turbulence[zoom][coverage]` — inverted 6-octave warped FBM.
struct TurbulenceMask {
    zoom: f64,
    coverage: f64,
    seed: i64,
}

impl WorldMask for TurbulenceMask {
    fn test(&self, _world: &World, pos: &BlockPos) -> bool {
        let fractal = Fractal::new(self.seed, 6, FractalKind::Fbm);
        let frequency = self.zoom * 0.05;
        let sample = fractal.noise2(
            f64::from(pos.0.x) * frequency,
            f64::from(pos.0.z) * frequency,
        );
        sample.midpoint(1.0) <= 1.0 - threshold(self.coverage)
    }
}

/// `#crack[zoom][coverage][jitter]` — voronoi crack lines.
struct CrackMask {
    zoom: f64,
    coverage: f64,
    jitter: i32,
    seed: i64,
}

impl WorldMask for CrackMask {
    fn test(&self, _world: &World, pos: &BlockPos) -> bool {
        let seed = self.seed;
        let mut edges = Cellular::new(seed as i32);
        edges.frequency = self.zoom * 0.04;
        edges.distance = CellDistance::Natural;
        edges.return_type = CellReturn::Distance2Div;
        edges.jitter = 0.3 + f64::from(self.jitter) / 100.0;
        let edge_sample = edges.noise(f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z));
        let first = edge_sample.midpoint(1.0) >= 0.45;
        if !first {
            return false;
        }
        let mut cells = Cellular::new(seed as i32 + 1);
        cells.frequency = self.zoom * 0.03;
        cells.return_type = CellReturn::CellValue;
        let cell_sample = cells.noise(f64::from(pos.0.x), f64::from(pos.0.y), f64::from(pos.0.z));
        cell_sample.midpoint(1.0) > threshold(self.coverage)
    }
}

/// `#electric[zoom][coverage]` — inverted billowy fractal veins.
struct ElectricMask {
    zoom: f64,
    coverage: f64,
    seed: i64,
}

impl WorldMask for ElectricMask {
    fn test(&self, _world: &World, pos: &BlockPos) -> bool {
        let fractal = Fractal::new(self.seed, 8, FractalKind::Billow);
        let frequency = self.zoom * 0.02;
        let sample = fractal.noise3(
            f64::from(pos.0.x) * frequency,
            f64::from(pos.0.y) * frequency,
            f64::from(pos.0.z) * frequency,
        );
        let min = (100.0 - self.coverage * 0.65) / 100.0;
        sample.midpoint(1.0) <= 1.0 - min
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn threshold_maps_coverage() {
        assert!((threshold(50.0) - 0.5).abs() < 1e-9);
        assert!((threshold(0.0) - 0.8).abs() < 1e-9);
        assert!((threshold(100.0) - 0.2).abs() < 1e-9);
    }

    #[test]
    fn registered_masks_parse_through_worldedit() {
        register();
        // `#vor` parses into a world mask via the registry.
        let parsed = carrot_worldedit::mask::Mask::parse("#vor[1.0][70]", None);
        assert!(matches!(parsed, Ok(carrot_worldedit::mask::Mask::World(_))));
        // Unknown #names still fail.
        assert!(carrot_worldedit::mask::Mask::parse("#nope[1]", None).is_err());
        // Bad arcangle args surface the Portuguese error.
        assert!(carrot_worldedit::mask::Mask::parse("#arcangle[x][y]", None).is_err());
    }
}
