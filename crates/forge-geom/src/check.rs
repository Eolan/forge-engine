//! Checks on a mesh before it is cooked: the patterns the cluster LOD folds (#309).

use glam::{Vec2, Vec3};

use crate::procedural::TriMesh;

/// A face standing in front of another face of the same mesh that faces the same way
/// ([`stacked_faces`]): the two triangles' indices and how far apart their planes are, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StackedFaces {
    /// The triangle in front.
    pub front: usize,
    /// The triangle behind it.
    pub behind: usize,
    /// Metres between their planes.
    pub gap: f32,
}

/// The faces of `mesh` that stand in front of another of its faces facing the same way (within
/// 5°), their planes `near` to `far` metres apart, overlapping it (a corner of either inside the
/// other, seen along the normal): a drawer's front a centimetre proud of the body's face, a
/// door over a wardrobe's front. With geometry alone in the simplification error, the cluster
/// LOD folds such a panel back onto the face behind it at about a pixel of error, where the
/// two tie in depth and the jitter picks the winner each frame (#309). Cook such pieces with
/// [`crate::meshlet::CookOptions::HARD_SURFACE`], or model no face behind another.
pub fn stacked_faces(mesh: &TriMesh, near: f32, far: f32) -> Vec<StackedFaces> {
    struct Face {
        corners: [Vec3; 3],
        normal: Vec3,
        offset: f32,
    }
    let faces: Vec<Face> = mesh
        .indices
        .as_chunks::<3>()
        .0
        .iter()
        .filter_map(|t| {
            let corners = [0, 1, 2].map(|k| Vec3::from(mesh.positions[t[k] as usize]));
            let normal = (corners[1] - corners[0]).cross(corners[2] - corners[0]);
            (normal.length_squared() > 1e-12).then(|| {
                let normal = normal.normalize();
                Face {
                    corners,
                    normal,
                    offset: normal.dot(corners[0]),
                }
            })
        })
        .collect();
    // Faces by the way they face (a coarse grid of directions), each list sorted by its plane's
    // offset, so a face meets only the faces facing its way just behind it.
    let key = |n: Vec3| (n * 8.0).round().as_ivec3();
    let mut by_way: std::collections::BTreeMap<(i32, i32, i32), Vec<usize>> = Default::default();
    for (i, f) in faces.iter().enumerate() {
        let k = key(f.normal);
        by_way.entry((k.x, k.y, k.z)).or_default().push(i);
    }
    let cos_5 = 0.996;
    let mut out = Vec::new();
    for list in by_way.values_mut() {
        list.sort_by(|&a, &b| faces[a].offset.total_cmp(&faces[b].offset));
        for (k, &i) in list.iter().enumerate() {
            let front = &faces[i];
            // Those behind it, their offsets `near` to `far` lower.
            for &j in list[..k].iter().rev() {
                let behind = &faces[j];
                let gap = front.offset - behind.offset;
                if gap > far {
                    break;
                }
                if gap < near || front.normal.dot(behind.normal) < cos_5 {
                    continue;
                }
                if overlap(&front.corners, &behind.corners, front.normal) {
                    out.push(StackedFaces {
                        front: i,
                        behind: j,
                        gap,
                    });
                }
            }
        }
    }
    out
}

/// Whether two triangles overlap seen along `normal`: a corner of either inside the other, or
/// their middles inside each other.
fn overlap(a: &[Vec3; 3], b: &[Vec3; 3], normal: Vec3) -> bool {
    let (u, v) = normal.any_orthonormal_pair();
    let flat = |t: &[Vec3; 3]| t.map(|p| Vec2::new(p.dot(u), p.dot(v)));
    let (a, b) = (flat(a), flat(b));
    let inside = |t: &[Vec2; 3], p: Vec2| {
        let side = |s: Vec2, e: Vec2| (e - s).perp_dot(p - s);
        let d = [side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0])];
        // Strictly inside (by a millimetre), so faces that only touch along an edge do not count.
        d.iter().all(|&x| x > 1e-6) || d.iter().all(|&x| x < -1e-6)
    };
    let middle = |t: &[Vec2; 3]| (t[0] + t[1] + t[2]) / 3.0;
    a.iter().any(|&p| inside(&b, p))
        || b.iter().any(|&p| inside(&a, p))
        || inside(&b, middle(&a))
        || inside(&a, middle(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quad facing +z at height `z`, `half` across, as two triangles.
    fn quad(mesh: &mut TriMesh, z: f32, half: f32) {
        let base = mesh.positions.len() as u32;
        for (x, y) in [(-half, -half), (half, -half), (half, half), (-half, half)] {
            mesh.positions.push([x, y, z]);
            mesh.normals.push([0.0, 0.0, 1.0]);
        }
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    #[test]
    fn a_panel_proud_of_a_face_is_found() {
        // A body's front face, and a drawer's front 1.2 cm proud of it, smaller.
        let mut mesh = TriMesh::default();
        quad(&mut mesh, 0.0, 0.5);
        quad(&mut mesh, 0.012, 0.2);
        let found = stacked_faces(&mesh, 0.001, 0.03);
        assert!(!found.is_empty());
        assert!(found.iter().all(|s| s.front >= 2 && s.behind < 2));
        assert!(found.iter().all(|s| (s.gap - 0.012).abs() < 1e-5));
        // Further apart than `far`: none.
        let mut far = TriMesh::default();
        quad(&mut far, 0.0, 0.5);
        quad(&mut far, 0.1, 0.2);
        assert!(stacked_faces(&far, 0.001, 0.03).is_empty());
    }

    #[test]
    fn faces_side_by_side_or_facing_away_are_not() {
        // Two quads side by side at the same height (a face cut round a panel), and a face
        // facing the other way behind.
        let mut mesh = TriMesh::default();
        quad(&mut mesh, 0.0, 0.5);
        let base = mesh.positions.len() as u32;
        for (x, y) in [(0.6, -0.5), (1.6, -0.5), (1.6, 0.5), (0.6, 0.5)] {
            mesh.positions.push([x, y, 0.01]);
            mesh.normals.push([0.0, 0.0, 1.0]);
        }
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        let base = mesh.positions.len() as u32;
        for (x, y) in [(-0.2, -0.2), (-0.2, 0.2), (0.2, 0.2), (0.2, -0.2)] {
            mesh.positions.push([x, y, -0.01]);
            mesh.normals.push([0.0, 0.0, -1.0]);
        }
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        assert!(stacked_faces(&mesh, 0.001, 0.03).is_empty());
    }

    /// The homes' furniture as [`crate::furniture::home`] makes it: the wardrobe's doors stand
    /// proud of its body's front, the pattern #309 found (cooked with weighted normals since;
    /// the counter's cupboard doors and the bed's panels show it too, 20 and 5 faces).
    #[test]
    fn the_wardrobe_s_doors_stand_proud_of_its_front() {
        let wardrobe = crate::furniture::home("wardrobe", 1.2, 0.62).expect("a wardrobe");
        let found = stacked_faces(&wardrobe, 0.001, 0.03);
        assert!(!found.is_empty());
    }
}
