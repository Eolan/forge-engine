//! The `f32` error of positions far from the world's origin (issue #93), predicted on the CPU.
//!
//! Before issue #93's record ([`crate::cells`]), the renderer kept the instance table in
//! world-space `f32` (a `float4x4 model` and a `center` in `shaders/meshlet.slang`) and the
//! camera with it (a `camera_pos`, the view matrix). Far from the origin the positions' spacing
//! grows, one [`ulp`] of an `f32` being 1 mm at 10 km and 1 m at 10 000 km, and the view
//! transform takes a small difference of two large numbers. [`error`] reproduces the demos'
//! arithmetic of that record (the placement's `f32` translation, `FlyCamera::view`'s inverse,
//! the shader's `view_proj × (model × vertex)`) in `f32` against the same geometry in `f64`,
//! and the `(int3 cell, float3 local)` record the renderer now uses, and gives the error in
//! metres and in pixels. `--origin` in city-blocks and the ballad, with `tools/origins.sh`,
//! measures the real thing; [`table`] is what the docs quote.

use glam::{DMat4, DQuat, DVec2, DVec3, DVec4, Mat3, Mat4, Quat, Vec2, Vec3, Vec4};

/// The spacing of `f32` values at `x`, metres for a position: the smallest step a coordinate of
/// that magnitude can take, 2^(⌊log₂|x|⌋ − 23).
pub fn ulp(x: f32) -> f32 {
    let x = x.abs();
    x.next_up() - x
}

/// How the GPU stores an instance's position and the camera's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Record {
    /// The record before issue #93: world-space `f32` (a `float4x4 model`), the camera the
    /// same, and a world-space `view_proj`.
    WorldF32,
    /// D-004's amendment, the record since ([`crate::cells`]): an integer cell of `cell_size`
    /// metres and an `f32` offset within it, for the instances and the camera; the shader takes
    /// the cell difference in integers, so the camera-relative position is exact near the
    /// camera.
    CellLocal {
        /// Side of a cell, metres: a power of two, so its product with the cell difference is
        /// exact.
        cell_size: f64,
    },
}

/// A camera in the scene (before any offset from the world's origin) and the screen it draws.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// The camera, scene metres.
    pub camera: DVec3,
    /// Yaw about +Y, radians (`FlyCamera`).
    pub yaw: f64,
    /// Pitch about +X, radians.
    pub pitch: f64,
    /// Vertical field of view, radians.
    pub fov_y: f64,
    /// The near plane, metres.
    pub near: f64,
    /// The target's width, pixels.
    pub width: u32,
    /// The target's height, pixels.
    pub height: u32,
}

impl View {
    /// The city's south view at 1440p: over the south edge, looking north down a street
    /// (city-blocks' starting camera), through `FlyCamera`'s 70° lens.
    pub fn city_south_1440p() -> Self {
        Self {
            camera: DVec3::new(10.0, 45.0, 1260.0),
            yaw: 0.0,
            pitch: -0.12,
            fov_y: 70_f64.to_radians(),
            near: 0.05,
            width: 2560,
            height: 1440,
        }
    }

    fn rotation(&self) -> DQuat {
        DQuat::from_rotation_y(self.yaw) * DQuat::from_rotation_x(self.pitch)
    }
}

/// A vertex of the scene: its instance (position in scene metres, turn about +Y, scale) and
/// the vertex in object space.
#[derive(Clone, Copy, Debug)]
pub struct Sample {
    /// The instance's position, scene metres.
    pub instance: DVec3,
    /// The instance's turn about +Y, radians.
    pub yaw: f64,
    /// The instance's uniform scale.
    pub scale: f64,
    /// The vertex, object space.
    pub vertex: DVec3,
}

impl Sample {
    /// Objects in front of `view` at `distances` metres, a quarter of the way to the right
    /// edge, each a cube in proportion (a fifth of the distance across, at most 5 m) whose eight
    /// corners are the vertices.
    pub fn in_front(view: &View, distances: &[f64]) -> Vec<Self> {
        let rotation = view.rotation();
        let (forward, right) = (rotation * DVec3::NEG_Z, rotation * DVec3::X);
        let mut out = Vec::new();
        for &d in distances {
            let instance = view.camera + forward * d + right * (0.3 * d);
            let half = (0.1 * d).min(2.5);
            for corner in 0..8_u32 {
                let sign = |bit: u32| if (corner >> bit) & 1 == 1 { 1.0 } else { -1.0 };
                out.push(Self {
                    instance,
                    yaw: 0.7,
                    scale: 1.0,
                    vertex: DVec3::new(sign(0), sign(1), sign(2)) * half,
                });
            }
        }
        out
    }
}

/// What a record gets wrong: the largest errors over the samples.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Error {
    /// Of the vertex's position relative to the camera (view space), metres.
    pub metres: f64,
    /// On the screen, pixels (the larger of x and y).
    pub pixels: f64,
    /// Of the vertex's distance in front of the camera (clip `w`, the depth's source), as a
    /// share of it.
    pub depth: f64,
}

impl Error {
    fn max(self, other: Self) -> Self {
        Self {
            metres: self.metres.max(other.metres),
            pixels: self.pixels.max(other.pixels),
            depth: self.depth.max(other.depth),
        }
    }
}

/// The error of `record` for the vertices of `samples`, the scene moved `offset` from the
/// world's origin: the `f32` pipeline against the same geometry in `f64`.
pub fn error(view: &View, offset: DVec3, record: Record, samples: &[Sample]) -> Error {
    let aspect = f64::from(view.width) / f64::from(view.height);
    let camera = view.camera + offset;
    let rotation = view.rotation();
    // The reference: everything in f64.
    let view64 = DMat4::from_rotation_translation(rotation, camera).inverse();
    let proj64 = glam::dcamera::rh::proj::directx::perspective_infinite_reverse(
        view.fov_y, aspect, view.near,
    );
    // The f32 side: the projection and the rotation as `FlyCamera` builds them.
    let rotation32 =
        Quat::from_rotation_y(view.yaw as f32) * Quat::from_rotation_x(view.pitch as f32);
    let proj32 = glam::camera::rh::proj::directx::perspective_infinite_reverse(
        view.fov_y as f32,
        aspect as f32,
        view.near as f32,
    );
    let (width, height) = (f64::from(view.width), f64::from(view.height));
    let pixel64 = |clip: DVec4| {
        DVec2::new(
            (clip.x / clip.w * 0.5 + 0.5) * width,
            (0.5 - clip.y / clip.w * 0.5) * height,
        )
    };
    let pixel32 = |clip: Vec4| {
        Vec2::new(
            (clip.x / clip.w * 0.5 + 0.5) * width as f32,
            (0.5 - clip.y / clip.w * 0.5) * height as f32,
        )
        .as_dvec2()
    };
    // Today's view: `FlyCamera::view` in f32, and the shader's `view_proj`.
    let view32 = Mat4::from_rotation_translation(rotation32, camera.as_vec3()).inverse();
    let view_proj32 = proj32 * view32;
    // The cells' view: the rotation alone; the translation is taken in cells.
    let view_rotation32 = Mat3::from_quat(rotation32).transpose();
    let cell_of = |p: DVec3, size: f64| (p / size).floor();
    let local_of = |p: DVec3, size: f64| (p - cell_of(p, size) * size).as_vec3();
    let mut worst = Error::default();
    for s in samples {
        let position = s.instance + offset;
        let model64 = DMat4::from_scale_rotation_translation(
            DVec3::splat(s.scale),
            DQuat::from_rotation_y(s.yaw),
            position,
        );
        let world64 = model64.transform_point3(s.vertex);
        let in_view64 = view64.transform_point3(world64);
        let clip64 = proj64 * in_view64.extend(1.0);
        let (in_view32, clip32) = match record {
            Record::WorldF32 => {
                let model32 = Mat4::from_scale_rotation_translation(
                    Vec3::splat(s.scale as f32),
                    Quat::from_rotation_y(s.yaw as f32),
                    position.as_vec3(),
                );
                let world32 = model32.transform_point3(s.vertex.as_vec3());
                (
                    view32.transform_point3(world32),
                    view_proj32 * world32.extend(1.0),
                )
            }
            Record::CellLocal { cell_size } => {
                let cells = (cell_of(position, cell_size) - cell_of(camera, cell_size)).as_ivec3();
                let relative = cells.as_vec3() * cell_size as f32
                    + (local_of(position, cell_size) - local_of(camera, cell_size));
                let rotation_scale =
                    Mat3::from_quat(Quat::from_rotation_y(s.yaw as f32)) * s.scale as f32;
                let world_relative = rotation_scale * s.vertex.as_vec3() + relative;
                let in_view = view_rotation32 * world_relative;
                (in_view, proj32 * in_view.extend(1.0))
            }
        };
        worst = worst.max(Error {
            metres: (in_view32.as_dvec3() - in_view64).length(),
            pixels: (pixel32(clip32) - pixel64(clip64)).abs().max_element(),
            depth: (f64::from(clip32.w) - clip64.w).abs() / clip64.w,
        });
    }
    worst
}

/// One row of [`table`]: the scene moved `offset_m` from the origin along every axis.
#[derive(Clone, Copy, Debug)]
pub struct Row {
    /// The offset, metres.
    pub offset_m: f64,
    /// The spacing of `f32` positions at the camera there ([`ulp`] of its largest coordinate).
    pub ulp_m: f32,
    /// The record before issue #93.
    pub world_f32: Error,
    /// Cells of 1 km with an `f32` offset inside (the record since).
    pub cells_1km: Error,
}

/// The offsets the measurement uses (`tools/origins.sh`), metres.
pub const OFFSETS_M: [f64; 5] = [0.0, 1e4, 1e5, 1e6, 1e7];

/// The city's south view at 1440p moved by each of [`OFFSETS_M`] along every axis, for objects
/// 2 m to 1 km in front of the camera: the record before issue #93 against cells of 1 km.
pub fn table() -> Vec<Row> {
    let view = View::city_south_1440p();
    let samples = Sample::in_front(&view, &[2.0, 10.0, 100.0, 1000.0]);
    OFFSETS_M
        .iter()
        .map(|&offset_m| {
            let offset = DVec3::splat(offset_m);
            Row {
                offset_m,
                ulp_m: ulp((view.camera + offset).max_element() as f32),
                world_f32: error(&view, offset, Record::WorldF32, &samples),
                cells_1km: error(
                    &view,
                    offset,
                    Record::CellLocal { cell_size: 1024.0 },
                    &samples,
                ),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_spacing_of_f32_positions_doubles_with_every_power_of_two() {
        assert_eq!(ulp(1e4), 2_f32.powi(-10)); // 0.98 mm
        assert_eq!(ulp(1e5), 2_f32.powi(-7)); // 7.8 mm
        assert_eq!(ulp(1e6), 2_f32.powi(-4)); // 6.25 cm
        assert_eq!(ulp(1e7), 1.0); // 1 m
        assert_eq!(ulp(-1e7), 1.0);
        assert_eq!(ulp(1260.0), 2_f32.powi(-13)); // 0.12 mm: the city's south edge today
        assert_eq!(ulp(2048.0), 2_f32.powi(-12));
    }

    #[test]
    fn far_from_the_origin_the_world_f32_record_breaks_and_the_cells_do_not() {
        // The table the docs quote (`cargo test -p forge-render precision -- --nocapture`), and
        // today's record by the object's distance.
        let rows = table();
        let view = View::city_south_1440p();
        for r in &rows {
            println!(
                "{:>10} m: spacing {:.3e} m; world f32: {:.3e} m, {:.3} px, depth {:.1e}; cells: {:.3e} m, {:.4} px, depth {:.1e}",
                r.offset_m,
                r.ulp_m,
                r.world_f32.metres,
                r.world_f32.pixels,
                r.world_f32.depth,
                r.cells_1km.metres,
                r.cells_1km.pixels,
                r.cells_1km.depth
            );
            for d in [2.0, 10.0, 100.0, 1000.0] {
                let samples = Sample::in_front(&view, &[d]);
                let e = error(&view, DVec3::splat(r.offset_m), Record::WorldF32, &samples);
                println!(
                    "             at {d:>5} m: world f32 {:.3e} m, {:.3} px, depth {:.1e}",
                    e.metres, e.pixels, e.depth
                );
            }
        }
        let at = |offset: f64| rows.iter().find(|r| r.offset_m == offset).expect("row");
        // At the origin today's record is about as good as the cells: the city's south edge is
        // 1.3 km out, so a position there is an f32 of 0.12 mm spacing, a twentieth of a pixel
        // on an object 2 m away.
        assert!(at(0.0).world_f32.pixels < 0.1);
        assert!(at(0.0).world_f32.metres < 1e-3);
        // The cells stay there at every offset.
        for r in &rows {
            assert!(r.cells_1km.pixels < 0.05, "{r:?}");
            assert!(r.cells_1km.metres < 1e-3, "{r:?}");
        }
        // Today's record drifts with the offset, tenfold per decade: half a pixel at 10 km,
        // nine at 100 km, tens at 1 000 km, hundreds and a depth off by a fifth at 10 000 km,
        // all on an object 2 m away; a thousand times less on one 1 km away.
        let world: Vec<f64> = rows.iter().map(|r| r.world_f32.pixels).collect();
        assert!(world.windows(2).all(|w| w[0] < w[1]), "{world:?}");
        assert!(at(1e4).world_f32.pixels < 1.0);
        assert!(at(1e5).world_f32.pixels > 1.0);
        assert!(at(1e6).world_f32.pixels > 10.0);
        assert!(at(1e7).world_f32.pixels > 100.0);
        assert!(at(1e7).world_f32.depth > 0.1);
        assert!(at(1e7).world_f32.metres > 1.0);
    }
}
