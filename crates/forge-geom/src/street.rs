//! A street lamp (D-065, #296; city-plan's until then): its model, and where its lenses are, for
//! the lamps that light from them (`forge_render::lamps::street_lights`). It stands on `y = 0`
//! at the origin, its arms along `z`.

use glam::Vec3;

use crate::procedural::TriMesh;

/// The lenses' middles along `z`, metres: under the arm over the road, then under a double lamp's
/// second arm, over the pavement.
pub const LENS_Z: [f32; 2] = [-1.40, 1.00];
/// A lens's size, metres: across (`x`) and along its arm (`z`).
pub const LENS_SIZE: [f32; 2] = [0.24, 0.56];
/// The lenses' height, metres.
pub const LENS_Y: f32 = 7.69;

/// Adds a triangle to a mesh, turned so its face looks along `normal`.
fn triangle(mesh: &mut TriMesh, mut p: [Vec3; 3], normal: Vec3, section: u8) {
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(normal) < 0.0 {
        p.swap(1, 2);
    }
    let base = mesh.positions.len() as u32;
    for q in p {
        mesh.positions.push(q.to_array());
        mesh.normals.push(normal.to_array());
    }
    mesh.indices.extend([base, base + 1, base + 2]);
    mesh.sections.push(section);
}

/// A street lamp 8 m tall: a hexagonal pole and an arm reaching 1.4 m along −z over the road, and
/// when `double` a second reaching 1 m along +z over the pavement (section 0); under each arm's end
/// a dark housing (section 0) lit only underneath (section 1, emissive): shielded, no light into
/// the sky (the owner, 2026-10-11).
pub fn lamp(double: bool) -> TriMesh {
    let mut mesh = TriMesh::default();
    let sides = 6;
    for k in 0..sides {
        let a = k as f32 / sides as f32 * std::f32::consts::TAU;
        let b = (k + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let (pa, pb) = (
            Vec3::new(a.cos() * 0.07, 0.0, a.sin() * 0.07),
            Vec3::new(b.cos() * 0.07, 0.0, b.sin() * 0.07),
        );
        let up = Vec3::Y * 8.0;
        let out = ((pa + pb) * 0.5).normalize();
        triangle(&mut mesh, [pa, pb, pb + up], out, 0);
        triangle(&mut mesh, [pa, pb + up, pa + up], out, 0);
    }
    // A box from `lo` to `hi` in a section.
    let boxed = |mesh: &mut TriMesh, lo: Vec3, hi: Vec3, section: u8| {
        let c = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { hi.x } else { lo.x },
                if y { hi.y } else { lo.y },
                if z { hi.z } else { lo.z },
            )
        };
        let faces = [
            (
                Vec3::X,
                [
                    c(true, false, false),
                    c(true, true, false),
                    c(true, true, true),
                    c(true, false, true),
                ],
            ),
            (
                -Vec3::X,
                [
                    c(false, false, false),
                    c(false, false, true),
                    c(false, true, true),
                    c(false, true, false),
                ],
            ),
            (
                Vec3::Y,
                [
                    c(false, true, false),
                    c(false, true, true),
                    c(true, true, true),
                    c(true, true, false),
                ],
            ),
            (
                -Vec3::Y,
                [
                    c(false, false, false),
                    c(true, false, false),
                    c(true, false, true),
                    c(false, false, true),
                ],
            ),
            (
                Vec3::Z,
                [
                    c(false, false, true),
                    c(true, false, true),
                    c(true, true, true),
                    c(false, true, true),
                ],
            ),
            (
                -Vec3::Z,
                [
                    c(false, false, false),
                    c(false, true, false),
                    c(true, true, false),
                    c(true, false, false),
                ],
            ),
        ];
        for (n, q) in faces {
            triangle(mesh, [q[0], q[1], q[2]], n, section);
            triangle(mesh, [q[0], q[2], q[3]], n, section);
        }
    };
    // The arms: over the road along −z, and over the pavement along +z.
    let arms: &[f32] = if double { &[-1.0, 1.0] } else { &[-1.0] };
    for &way in arms {
        let reach = if way < 0.0 { 1.45 } else { 1.05 };
        let (z0, z1) = if way < 0.0 {
            (-reach, 0.0)
        } else {
            (0.0, reach)
        };
        boxed(
            &mut mesh,
            Vec3::new(-0.04, 7.85, z0),
            Vec3::new(0.04, 7.95, z1),
            0,
        );
        // The housing, dark, and under it the lit lens, facing down only.
        let (h0, h1) = if way < 0.0 {
            (-reach - 0.25, -reach + 0.35)
        } else {
            (reach - 0.35, reach + 0.25)
        };
        boxed(
            &mut mesh,
            Vec3::new(-0.14, 7.70, h0),
            Vec3::new(0.14, 7.85, h1),
            0,
        );
        let lens = [
            Vec3::new(-0.12, LENS_Y, h0 + 0.02),
            Vec3::new(0.12, LENS_Y, h0 + 0.02),
            Vec3::new(0.12, LENS_Y, h1 - 0.02),
            Vec3::new(-0.12, LENS_Y, h1 - 0.02),
        ];
        triangle(&mut mesh, [lens[0], lens[1], lens[2]], -Vec3::Y, 1);
        triangle(&mut mesh, [lens[0], lens[2], lens[3]], -Vec3::Y, 1);
    }
    mesh
}
