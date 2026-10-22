//! A cell's proxy (D-077, #338): a cell's meshes merged into one, which the cell streamer shows
//! beyond the reach of the cell's full content (`forge_render::CellSource::load_proxy`). Cooked
//! once like any mesh ([`crate::cache::cook_cached`]), its cluster DAG and the page streaming bring
//! its detail down with distance: one instance a far cell, swapped with the content it stands for
//! in one edit, so the two never show together.
//!
//! What goes in is the game's choice: the parts seen from afar (a building's shell, its roof, the
//! big trees), not the furniture inside.

use glam::{Quat, Vec3};
use thiserror::Error;

use crate::procedural::TriMesh;

/// One placed mesh of a cell, for [`merge`].
#[derive(Clone, Copy, Debug)]
pub struct Part<'a> {
    /// Its mesh.
    pub mesh: &'a TriMesh,
    /// Where it stands from the proxy's origin, metres.
    pub at: Vec3,
    /// Its turn.
    pub rotation: Quat,
    /// Its uniform scale, above zero.
    pub scale: f32,
    /// Its instance's material row: the mesh's section `s` draws row `material + s`.
    pub material: u32,
}

/// Why parts cannot be merged.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum MergeError {
    /// No triangles at all.
    #[error("no triangles to merge")]
    Empty,
    /// A triangle's row lies past what a mesh's sections reach from the parts' lowest row.
    #[error("material row {row} is more than 255 rows past the lowest, {lowest}")]
    Rows {
        /// The parts' lowest row.
        lowest: u32,
        /// The row out of reach.
        row: u32,
    },
    /// More vertices than one mesh's 32-bit indices hold.
    #[error("{0} vertices: more than one mesh holds")]
    TooLarge(u64),
}

/// The parts as one mesh in the proxy's frame, and the material row its instance takes: the lowest
/// of the parts' rows, each triangle's section the distance from there to the row it drew, so one
/// instance draws every part as its own instance did. Texture coordinates are kept when any part
/// has them (zeros for the parts without).
pub fn merge(parts: &[Part]) -> Result<(TriMesh, u32), MergeError> {
    let parts: Vec<&Part> = parts
        .iter()
        .filter(|p| p.mesh.triangle_count() > 0)
        .collect();
    let lowest = parts
        .iter()
        .map(|p| p.material)
        .min()
        .ok_or(MergeError::Empty)?;
    let vertices: u64 = parts.iter().map(|p| p.mesh.positions.len() as u64).sum();
    if vertices > u64::from(u32::MAX) {
        return Err(MergeError::TooLarge(vertices));
    }
    let uvs = parts.iter().any(|p| !p.mesh.uvs.is_empty());
    let mut out = TriMesh::default();
    for part in parts {
        let mesh = part.mesh;
        let base = out.positions.len() as u32;
        out.positions.extend(
            mesh.positions
                .iter()
                .map(|&p| (part.at + part.rotation * (Vec3::from(p) * part.scale)).to_array()),
        );
        out.normals.extend(
            mesh.normals
                .iter()
                .map(|&n| (part.rotation * Vec3::from(n)).to_array()),
        );
        // A mesh without normals gets its faces' (below, once every part is in).
        out.normals.resize(out.positions.len(), [0.0; 3]);
        if uvs {
            out.uvs.extend(mesh.uvs.iter().copied());
            out.uvs.resize(out.positions.len(), [0.0; 2]);
        }
        out.indices.extend(mesh.indices.iter().map(|&i| base + i));
        for t in 0..mesh.triangle_count() {
            let row = part.material + u32::from(mesh.section(t));
            let section =
                u8::try_from(row - lowest).map_err(|_| MergeError::Rows { lowest, row })?;
            out.sections.push(section);
        }
    }
    if out.normals.contains(&[0.0; 3]) {
        let faces = {
            let mut m = out.clone();
            m.recompute_normals();
            m.normals
        };
        for (n, f) in out.normals.iter_mut().zip(faces) {
            if *n == [0.0; 3] {
                *n = f;
            }
        }
    }
    if out.sections.iter().all(|&s| s == 0) {
        out.sections.clear();
    }
    Ok((out, lowest))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit triangle in the XZ plane facing up, with `sections`.
    fn triangle(sections: Vec<u8>, uvs: bool) -> TriMesh {
        TriMesh {
            positions: vec![[0.0, 0.0, 0.0], [0.0, 0.0, 1.0], [1.0, 0.0, 0.0]],
            normals: vec![[0.0, 1.0, 0.0]; 3],
            indices: vec![0, 1, 2],
            sections,
            uvs: if uvs {
                vec![[0.0, 0.0], [0.0, 1.0], [1.0, 0.0]]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn merged_parts_keep_their_places_and_their_rows() {
        let a = triangle(Vec::new(), false);
        let b = triangle(vec![2], true);
        let (mesh, row) = merge(&[
            Part {
                mesh: &a,
                at: Vec3::new(10.0, 0.0, 0.0),
                rotation: Quat::IDENTITY,
                scale: 2.0,
                material: 7,
            },
            Part {
                mesh: &b,
                at: Vec3::new(0.0, 5.0, 0.0),
                // Upside down: its normal points down.
                rotation: Quat::from_rotation_x(std::f32::consts::PI),
                scale: 1.0,
                material: 5,
            },
        ])
        .expect("merged");
        assert_eq!(row, 5);
        assert_eq!(mesh.triangle_count(), 2);
        // The first part's rows from 5: 7 + 0; the second's 5 + 2.
        assert_eq!(mesh.sections, vec![2, 2]);
        assert_eq!(mesh.positions[1], [10.0, 0.0, 2.0]);
        assert_eq!(mesh.indices[3..], [3, 4, 5]);
        assert!(
            (mesh.positions[4][2] + 1.0).abs() < 1e-6,
            "{:?}",
            mesh.positions[4]
        );
        assert!(
            (mesh.normals[3][1] + 1.0).abs() < 1e-6,
            "{:?}",
            mesh.normals[3]
        );
        // Texture coordinates for all, the first part's zeros.
        assert_eq!(mesh.uvs.len(), 6);
        assert_eq!(mesh.uvs[1], [0.0, 0.0]);
        assert_eq!(mesh.uvs[4], [0.0, 1.0]);
    }

    #[test]
    fn rows_out_of_a_sections_reach_and_empty_parts_are_refused() {
        let a = triangle(Vec::new(), false);
        let part = |material| Part {
            mesh: &a,
            at: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: 1.0,
            material,
        };
        assert_eq!(
            merge(&[part(0), part(256)]).err(),
            Some(MergeError::Rows {
                lowest: 0,
                row: 256
            })
        );
        assert!(merge(&[part(0), part(255)]).is_ok());
        assert_eq!(merge(&[]).err(), Some(MergeError::Empty));
        let none = TriMesh::default();
        assert_eq!(
            merge(&[Part {
                mesh: &none,
                ..part(0)
            }])
            .err(),
            Some(MergeError::Empty)
        );
    }
}
