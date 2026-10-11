//! Street furniture as part programs: café terraces and tram stops. Each piece uses the kit's
//! section layout ([`crate::modules`]: stone, frame, glass, paint, iron, fabric, wood), so the
//! kit's material groups dress it, and stands in the kit's frame: `x` along the facade or the
//! track, `y` up from the ground, `+z` away from the wall or the track.

use glam::Vec3;

use crate::modules::{
    ALL, FABRIC, GLASS, METAL, NEG_Y, NO_BACK, POS_Y, SHUTTER, STONE, WOOD, cuboid, quad,
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

/// A street-workout park round the origin, 14 m along `x` by 10 m, open on every side: a rubber
/// floor (the kit's stone colour), pull-up bars at three heights, monkey bars, parallel bars, a
/// dip station, wall bars, push-up handles and an inclined sit-up bench (the owner, 2026-10-11:
/// workout equipment rather than playgrounds; the sandbox has no children).
pub fn street_workout() -> TriMesh {
    let mut m = TriMesh::default();
    let v = Vec3::new;
    let (hx, hz) = (7.0, 5.0);
    let post = |m: &mut TriMesh, x: f32, z: f32, h: f32| {
        cuboid(
            m,
            v(x - 0.05, 0.0, z - 0.05),
            v(x + 0.05, h, z + 0.05),
            METAL,
            ALL & !NEG_Y,
        );
    };
    // A bar from `a` to `b` along x or z, 4 cm thick.
    let bar = |m: &mut TriMesh, a: Vec3, b: Vec3| {
        let (lo, hi) = (a.min(b), a.max(b));
        cuboid(
            m,
            lo - Vec3::splat(0.02),
            hi + Vec3::splat(0.02),
            METAL,
            ALL,
        );
    };
    cuboid(
        &mut m,
        v(-hx, 0.0, -hz),
        v(hx, 0.04, hz),
        STONE,
        ALL & !NEG_Y,
    );
    // Pull-up bars at 2.0, 2.3 and 2.6 m, 1.2 m wide each, on a row of posts.
    for (k, h) in [2.0, 2.3, 2.6].into_iter().enumerate() {
        let x = -6.0 + k as f32 * 1.2;
        post(&mut m, x, -3.5, h + 0.05);
        post(&mut m, x + 1.2, -3.5, h + 0.05);
        bar(&mut m, v(x, h, -3.5), v(x + 1.2, h, -3.5));
    }
    // Monkey bars: two rails 2.3 m up, 4 m long, rungs every 0.4 m.
    for (x, z) in [(-2.0, -4.0), (2.0, -4.0), (-2.0, -3.2), (2.0, -3.2)] {
        post(&mut m, x, z, 2.35);
    }
    for z in [-4.0, -3.2] {
        bar(&mut m, v(-2.0, 2.3, z), v(2.0, 2.3, z));
    }
    for k in 0..=10 {
        let x = -2.0 + k as f32 * 0.4;
        bar(&mut m, v(x, 2.3, -4.0), v(x, 2.3, -3.2));
    }
    // Parallel bars 1.2 m up, 2.5 m long, 0.5 m apart; and a dip station beside them.
    for z in [0.0, 0.5] {
        for x in [-5.5, -3.0] {
            post(&mut m, x, z, 1.2);
        }
        bar(&mut m, v(-5.5, 1.2, z), v(-3.0, 1.2, z));
    }
    for z in [1.8, 2.4] {
        post(&mut m, -4.5, z, 1.1);
        bar(&mut m, v(-4.5, 1.1, z), v(-3.8, 1.1, z));
    }
    // Wall bars: two posts 2.4 m tall, a rung every 0.25 m.
    post(&mut m, 4.0, 3.8, 2.45);
    post(&mut m, 5.0, 3.8, 2.45);
    for k in 1..=9 {
        let y = k as f32 * 0.25;
        bar(&mut m, v(4.0, y, 3.8), v(5.0, y, 3.8));
    }
    // Push-up handles: low bars on short posts.
    for x in [0.0, 1.2] {
        post(&mut m, x, 2.0, 0.3);
        post(&mut m, x, 2.6, 0.3);
        bar(&mut m, v(x, 0.3, 2.0), v(x, 0.3, 2.6));
    }
    // An inclined sit-up bench: a wooden board from 0.3 to 0.7 m up on its frame.
    let board = [
        v(4.0, 0.3, 0.0),
        v(4.0, 0.3, 0.4),
        v(5.8, 0.7, 0.4),
        v(5.8, 0.7, 0.0),
    ];
    quad(&mut m, board, WOOD);
    quad(&mut m, [board[3], board[2], board[1], board[0]], WOOD);
    post(&mut m, 4.1, 0.2, 0.3);
    post(&mut m, 5.7, 0.2, 0.7);
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

/// A chess table round the origin for a square: a stone pedestal table 0.8 m across, its top
/// inlaid with a board of 8 × 8 squares in stone and wood (close in tone, so the board does not
/// shimmer from afar), and two stools facing each other along z (the owner, 2026-10-11: give
/// people something to do in the squares).
pub fn chess_table() -> TriMesh {
    let mut m = TriMesh::default();
    let top = 0.74;
    cuboid(
        &mut m,
        Vec3::new(-0.12, 0.0, -0.12),
        Vec3::new(0.12, top - 0.06, 0.12),
        STONE,
        ALL & !NEG_Y,
    );
    cuboid(
        &mut m,
        Vec3::new(-0.4, top - 0.06, -0.4),
        Vec3::new(0.4, top, 0.4),
        STONE,
        ALL,
    );
    let cell = 0.07;
    for i in 0..8 {
        for k in 0..8 {
            let (x, z) = (-0.28 + i as f32 * cell, -0.28 + k as f32 * cell);
            let section = if (i + k) % 2 == 0 { STONE } else { WOOD };
            quad(
                &mut m,
                [
                    Vec3::new(x, top + 0.002, z),
                    Vec3::new(x, top + 0.002, z + cell),
                    Vec3::new(x + cell, top + 0.002, z + cell),
                    Vec3::new(x + cell, top + 0.002, z),
                ],
                section,
            );
        }
    }
    for (p, _) in chess_seats() {
        cuboid(
            &mut m,
            Vec3::new(p.x - 0.18, 0.0, p.z - 0.18),
            Vec3::new(p.x + 0.18, 0.45, p.z + 0.18),
            STONE,
            ALL & !NEG_Y,
        );
    }
    m
}

/// A chess table's two seats ([`chess_table`]): where each player sits and the way they face.
pub fn chess_seats() -> [(Vec3, Vec3); 2] {
    [
        (Vec3::new(0.0, 0.0, 0.62), Vec3::NEG_Z),
        (Vec3::new(0.0, 0.0, -0.62), Vec3::Z),
    ]
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

/// A tram stop's platform `length` metres along its track, 2.3 m deep and 28 cm high (level with
/// a low-floor tram's floor), its edge on the track's side at `z = 0`. In its middle a shelter of
/// glass on a steel frame with a bench, and a totem with the next trams' times.
pub fn tram_stop(length: f32) -> TriMesh {
    let mut m = TriMesh::default();
    // 2.3 m deep: from a tram's side to 5.25 m off a tram avenue's centreline, which leaves
    // the cyclists by the kerb clear (the owner saw people through a platform).
    let (deep, high) = (2.3, 0.28);
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
    let (z0, z1) = (0.8, 2.05);
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
                street_workout(),
                (Vec3::new(-7.1, 0.0, -5.1), Vec3::new(7.1, 2.7, 5.1)),
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
