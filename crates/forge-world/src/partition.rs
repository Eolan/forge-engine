//! Partitions of a world into square cells at quadtree levels, named by [`CellId`]: the flat
//! grid of a level, a moon or a test scene, and the cube sphere of a planet (the six faces of a
//! cube projected onto the sphere with the equi-angular warp, so cells keep nearly the same
//! size over a face: Zucker & Higashi, `docs/research/large-worlds.md` §8). Both give the cell
//! of a point, the centre and size of a cell, and a way to step around a point on the surface,
//! which is all the streaming plan ([`crate::streaming`]) needs.
//!
//! Deterministic (D-016): the cube sphere's `atan` and `tan` come from `forge_core::dmath`.

use forge_core::dmath;
use glam::{DVec2, DVec3};

use crate::cell_id::CellId;

/// A surface cut into square cells at quadtree levels.
pub trait Partition {
    /// The cell at `level` holding `point` (metres in the partition's frame).
    fn cell_of(&self, point: DVec3, level: u8) -> CellId;
    /// The centre of `cell` on the surface, metres in the partition's frame.
    fn cell_center(&self, cell: CellId) -> DVec3;
    /// A cell's side at `level`, metres (on the sphere, the arc through a face's centre).
    fn cell_size(&self, level: u8) -> f64;
    /// `point` moved `east` and `north` metres along the surface.
    fn step(&self, point: DVec3, east: f64, north: f64) -> DVec3;
}

/// A flat grid in the x–z plane (+Y up), the level-0 cell `root_size` metres across and each
/// level halving it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FlatGrid {
    /// Side of a level-0 cell, metres.
    pub root_size: f64,
}

impl Partition for FlatGrid {
    fn cell_of(&self, point: DVec3, level: u8) -> CellId {
        let size = self.cell_size(level);
        CellId::flat(
            level,
            (point.x / size).floor() as i32,
            (point.z / size).floor() as i32,
        )
    }

    fn cell_center(&self, cell: CellId) -> DVec3 {
        let size = self.cell_size(cell.level());
        let (x, y) = cell.xy();
        DVec3::new((x as f64 + 0.5) * size, 0.0, (y as f64 + 0.5) * size)
    }

    fn cell_size(&self, level: u8) -> f64 {
        self.root_size / f64::from(1_u32 << level)
    }

    fn step(&self, point: DVec3, east: f64, north: f64) -> DVec3 {
        point + DVec3::new(east, 0.0, -north)
    }
}

/// A face of the cube sphere, by the axis it faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Face {
    /// +X.
    PosX = 0,
    /// −X.
    NegX = 1,
    /// +Y.
    PosY = 2,
    /// −Y.
    NegY = 3,
    /// +Z.
    PosZ = 4,
    /// −Z.
    NegZ = 5,
}

impl Face {
    /// The six faces, in index order.
    pub const ALL: [Face; 6] = [
        Face::PosX,
        Face::NegX,
        Face::PosY,
        Face::NegY,
        Face::PosZ,
        Face::NegZ,
    ];

    /// The face of index `index` (0–5).
    pub fn from_index(index: u8) -> Self {
        Self::ALL[usize::from(index)]
    }

    /// Its index (0–5), as [`CellId`] stores it.
    pub fn index(self) -> u8 {
        self as u8
    }

    /// The face's outward normal and its two in-face axes, right-handed (`u × v = normal`).
    fn axes(self) -> (DVec3, DVec3, DVec3) {
        match self {
            Face::PosX => (DVec3::X, DVec3::Y, DVec3::Z),
            Face::NegX => (DVec3::NEG_X, DVec3::Z, DVec3::Y),
            Face::PosY => (DVec3::Y, DVec3::Z, DVec3::X),
            Face::NegY => (DVec3::NEG_Y, DVec3::X, DVec3::Z),
            Face::PosZ => (DVec3::Z, DVec3::X, DVec3::Y),
            Face::NegZ => (DVec3::NEG_Z, DVec3::Y, DVec3::X),
        }
    }
}

/// A sphere of `radius` metres, its surface cut by the six cube faces, each face into
/// `2^level × 2^level` cells of equal angle (the equi-angular warp).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CubeSphere {
    /// The sphere's radius, metres.
    pub radius: f64,
}

impl CubeSphere {
    /// The face a direction falls on (ties to the first of x, y, z) and its equi-angular
    /// coordinates on it, `(s, t)` in `[−1, 1]`.
    pub fn face_coords(direction: DVec3) -> (Face, DVec2) {
        let a = direction.abs();
        let face = if a.x >= a.y && a.x >= a.z {
            if direction.x >= 0.0 {
                Face::PosX
            } else {
                Face::NegX
            }
        } else if a.y >= a.z {
            if direction.y >= 0.0 {
                Face::PosY
            } else {
                Face::NegY
            }
        } else if direction.z >= 0.0 {
            Face::PosZ
        } else {
            Face::NegZ
        };
        let (normal, u, v) = face.axes();
        let depth = direction.dot(normal);
        // The gnomonic coordinates on the cube's face, then the equal-angle warp.
        let gnomonic = DVec2::new(direction.dot(u), direction.dot(v)) / depth;
        let warp = |g: f64| dmath::atan(g) * (4.0 / std::f64::consts::PI);
        (
            face,
            DVec2::new(warp(gnomonic.x), warp(gnomonic.y)).clamp(DVec2::NEG_ONE, DVec2::ONE),
        )
    }

    /// The unit direction of equi-angular coordinates `st` on `face`.
    pub fn direction(face: Face, st: DVec2) -> DVec3 {
        let (normal, u, v) = face.axes();
        let unwarp = |s: f64| dmath::tan(s * std::f64::consts::FRAC_PI_4);
        (normal + u * unwarp(st.x) + v * unwarp(st.y)).normalize()
    }

    /// The point `height` metres above the surface in direction `direction`.
    pub fn surface_point(&self, direction: DVec3, height: f64) -> DVec3 {
        direction.normalize() * (self.radius + height)
    }

    /// The cell at `level` of face `face` holding `st`.
    fn cell_of_coords(face: Face, st: DVec2, level: u8) -> CellId {
        let n = f64::from(1_u32 << level);
        let index = |s: f64| ((s + 1.0) * 0.5 * n).floor().clamp(0.0, n - 1.0) as u32;
        CellId::cube(face.index(), level, index(st.x), index(st.y))
    }

    /// The local east and north at `point` (unit tangents; at the poles, east follows +X).
    fn tangents(point: DVec3) -> (DVec3, DVec3) {
        let up = point.normalize();
        let reference = if up.y.abs() > 0.999 {
            DVec3::X
        } else {
            DVec3::Y
        };
        let east = reference.cross(up).normalize();
        let north = up.cross(east);
        (east, north)
    }
}

impl Partition for CubeSphere {
    fn cell_of(&self, point: DVec3, level: u8) -> CellId {
        let (face, st) = Self::face_coords(point);
        Self::cell_of_coords(face, st, level)
    }

    fn cell_center(&self, cell: CellId) -> DVec3 {
        let n = f64::from(1_u32 << cell.level());
        let (x, y) = cell.xy();
        let st = DVec2::new(
            (x as f64 + 0.5) / n * 2.0 - 1.0,
            (y as f64 + 0.5) / n * 2.0 - 1.0,
        );
        Self::direction(Face::from_index(cell.face()), st) * self.radius
    }

    fn cell_size(&self, level: u8) -> f64 {
        std::f64::consts::FRAC_PI_2 * self.radius / f64::from(1_u32 << level)
    }

    fn step(&self, point: DVec3, east: f64, north: f64) -> DVec3 {
        let (e, n) = Self::tangents(point);
        let radius = point.length();
        (point + e * east + n * north).normalize() * radius
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flat_grid_halves_its_cells_per_level() {
        let grid = FlatGrid { root_size: 4096.0 };
        assert_eq!(grid.cell_size(2), 1024.0);
        let cell = grid.cell_of(DVec3::new(-1.0, 50.0, 2500.0), 2);
        assert_eq!(cell, CellId::flat(2, -1, 2));
        assert_eq!(grid.cell_center(cell), DVec3::new(-512.0, 0.0, 2560.0));
        assert_eq!(
            grid.step(DVec3::ZERO, 10.0, 20.0),
            DVec3::new(10.0, 0.0, -20.0)
        );
    }

    #[test]
    fn faces_and_coordinates_round_trip() {
        for face in Face::ALL {
            for (s, t) in [(0.0, 0.0), (0.5, -0.25), (-0.999, 0.999), (0.3, 0.7)] {
                let d = CubeSphere::direction(face, DVec2::new(s, t));
                let (back, st) = CubeSphere::face_coords(d);
                assert_eq!(back, face, "{face:?} ({s}, {t})");
                assert!(
                    (st - DVec2::new(s, t)).length() < 1e-9,
                    "{face:?} ({s}, {t}): {st}"
                );
            }
            // The axes are right-handed.
            let (normal, u, v) = face.axes();
            assert_eq!(u.cross(v), normal);
        }
        assert_eq!(CubeSphere::face_coords(DVec3::X).0, Face::PosX);
        assert_eq!(CubeSphere::face_coords(DVec3::NEG_Y).0, Face::NegY);
        assert_eq!(
            CubeSphere::face_coords(DVec3::new(0.0, 0.0, -3.0)).0,
            Face::NegZ
        );
    }

    #[test]
    fn cells_are_equal_in_angle_and_follow_the_point_across_faces() {
        let planet = CubeSphere {
            radius: 1_500_000.0,
        };
        // The face's centre falls in the middle cell; level 3 splits a face 8 × 8.
        assert_eq!(planet.cell_of(DVec3::X * 2e6, 3), CellId::cube(0, 3, 4, 4));
        assert_eq!(
            planet.cell_size(0),
            std::f64::consts::FRAC_PI_2 * 1_500_000.0
        );
        assert!((planet.cell_size(10) - 2301.0).abs() < 1.0);
        // A cell's centre lies in the cell.
        let cell = CellId::cube(4, 7, 100, 3);
        assert_eq!(planet.cell_of(planet.cell_center(cell), 7), cell);
        assert!((planet.cell_center(cell).length() - 1_500_000.0).abs() < 1e-6);
        // Equal angle: the first and the middle cell of a row span the same angle.
        let angle = |a: CellId, b: CellId| {
            planet
                .cell_center(a)
                .normalize()
                .dot(planet.cell_center(b).normalize())
                .acos()
        };
        let edge = angle(CellId::cube(0, 6, 0, 32), CellId::cube(0, 6, 1, 32));
        let middle = angle(CellId::cube(0, 6, 31, 32), CellId::cube(0, 6, 32, 32));
        assert!((edge / middle - 1.0).abs() < 0.02, "{edge} vs {middle}");
        // Stepping east from +X for a quarter of the circumference reaches −Z's face.
        let start = planet.surface_point(DVec3::X, 0.0);
        let quarter = std::f64::consts::FRAC_PI_2 * planet.radius;
        let mut p = start;
        for _ in 0..100 {
            p = planet.step(p, quarter / 100.0, 0.0);
        }
        assert_eq!(CubeSphere::face_coords(p).0, Face::NegZ);
        assert!((p.length() - planet.radius).abs() < 1e-6);
    }
}
