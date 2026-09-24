//! Procedural test meshes: a noise-displaced cube-sphere "asteroid" with welded seams.

use std::collections::HashMap;

use forge_core::Seed;
use forge_core::hash::{hash_cell3, unit_f32};
use glam::Vec3;

/// An indexed triangle mesh with per-vertex normals.
#[derive(Clone, Debug, Default)]
pub struct TriMesh {
    /// Positions.
    pub positions: Vec<[f32; 3]>,
    /// Unit normals.
    pub normals: Vec<[f32; 3]>,
    /// Triangle list, counter-clockwise front faces.
    pub indices: Vec<u32>,
}

impl TriMesh {
    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Recomputes area-weighted vertex normals from the faces.
    pub fn recompute_normals(&mut self) {
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for tri in self.indices.as_chunks::<3>().0 {
            let a = Vec3::from(self.positions[tri[0] as usize]);
            let b = Vec3::from(self.positions[tri[1] as usize]);
            let c = Vec3::from(self.positions[tri[2] as usize]);
            let n = (b - a).cross(c - a);
            for &i in tri {
                normals[i as usize] += n;
            }
        }
        self.normals = normals
            .into_iter()
            .map(|n| n.normalize_or_zero().to_array())
            .collect();
    }
}

/// Trilinear value noise in `[-1, 1]` from the engine's cell hash.
fn value_noise(seed: u64, p: Vec3) -> f32 {
    let base = p.floor();
    let f = p - base;
    let s = f * f * (3.0 - 2.0 * f);
    let (x, y, z) = (base.x as i32, base.y as i32, base.z as i32);
    let v =
        |dx: i32, dy: i32, dz: i32| unit_f32(hash_cell3(seed, x + dx, y + dy, z + dz)) * 2.0 - 1.0;
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(v(0, 0, 0), v(1, 0, 0), s.x);
    let x10 = lerp(v(0, 1, 0), v(1, 1, 0), s.x);
    let x01 = lerp(v(0, 0, 1), v(1, 0, 1), s.x);
    let x11 = lerp(v(0, 1, 1), v(1, 1, 1), s.x);
    let y0 = lerp(x00, x10, s.y);
    let y1 = lerp(x01, x11, s.y);
    lerp(y0, y1, s.z)
}

pub(crate) fn fbm(seed: u64, mut p: Vec3, octaves: u32) -> f32 {
    let mut amplitude = 0.5;
    let mut sum = 0.0;
    let mut norm = 0.0;
    for octave in 0..octaves {
        sum += amplitude * value_noise(seed.wrapping_add(u64::from(octave) * 0x9E37_79B9), p);
        norm += amplitude;
        amplitude *= 0.5;
        p = p * 2.03 + Vec3::splat(17.1);
    }
    sum / norm
}

/// A cube-sphere of `segments × segments` quads per face, displaced by fractal noise.
///
/// `roughness` scales the displacement relative to `radius`. Seams between faces are welded
/// so normals are continuous; triangle count is `6 × segments² × 2`.
pub fn asteroid(seed: Seed, segments: u32, radius: f32, roughness: f32) -> TriMesh {
    let segments = segments.max(1);
    let noise_seed = seed.derive_str("asteroid").value();
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut welded: HashMap<[i32; 3], u32> = HashMap::new();

    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::Z, Vec3::NEG_X),
        (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    let quantize = |p: Vec3| {
        [
            (p.x * 1e4).round() as i32,
            (p.y * 1e4).round() as i32,
            (p.z * 1e4).round() as i32,
        ]
    };
    for (normal, up, right) in faces {
        let mut grid: Vec<u32> = Vec::with_capacity(((segments + 1) * (segments + 1)) as usize);
        for j in 0..=segments {
            for i in 0..=segments {
                let u = i as f32 / segments as f32 * 2.0 - 1.0;
                let v = j as f32 / segments as f32 * 2.0 - 1.0;
                let cube = normal + right * u + up * v;
                let dir = cube.normalize();
                let key = quantize(dir);
                let index = *welded.entry(key).or_insert_with(|| {
                    let n = fbm(noise_seed, dir * 3.0, 5);
                    let n2 = fbm(noise_seed ^ 0x55, dir * 11.0, 3);
                    let r = radius * (1.0 + roughness * (n + 0.35 * n2));
                    positions.push((dir * r).to_array());
                    (positions.len() - 1) as u32
                });
                grid.push(index);
            }
        }
        let stride = segments + 1;
        for j in 0..segments {
            for i in 0..segments {
                let a = grid[(j * stride + i) as usize];
                let b = grid[(j * stride + i + 1) as usize];
                let c = grid[((j + 1) * stride + i) as usize];
                let d = grid[((j + 1) * stride + i + 1) as usize];
                // Two counter-clockwise triangles seen from outside.
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    let mut mesh = TriMesh {
        positions,
        normals: Vec::new(),
        indices,
    };
    mesh.recompute_normals();
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asteroid_is_closed_and_deterministic() {
        let a = asteroid(Seed::new(1), 8, 1.0, 0.3);
        let b = asteroid(Seed::new(1), 8, 1.0, 0.3);
        assert_eq!(a.positions, b.positions);
        assert_eq!(a.triangle_count(), 6 * 8 * 8 * 2);
        // A closed genus-0 mesh: V - E + F = 2, with E = 3F/2.
        let f = a.triangle_count() as i64;
        let v = a.positions.len() as i64;
        assert_eq!(v - 3 * f / 2 + f, 2);
        for n in &a.normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((len - 1.0).abs() < 1e-3);
        }
    }
}

#[cfg(test)]
mod fold_tests {
    use super::*;

    /// Triangles whose geometric normal points into the body (folds of the displacement).
    fn folded_triangles(mesh: &TriMesh) -> usize {
        let centre = mesh
            .positions
            .iter()
            .fold(Vec3::ZERO, |s, p| s + Vec3::from(*p))
            / mesh.positions.len() as f32;
        mesh.indices
            .as_chunks::<3>()
            .0
            .iter()
            .filter(|tri| {
                let a = Vec3::from(mesh.positions[tri[0] as usize]);
                let b = Vec3::from(mesh.positions[tri[1] as usize]);
                let c = Vec3::from(mesh.positions[tri[2] as usize]);
                let n = (b - a).cross(c - a);
                n.dot((a + b + c) / 3.0 - centre) < 0.0
            })
            .count()
    }

    #[test]
    fn asteroids_do_not_fold() {
        let recipes: [(u32, f32, f32); 7] = [
            (48, 1.0, 0.45),
            (64, 1.8, 0.40),
            (72, 2.6, 0.35),
            (96, 4.0, 0.30),
            (128, 7.0, 0.28),
            (160, 14.0, 0.25),
            (192, 30.0, 0.22),
        ];
        let mut total = 0;
        for (i, (segments, radius, roughness)) in recipes.iter().enumerate() {
            let mesh = asteroid(Seed::new(700 + i as u64), *segments, *radius, *roughness);
            let folds = folded_triangles(&mesh);
            eprintln!(
                "recipe {i}: {segments} segments, roughness {roughness}: {folds} / {} folded",
                mesh.triangle_count()
            );
            total += folds;
        }
        assert_eq!(
            total, 0,
            "folded triangles make holes and defeat cone culling"
        );
    }
}
