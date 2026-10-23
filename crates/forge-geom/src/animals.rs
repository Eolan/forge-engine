//! Placeholder animals (#269's fauna on the first map): blocky bodies at a species' size in a few
//! poses each, as [`crate::figures`] draws its people before real ones. An animal's feet stand
//! on `y = 0`, it faces `+x`, its sides along `z`. Two sections: [`COAT`] (its fur or plumage)
//! and [`DARK`] (its beak or nose, its legs' ends, its eyes), so one material group colours it.

use glam::{Quat, Vec3};

use crate::modules::{ALL, cuboid};
use crate::procedural::TriMesh;

/// Its fur or plumage.
pub const COAT: u8 = 0;
/// Its beak or nose, its paws or feet, its eyes.
pub const DARK: u8 = 1;
/// The number of sections.
pub const SECTIONS: u8 = 2;

/// A box of half sizes `half` turned by `rotation` round `centre`.
fn part(m: &mut TriMesh, centre: Vec3, half: Vec3, rotation: Quat, section: u8) {
    let mut b = TriMesh::default();
    cuboid(&mut b, -half, half, section, ALL);
    let base = m.positions.len() as u32;
    for (p, n) in b.positions.iter().zip(&b.normals) {
        m.positions
            .push((centre + rotation * Vec3::from(*p)).to_array());
        m.normals.push((rotation * Vec3::from(*n)).to_array());
    }
    m.indices.extend(b.indices.iter().map(|i| base + i));
    m.sections.extend(b.sections);
}

/// A bird's build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bird {
    /// Its length, beak to tail, metres.
    pub body: f32,
    /// Its wingspan over its length (a sparrow's 1.6, a swift's 2.5).
    pub span: f32,
    /// Its tail's share of its length.
    pub tail: f32,
}

/// What a bird's wings do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wings {
    /// Folded along its sides: perched, hopping.
    Folded,
    /// Spread, raised this many radians from level (negative: down).
    Spread(f32),
}

/// A bird of `build` with its `wings`: a body a third as thick as long, a head, a beak, a tail,
/// and its wings folded on its back or spread from its shoulders.
pub fn bird(build: Bird, wings: Wings) -> TriMesh {
    let mut m = TriMesh::default();
    let l = build.body;
    let id = Quat::IDENTITY;
    let thick = 0.32 * l;
    let mid = Vec3::new(0.0, 0.25 * l + thick * 0.5, 0.0);
    // The body, its head over its front, its beak, its tail behind and a little down.
    part(
        &mut m,
        mid,
        Vec3::new(0.3 * l, thick * 0.5, thick * 0.45),
        id,
        COAT,
    );
    let head = mid + Vec3::new(0.3 * l, 0.12 * l, 0.0);
    part(&mut m, head, Vec3::splat(0.11 * l), id, COAT);
    part(
        &mut m,
        head + Vec3::new(0.14 * l, -0.02 * l, 0.0),
        Vec3::new(0.05 * l, 0.03 * l, 0.03 * l),
        id,
        DARK,
    );
    let tail = Quat::from_rotation_z(0.15);
    part(
        &mut m,
        mid + Vec3::new(-0.3 * l - build.tail * l * 0.5, -0.03 * l, 0.0),
        Vec3::new(build.tail * l * 0.5, 0.015 * l, thick * 0.4),
        tail,
        COAT,
    );
    // Its legs, under the body: two short dark posts.
    for z in [-0.06 * l, 0.06 * l] {
        part(
            &mut m,
            Vec3::new(0.0, 0.125 * l, z),
            Vec3::new(0.015 * l, 0.125 * l, 0.015 * l),
            id,
            DARK,
        );
    }
    match wings {
        Wings::Folded => {
            for side in [-1.0, 1.0] {
                part(
                    &mut m,
                    mid + Vec3::new(-0.08 * l, 0.05 * l, side * thick * 0.48),
                    Vec3::new(0.3 * l, thick * 0.3, 0.02 * l),
                    id,
                    COAT,
                );
            }
        }
        Wings::Spread(up) => {
            // Each wing from the shoulder out along its side, its chord a third of the length,
            // turned up (or down) about the body's axis.
            let half_span = (build.span * l - thick) * 0.5;
            for side in [-1.0f32, 1.0] {
                let r = Quat::from_rotation_x(-side * up);
                let root = mid + Vec3::new(0.02 * l, thick * 0.25, side * thick * 0.45);
                part(
                    &mut m,
                    root + r * Vec3::new(0.0, 0.0, side * half_span * 0.5),
                    Vec3::new(0.17 * l, 0.012 * l, half_span * 0.5),
                    r,
                    COAT,
                );
            }
        }
    }
    m
}

/// A four-legged animal's build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quadruped {
    /// Its body's length, nose to rump, metres.
    pub body: f32,
    /// Its height at the shoulder over its length (a cat's 0.55, a dog's 0.75).
    pub shoulder: f32,
    /// Its tail's length over its body's, and how far it is raised, radians.
    pub tail: (f32, f32),
    /// Its ears' height over its body's length (a cat's pointed, a dog's).
    pub ears: f32,
}

/// What a four-legged animal does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gait {
    /// Standing or walking, its legs swung this many radians (positive: the near fore leg
    /// forward); 0 stands square.
    Stride(f32),
    /// Lying down, its legs folded under it.
    Lying,
}

/// A four-legged animal of `build` in `gait`: a body, a neck and head with its nose and ears, four
/// legs and a tail.
pub fn quadruped(build: Quadruped, gait: Gait) -> TriMesh {
    let mut m = TriMesh::default();
    let l = build.body;
    let id = Quat::IDENTITY;
    let shoulder = build.shoulder * l;
    let (leg, lying) = match gait {
        Gait::Stride(_) => (shoulder * 0.62, false),
        Gait::Lying => (0.0, true),
    };
    let girth = shoulder - shoulder * 0.62;
    let mid = Vec3::new(0.0, leg + girth * 0.5, 0.0);
    let width = girth * 0.8;
    part(
        &mut m,
        mid,
        Vec3::new(0.38 * l, girth * 0.5, width * 0.5),
        id,
        COAT,
    );
    // The head on its neck, raised over the shoulders, its nose and ears.
    let head = mid + Vec3::new(0.42 * l, girth * 0.55, 0.0);
    part(
        &mut m,
        head,
        Vec3::new(0.1 * l, 0.075 * l, 0.07 * l),
        id,
        COAT,
    );
    part(
        &mut m,
        head + Vec3::new(0.11 * l, -0.02 * l, 0.0),
        Vec3::new(0.025 * l, 0.02 * l, 0.025 * l),
        id,
        DARK,
    );
    for z in [-0.045 * l, 0.045 * l] {
        part(
            &mut m,
            head + Vec3::new(-0.02 * l, 0.075 * l + build.ears * l * 0.5, z),
            Vec3::new(0.02 * l, build.ears * l * 0.5, 0.015 * l),
            id,
            COAT,
        );
    }
    // The tail from the rump, raised.
    let (tail, raise) = build.tail;
    let r = Quat::from_rotation_z(-raise);
    let rump = mid + Vec3::new(-0.38 * l, girth * 0.3, 0.0);
    part(
        &mut m,
        rump + r * Vec3::new(-tail * l * 0.5, 0.0, 0.0),
        Vec3::new(tail * l * 0.5, 0.02 * l, 0.02 * l),
        r,
        COAT,
    );
    // The legs: fore and hind, near and far, swung in diagonal pairs (a walk's trot), or folded
    // under the body lying down.
    let swing = match gait {
        Gait::Stride(s) => s,
        Gait::Lying => 0.0,
    };
    for (x, z, turn) in [
        (0.3 * l, width * 0.35, swing),
        (0.3 * l, -width * 0.35, -swing),
        (-0.3 * l, width * 0.35, -swing),
        (-0.3 * l, -width * 0.35, swing),
    ] {
        if lying {
            part(
                &mut m,
                Vec3::new(x + 0.08 * l, 0.03 * l, z * 1.2),
                Vec3::new(0.12 * l, 0.03 * l, 0.03 * l),
                id,
                COAT,
            );
            continue;
        }
        let r = Quat::from_rotation_z(turn);
        let hip = Vec3::new(x, leg, z);
        part(
            &mut m,
            hip + r * Vec3::new(0.0, -leg * 0.5, 0.0),
            Vec3::new(0.03 * l, leg * 0.5, 0.03 * l),
            r,
            COAT,
        );
        part(
            &mut m,
            hip + r * Vec3::new(0.01 * l, -leg + 0.012 * l, 0.0),
            Vec3::new(0.04 * l, 0.012 * l, 0.032 * l),
            r,
            DARK,
        );
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extent(m: &TriMesh) -> (Vec3, Vec3) {
        m.positions.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(lo, hi), &p| (lo.min(Vec3::from(p)), hi.max(Vec3::from(p))),
        )
    }

    #[test]
    fn a_bird_stands_on_its_feet_at_its_length_and_spreads_its_span() {
        let sparrow = Bird {
            body: 0.15,
            span: 1.6,
            tail: 0.35,
        };
        let (lo, hi) = extent(&bird(sparrow, Wings::Folded));
        assert!(lo.y.abs() < 1e-4, "{lo}");
        assert!((hi.x - lo.x - 0.15).abs() < 0.05, "{lo} {hi}");
        let (lo, hi) = extent(&bird(sparrow, Wings::Spread(0.0)));
        assert!((hi.z - lo.z - 0.24).abs() < 0.03, "{lo} {hi}");
        let (_, raised) = extent(&bird(sparrow, Wings::Spread(0.7)));
        assert!(raised.y > hi.y + 0.03);
        assert_eq!(
            bird(sparrow, Wings::Folded).sections.len() * 3,
            bird(sparrow, Wings::Folded).indices.len()
        );
    }

    #[test]
    fn a_quadruped_stands_at_its_shoulder_and_lies_lower() {
        let cat = Quadruped {
            body: 0.46,
            shoulder: 0.55,
            tail: (0.6, 0.6),
            ears: 0.07,
        };
        let (lo, hi) = extent(&quadruped(cat, Gait::Stride(0.0)));
        assert!(lo.y.abs() < 1e-4, "{lo}");
        assert!(hi.y > 0.25 && hi.y < 0.45, "{hi}");
        let (_, lying) = extent(&quadruped(cat, Gait::Lying));
        assert!(lying.y < hi.y - 0.1, "{lying} {hi}");
    }
}
