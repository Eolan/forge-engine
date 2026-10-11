//! Vehicles as part programs (`procedural-content.md` §3.4): the first is a low-floor tram in
//! articulated sections, each placed on its track by its two ends (D-059's step 7, #253). Its
//! power comes from the ground, as on Bordeaux's and Nice's lines, so it carries no pantograph
//! and the streets no overhead wires.
//!
//! A section lies along `x` (forward `+x`), centred on its origin, its wheels on `y = 0` (the
//! rails' top), `z` across. Every section uses one section layout, [`BODY`] to [`UNDER`], so one
//! material group gives a tram its livery.

use glam::Vec3;

use crate::modules::{ALL, NEG_X, NEG_Y, NEG_Z, POS_X, POS_Y, POS_Z, cuboid, quad};
use crate::procedural::TriMesh;

/// The body's paint.
pub const BODY: u8 = 0;
/// Windows and doors.
pub const GLASS: u8 = 1;
/// The livery's stripe.
pub const STRIPE: u8 = 2;
/// The running gear, skirts, bellows and roof pods.
pub const UNDER: u8 = 3;
/// The number of sections.
pub const SECTIONS: u8 = 4;

/// A section's length between its articulations, metres.
pub const SECTION_LENGTH: f32 = 6.4;
/// The gap between two sections, metres, closed by the bellows.
pub const SECTION_GAP: f32 = 0.3;
/// The car's width, metres.
pub const TRAM_WIDTH: f32 = 2.4;
/// How far a cab's nose stands past its section, metres.
pub const NOSE: f32 = 0.55;

/// A quad turned to face `out`.
fn facing(m: &mut TriMesh, mut p: [Vec3; 4], out: Vec3, section: u8) {
    if (p[2] - p[0]).cross(p[3] - p[1]).dot(out) < 0.0 {
        p.reverse();
    }
    quad(m, p, section);
}

/// One of a tram's sections: a cab, its nose forward, when `cab`; else a passenger section with
/// its bellows at both ends. Sides of glass between pillars over a painted skirt with the
/// livery's stripe, a door pair in the middle, the roof's equipment pods.
pub fn tram_section(cab: bool) -> TriMesh {
    let mut m = TriMesh::default();
    let (h, w) = (SECTION_LENGTH * 0.5, TRAM_WIDTH * 0.5);
    let sides = POS_Z | NEG_Z;
    // Running gear and skirt.
    cuboid(
        &mut m,
        Vec3::new(-h + 0.3, 0.05, -w + 0.12),
        Vec3::new(h - 0.3, 0.4, w - 0.12),
        UNDER,
        ALL,
    );
    // The lower body either side of the doors, the stripe over it.
    let door = 0.7;
    for (x0, x1) in [(-h, -door), (door, h)] {
        cuboid(
            &mut m,
            Vec3::new(x0, 0.4, -w),
            Vec3::new(x1, 1.05, w),
            BODY,
            sides | POS_Y | NEG_Y,
        );
        cuboid(
            &mut m,
            Vec3::new(x0, 0.82, -w - 0.01),
            Vec3::new(x1, 0.98, w + 0.01),
            STRIPE,
            sides,
        );
    }
    // The doors: glazed, a little inside the body, floor to roof line.
    cuboid(
        &mut m,
        Vec3::new(-door, 0.4, -w + 0.04),
        Vec3::new(door, 2.55, w - 0.04),
        GLASS,
        sides,
    );
    // The window band: glass a little inside the body, pillars flush with it.
    cuboid(
        &mut m,
        Vec3::new(-h, 1.05, -w + 0.03),
        Vec3::new(h, 2.55, w - 0.03),
        GLASS,
        sides,
    );
    let mut x = -h;
    while x <= h + 1e-3 {
        if x.abs() > door + 0.05 || (x.abs() - door).abs() < 0.1 {
            let (a, b) = ((x - 0.08).max(-h), (x + 0.08).min(h));
            cuboid(
                &mut m,
                Vec3::new(a, 1.05, -w),
                Vec3::new(b, 2.55, w),
                BODY,
                sides,
            );
        }
        x += 1.6;
    }
    // The upper body and the roof with its pods.
    cuboid(
        &mut m,
        Vec3::new(-h, 2.55, -w),
        Vec3::new(h, 3.2, w),
        BODY,
        sides | POS_Y,
    );
    cuboid(
        &mut m,
        Vec3::new(-h + 0.8, 3.2, -0.8),
        Vec3::new(h - 0.8, 3.48, 0.8),
        UNDER,
        ALL & !NEG_Y,
    );
    // The ends: a bellows to the next section, or the cab's nose.
    let bellows = |m: &mut TriMesh, x0: f32, x1: f32| {
        cuboid(
            m,
            Vec3::new(x0, 0.45, -w + 0.15),
            Vec3::new(x1, 3.05, w - 0.15),
            UNDER,
            ALL,
        );
    };
    bellows(&mut m, -h - SECTION_GAP * 0.5, -h);
    if cab {
        nose(&mut m, h, w);
    } else {
        bellows(&mut m, h, h + SECTION_GAP * 0.5);
    }
    m
}

/// A cab's nose past `x = h`: the lower front, a raked windscreen, its painted cheeks and the
/// headlights' band.
fn nose(m: &mut TriMesh, h: f32, w: f32) {
    let f = h + NOSE;
    // The lower front, rounded off by a step.
    cuboid(
        m,
        Vec3::new(h, 0.3, -w + 0.05),
        Vec3::new(f, 1.05, w - 0.05),
        BODY,
        POS_X | POS_Y | NEG_Y | POS_Z | NEG_Z,
    );
    cuboid(
        m,
        Vec3::new(h, 0.82, -w + 0.04),
        Vec3::new(f + 0.01, 0.98, w - 0.04),
        STRIPE,
        POS_X | POS_Z | NEG_Z,
    );
    // The windscreen, raked back to the roof line.
    let top = 2.9;
    let screen = [
        Vec3::new(f, 1.05, -w + 0.08),
        Vec3::new(f, 1.05, w - 0.08),
        Vec3::new(h + 0.08, top, w - 0.08),
        Vec3::new(h + 0.08, top, -w + 0.08),
    ];
    facing(m, screen, Vec3::new(1.0, 0.4, 0.0), GLASS);
    // Its cheeks.
    for z in [-w + 0.05, w - 0.05] {
        let out = Vec3::new(0.0, 0.0, z.signum());
        facing(
            m,
            [
                Vec3::new(h, 1.05, z),
                Vec3::new(f, 1.05, z),
                Vec3::new(h + 0.08, top, z),
                Vec3::new(h, top, z),
            ],
            out,
            BODY,
        );
    }
    // The roof's front over the screen.
    cuboid(
        m,
        Vec3::new(h, top, -w),
        Vec3::new(h + 0.12, 3.2, w),
        BODY,
        POS_X | POS_Y | POS_Z | NEG_Z | NEG_X,
    );
}

/// A river shuttle boat, electric, for about twenty people (Nantes' Navibus at a smaller size):
/// 9 m long and 3.2 m wide along `x`, its waterline at `y = 0`. A hull tapering to the bow
/// ([`BODY`]), a glazed cabin ([`GLASS`] between [`BODY`] posts), a roof of solar panels
/// ([`UNDER`]), the livery's band along the gunwale ([`STRIPE`]), ramps at both ends to land
/// either way.
pub fn shuttle_boat() -> TriMesh {
    let mut m = TriMesh::default();
    let (half_l, half_w) = (4.5_f32, 1.6_f32);
    // The hull: a box, its ends' corners drawn in to bows at both ends.
    let deck = 0.55;
    let keel = -0.45;
    let ring = |y: f32, narrow: f32| {
        [
            Vec3::new(-half_l, y, -half_w * narrow),
            Vec3::new(-half_l + 1.2, y, -half_w),
            Vec3::new(half_l - 1.2, y, -half_w),
            Vec3::new(half_l, y, -half_w * narrow),
            Vec3::new(half_l, y, half_w * narrow),
            Vec3::new(half_l - 1.2, y, half_w),
            Vec3::new(-half_l + 1.2, y, half_w),
            Vec3::new(-half_l, y, half_w * narrow),
        ]
    };
    let (top, bottom) = (ring(deck, 0.45), ring(keel, 0.3));
    for i in 0..8 {
        let j = (i + 1) % 8;
        let out = ((top[i] + top[j]) * 0.5).with_y(0.0);
        facing(&mut m, [bottom[i], bottom[j], top[j], top[i]], out, BODY);
    }
    // The deck and the bottom, as fans from the middle.
    for i in 0..8 {
        let j = (i + 1) % 8;
        let c = Vec3::new(0.0, deck, 0.0);
        facing(&mut m, [c, top[j], top[i], c], Vec3::Y, BODY);
        let c = Vec3::new(0.0, keel, 0.0);
        facing(&mut m, [c, bottom[i], bottom[j], c], -Vec3::Y, BODY);
    }
    // The band along the gunwale.
    for i in 0..8 {
        let j = (i + 1) % 8;
        let out = ((top[i] + top[j]) * 0.5).with_y(0.0).normalize_or_zero() * 0.01;
        let (a, b) = (top[i] + out, top[j] + out);
        let d = Vec3::new(0.0, -0.18, 0.0);
        facing(&mut m, [a + d, b + d, b, a], out, STRIPE);
    }
    // The cabin: glass sides between posts, a solar roof overhanging.
    let (cx, cz, ch) = (2.6, 1.25, deck + 2.1);
    cuboid(
        &mut m,
        Vec3::new(-cx, deck, -cz),
        Vec3::new(cx, deck + 0.7, cz),
        BODY,
        ALL & !NEG_Y,
    );
    cuboid(
        &mut m,
        Vec3::new(-cx, deck + 0.7, -cz + 0.02),
        Vec3::new(cx, ch, cz - 0.02),
        GLASS,
        ALL & !NEG_Y & !POS_Y,
    );
    for x in [-cx, -cx / 3.0, cx / 3.0, cx] {
        for z in [-cz, cz] {
            cuboid(
                &mut m,
                Vec3::new(x - 0.06, deck + 0.7, z - 0.06),
                Vec3::new(x + 0.06, ch, z + 0.06),
                BODY,
                ALL & !NEG_Y & !POS_Y,
            );
        }
    }
    cuboid(
        &mut m,
        Vec3::new(-cx - 0.5, ch, -cz - 0.2),
        Vec3::new(cx + 0.5, ch + 0.12, cz + 0.2),
        BODY,
        ALL,
    );
    cuboid(
        &mut m,
        Vec3::new(-cx - 0.4, ch + 0.12, -cz - 0.1),
        Vec3::new(cx + 0.4, ch + 0.16, cz + 0.1),
        UNDER,
        POS_Y | POS_X | NEG_X | POS_Z | NEG_Z,
    );
    // The ramps, raised at both ends.
    for s in [-1.0_f32, 1.0] {
        let x0 = s * (half_l - 0.15);
        let x1 = s * (half_l - 1.1);
        cuboid(
            &mut m,
            Vec3::new(x0.min(x1), deck, -0.55),
            Vec3::new(x0.max(x1), deck + 0.08, 0.55),
            UNDER,
            ALL,
        );
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_stay_within_their_gauge() {
        for cab in [false, true] {
            let m = tram_section(cab);
            assert!(m.triangle_count() > 50);
            assert_eq!(m.sections.len(), m.triangle_count());
            assert!(m.sections.iter().all(|&s| s < SECTIONS));
            let half = SECTION_LENGTH * 0.5;
            for p in &m.positions {
                let p = Vec3::from(*p);
                assert!(p.y >= 0.0 && p.y <= 3.5, "{p}");
                assert!(p.z.abs() <= TRAM_WIDTH * 0.5 + 0.02, "{p}");
                let front = if cab { half + NOSE } else { half } + SECTION_GAP * 0.5 + 1e-3;
                assert!(
                    p.x >= -half - SECTION_GAP * 0.5 - 1e-3 && p.x <= front,
                    "{p}"
                );
            }
            assert!(
                m.normals
                    .iter()
                    .all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-4)
            );
        }
    }

    #[test]
    fn the_boat_floats_within_its_size() {
        let m = shuttle_boat();
        assert!(m.sections.iter().all(|&s| s < SECTIONS));
        for p in &m.positions {
            let p = Vec3::from(*p);
            assert!(
                p.x.abs() <= 5.0 && p.z.abs() <= 1.7 && p.y >= -0.5 && p.y <= 3.0,
                "{p}"
            );
        }
        assert!(
            m.normals
                .iter()
                .all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3)
        );
    }
}
