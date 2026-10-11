//! Street furniture as part programs: café terraces and tram stops. Each piece uses the kit's
//! section layout ([`crate::modules`]: stone, frame, glass, paint, iron, fabric, wood), so the
//! kit's material groups dress it, and stands in the kit's frame: `x` along the facade or the
//! track, `y` up from the ground, `+z` away from the wall or the track.

use glam::Vec3;

use crate::modules::{
    ALL, FABRIC, GLASS, METAL, NEG_Y, NO_BACK, POS_Y, SHUTTER, STONE, WOOD, cuboid, quad,
};
use crate::procedural::TriMesh;

/// A café's terrace `w` metres along its shopfront: round tables a pace apart, each with two
/// chairs facing across it under a parasol, between 0.6 and 2.6 m out from the wall.
pub fn terrace(w: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let tables = (w / 1.6).floor().max(1.0) as u32;
    let step = w / tables as f32;
    for k in 0..tables {
        let c = Vec3::new(step * (k as f32 + 0.5), 0.0, 1.6);
        table(&mut m, c);
        for side in [-1.0_f32, 1.0] {
            chair(&mut m, c + Vec3::new(0.0, 0.0, 0.62 * side), -side);
        }
        parasol(&mut m, c);
    }
    m
}

/// The seats of [`terrace`]`(w)`: each chair's place on the ground under its seat and the way a
/// person on it faces (towards the table), in the terrace's frame.
pub fn terrace_seats(w: f32) -> Vec<(Vec3, Vec3)> {
    let tables = (w / 1.6).floor().max(1.0) as u32;
    let step = w / tables as f32;
    let mut out = Vec::new();
    for k in 0..tables {
        let c = Vec3::new(step * (k as f32 + 0.5), 0.0, 1.6);
        for side in [-1.0_f32, 1.0] {
            out.push((
                c + Vec3::new(0.0, 0.0, 0.62 * side),
                Vec3::new(0.0, 0.0, -side),
            ));
        }
    }
    out
}

/// A round table on a pedestal: an octagonal wooden top 0.7 m across, 0.75 m up.
fn table(m: &mut TriMesh, c: Vec3) {
    prism(m, c + Vec3::Y * 0.72, 0.35, 0.04, 8, WOOD);
    cuboid(
        m,
        c + Vec3::new(-0.03, 0.0, -0.03),
        c + Vec3::new(0.03, 0.72, 0.03),
        METAL,
        ALL & !POS_Y & !NEG_Y,
    );
    cuboid(
        m,
        c + Vec3::new(-0.22, 0.0, -0.22),
        c + Vec3::new(0.22, 0.03, 0.22),
        METAL,
        ALL & !NEG_Y,
    );
}

/// A bistro chair at `c`, its back on the `-facing` side: four iron legs, a woven seat, a back.
fn chair(m: &mut TriMesh, c: Vec3, facing: f32) {
    let (half, seat) = (0.21, 0.46);
    for (dx, dz) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
        let p = c + Vec3::new(dx * (half - 0.02), 0.0, dz * (half - 0.02));
        cuboid(
            m,
            p - Vec3::new(0.012, 0.0, 0.012),
            p + Vec3::new(0.012, seat, 0.012),
            METAL,
            ALL & !POS_Y & !NEG_Y,
        );
    }
    cuboid(
        m,
        c + Vec3::new(-half, seat, -half),
        c + Vec3::new(half, seat + 0.04, half),
        FABRIC,
        ALL,
    );
    let back = c.z - facing * (half - 0.02);
    cuboid(
        m,
        Vec3::new(c.x - half, seat, back - 0.015),
        Vec3::new(c.x + half, seat + 0.42, back + 0.015),
        METAL,
        ALL,
    );
}

/// A square market parasol over `c`: a pole and a canopy of four sloping panels, its eaves 2.2 m
/// up, 1.1 m from the pole each way.
fn parasol(m: &mut TriMesh, c: Vec3) {
    cuboid(
        m,
        c + Vec3::new(-0.02, 0.76, -0.02),
        c + Vec3::new(0.02, 2.55, 0.02),
        METAL,
        ALL & !NEG_Y,
    );
    let (r, eave, peak) = (1.1, 2.2, 2.6);
    let top = c + Vec3::Y * peak;
    let corners = [
        Vec3::new(-r, eave, -r),
        Vec3::new(r, eave, -r),
        Vec3::new(r, eave, r),
        Vec3::new(-r, eave, r),
    ]
    .map(|v| Vec3::new(c.x, 0.0, c.z) + v);
    for i in 0..4 {
        let (a, b) = (corners[i], corners[(i + 1) % 4]);
        // Each panel seen from above and from below.
        quad(m, [b, a, top, top], FABRIC);
        quad(m, [a, b, top, top], FABRIC);
    }
}

/// A prism of `sides` sides round `c` (its top face at `c.y`), `r` across its corners, `t` thick.
fn prism(m: &mut TriMesh, c: Vec3, r: f32, t: f32, sides: u32, section: u8) {
    let ring = |k: u32| {
        let a = k as f32 / sides as f32 * std::f32::consts::TAU;
        Vec3::new(a.cos() * r, 0.0, a.sin() * r)
    };
    for k in 0..sides {
        let (p, q) = (ring(k), ring(k + 1));
        let (top, bottom) = (c, c - Vec3::Y * t);
        // Top and bottom fans, and the rim.
        quad(m, [top, top + q, top + p, top], section);
        quad(m, [bottom, bottom + p, bottom + q, bottom], section);
        quad(m, [bottom + p, top + p, top + q, bottom + q], section);
    }
}

/// A park bench 1.8 m long along `x`, facing `+z`: wooden slats on two cast-iron end frames, a
/// slatted back.
pub fn bench() -> TriMesh {
    let mut m = TriMesh::default();
    for x in [-0.8_f32, 0.8] {
        // The end frame: a leg front and back, an arm.
        for z in [-0.2_f32, 0.2] {
            cuboid(
                &mut m,
                Vec3::new(x - 0.03, 0.0, z - 0.03),
                Vec3::new(x + 0.03, 0.45, z + 0.03),
                METAL,
                ALL & !NEG_Y & !POS_Y,
            );
        }
        cuboid(
            &mut m,
            Vec3::new(x - 0.03, 0.42, -0.26),
            Vec3::new(x + 0.03, 0.85, -0.2),
            METAL,
            ALL & !NEG_Y,
        );
        cuboid(
            &mut m,
            Vec3::new(x - 0.03, 0.62, -0.22),
            Vec3::new(x + 0.03, 0.66, 0.22),
            METAL,
            ALL,
        );
    }
    for k in 0..4 {
        let z = -0.18 + k as f32 * 0.12;
        cuboid(
            &mut m,
            Vec3::new(-0.9, 0.42, z),
            Vec3::new(0.9, 0.46, z + 0.09),
            WOOD,
            ALL,
        );
    }
    for k in 0..3 {
        let y = 0.55 + k as f32 * 0.11;
        cuboid(
            &mut m,
            Vec3::new(-0.9, y, -0.25),
            Vec3::new(0.9, y + 0.08, -0.22),
            WOOD,
            ALL,
        );
    }
    m
}

/// A square's fountain round the origin: an octagonal stone basin 6.4 m across, brimming with
/// water ([`GLASS`]'s section), a column in its middle carrying a bowl.
pub fn fountain() -> TriMesh {
    let mut m = TriMesh::default();
    prism(&mut m, Vec3::new(0.0, 0.5, 0.0), 3.2, 0.5, 8, STONE);
    prism(&mut m, Vec3::new(0.0, 0.52, 0.0), 2.9, 0.02, 8, GLASS);
    prism(&mut m, Vec3::new(0.0, 1.5, 0.0), 0.25, 1.0, 8, STONE);
    prism(&mut m, Vec3::new(0.0, 1.65, 0.0), 0.9, 0.15, 8, STONE);
    prism(&mut m, Vec3::new(0.0, 1.67, 0.0), 0.8, 0.02, 8, GLASS);
    m
}

/// A tram stop's platform `length` metres along its track, 2.6 m deep and 28 cm high (level with
/// a low-floor tram's floor), its edge on the track's side at `z = 0`. In its middle a shelter of
/// glass on a steel frame with a bench, and a totem with the next trams' times.
pub fn tram_stop(length: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let (deep, high) = (2.6, 0.28);
    cuboid(
        &mut m,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(length, high, deep),
        STONE,
        ALL & !NEG_Y,
    );
    // The tactile strip along its edge.
    cuboid(
        &mut m,
        Vec3::new(0.0, high, 0.1),
        Vec3::new(length, high + 0.005, 0.5),
        SHUTTER,
        POS_Y,
    );
    // The shelter: four posts, a glass roof, a glass back, a bench.
    let (x0, x1) = (length * 0.5 - 3.0, length * 0.5 + 3.0);
    let (z0, z1) = (0.9, 2.4);
    let roof = high + 2.6;
    for x in [x0, x1] {
        for z in [z0, z1] {
            cuboid(
                &mut m,
                Vec3::new(x - 0.04, high, z - 0.04),
                Vec3::new(x + 0.04, roof, z + 0.04),
                METAL,
                ALL & !NEG_Y & !POS_Y,
            );
        }
    }
    cuboid(
        &mut m,
        Vec3::new(x0 - 0.2, roof, z0 - 0.3),
        Vec3::new(x1 + 0.2, roof + 0.08, z1 + 0.15),
        GLASS,
        ALL,
    );
    cuboid(
        &mut m,
        Vec3::new(x0 - 0.2, roof - 0.12, z0 - 0.3),
        Vec3::new(x1 + 0.2, roof, z1 + 0.15),
        METAL,
        NO_BACK,
    );
    cuboid(
        &mut m,
        Vec3::new(x0, high + 0.3, z1 - 0.01),
        Vec3::new(x1, roof - 0.15, z1 + 0.01),
        GLASS,
        ALL,
    );
    cuboid(
        &mut m,
        Vec3::new(x0 + 0.5, high + 0.42, z1 - 0.45),
        Vec3::new(x1 - 0.5, high + 0.47, z1 - 0.05),
        WOOD,
        ALL,
    );
    // The totem: a painted post with a lit screen facing the platform's middle.
    let tx = x0 - 2.0;
    cuboid(
        &mut m,
        Vec3::new(tx - 0.25, high, 1.6),
        Vec3::new(tx + 0.25, high + 2.4, 1.75),
        SHUTTER,
        ALL & !NEG_Y,
    );
    cuboid(
        &mut m,
        Vec3::new(tx - 0.2, high + 1.3, 1.585),
        Vec3::new(tx + 0.2, high + 2.1, 1.6),
        GLASS,
        ALL,
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::SECTIONS;

    #[test]
    fn furniture_stays_in_its_place() {
        for (m, (lo, hi)) in [
            (
                terrace(4.0),
                (Vec3::new(-0.2, 0.0, 0.4), Vec3::new(4.2, 2.7, 2.8)),
            ),
            (
                tram_stop(32.0),
                (Vec3::new(-0.1, 0.0, 0.0), Vec3::new(32.1, 3.1, 2.7)),
            ),
        ] {
            assert!(m.triangle_count() > 20);
            assert_eq!(m.sections.len(), m.triangle_count());
            assert!(m.sections.iter().all(|&s| s < SECTIONS));
            for p in &m.positions {
                let p = Vec3::from(*p);
                assert!(p.cmpge(lo).all() && p.cmple(hi).all(), "{p}");
            }
        }
        // A table a pace and a half.
        let tris = |w: f32| terrace(w).triangle_count();
        assert_eq!(tris(3.2), 2 * tris(1.6));
    }
}
