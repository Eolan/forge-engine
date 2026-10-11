//! Street furniture as part programs: café terraces and tram stops. Each piece uses the kit's
//! section layout ([`crate::modules`]: stone, frame, glass, paint, iron, fabric, wood), so the
//! kit's material groups dress it, and stands in the kit's frame: `x` along the facade or the
//! track, `y` up from the ground, `+z` away from the wall or the track.

use glam::Vec3;

use crate::modules::{
    ALL, FABRIC, GLASS, METAL, NEG_Y, NO_BACK, PLASTER, POS_Y, SHUTTER, STONE, WOOD, cuboid, quad,
};
use crate::procedural::TriMesh;

/// A café's terrace `w` metres along its shopfront in one of four layouts (`variant`): round
/// tables in two staggered rows, turned their own ways, two to four bistro chairs round each,
/// a parasol over every other table (see `terrace_layout`).
pub fn terrace(w: f32, variant: u32) -> TriMesh {
    let mut m = TriMesh::default();
    for (k, (c, chairs)) in terrace_layout(w, variant).into_iter().enumerate() {
        table(&mut m, c);
        for (p, facing) in chairs {
            let mut one = TriMesh::default();
            chair(&mut one, Vec3::ZERO, 1.0);
            // The chair faces +z: turned to face its table.
            let turn = glam::Quat::from_rotation_y(facing.x.atan2(facing.z));
            let base = m.positions.len() as u32;
            for (q, n) in one.positions.iter().zip(&one.normals) {
                m.positions.push((p + turn * Vec3::from(*q)).to_array());
                m.normals.push((turn * Vec3::from(*n)).to_array());
            }
            m.indices.extend(one.indices.iter().map(|i| base + i));
            m.sections.extend(one.sections);
        }
        if k % 2 == 0 {
            parasol(&mut m, c);
        }
    }
    m
}

/// The seats of [`terrace`]`(w, variant)`: each chair's place on the ground under its seat and the
/// way a person on it faces (towards the table), in the terrace's frame.
pub fn terrace_seats(w: f32, variant: u32) -> Vec<(Vec3, Vec3)> {
    terrace_layout(w, variant)
        .into_iter()
        .flat_map(|(_, chairs)| chairs)
        .collect()
}

/// A terrace's tables and their chairs, the same for the same `w` and `variant`: a table every
/// 1.3 m or so along the shopfront, in two staggered rows (0.95 and 1.7 m out), each a little off
/// its place and turned its own way, with two to four chairs round it 0.62 m from its middle
/// (the owner, 2026-10-11: several tables round, not aligned, a bit organic). Everything stays
/// within 2.65 m of the wall: the walkers pass at 3 m.
fn terrace_layout(w: f32, variant: u32) -> Vec<(Vec3, Vec<(Vec3, Vec3)>)> {
    let hash = |k: u32, salt: u32| {
        let mut x = k.wrapping_mul(0x9E37_79B9)
            ^ salt.wrapping_mul(0x85EB_CA6B)
            ^ variant.wrapping_mul(0xC2B2_AE35);
        x ^= x >> 15;
        x = x.wrapping_mul(0x2C1B_3C6D);
        x ^= x >> 12;
        (x & 0xFFFF) as f32 / 65_535.0
    };
    let tables = (w / 1.3).floor().max(1.0) as u32;
    let step = w / tables as f32;
    (0..tables)
        .map(|k| {
            let row = if (k + variant).is_multiple_of(2) {
                0.95
            } else {
                1.7
            };
            let x = step * (k as f32 + 0.5) + (hash(k, 1) - 0.5) * step * 0.4;
            let z = row + (hash(k, 2) - 0.5) * 0.2;
            let c = Vec3::new(x.clamp(0.45, (w - 0.45).max(0.45)), 0.0, z);
            let chairs = 2 + (hash(k, 3) * 2.99) as u32;
            let turn = hash(k, 4) * std::f32::consts::TAU;
            let seats = (0..chairs)
                .map(|j| {
                    let a = turn + j as f32 * std::f32::consts::TAU / chairs as f32;
                    let out = Vec3::new(a.cos(), 0.0, a.sin());
                    (c + out * 0.62, -out)
                })
                .collect();
            (c, seats)
        })
        .collect()
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

/// A children's playground round the origin, 16 m along `x` by 12 m: a soft floor (the kit's
/// fabric colour), a low iron fence with a gate on the `-z` side, two swings on a frame, a slide
/// from a small tower with its ladder, a climbing frame, and a sandpit in a stone border (the
/// owner, 2026-10-11: playgrounds in the city).
pub fn playground() -> TriMesh {
    let mut m = TriMesh::default();
    let v = Vec3::new;
    let (hx, hz) = (8.0, 6.0);
    let post = |m: &mut TriMesh, x: f32, z: f32, h: f32, r: f32, section: u8| {
        cuboid(
            m,
            v(x - r, 0.0, z - r),
            v(x + r, h, z + r),
            section,
            ALL & !NEG_Y,
        );
    };
    cuboid(
        &mut m,
        v(-hx, 0.0, -hz),
        v(hx, 0.05, hz),
        FABRIC,
        ALL & !NEG_Y,
    );
    // The fence: a post every 2 m round the edge, a rail 0.9 m up; the gate a gap in the -z side.
    for (a, b) in [
        (v(-hx, 0.0, -hz), v(-1.0, 0.0, -hz)),
        (v(1.0, 0.0, -hz), v(hx, 0.0, -hz)),
        (v(hx, 0.0, -hz), v(hx, 0.0, hz)),
        (v(hx, 0.0, hz), v(-hx, 0.0, hz)),
        (v(-hx, 0.0, hz), v(-hx, 0.0, -hz)),
    ] {
        let posts = ((b - a).length() / 2.0).ceil().max(1.0) as u32;
        for k in 0..=posts {
            let p = a + (b - a) * (k as f32 / posts as f32);
            post(&mut m, p.x, p.z, 0.95, 0.03, METAL);
        }
        let (lo, hi) = (a.min(b), a.max(b));
        cuboid(
            &mut m,
            v(lo.x - 0.02, 0.86, lo.z - 0.02),
            v(hi.x + 0.02, 0.92, hi.z + 0.02),
            METAL,
            ALL,
        );
    }
    // Two swings: a frame of four posts and a beam 2.4 m up, each seat on two hangers.
    for (x, z) in [(-6.6, -1.8), (-6.6, 1.8), (-3.4, -1.8), (-3.4, 1.8)] {
        post(&mut m, x, z, 2.4, 0.05, METAL);
    }
    for z in [-1.8, 1.8] {
        cuboid(
            &mut m,
            v(-6.7, 2.35, z - 0.05),
            v(-3.3, 2.45, z + 0.05),
            METAL,
            ALL,
        );
    }
    for x in [-5.6, -4.4] {
        for z in [-0.25, 0.25] {
            cuboid(
                &mut m,
                v(x - 0.01, 0.45, z - 0.01),
                v(x + 0.01, 2.35, z + 0.01),
                METAL,
                ALL,
            );
        }
        cuboid(
            &mut m,
            v(x - 0.22, 0.42, -0.28),
            v(x + 0.22, 0.47, 0.28),
            WOOD,
            ALL,
        );
    }
    // The beam across the two frames.
    cuboid(&mut m, v(-5.05, 2.4, -1.8), v(-4.95, 2.5, 1.8), METAL, ALL);
    // The slide: a tower with a 1.4 m deck, its ladder, and the chute down to the floor.
    for (x, z) in [(2.0, 1.0), (3.2, 1.0), (2.0, 2.2), (3.2, 2.2)] {
        post(&mut m, x, z, 2.4, 0.06, WOOD);
    }
    cuboid(&mut m, v(1.94, 1.36, 0.94), v(3.26, 1.44, 2.26), WOOD, ALL);
    cuboid(
        &mut m,
        v(1.94, 2.3, 0.94),
        v(3.26, 2.42, 2.26),
        SHUTTER,
        ALL,
    );
    for k in 0..5 {
        let y = 0.3 + k as f32 * 0.27;
        cuboid(&mut m, v(1.4, y, 1.2), v(1.94, y + 0.04, 2.0), WOOD, ALL);
    }
    let (top, foot, (z0, z1)) = ((3.26, 1.42), (6.4, 0.3), (1.25, 1.95));
    let chute = [
        v(top.0, top.1, z0),
        v(top.0, top.1, z1),
        v(foot.0, foot.1, z1),
        v(foot.0, foot.1, z0),
    ];
    quad(&mut m, chute, SHUTTER);
    quad(&mut m, [chute[3], chute[2], chute[1], chute[0]], SHUTTER);
    // The climbing frame: a cube of bars, 2 m a side.
    for x in [-1.5, -0.5, 0.5] {
        for z in [2.5, 3.5, 4.5] {
            post(&mut m, x, z, 2.0, 0.03, METAL);
        }
    }
    for y in [0.7, 1.35, 2.0] {
        for z in [2.5, 3.5, 4.5] {
            cuboid(
                &mut m,
                v(-1.53, y - 0.03, z - 0.03),
                v(0.53, y + 0.03, z + 0.03),
                METAL,
                ALL,
            );
        }
        for x in [-1.5, -0.5, 0.5] {
            cuboid(
                &mut m,
                v(x - 0.03, y - 0.03, 2.47),
                v(x + 0.03, y + 0.03, 4.53),
                METAL,
                ALL,
            );
        }
    }
    // The sandpit: a stone border round sand (the render's warm colour).
    let (sx0, sx1, sz0, sz1) = (2.5, 6.5, -4.8, -2.0);
    cuboid(
        &mut m,
        v(sx0, 0.0, sz0),
        v(sx1, 0.12, sz1),
        PLASTER,
        ALL & !NEG_Y,
    );
    for (a, b) in [
        (v(sx0 - 0.2, 0.0, sz0 - 0.2), v(sx1 + 0.2, 0.3, sz0)),
        (v(sx0 - 0.2, 0.0, sz1), v(sx1 + 0.2, 0.3, sz1 + 0.2)),
        (v(sx0 - 0.2, 0.0, sz0), v(sx0, 0.3, sz1)),
        (v(sx1, 0.0, sz0), v(sx1 + 0.2, 0.3, sz1)),
    ] {
        cuboid(&mut m, a, b, STONE, ALL & !NEG_Y);
    }
    m
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
                terrace(4.0, 1),
                (Vec3::new(-0.7, 0.0, -0.1), Vec3::new(4.7, 2.7, 2.95)),
            ),
            (
                playground(),
                (Vec3::new(-8.1, 0.0, -6.1), Vec3::new(8.1, 2.6, 6.1)),
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
        // Two to four chairs a table, the same layout for the same variant, within reach of the
        // walkers' line 3 m out; the variants differ.
        for variant in 0..4 {
            let seats = terrace_seats(6.5, variant);
            assert!((10..=20).contains(&seats.len()), "{}", seats.len());
            assert_eq!(seats, terrace_seats(6.5, variant));
            assert!(seats.iter().all(|(p, _)| p.z > 0.0 && p.z + 0.22 < 2.7));
        }
        assert_ne!(terrace_seats(6.5, 0), terrace_seats(6.5, 1));
    }
}
