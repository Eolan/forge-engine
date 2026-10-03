//! Fracture ahead of time (issue #142): a convex solid cut into the Voronoi cells of points
//! inside it, each cell a convex polyhedron (the solid clipped by the planes halfway between its
//! point and every other), so each piece can be a convex hull in the physics and a mesh of its
//! own, its cut faces drawn with the material inside the solid.
//!
//! The cuts are made in `f64` with sums, products, quotients and no trigonometry (the faces'
//! corners are ordered by a pseudo-angle), so the pieces are the same bits on every platform:
//! their hulls feed the physics.

use glam::{DVec3, Vec3};

use crate::procedural::TriMesh;

/// A face of a polyhedron: a convex polygon, counter-clockwise seen from outside.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    /// Its corners.
    pub points: Vec<DVec3>,
    /// Whether a cut made it (drawn with the material inside the solid).
    pub cut: bool,
}

/// A convex polyhedron as its faces.
#[derive(Clone, Debug, PartialEq)]
pub struct Polyhedron {
    /// Its faces, each outward.
    pub faces: Vec<Face>,
}

/// How close to a plane a corner counts as on it, metres.
const ON_PLANE: f64 = 1e-9;

impl Polyhedron {
    /// A box of half sizes `half` about the origin.
    pub fn cuboid(half: DVec3) -> Self {
        let c = |x: f64, y: f64, z: f64| DVec3::new(x * half.x, y * half.y, z * half.z);
        let face = |points: [DVec3; 4]| Face {
            points: points.to_vec(),
            cut: false,
        };
        Self {
            faces: vec![
                face([
                    c(1., -1., -1.),
                    c(1., 1., -1.),
                    c(1., 1., 1.),
                    c(1., -1., 1.),
                ]),
                face([
                    c(-1., -1., -1.),
                    c(-1., -1., 1.),
                    c(-1., 1., 1.),
                    c(-1., 1., -1.),
                ]),
                face([
                    c(-1., 1., -1.),
                    c(-1., 1., 1.),
                    c(1., 1., 1.),
                    c(1., 1., -1.),
                ]),
                face([
                    c(-1., -1., -1.),
                    c(1., -1., -1.),
                    c(1., -1., 1.),
                    c(-1., -1., 1.),
                ]),
                face([
                    c(-1., -1., 1.),
                    c(1., -1., 1.),
                    c(1., 1., 1.),
                    c(-1., 1., 1.),
                ]),
                face([
                    c(-1., -1., -1.),
                    c(-1., 1., -1.),
                    c(1., 1., -1.),
                    c(1., -1., -1.),
                ]),
            ],
        }
    }

    /// What lies on the side of the plane `normal · x ≤ offset`, the cut closed by a face of
    /// its own (marked `cut`); `None` when nothing does.
    pub fn clip(&self, normal: DVec3, offset: f64) -> Option<Self> {
        let mut faces = Vec::with_capacity(self.faces.len() + 1);
        let mut on_cut = Vec::new();
        for face in &self.faces {
            let mut points = Vec::with_capacity(face.points.len() + 2);
            let n = face.points.len();
            for k in 0..n {
                let (a, b) = (face.points[k], face.points[(k + 1) % n]);
                let (da, db) = (normal.dot(a) - offset, normal.dot(b) - offset);
                if da <= ON_PLANE {
                    points.push(a);
                    if da >= -ON_PLANE {
                        on_cut.push(a);
                    }
                }
                if (da < -ON_PLANE && db > ON_PLANE) || (da > ON_PLANE && db < -ON_PLANE) {
                    let p = a + (b - a) * (da / (da - db));
                    points.push(p);
                    on_cut.push(p);
                }
            }
            if points.len() >= 3 {
                faces.push(Face {
                    points,
                    cut: face.cut,
                });
            }
        }
        if faces.is_empty() {
            return None;
        }
        // The cut's face: the corners on the plane, in order round their middle seen from
        // outside (from +normal).
        let mut corners: Vec<DVec3> = Vec::with_capacity(on_cut.len());
        for p in on_cut {
            if corners.iter().all(|q| q.distance_squared(p) > 1e-18) {
                corners.push(p);
            }
        }
        if corners.len() >= 3 {
            let middle = corners.iter().sum::<DVec3>() / corners.len() as f64;
            let u = normal.any_orthonormal_vector();
            let v = normal.normalize().cross(u);
            corners.sort_by(|a, b| {
                let (pa, pb) = (a - middle, b - middle);
                pseudo_angle(pa.dot(u), pa.dot(v)).total_cmp(&pseudo_angle(pb.dot(u), pb.dot(v)))
            });
            faces.push(Face {
                points: corners,
                cut: true,
            });
        }
        (faces.len() >= 4).then_some(Self { faces })
    }

    /// Its volume, m³.
    pub fn volume(&self) -> f64 {
        self.fan().map(|(a, b, c)| a.dot(b.cross(c))).sum::<f64>() / 6.0
    }

    /// Its centre of volume.
    pub fn centroid(&self) -> DVec3 {
        let (mut sum, mut volume) = (DVec3::ZERO, 0.0);
        for (a, b, c) in self.fan() {
            let v = a.dot(b.cross(c));
            sum += v * (a + b + c) / 4.0;
            volume += v;
        }
        sum / volume
    }

    /// Its corners, each once.
    pub fn corners(&self) -> Vec<DVec3> {
        let mut out: Vec<DVec3> = Vec::new();
        for p in self.faces.iter().flat_map(|f| &f.points) {
            if out.iter().all(|q| q.distance_squared(*p) > 1e-18) {
                out.push(*p);
            }
        }
        out
    }

    /// Its faces as triangles about `origin`, each face flat and its own vertices; section 0
    /// for the solid's surface, 1 for the cuts.
    pub fn mesh(&self, origin: DVec3) -> TriMesh {
        let mut mesh = TriMesh::default();
        for face in &self.faces {
            let p = &face.points;
            let normal = (1..p.len() - 1)
                .map(|k| (p[k] - p[0]).cross(p[k + 1] - p[0]))
                .sum::<DVec3>()
                .normalize_or_zero()
                .as_vec3();
            let first = mesh.positions.len() as u32;
            for &q in p {
                mesh.positions.push((q - origin).as_vec3().to_array());
                mesh.normals.push(normal.to_array());
            }
            for k in 1..p.len() as u32 - 1 {
                mesh.indices.extend([first, first + k, first + k + 1]);
                mesh.sections.push(u8::from(face.cut));
            }
        }
        mesh
    }

    /// Its triangles from each face's first corner.
    fn fan(&self) -> impl Iterator<Item = (DVec3, DVec3, DVec3)> + '_ {
        self.faces.iter().flat_map(|f| {
            (1..f.points.len() - 1).map(move |k| (f.points[0], f.points[k], f.points[k + 1]))
        })
    }
}

/// The Voronoi cells of `seeds` within `solid`, in the seeds' order: each the part of the solid
/// nearer its seed than any other. A seed outside the solid may have none.
pub fn voronoi(solid: &Polyhedron, seeds: &[DVec3]) -> Vec<Option<Polyhedron>> {
    seeds
        .iter()
        .enumerate()
        .map(|(i, &s)| {
            let mut cell = solid.clone();
            for (j, &t) in seeds.iter().enumerate() {
                if i == j {
                    continue;
                }
                let normal = t - s;
                cell = cell.clip(normal, normal.dot(0.5 * (s + t)))?;
            }
            Some(cell)
        })
        .collect()
}

/// An angle's stand-in in [0, 4) from its cosine and sine's ratio, rising with the angle: what
/// sorting by angle needs, without trigonometry.
fn pseudo_angle(x: f64, y: f64) -> f64 {
    let p = x / (x.abs() + y.abs()).max(f64::MIN_POSITIVE);
    if y < 0.0 { 3.0 + p } else { 1.0 - p }
}

/// A mesh's vertices as the physics' points (its hull).
pub fn hull_points(mesh: &TriMesh) -> Vec<Vec3> {
    mesh.positions.iter().map(|&p| Vec3::from(p)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_cut_in_two_keeps_its_volume() {
        let solid = Polyhedron::cuboid(DVec3::new(1.0, 0.5, 0.25));
        assert!((solid.volume() - 1.0).abs() < 1e-12);
        let normal = DVec3::new(1.0, 2.0, 0.5);
        let a = solid.clip(normal, 0.3).unwrap();
        let b = solid.clip(-normal, -0.3).unwrap();
        assert!((a.volume() + b.volume() - 1.0).abs() < 1e-12);
        // Each has one cut face, closed: its corners on the plane.
        for (piece, sign) in [(&a, 1.0), (&b, -1.0)] {
            let cut: Vec<_> = piece.faces.iter().filter(|f| f.cut).collect();
            assert_eq!(cut.len(), 1);
            assert!(
                cut[0]
                    .points
                    .iter()
                    .all(|p| (sign * normal.dot(*p) - sign * 0.3).abs() < 1e-9)
            );
        }
        // Nothing beyond the box: the plane misses it.
        assert!(solid.clip(DVec3::X, -2.0).is_none());
        assert_eq!(solid.clip(DVec3::X, 2.0).unwrap().volume(), solid.volume());
    }

    #[test]
    fn voronoi_cells_fill_the_solid_without_overlap() {
        let solid = Polyhedron::cuboid(DVec3::new(0.2, 1.2, 0.2));
        // Twelve points inside, from a hash.
        let unit = |k: u64| {
            let z = k.wrapping_mul(0x9e37_79b9_7f4a_7c15);
            ((z ^ (z >> 29)).wrapping_mul(0xbf58_476d_1ce4_e5b9) >> 11) as f64 / (1u64 << 53) as f64
        };
        let seeds: Vec<DVec3> = (0..12)
            .map(|k| {
                DVec3::new(
                    (2.0 * unit(3 * k + 1) - 1.0) * 0.2,
                    (2.0 * unit(3 * k + 2) - 1.0) * 1.2,
                    (2.0 * unit(3 * k + 3) - 1.0) * 0.2,
                )
            })
            .collect();
        let cells: Vec<Polyhedron> = voronoi(&solid, &seeds).into_iter().flatten().collect();
        assert_eq!(cells.len(), 12);
        let total: f64 = cells.iter().map(Polyhedron::volume).sum();
        assert!((total - solid.volume()).abs() < 1e-9, "{total}");
        // Each cell holds its seed and is closed: its mesh's faces sum to no area vector.
        for (cell, seed) in cells.iter().zip(&seeds) {
            assert!(cell.volume() > 0.0);
            assert!(
                cell.faces.iter().all(|f| {
                    let n = (f.points[1] - f.points[0]).cross(f.points[2] - f.points[0]);
                    n.dot(*seed - f.points[0]) <= 1e-9
                }),
                "a face looks away from its seed"
            );
            let mesh = cell.mesh(cell.centroid());
            let area: Vec3 = mesh
                .indices
                .chunks(3)
                .map(|t| {
                    let p = |i: u32| Vec3::from(mesh.positions[i as usize]);
                    (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]))
                })
                .sum();
            assert!(area.length() < 1e-4, "{area}");
        }
    }
}
