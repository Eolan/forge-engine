//! Positions as an integer cell and an `f32` offset inside it (issue #93, D-004's amendment):
//! the form the GPU instance table and the frame block store, and the last step of the frame
//! tree ([`crate::frame`]) towards the renderer. The camera-relative position of an instance
//! is its cell difference to the camera's, taken in integers (exact), times the cell size (a
//! power of two, so the product is exact), plus the difference of the two offsets: exact near
//! the camera wherever the scene stands in the world, so the table is never rewritten when the
//! camera moves and holds no `f64` (Freese 2004; `docs/research/large-worlds.md` §1).

use glam::{DVec3, IVec3, Vec3};

/// Side of a cell, metres: 2¹⁰, so `cells × CELL_SIZE` is exact in `f32` up to 2²⁴ cells
/// (17 billion km), and an offset inside a cell is exact to 2⁻¹³ m (0.12 mm). `CELL_SIZE` in
/// `shaders/meshlet.slang` mirrors it.
pub const CELL_SIZE: f32 = 1024.0;

/// A position as an integer cell and an `f32` offset inside it, metres.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CellPos {
    /// The cell: `floor(position / CELL_SIZE)` per axis when normalised.
    pub cell: IVec3,
    /// Metres from the cell's corner, in `[0, CELL_SIZE)` when normalised; an unnormalised
    /// offset may lie outside, which every computation tolerates at a cost in precision.
    pub local: Vec3,
}

impl CellPos {
    /// The world's origin.
    pub const ORIGIN: Self = Self {
        cell: IVec3::ZERO,
        local: Vec3::ZERO,
    };

    /// Splits an `f64` position: the cell is the floor, the offset inside it rounded to `f32`
    /// (to 0.12 mm at most).
    pub fn from_f64(p: DVec3) -> Self {
        let cell = (p / f64::from(CELL_SIZE)).floor();
        Self {
            cell: cell.as_ivec3(),
            local: (p - cell * f64::from(CELL_SIZE)).as_vec3(),
        }
    }

    /// The position `offset` metres from `self`, normalised.
    pub fn offset(self, offset: Vec3) -> Self {
        Self {
            cell: self.cell,
            local: self.local + offset,
        }
        .normalized()
    }

    /// The same position with the offset carried into the cell, in `[0, CELL_SIZE)`.
    pub fn normalized(self) -> Self {
        let carry = (self.local / CELL_SIZE).floor();
        Self {
            cell: self.cell + carry.as_ivec3(),
            local: self.local - carry * CELL_SIZE,
        }
    }

    /// `self − other`, metres: the cell difference in integers first, then the offsets (the
    /// shaders' `instance_relative`).
    pub fn relative_to(self, other: Self) -> Vec3 {
        (self.cell - other.cell).as_vec3() * CELL_SIZE + (self.local - other.local)
    }

    /// The position in `f64`, metres (logs and tests).
    pub fn to_f64(self) -> DVec3 {
        self.cell.as_dvec3() * f64::from(CELL_SIZE) + self.local.as_dvec3()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{Mat3, Quat};

    #[test]
    fn a_position_splits_into_its_cell_and_is_exact_when_joined() {
        let p = CellPos::from_f64(DVec3::new(10.0, 45.0, 1260.0));
        assert_eq!(p.cell, IVec3::new(0, 0, 1));
        assert_eq!(p.local, Vec3::new(10.0, 45.0, 236.0));
        assert_eq!(p.to_f64(), DVec3::new(10.0, 45.0, 1260.0));
        let far = CellPos::from_f64(DVec3::splat(1e7 + 0.5));
        assert_eq!(far.cell, IVec3::splat(9765));
        assert_eq!(far.local, Vec3::splat(640.5)); // 1e7 = 9765 × 1024 + 640
        let negative = CellPos::from_f64(DVec3::new(-0.5, -1024.0, -1024.5));
        assert_eq!(negative.cell, IVec3::new(-1, -1, -2));
        assert_eq!(negative.local, Vec3::new(1023.5, 0.0, 1023.5));
    }

    #[test]
    fn offsets_carry_into_the_cell() {
        let p = CellPos::ORIGIN.offset(Vec3::new(2500.0, -1.0, 1024.0));
        assert_eq!(p.cell, IVec3::new(2, -1, 1));
        assert_eq!(p.local, Vec3::new(452.0, 1023.0, 0.0));
        assert_eq!(p, p.normalized());
    }

    #[test]
    fn the_difference_of_two_far_positions_is_exact_near_the_camera() {
        // Two points 1.25 m apart, 10 000 km from the origin: the f32 difference of their
        // world positions could only be a whole metre; the cells give it exactly.
        let camera = CellPos::from_f64(DVec3::new(1e7, 1e7, 1e7));
        let object = CellPos::from_f64(DVec3::new(1e7 + 1.25, 1e7 - 0.75, 1e7 + 2048.0));
        assert_eq!(object.relative_to(camera), Vec3::new(1.25, -0.75, 2048.0));
        assert_eq!(camera.relative_to(object), Vec3::new(-1.25, 0.75, -2048.0));
        // Across a cell border, still exact.
        let a = CellPos::from_f64(DVec3::new(1023.75, 0.0, 0.0));
        let b = CellPos::from_f64(DVec3::new(1024.25, 0.0, 0.0));
        assert_eq!(b.relative_to(a), Vec3::new(0.5, 0.0, 0.0));
    }

    /// `quat_matrix` in `meshlet.slang`, row by row, as glam builds a rotation from the same
    /// quaternion: the shader's instance frame is the CPU's.
    fn shader_quat_matrix([x, y, z, w]: [f32; 4]) -> Mat3 {
        let row0 = Vec3::new(
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - w * z),
            2.0 * (x * z + w * y),
        );
        let row1 = Vec3::new(
            2.0 * (x * y + w * z),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - w * x),
        );
        let row2 = Vec3::new(
            2.0 * (x * z - w * y),
            2.0 * (y * z + w * x),
            1.0 - 2.0 * (x * x + y * y),
        );
        Mat3::from_cols(row0, row1, row2).transpose()
    }

    #[test]
    fn the_shaders_quaternion_matrix_is_glams() {
        for (yaw, tilt) in [(0.0, 0.0), (0.7, 0.0), (0.0, 0.2), (2.5, -0.4), (-1.1, 0.3)] {
            let q = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(tilt);
            let ours = shader_quat_matrix(q.to_array());
            let glam = Mat3::from_quat(q);
            assert!(
                ours.abs_diff_eq(glam, 1e-6),
                "yaw {yaw} tilt {tilt}: {ours} vs {glam}"
            );
            // The placement's product of the two half-angle quaternions (`place` in the shader),
            // (cy·sx, cx·sy, −sy·sx, cy·cx), is the same rotation.
            let (sy, cy) = (0.5 * yaw).sin_cos();
            let (sx, cx) = (0.5 * tilt).sin_cos();
            let placed = Quat::from_xyzw(cy * sx, cx * sy, -sy * sx, cy * cx);
            assert!(placed.abs_diff_eq(q, 1e-6) || placed.abs_diff_eq(-q, 1e-6));
        }
    }
}
