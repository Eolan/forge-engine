//! The island's stones (#130): rocks shaped by what they are made of. Granite weathers along its
//! joints to rounded corestones, which stand stacked in tors and split off domes in slabs;
//! limestone breaks along its bedding and its joints into angular blocks and flags.
//!
//! Each piece is a cube-sphere of `segments × segments` quads per face, welded at the seams like
//! [`crate::procedural::asteroid`], pushed out to a superellipsoid (rounder or squarer) and
//! roughened by noise; a block is then cut by planes. A stone stands on the ground at the
//! origin: its lowest point a tenth of its height under `y = 0`.

use std::collections::HashMap;

use forge_core::Seed;
use glam::Vec3;

use crate::procedural::{TriMesh, fbm};

/// What a stone is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoneShape {
    /// A granite corestone: a block of the joints, weathered round.
    Corestone,
    /// A granite tor: two or three corestones stacked, smaller up the stack, a little off
    /// each other's middle.
    Tor,
    /// A granite slab: a sheet split off a dome, thin and rounded at its edges.
    Slab,
    /// A limestone block: flat bedding faces top and bottom (dipping a little), broken faces
    /// round it meeting at sharp edges, pitted by solution. Thin, it is a flag.
    Block,
}

/// A stone (see [`stone`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Stone {
    /// Seed of its proportions and its noise.
    pub seed: u64,
    /// What it is.
    pub shape: StoneShape,
    /// Half its length, height and width, metres (a tor's: its foot stone's).
    pub size: [f32; 3],
    /// Quads per cube-face side of each piece (12 × segments² triangles a piece).
    pub segments: u32,
}

/// The stone `s`, standing on the ground at the origin.
pub fn stone(s: &Stone) -> TriMesh {
    let root = Seed::new(s.seed);
    let size = Vec3::from_array(s.size);
    let mut mesh = TriMesh::default();
    match s.shape {
        StoneShape::Corestone => {
            append(&mut mesh, rounded(root, size, 3.0, s.segments), Vec3::ZERO)
        }
        StoneShape::Slab => append(&mut mesh, rounded(root, size, 2.4, s.segments), Vec3::ZERO),
        StoneShape::Block => append(&mut mesh, block(root, size, s.segments), Vec3::ZERO),
        StoneShape::Tor => {
            let mut rng = root.derive_str("tor").rng();
            let pieces = 2 + u32::from(rng.next_f32() < 0.5);
            let mut piece_size = size;
            let mut top = 0.0;
            for k in 0..pieces {
                let piece = rounded(root.derive(u64::from(k)), piece_size, 2.8, s.segments);
                let (low, high) = height_range(&piece);
                // Each stone sinks a little into the one under it.
                let sink = if k == 0 { 0.0 } else { 0.15 * (high - low) };
                let offset = Vec3::new(
                    (rng.next_f32() - 0.5) * 0.4 * piece_size.x,
                    top - low - sink,
                    (rng.next_f32() - 0.5) * 0.4 * piece_size.z,
                );
                top = offset.y + high;
                append(&mut mesh, piece, offset);
                piece_size *= Vec3::new(
                    0.6 + 0.25 * rng.next_f32(),
                    0.7 + 0.25 * rng.next_f32(),
                    0.6 + 0.25 * rng.next_f32(),
                );
            }
        }
    }
    // On the ground: the lowest point a tenth of the height under it.
    let (low, high) = height_range(&mesh);
    let lift = -low - 0.1 * (high - low);
    for p in &mut mesh.positions {
        p[1] += lift;
    }
    mesh.recompute_normals();
    mesh
}

/// The lowest and highest `y` of `mesh`.
fn height_range(mesh: &TriMesh) -> (f32, f32) {
    mesh.positions
        .iter()
        .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        })
}

/// `piece` moved by `offset` and added to `mesh`.
fn append(mesh: &mut TriMesh, piece: TriMesh, offset: Vec3) {
    let base = mesh.positions.len() as u32;
    mesh.positions.extend(
        piece
            .positions
            .iter()
            .map(|p| (Vec3::from_array(*p) + offset).to_array()),
    );
    mesh.indices.extend(piece.indices.iter().map(|i| i + base));
}

/// A welded cube-sphere of `segments × segments` quads per face, each vertex at `at(direction)`.
fn cube_sphere(segments: u32, mut at: impl FnMut(Vec3) -> Vec3) -> TriMesh {
    let segments = segments.max(1);
    let faces: [(Vec3, Vec3, Vec3); 6] = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::Z, Vec3::NEG_X),
        (Vec3::Z, Vec3::Y, Vec3::NEG_X),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut welded: HashMap<[i32; 3], u32> = HashMap::new();
    for (normal, up, right) in faces {
        let mut grid = Vec::with_capacity(((segments + 1) * (segments + 1)) as usize);
        for j in 0..=segments {
            for i in 0..=segments {
                let u = i as f32 / segments as f32 * 2.0 - 1.0;
                let v = j as f32 / segments as f32 * 2.0 - 1.0;
                let dir = (normal + right * u + up * v).normalize();
                let key = [
                    (dir.x * 1e4).round() as i32,
                    (dir.y * 1e4).round() as i32,
                    (dir.z * 1e4).round() as i32,
                ];
                let index = *welded.entry(key).or_insert_with(|| {
                    positions.push(at(dir).to_array());
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
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
    }
    TriMesh {
        positions,
        normals: Vec::new(),
        indices,
        sections: Vec::new(),
    }
}

/// The distance along unit `dir` to the superellipsoid of half-extents `half` and exponent `e`
/// (2: an ellipsoid; higher, squarer).
fn superellipsoid(dir: Vec3, half: Vec3, e: f32) -> f32 {
    let q = (dir / half).abs();
    (q.x.powf(e) + q.y.powf(e) + q.z.powf(e)).powf(-1.0 / e)
}

/// A rounded stone: the superellipsoid of `half` and exponent `e`, swelling and hollowed by
/// broad noise and grained by fine noise.
fn rounded(seed: Seed, half: Vec3, e: f32, segments: u32) -> TriMesh {
    let broad = seed.derive_str("broad").value();
    let fine = seed.derive_str("fine").value();
    let least = half.min_element();
    cube_sphere(segments, |dir| {
        let r = superellipsoid(dir, half, e);
        let bump = least * (0.12 * fbm(broad, dir * 2.2, 4) + 0.025 * fbm(fine, dir * 9.0, 3));
        dir * (r + bump)
    })
}

/// A limestone block (see [`StoneShape::Block`]).
fn block(seed: Seed, half: Vec3, segments: u32) -> TriMesh {
    let mut rng = seed.derive_str("block planes").rng();
    let pits = seed.derive_str("pits").value();
    let fine = seed.derive_str("fine").value();
    let least = half.min_element();
    let mut mesh = cube_sphere(segments, |dir| {
        let r = superellipsoid(dir, half, 6.0);
        // Solution pits: the hollows of a ridged noise, and a fine grain.
        let pit = 1.0 - fbm(pits, dir * 5.0, 3).abs() * 2.0;
        let bump = least * (-0.035 * pit.max(0.0).powi(3) + 0.012 * fbm(fine, dir * 14.0, 3));
        dir * (r + bump)
    });
    // The bedding, top and bottom, dipping a few degrees; the joints round it, near upright.
    let dip = Vec3::new(rng.next_f32() - 0.5, 0.0, rng.next_f32() - 0.5) * 0.15;
    let mut planes = vec![
        (
            (Vec3::Y + dip).normalize(),
            half.y * (0.82 + 0.12 * rng.next_f32()),
        ),
        (
            (Vec3::NEG_Y + dip).normalize(),
            half.y * (0.85 + 0.1 * rng.next_f32()),
        ),
    ];
    let joints = 2 + (rng.next_f32() * 3.0) as u32;
    for _ in 0..joints {
        let a = rng.next_f32() * std::f32::consts::TAU;
        let n = Vec3::new(a.cos(), (rng.next_f32() - 0.5) * 0.3, a.sin()).normalize();
        // The support of the box along n, cut back to 0.7 to 0.92 of it.
        let support = (n.abs() * half).element_sum();
        planes.push((n, support * (0.62 + 0.25 * rng.next_f32())));
    }
    for p in &mut mesh.positions {
        let mut v = Vec3::from_array(*p);
        for _ in 0..3 {
            for &(n, d) in &planes {
                let beyond = v.dot(n) - d;
                if beyond > 0.0 {
                    v -= n * beyond;
                }
            }
        }
        *p = v.to_array();
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_stone_is_closed_on_the_ground_and_within_its_size() {
        for shape in [
            StoneShape::Corestone,
            StoneShape::Tor,
            StoneShape::Slab,
            StoneShape::Block,
        ] {
            let s = Stone {
                seed: 5,
                shape,
                size: [1.5, 1.0, 1.2],
                segments: 12,
            };
            let mesh = stone(&s);
            assert_eq!(
                mesh.positions,
                stone(&s).positions,
                "{shape:?}: deterministic"
            );
            // Each piece is a closed genus-0 mesh: V - E + F = 2 per piece.
            let pieces = (mesh.triangle_count() / (12 * 12 * 12)) as i64;
            let f = mesh.triangle_count() as i64;
            let v = mesh.positions.len() as i64;
            assert_eq!(v - 3 * f / 2 + f, 2 * pieces, "{shape:?}: closed");
            // Its lowest point a tenth of its height under the ground.
            let (low, high) = height_range(&mesh);
            assert!(
                (low + 0.1 * (high - low)).abs() < 1e-4,
                "{shape:?}: {low} {high}"
            );
            // Within 1.25 of its size across, and of three stacked for a tor.
            let reach = mesh
                .positions
                .iter()
                .map(|p| p[0].abs().max(p[2].abs()))
                .fold(0.0, f32::max);
            assert!(reach < 1.5 * 1.25, "{shape:?}: {reach}");
            let tall = if shape == StoneShape::Tor { 3.0 } else { 1.0 };
            assert!(high - low < 2.0 * 1.25 * tall, "{shape:?}: {}", high - low);
            assert!(mesh.normals.iter().all(|n| n.iter().all(|c| c.is_finite())));
        }
    }

    #[test]
    fn a_block_has_flat_faces_a_corestone_none() {
        // The largest share of vertices facing one way: a face of the block's bedding or joints.
        let flattest = |shape| {
            let mesh = stone(&Stone {
                seed: 3,
                shape,
                size: [1.0, 0.6, 1.0],
                segments: 24,
            });
            let mut facing = HashMap::<[i32; 3], u32>::new();
            for n in &mesh.normals {
                *facing
                    .entry(n.map(|c| (c * 50.0).round() as i32))
                    .or_default() += 1;
            }
            *facing.values().max().unwrap() as f32 / mesh.normals.len() as f32
        };
        let (block, corestone) = (flattest(StoneShape::Block), flattest(StoneShape::Corestone));
        assert!(block > 0.05, "{block}");
        assert!(corestone < 0.02, "{corestone}");
    }
}
