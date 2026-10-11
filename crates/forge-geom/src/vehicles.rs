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
}
