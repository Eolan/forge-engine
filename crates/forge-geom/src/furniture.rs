//! Street furniture as part programs: café terraces and tram stops, and a home's furniture. Each
//! section layout ([`crate::modules`]: stone, frame, glass, paint, iron, fabric, wood), so the
//! kit's material groups dress it, and stands in the kit's frame: `x` along the facade or the
//! track, `y` up from the ground, `+z` away from the wall or the track.

use glam::Vec3;

use crate::modules::{
    ALL, FABRIC, GLASS, METAL, NEG_X, NEG_Y, NEG_Z, NO_BACK, POS_X, POS_Y, POS_Z, SHUTTER, STONE,
    WOOD, cuboid, quad, sheet, triangle,
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

/// A market stall 3 m along `x` and 2 m deep, centred on the origin, its front to `+z`: a trestle
/// table along the front with crates on it, four poles, and a canvas roof sloping down to the
/// front from 2.5 m to 2.3 m (the walkers pass under its edge).
pub fn market_stall() -> TriMesh {
    let mut m = TriMesh::default();
    let (hw, hd) = (1.5_f32, 1.0_f32);
    // The table: a board on two trestles, 0.85 m high, 0.9 m deep at the front.
    let (t0, t1) = (hd - 1.0, hd - 0.1);
    cuboid(
        &mut m,
        Vec3::new(-hw + 0.1, 0.80, t0),
        Vec3::new(hw - 0.1, 0.85, t1),
        WOOD,
        ALL,
    );
    for x in [-hw + 0.4, hw - 0.4] {
        cuboid(
            &mut m,
            Vec3::new(x - 0.04, 0.0, t0 + 0.1),
            Vec3::new(x + 0.04, 0.80, t1 - 0.1),
            WOOD,
            ALL & !NEG_Y,
        );
    }
    // Crates along the table, tilted to the buyers by their height.
    for k in 0..5 {
        let x = -hw + 0.35 + k as f32 * 0.575;
        cuboid(
            &mut m,
            Vec3::new(x - 0.25, 0.85, t0 + 0.15),
            Vec3::new(x + 0.25, 0.85 + 0.12 + 0.02 * (k % 2) as f32, t1 - 0.1),
            SHUTTER,
            ALL & !NEG_Y,
        );
    }
    // The poles and the canvas, its valance along the front.
    for (x, z, h) in [
        (-hw, -hd, 2.5),
        (hw, -hd, 2.5),
        (-hw, hd, 2.3),
        (hw, hd, 2.3),
    ] {
        cuboid(
            &mut m,
            Vec3::new(x - 0.025, 0.0, z - 0.025),
            Vec3::new(x + 0.025, h, z + 0.025),
            METAL,
            ALL & !NEG_Y,
        );
    }
    let roof = [
        Vec3::new(-hw - 0.05, 2.3, hd + 0.1),
        Vec3::new(hw + 0.05, 2.3, hd + 0.1),
        Vec3::new(hw + 0.05, 2.52, -hd - 0.05),
        Vec3::new(-hw - 0.05, 2.52, -hd - 0.05),
    ];
    quad(&mut m, roof, FABRIC);
    // Its underside 2 cm under it: the two faces on one plane folded into each other in the
    // coarser levels, and half the canvas went missing from the air.
    let under = roof.map(|p| p - Vec3::Y * 0.02);
    quad(&mut m, [under[1], under[0], under[3], under[2]], FABRIC);
    let valance = [
        Vec3::new(-hw - 0.05, 2.12, hd + 0.1),
        Vec3::new(hw + 0.05, 2.12, hd + 0.1),
        Vec3::new(hw + 0.05, 2.3, hd + 0.1),
        Vec3::new(-hw - 0.05, 2.3, hd + 0.1),
    ];
    sheet(&mut m, valance, FABRIC);
    m
}

/// A row of three bike stands 2.65 m long along `x`, centred on the origin: Sheffield stands, each
/// an upturned U of steel 0.75 m tall and 0.7 m long in the row's line, every 0.95 m, a bike
/// locked along each side.
pub fn bike_rack() -> TriMesh {
    let mut m = TriMesh::default();
    let (t, top) = (0.025_f32, 0.75_f32);
    for k in -1..=1 {
        let x = k as f32 * 0.95;
        for leg in [x - 0.35, x + 0.35] {
            cuboid(
                &mut m,
                Vec3::new(leg - t, 0.0, -t),
                Vec3::new(leg + t, top, t),
                METAL,
                ALL & !NEG_Y & !POS_Y,
            );
        }
        cuboid(
            &mut m,
            Vec3::new(x - 0.35 - t, top, -t),
            Vec3::new(x + 0.35 + t, top + 2.0 * t, t),
            METAL,
            ALL & !NEG_Y,
        );
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

/// A bollard on the origin: a steel post 0.9 m tall and 0.16 m across with a stone band near its
/// top, which shows it to anyone low-sighted and in the dark. A row of them closes a pedestrian
/// street's mouth to vehicles (D-061), 1.5 m apart so a wheelchair passes between.
pub fn bollard() -> TriMesh {
    let mut m = TriMesh::default();
    prism(&mut m, Vec3::new(0.0, 0.72, 0.0), 0.08, 0.72, 8, METAL);
    prism(&mut m, Vec3::new(0.0, 0.8, 0.0), 0.085, 0.08, 8, STONE);
    prism(&mut m, Vec3::new(0.0, 0.9, 0.0), 0.08, 0.1, 8, METAL);
    m
}

/// A taxi stand's totem on the origin, facing ±`x`: a steel post 2.6 m tall carrying a lit panel
/// 0.5 m wide and 0.9 m tall at eye height on both faces (its glass lit like a tram stop's), where
/// the automated taxis wait to be called (D-061).
pub fn taxi_totem() -> TriMesh {
    let mut m = TriMesh::default();
    prism(&mut m, Vec3::new(0.0, 2.6, 0.0), 0.06, 2.6, 8, METAL);
    cuboid(
        &mut m,
        Vec3::new(-0.06, 1.6, -0.25),
        Vec3::new(0.06, 2.5, 0.25),
        METAL,
        ALL,
    );
    for side in [-1.0_f32, 1.0] {
        let x = 0.061 * side;
        let (a, b, c, d) = (
            Vec3::new(x, 1.65, -0.22),
            Vec3::new(x, 1.65, 0.22),
            Vec3::new(x, 2.45, 0.22),
            Vec3::new(x, 2.45, -0.22),
        );
        if side > 0.0 {
            quad(&mut m, [a, d, c, b], GLASS);
        } else {
            quad(&mut m, [a, b, c, d], GLASS);
        }
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

/// A tram stop's platform `length` metres along its track, `deep` metres deep (2.3 at least: the
/// shelter's) and 28 cm high (level with a low-floor tram's floor), its edge on the track's side
/// at `z = 0`. In its middle a shelter of glass on a steel frame with a bench, and a totem with
/// the next trams' times. Deep enough, it reaches across to the pavement as one boarding area,
/// with a cycle track behind the shelter; with `back`, its back slopes down 1 in 12 as well (to
/// `z = deep + 3.36`), so a raised pavement meets it level anywhere along it.
pub fn tram_stop(length: f32, deep: f32, back: bool) -> TriMesh {
    let mut m = TriMesh::default();
    let (deep, high) = (deep.max(2.3), 0.28);
    if back {
        let run = high * 12.0;
        let (top, foot) = (Vec3::new(0.0, high, deep), Vec3::new(0.0, 0.0, deep + run));
        let x = Vec3::new(length, 0.0, 0.0);
        quad(&mut m, [foot, foot + x, top + x, top], STONE);
        let low = Vec3::new(0.0, 0.0, deep);
        for (side, out) in [(Vec3::ZERO, -1.0_f32), (x, 1.0)] {
            let tri = [low + side, top + side, foot + side];
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            if n.x * out >= 0.0 {
                triangle(&mut m, tri, STONE);
            } else {
                triangle(&mut m, [tri[0], tri[2], tri[1]], STONE);
            }
        }
        // Each back corner, where an end ramp meets the back slope: a hip of two planes, each
        // carrying one of them on, so no notch is left between the two.
        for (x0, out) in [(0.0, -1.0_f32), (length, 1.0)] {
            let top = Vec3::new(x0, high, deep);
            let a = Vec3::new(x0 + out * run, 0.0, deep);
            let b = Vec3::new(x0 + out * run, 0.0, deep + run);
            let c = Vec3::new(x0, 0.0, deep + run);
            for tri in [[top, a, b], [top, b, c]] {
                if (tri[1] - tri[0]).cross(tri[2] - tri[0]).y >= 0.0 {
                    triangle(&mut m, tri, STONE);
                } else {
                    triangle(&mut m, [tri[0], tri[2], tri[1]], STONE);
                }
            }
        }
    }
    cuboid(
        &mut m,
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(length, high, deep),
        STONE,
        ALL & !NEG_Y,
    );
    // A ramp down to the road at each end, the platform's whole depth, 1 in 12 (3.4 m): the
    // platform was reached only by a 28 cm step (the owner: not accessible).
    let run = high * 12.0;
    for (x0, x1) in [(0.0, -run), (length, length + run)] {
        let (top, foot) = (Vec3::new(x0, high, 0.0), Vec3::new(x1, 0.0, 0.0));
        let z = Vec3::new(0.0, 0.0, deep);
        // Its slope, facing up and out.
        let slope = [foot, top, top + z, foot + z];
        let up = (top - foot).cross(z);
        if up.y > 0.0 {
            quad(&mut m, slope, STONE);
        } else {
            quad(&mut m, [foot + z, top + z, top, foot], STONE);
        }
        // Its two sides.
        let low = Vec3::new(x0, 0.0, 0.0);
        for (side, out) in [(Vec3::ZERO, -1.0_f32), (z, 1.0)] {
            let tri = [low + side, top + side, foot + side];
            let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
            if n.z * out >= 0.0 {
                triangle(&mut m, tri, STONE);
            } else {
                triangle(&mut m, [tri[0], tri[2], tri[1]], STONE);
            }
        }
    }
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

/// A home's furniture (the village's houses, D-062, #266), each `w` wide and `d` deep round the
/// origin on the floor, facing +z (its back at −z): the footprints that `forge-procgen`'s
/// `city::furnish` places. Stacked parts leave out the faces they rest on, so no two faces share
/// a plane.
pub fn home(name: &str, w: f32, d: f32) -> Option<TriMesh> {
    let mut m = TriMesh::default();
    let (x0, x1, z0, z1) = (-w * 0.5, w * 0.5, -d * 0.5, d * 0.5);
    let leg = |m: &mut TriMesh, x: f32, z: f32, top: f32, s: f32, section: u8| {
        cuboid(
            m,
            Vec3::new(x - s, 0.0, z - s),
            Vec3::new(x + s, top, z + s),
            section,
            ALL & !NEG_Y & !POS_Y,
        );
    };
    let sides = ALL & !NEG_Y;
    let corners = |inset: f32| {
        [
            (x0 + inset, z0 + inset),
            (x1 - inset, z0 + inset),
            (x0 + inset, z1 - inset),
            (x1 - inset, z1 - inset),
        ]
    };
    match name {
        "bed" | "single_bed" => {
            // The frame on four legs, the mattress, the headboard at the back, the pillows.
            for (x, z) in corners(0.06) {
                leg(&mut m, x, z, 0.12, 0.03, WOOD);
            }
            let (lo, hi) = (Vec3::new(x0, 0.12, z0 + 0.06), Vec3::new(x1, 0.32, z1));
            cuboid(&mut m, lo, hi, WOOD, ALL & !POS_Y);
            let (lo, hi) = (
                Vec3::new(x0 + 0.02, 0.32, z0 + 0.08),
                Vec3::new(x1 - 0.02, 0.52, z1 - 0.02),
            );
            cuboid(&mut m, lo, hi, FABRIC, ALL & !NEG_Y);
            let (lo, hi) = (Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.95, z0 + 0.06));
            cuboid(&mut m, lo, hi, WOOD, sides);
            let pillows = if name == "bed" { 2 } else { 1 };
            let pw = (w - 0.12) / pillows as f32;
            for k in 0..pillows {
                let px = x0 + 0.06 + k as f32 * pw;
                let (lo, hi) = (
                    Vec3::new(px + 0.03, 0.52, z0 + 0.12),
                    Vec3::new(px + pw - 0.03, 0.62, z0 + 0.5),
                );
                cuboid(&mut m, lo, hi, FABRIC, ALL & !NEG_Y);
            }
        }
        "wardrobe" => {
            let (lo, hi) = (Vec3::new(x0, 0.0, z0), Vec3::new(x1, 2.0, z1));
            cuboid(&mut m, lo, hi, WOOD, sides);
            // Two doors' panels and their handles.
            for k in 0..2 {
                let a = x0 + k as f32 * w * 0.5 + 0.06;
                let b = x0 + (k + 1) as f32 * w * 0.5 - 0.06;
                let (lo, hi) = (Vec3::new(a, 0.12, z1), Vec3::new(b, 1.88, z1 + 0.015));
                cuboid(&mut m, lo, hi, WOOD, ALL & !NEG_Z);
                let hx = if k == 0 { b - 0.06 } else { a + 0.06 };
                let (lo, hi) = (
                    Vec3::new(hx - 0.01, 0.95, z1 + 0.015),
                    Vec3::new(hx + 0.01, 1.15, z1 + 0.035),
                );
                cuboid(&mut m, lo, hi, METAL, ALL & !NEG_Z);
            }
        }
        "sofa" => {
            for (x, z) in corners(0.08) {
                leg(&mut m, x, z, 0.1, 0.025, WOOD);
            }
            let (lo, hi) = (Vec3::new(x0, 0.1, z0), Vec3::new(x1, 0.42, z1));
            cuboid(&mut m, lo, hi, FABRIC, ALL & !POS_Y);
            let (lo, hi) = (Vec3::new(x0, 0.42, z0), Vec3::new(x1, 0.85, z0 + 0.22));
            cuboid(&mut m, lo, hi, FABRIC, ALL & !NEG_Y);
            for x in [x0, x1 - 0.18] {
                let (lo, hi) = (Vec3::new(x, 0.42, z0 + 0.22), Vec3::new(x + 0.18, 0.64, z1));
                cuboid(&mut m, lo, hi, FABRIC, ALL & !NEG_Y & !NEG_Z);
            }
            let seat = (w - 0.36) / 3.0;
            for k in 0..3 {
                let a = x0 + 0.18 + k as f32 * seat;
                let (lo, hi) = (
                    Vec3::new(a + 0.01, 0.42, z0 + 0.24),
                    Vec3::new(a + seat - 0.01, 0.5, z1 - 0.02),
                );
                cuboid(&mut m, lo, hi, FABRIC, ALL & !NEG_Y);
            }
        }
        "low_table" | "table" | "desk" => {
            let top = if name == "low_table" { 0.42 } else { 0.75 };
            let (lo, hi) = (Vec3::new(x0, top - 0.04, z0), Vec3::new(x1, top, z1));
            cuboid(&mut m, lo, hi, WOOD, ALL);
            for (x, z) in corners(0.06) {
                leg(&mut m, x, z, top - 0.04, 0.025, WOOD);
            }
            if name == "desk" {
                // A block of drawers under one end.
                let (lo, hi) = (
                    Vec3::new(x1 - 0.42, 0.0, z0 + 0.02),
                    Vec3::new(x1 - 0.1, top - 0.04, z1 - 0.04),
                );
                cuboid(&mut m, lo, hi, WOOD, sides & !POS_Y);
            }
        }
        "chair" => {
            for (x, z) in corners(0.04) {
                leg(&mut m, x, z, 0.43, 0.018, WOOD);
            }
            let (lo, hi) = (Vec3::new(x0, 0.43, z0), Vec3::new(x1, 0.46, z1));
            cuboid(&mut m, lo, hi, WOOD, ALL);
            let (lo, hi) = (Vec3::new(x0, 0.46, z0), Vec3::new(x1, 0.9, z0 + 0.03));
            cuboid(&mut m, lo, hi, WOOD, ALL & !NEG_Y);
        }
        "counter" => {
            // The cupboards on a plinth, the stone worktop, a sink and a hob.
            let (lo, hi) = (Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.1, z1 - 0.06));
            cuboid(&mut m, lo, hi, STONE, sides & !POS_Y);
            let (lo, hi) = (Vec3::new(x0, 0.1, z0), Vec3::new(x1, 0.86, z1 - 0.02));
            cuboid(&mut m, lo, hi, WOOD, ALL & !NEG_Y & !POS_Y);
            let (lo, hi) = (Vec3::new(x0, 0.86, z0), Vec3::new(x1, 0.9, z1));
            cuboid(&mut m, lo, hi, STONE, ALL & !NEG_Y);
            let doors = (w / 0.6).round().max(1.0) as i32;
            let dw = w / doors as f32;
            for k in 0..doors {
                let a = x0 + k as f32 * dw;
                let (lo, hi) = (
                    Vec3::new(a + 0.02, 0.14, z1 - 0.02),
                    Vec3::new(a + dw - 0.02, 0.82, z1 - 0.005),
                );
                cuboid(&mut m, lo, hi, WOOD, ALL & !NEG_Z);
            }
            for (a, b) in [(x0 + 0.3, x0 + 0.8), (x1 - 0.8, x1 - 0.2)] {
                let (lo, hi) = (Vec3::new(a, 0.9, z0 + 0.1), Vec3::new(b, 0.92, z1 - 0.12));
                cuboid(&mut m, lo, hi, METAL, ALL & !NEG_Y);
            }
        }
        "bath" => {
            // A rim round a well whose floor stands 20 cm up, and a tap.
            let r = 0.07;
            for (lo, hi, faces) in [
                (Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.58, z0 + r), sides),
                (Vec3::new(x0, 0.0, z1 - r), Vec3::new(x1, 0.58, z1), sides),
                (
                    Vec3::new(x0, 0.0, z0 + r),
                    Vec3::new(x0 + r, 0.58, z1 - r),
                    sides & !NEG_Z & !POS_Z,
                ),
                (
                    Vec3::new(x1 - r, 0.0, z0 + r),
                    Vec3::new(x1, 0.58, z1 - r),
                    sides & !NEG_Z & !POS_Z,
                ),
                (
                    Vec3::new(x0 + r, 0.0, z0 + r),
                    Vec3::new(x1 - r, 0.2, z1 - r),
                    POS_Y,
                ),
            ] {
                cuboid(&mut m, lo, hi, STONE, faces);
            }
            let (lo, hi) = (
                Vec3::new(x0 + 0.15, 0.58, z0 + 0.02),
                Vec3::new(x0 + 0.19, 0.78, z0 + 0.06),
            );
            cuboid(&mut m, lo, hi, METAL, ALL & !NEG_Y);
        }
        "basin" => {
            let (lo, hi) = (
                Vec3::new(-0.09, 0.0, z0 + 0.06),
                Vec3::new(0.09, 0.7, z0 + 0.24),
            );
            cuboid(&mut m, lo, hi, STONE, sides & !POS_Y);
            let (lo, hi) = (Vec3::new(x0, 0.7, z0), Vec3::new(x1, 0.85, z1));
            cuboid(&mut m, lo, hi, STONE, ALL & !NEG_Y);
            let (lo, hi) = (
                Vec3::new(-0.02, 0.85, z0 + 0.02),
                Vec3::new(0.02, 1.0, z0 + 0.12),
            );
            cuboid(&mut m, lo, hi, METAL, ALL & !NEG_Y);
        }
        "toilet" => {
            let (lo, hi) = (
                Vec3::new(x0 + 0.03, 0.0, z0 + 0.18),
                Vec3::new(x1 - 0.03, 0.4, z1),
            );
            cuboid(&mut m, lo, hi, STONE, sides & !POS_Y);
            let (lo, hi) = (Vec3::new(x0, 0.4, z0 + 0.18), Vec3::new(x1, 0.43, z1));
            cuboid(&mut m, lo, hi, WOOD, ALL & !NEG_Y);
            let (lo, hi) = (Vec3::new(x0, 0.0, z0), Vec3::new(x1, 0.78, z0 + 0.18));
            cuboid(&mut m, lo, hi, STONE, sides);
        }
        "shelves" => {
            for x in [x0, x1 - 0.03] {
                let (lo, hi) = (Vec3::new(x, 0.0, z0), Vec3::new(x + 0.03, 1.8, z1));
                cuboid(&mut m, lo, hi, WOOD, sides);
            }
            for k in 0..5 {
                let y = 0.08 + k as f32 * 0.42;
                let (lo, hi) = (
                    Vec3::new(x0 + 0.03, y - 0.02, z0),
                    Vec3::new(x1 - 0.03, y, z1),
                );
                cuboid(&mut m, lo, hi, WOOD, ALL & !NEG_X & !POS_X);
                // A row of books on all but the top board.
                if k == 4 {
                    continue;
                }
                let mut x = x0 + 0.05;
                let mut i = 0;
                while x + 0.05 < x1 - 0.05 {
                    let bw = 0.03 + (i * 7 % 5) as f32 * 0.008;
                    let bh = 0.22 + (i * 3 % 4) as f32 * 0.025;
                    let (lo, hi) = (
                        Vec3::new(x, y, z0 + 0.04),
                        Vec3::new(x + bw, y + bh, z1 - 0.03),
                    );
                    cuboid(&mut m, lo, hi, FABRIC, ALL & !NEG_Y);
                    x += bw + 0.004;
                    i += 1;
                }
            }
        }
        _ => return None,
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::SECTIONS;

    #[test]
    fn a_homes_furniture_stays_within_its_footprint() {
        // The footprints `forge-procgen`'s `city::furnish` places them by, and their heights.
        for (name, w, d, h) in [
            ("bed", 1.6, 2.05, 0.95),
            ("single_bed", 0.95, 2.05, 0.95),
            ("wardrobe", 1.2, 0.62, 2.0),
            ("sofa", 2.0, 0.9, 0.85),
            ("low_table", 1.0, 0.55, 0.42),
            ("counter", 2.4, 0.62, 0.92),
            ("table", 1.4, 0.85, 0.75),
            ("chair", 0.45, 0.5, 0.9),
            ("bath", 1.7, 0.72, 0.78),
            ("basin", 0.6, 0.48, 1.0),
            ("toilet", 0.4, 0.68, 0.78),
            ("desk", 1.2, 0.62, 0.75),
            ("shelves", 1.0, 0.36, 1.8),
        ] {
            let m = home(name, w, d).expect(name);
            assert!(m.triangle_count() > 0, "{name}");
            for p in &m.positions {
                let p = Vec3::from(*p);
                assert!(
                    p.x.abs() <= w * 0.5 + 1e-4
                        && p.z >= -d * 0.5 - 1e-4
                        && p.z <= d * 0.5 + 0.04
                        && p.y >= -1e-4
                        && p.y <= h + 1e-4,
                    "{name} {p}"
                );
            }
        }
        assert!(home("piano", 1.0, 1.0).is_none());
    }

    #[test]
    fn furniture_stays_in_its_place() {
        for (m, (lo, hi)) in [
            (
                bike_rack(),
                (Vec3::new(-1.35, 0.0, -0.15), Vec3::new(1.35, 0.8, 0.15)),
            ),
            (
                market_stall(),
                (Vec3::new(-1.6, 0.0, -1.1), Vec3::new(1.6, 2.6, 1.15)),
            ),
            (
                terrace(4.0, 1),
                (Vec3::new(-0.7, 0.0, -0.1), Vec3::new(4.7, 2.7, 2.95)),
            ),
            (
                street_workout(),
                (Vec3::new(-7.1, 0.0, -5.1), Vec3::new(7.1, 2.7, 5.1)),
            ),
            (
                tram_stop(32.0, 2.3, false),
                // With a 3.4 m ramp at each end.
                (Vec3::new(-3.4, 0.0, 0.0), Vec3::new(35.4, 3.1, 2.7)),
            ),
            (
                // A boarding area across to the pavement, its back sloped: the shelter kept.
                tram_stop(32.0, 3.55, true),
                (Vec3::new(-3.4, 0.0, 0.0), Vec3::new(35.4, 3.1, 3.55 + 3.36)),
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
