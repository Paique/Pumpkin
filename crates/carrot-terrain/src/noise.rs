//! Seeded noise for terrain masks and `terragen`.
//!
//! The 3D simplex is a faithful port of `WorldEdit`'s `SimplexNoise`
//! (Gustavson's algorithm) so the FAWE blob brush keeps its exact
//! distortion; value, cellular and fractal layers follow the `FastNoise`
//! semantics the Arceon reference masks expect.

use std::hash::{Hash, Hasher};

/// Deterministic 64-bit seed for a world position and seed integer.
fn hash_seed(seed: i32, x: i64, y: i64, z: i64) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    seed.hash(&mut hasher);
    x.hash(&mut hasher);
    y.hash(&mut hasher);
    z.hash(&mut hasher);
    hasher.finish()
}

/// A seeded simplex noise source (2D and 3D samples).
pub struct Simplex {
    perm: Vec<u8>,
}

impl Simplex {
    /// Builds the permutation table from `seed`.
    #[must_use]
    pub fn new(seed: i64) -> Self {
        let mut base: Vec<u8> = (0..=255).collect();
        // xorshift-driven Fisher-Yates, deterministic per seed.
        let mut state = (seed as u64) | 1;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for index in (1..base.len()).rev() {
            let pick = (next() % (index as u64 + 1)) as usize;
            base.swap(index, pick);
        }
        let mut perm = base.clone();
        perm.extend_from_slice(&base);
        Self { perm }
    }

    const GRAD3: [[f64; 3]; 12] = [
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
        [1.0, -1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, 1.0, 1.0],
        [-1.0, 1.0, 1.0],
        [1.0, -1.0, 1.0],
        [-1.0, -1.0, 1.0],
        [1.0, 0.0, 1.0],
        [-1.0, 0.0, 1.0],
        [1.0, 0.0, -1.0],
        [-1.0, 0.0, -1.0],
    ];

    /// 3D simplex noise in roughly `[-1, 1]` (Gustavson's constants).
    #[must_use]
    pub fn noise3(&self, x: f64, y: f64, z: f64) -> f64 {
        const F3: f64 = 1.0 / 3.0;
        const G3: f64 = 1.0 / 6.0;
        let skew = (x + y + z) * F3;
        let ix = (x + skew).floor() as i64;
        let iy = (y + skew).floor() as i64;
        let iz = (z + skew).floor() as i64;
        let unskew = (ix + iy + iz) as f64 * G3;
        let x0 = x - (ix as f64 - unskew);
        let y0 = y - (iy as f64 - unskew);
        let z0 = z - (iz as f64 - unskew);

        let (i1, j1, k1, i2, j2, k2) = if x0 >= y0 {
            if y0 >= z0 {
                (1, 0, 0, 1, 1, 0)
            } else if x0 >= z0 {
                (1, 0, 0, 1, 0, 1)
            } else {
                (0, 0, 1, 1, 0, 1)
            }
        } else if y0 < z0 {
            (0, 0, 1, 0, 1, 1)
        } else if x0 < z0 {
            (0, 1, 0, 0, 1, 1)
        } else {
            (0, 1, 0, 1, 1, 0)
        };

        let x1 = x0 - i1 as f64 + G3;
        let y1 = y0 - j1 as f64 + G3;
        let z1 = z0 - k1 as f64 + G3;
        let x2 = x0 - i2 as f64 + 2.0 * G3;
        let y2 = y0 - j2 as f64 + 2.0 * G3;
        let z2 = z0 - k2 as f64 + 2.0 * G3;
        let x3 = x0 - 1.0 + 3.0 * G3;
        let y3 = y0 - 1.0 + 3.0 * G3;
        let z3 = z0 - 1.0 + 3.0 * G3;

        let corner = |gi: i64| {
            let index = gi.rem_euclid(256) as usize;
            usize::from(self.perm[index])
        };
        let ii = corner(ix);
        let jj = corner(iy);
        let kk = corner(iz);

        let mut total = 0.0;
        let mut contribute = |x: f64, y: f64, z: f64, hash: usize| {
            let t = 0.6 - x * x - y * y - z * z;
            if t > 0.0 {
                let grad = Self::GRAD3[hash % 12];
                let t2 = t * t;
                total += t2 * t2 * (grad[0] * x + grad[1] * y + grad[2] * z);
            }
        };
        contribute(x0, y0, z0, ii + jj + kk);
        contribute(x1, y1, z1, ii + i1 as usize + jj + k1 as usize + kk);
        contribute(x2, y2, z2, ii + i2 as usize + jj + k2 as usize + kk);
        contribute(x3, y3, z3, ii + 1 + jj + 1 + kk);
        32.0 * total
    }

    /// 2D simplex via the 3D sampler on the `z = 0` plane (matches how
    /// the reference drives column noise from two coordinates).
    #[must_use]
    pub fn noise2(&self, x: f64, y: f64) -> f64 {
        self.noise3(x, y, 0.0)
    }
}

/// Fractal composition of a noise source.
pub struct Fractal {
    source: Simplex,
    pub octaves: u32,
    pub gain: f64,
    pub lacunarity: f64,
    pub kind: FractalKind,
}

/// How octave samples combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FractalKind {
    /// Weighted sum (FBM).
    Fbm,
    /// `|sample| * 2 - 1` per octave (billowy).
    Billow,
    /// `1 - |sample|` per octave (ridged).
    Rigid,
}

impl Fractal {
    /// Standard reference defaults: gain 0.5, lacunarity 2.0.
    #[must_use]
    pub fn new(seed: i64, octaves: u32, kind: FractalKind) -> Self {
        Self {
            source: Simplex::new(seed),
            octaves: octaves.max(1),
            gain: 0.5,
            lacunarity: 2.0,
            kind,
        }
    }

    /// 2D fractal sample normalized back into `[-1, 1]`.
    #[must_use]
    pub fn noise2(&self, x: f64, y: f64) -> f64 {
        let mut total = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut norm = 0.0;
        for _ in 0..self.octaves {
            let sample = self.source.noise2(x * frequency, y * frequency);
            let sample = match self.kind {
                FractalKind::Fbm => sample,
                FractalKind::Billow => sample.abs() * 2.0 - 1.0,
                FractalKind::Rigid => 1.0 - sample.abs() * 2.0,
            };
            total += sample * amplitude;
            norm += amplitude;
            amplitude *= self.gain;
            frequency *= self.lacunarity;
        }
        if norm > 0.0 { total / norm } else { total }
    }

    /// 3D fractal sample normalized back into `[-1, 1]`.
    #[must_use]
    pub fn noise3(&self, x: f64, y: f64, z: f64) -> f64 {
        let mut total = 0.0;
        let mut amplitude = 1.0;
        let mut frequency = 1.0;
        let mut norm = 0.0;
        for _ in 0..self.octaves {
            let sample = self
                .source
                .noise3(x * frequency, y * frequency, z * frequency);
            let sample = match self.kind {
                FractalKind::Fbm => sample,
                FractalKind::Billow => sample.abs() * 2.0 - 1.0,
                FractalKind::Rigid => 1.0 - sample.abs() * 2.0,
            };
            total += sample * amplitude;
            norm += amplitude;
            amplitude *= self.gain;
            frequency *= self.lacunarity;
        }
        if norm > 0.0 { total / norm } else { total }
    }
}

/// How distances to a cell's feature points are measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellDistance {
    /// Straight-line distance.
    Euclidean,
    /// Axis-summed distance.
    Manhattan,
    /// Mix of both (natural).
    Natural,
}

/// What a cellular sample returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellReturn {
    /// Distance to the closest point.
    Distance,
    /// Distance to the second closest point.
    Distance2,
    /// `d1 + d2`.
    Distance2Add,
    /// `d2 - d1`.
    Distance2Sub,
    /// `d1 * d2`.
    Distance2Mul,
    /// `d1 / d2` scaled to `[-1, 1]` (edges approach 0).
    Distance2Div,
    /// Per-cell constant hash value.
    CellValue,
}

/// Jittered-grid (Voronoi) cellular noise.
pub struct Cellular {
    pub seed: i32,
    pub frequency: f64,
    pub distance: CellDistance,
    pub return_type: CellReturn,
    pub jitter: f64,
}

impl Cellular {
    /// Reference defaults: Euclidean, `Distance`, jitter 1.0.
    #[must_use]
    pub const fn new(seed: i32) -> Self {
        Self {
            seed,
            frequency: 0.01,
            distance: CellDistance::Euclidean,
            return_type: CellReturn::Distance,
            jitter: 1.0,
        }
    }

    /// Raw `(d1, d2, cell_value)` in jittered space around `(x, y, z)`.
    #[must_use]
    pub fn raw(&self, x: f64, y: f64, z: f64) -> (f64, f64, f64) {
        let sx = x * self.frequency;
        let sy = y * self.frequency;
        let sz = z * self.frequency;
        let cx = sx.floor();
        let cy = sy.floor();
        let cz = sz.floor();

        let mut d1 = f64::INFINITY;
        let mut d2 = f64::INFINITY;
        let mut value = 0.0;
        for ox in -1..=1 {
            for oy in -1..=1 {
                for oz in -1..=1 {
                    let nx = cx + ox as f64;
                    let ny = cy + oy as f64;
                    let nz = cz + oz as f64;
                    let hash = hash_seed(self.seed, nx as i64, ny as i64, nz as i64);
                    // Feature point jittered inside the cell by hash.
                    let unit = |bits: u64| ((bits & 0xFFFF_FFFF) as f64) / 65536.0 / 65536.0;
                    let fx = nx + (unit(hash) - 0.5) * self.jitter;
                    let fy = ny + (unit(hash >> 32) - 0.5) * self.jitter;
                    let fz = nz + (unit(hash ^ 0x9E37_79B9_7F4A_7C15) - 0.5) * self.jitter;
                    let dx = fx - sx;
                    let dy = fy - sy;
                    let dz = fz - sz;
                    let dist = match self.distance {
                        CellDistance::Euclidean => (dx * dx + dy * dy + dz * dz).sqrt(),
                        CellDistance::Manhattan => dx.abs() + dy.abs() + dz.abs(),
                        CellDistance::Natural => {
                            let euclid = (dx * dx + dy * dy + dz * dz).sqrt();
                            (euclid + dx.abs() + dy.abs() + dz.abs()) * 0.5
                        }
                    };
                    if dist < d1 {
                        d2 = d1;
                        d1 = dist;
                        value = unit(hash.rotate_left(17)) * 2.0 - 1.0;
                    } else if dist < d2 {
                        d2 = dist;
                    }
                }
            }
        }
        (d1, d2, value)
    }

    /// Cellular sample roughly in `[-1, 1]`.
    #[must_use]
    pub fn noise(&self, x: f64, y: f64, z: f64) -> f64 {
        let (d1, d2, value) = self.raw(x, y, z);
        // Distances live in scaled space where cells span about one
        // unit, so clamping keeps the reference's [0, 1] band.
        let (a, b) = (d1.clamp(0.0, 1.0), d2.clamp(0.0, 1.0));
        match self.return_type {
            CellReturn::Distance => a * 2.0 - 1.0,
            CellReturn::Distance2 => b * 2.0 - 1.0,
            CellReturn::Distance2Add => (a + b).min(1.5) / 1.5 * 2.0 - 1.0,
            CellReturn::Distance2Sub => (b - a) * 2.0 - 1.0,
            CellReturn::Distance2Mul => (a * b) * 2.0 - 1.0,
            CellReturn::Distance2Div => {
                if b > 0.0 {
                    (a / b) * 2.0 - 1.0
                } else {
                    -1.0
                }
            }
            CellReturn::CellValue => value,
        }
    }
}

/// Hash-based white noise in `[-1, 1]` for a lattice point.
#[must_use]
pub fn white_noise(seed: i32, x: i64, y: i64, z: i64) -> f64 {
    let hash = hash_seed(seed, x, y, z);
    ((hash >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}

/// Smooth-interpolated value noise in `[-1, 1]` (quintic fade).
#[must_use]
pub fn value_noise(seed: i32, x: f64, y: f64, z: f64) -> f64 {
    let fade = |t: f64| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let x0 = x.floor();
    let y0 = y.floor();
    let z0 = z.floor();
    let tx = fade(x - x0);
    let ty = fade(y - y0);
    let tz = fade(z - z0);
    let lerp = |a: f64, b: f64, t: f64| a + (b - a) * t;

    let c000 = white_noise(seed, x0 as i64, y0 as i64, z0 as i64);
    let c100 = white_noise(seed, x0 as i64 + 1, y0 as i64, z0 as i64);
    let c010 = white_noise(seed, x0 as i64, y0 as i64 + 1, z0 as i64);
    let c110 = white_noise(seed, x0 as i64 + 1, y0 as i64 + 1, z0 as i64);
    let c001 = white_noise(seed, x0 as i64, y0 as i64, z0 as i64 + 1);
    let c101 = white_noise(seed, x0 as i64 + 1, y0 as i64, z0 as i64 + 1);
    let c011 = white_noise(seed, x0 as i64, y0 as i64 + 1, z0 as i64 + 1);
    let c111 = white_noise(seed, x0 as i64 + 1, y0 as i64 + 1, z0 as i64 + 1);

    lerp(
        lerp(lerp(c000, c100, tx), lerp(c010, c110, tx), ty),
        lerp(lerp(c001, c101, tx), lerp(c011, c111, tx), ty),
        tz,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simplex_is_deterministic_and_bounded() {
        let noise = Simplex::new(42);
        let a = noise.noise3(1.51, 2.37, 3.14);
        let b = noise.noise3(1.51, 2.37, 3.14);
        assert_eq!(a, b);
        assert!((-1.5..=1.5).contains(&a));
    }

    #[test]
    fn different_seeds_differ() {
        let a = Simplex::new(1).noise3(0.37, 1.42, 2.9);
        let b = Simplex::new(2).noise3(0.37, 1.42, 2.9);
        assert_ne!(a, b);
    }

    #[test]
    fn cellular_edges_score_high() {
        // Distance2Div approaches 0 (high) near cell borders.
        let mut cell = Cellular::new(7);
        cell.frequency = 0.05;
        cell.return_type = CellReturn::Distance2Div;
        let mut best: f64 = -2.0;
        let mut worst: f64 = 2.0;
        for step in 0..400 {
            let v = cell.noise(f64::from(step) * 0.01, 0.0, 0.0);
            best = best.max(v);
            worst = worst.min(v);
        }
        assert!(best > worst);
        assert!(best <= 1.0 && worst >= -1.0);
    }

    #[test]
    fn fractal_stays_bounded() {
        for kind in [FractalKind::Fbm, FractalKind::Billow, FractalKind::Rigid] {
            let fractal = Fractal::new(3, 5, kind);
            for step in 0..64 {
                let v = fractal.noise2(f64::from(step) * 0.13, f64::from(step) * -0.07);
                assert!((-1.2..=1.2).contains(&v), "{kind:?} out of range: {v}");
            }
        }
    }
}
