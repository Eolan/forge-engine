//! Placeholder people: blocky figures in mid-stride, for the population's travellers before
//! #251's humans (D-059's step 6; the owner's "simple free elements first" for a proof of
//! concept). A figure stands on `y = 0`, faces `+x`, its shoulders along `z`. Every figure uses
//! one section layout ([`SKIN`] to [`SHOES`]), so one material group gives it its clothes.

use glam::{Quat, Vec3};

use crate::modules::{ALL, cuboid};
use crate::procedural::TriMesh;

/// The face and the hands.
pub const SKIN: u8 = 0;
/// The shirt, jumper or jacket.
pub const TOP: u8 = 1;
/// The trousers or the skirt.
pub const BOTTOM: u8 = 2;
/// The hair and the shoes.
pub const SHOES: u8 = 3;
/// The number of sections.
pub const SECTIONS: u8 = 4;

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

/// A figure `height` metres tall in mid-stride, its legs swung `stride` radians (positive: the
/// left leg forward; 0 stands), the arms swinging
/// against them.
pub fn walker(height: f32, stride: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let k = height / 1.72;
    let swing = stride;
    let hip = 0.92 * k;
    let leg = 0.9 * k;
    // The legs from the hips, swung about z (the foot forward along +x for a positive turn).
    for (z, turn) in [(0.09 * k, swing), (-0.09 * k, -swing)] {
        let r = Quat::from_rotation_z(turn);
        let top = Vec3::new(0.0, hip, z);
        part(
            &mut m,
            top + r * Vec3::new(0.0, -leg * 0.5, 0.0),
            Vec3::new(0.065, leg * 0.5, 0.07) * k,
            r,
            BOTTOM,
        );
        part(
            &mut m,
            top + r * Vec3::new(0.05 * k, -leg + 0.04 * k, 0.0),
            Vec3::new(0.12, 0.04, 0.06) * k,
            r,
            SHOES,
        );
    }
    // The body, the arms swinging the other way, the head and its hair.
    part(
        &mut m,
        Vec3::new(0.0, hip + 0.29 * k, 0.0),
        Vec3::new(0.11, 0.3, 0.18) * k,
        Quat::IDENTITY,
        TOP,
    );
    let shoulder = hip + 0.54 * k;
    for (z, turn) in [(0.24 * k, -swing), (-0.24 * k, swing)] {
        let r = Quat::from_rotation_z(turn * 1.1);
        let top = Vec3::new(0.0, shoulder, z);
        part(
            &mut m,
            top + r * Vec3::new(0.0, -0.28 * k, 0.0),
            Vec3::new(0.05, 0.28, 0.05) * k,
            r,
            TOP,
        );
        part(
            &mut m,
            top + r * Vec3::new(0.0, -0.6 * k, 0.0),
            Vec3::new(0.04, 0.05, 0.04) * k,
            r,
            SKIN,
        );
    }
    let head = shoulder + 0.18 * k;
    part(
        &mut m,
        Vec3::new(0.0, head, 0.0),
        Vec3::new(0.1, 0.12, 0.085) * k,
        Quat::IDENTITY,
        SKIN,
    );
    part(
        &mut m,
        Vec3::new(-0.015 * k, head + 0.09 * k, 0.0),
        Vec3::new(0.105, 0.045, 0.09) * k,
        Quat::IDENTITY,
        SHOES,
    );
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_walker_stands_on_the_ground_at_its_height() {
        for stride in [-0.3, 0.0, 0.3] {
            let m = walker(1.72, stride);
            let (lo, hi) = m.positions.iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
            );
            assert!(lo.y.abs() < 0.08, "{lo}");
            assert!((hi.y - 1.75).abs() < 0.08, "{hi}");
            assert!(hi.z < 0.4 && lo.z > -0.4);
            assert!(m.sections.iter().all(|&s| s < SECTIONS));
            assert_eq!(m.sections.len(), m.triangle_count());
        }
    }
}
