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

/// A cyclist on a city bike, riding along `+x`: two wheels (shoes' section, their tyres), the
/// frame and the handlebars (bottom's), the rider upright on the saddle, hands on the bars.
pub fn cyclist(height: f32) -> TriMesh {
    let mut m = TriMesh::default();
    let k = height / 1.72;
    let r = 0.34;
    // The wheels: thin octagonal rings in the x–y plane, as boxes round each rim.
    for cx in [-0.52, 0.52] {
        for i in 0..8 {
            let a = i as f32 / 8.0 * std::f32::consts::TAU;
            let at = Vec3::new(cx + a.cos() * r, r + a.sin() * r, 0.0);
            part(
                &mut m,
                at,
                Vec3::new(0.02, 0.135, 0.02),
                Quat::from_rotation_z(a),
                SHOES,
            );
        }
    }
    // The frame: down tube, top tube, seat post, fork, the bars.
    let bar = |m: &mut TriMesh, a: Vec3, b: Vec3, w: f32, section: u8| {
        let d = b - a;
        let r = Quat::from_rotation_arc(Vec3::Y, d.normalize());
        part(
            m,
            (a + b) * 0.5,
            Vec3::new(w, d.length() * 0.5, w),
            r,
            section,
        );
    };
    let (rear, front) = (Vec3::new(-0.52, r, 0.0), Vec3::new(0.52, r, 0.0));
    let (seat, head) = (Vec3::new(-0.18, 0.95, 0.0), Vec3::new(0.38, 0.98, 0.0));
    let crank = Vec3::new(-0.05, 0.32, 0.0);
    bar(&mut m, rear, seat, 0.02, BOTTOM);
    bar(&mut m, crank, seat, 0.025, BOTTOM);
    bar(&mut m, crank, head, 0.025, BOTTOM);
    bar(&mut m, rear, crank, 0.02, BOTTOM);
    bar(&mut m, front, head, 0.025, BOTTOM);
    part(
        &mut m,
        head + Vec3::new(0.05, 0.08, 0.0),
        Vec3::new(0.02, 0.02, 0.28),
        Quat::IDENTITY,
        SHOES,
    );
    // The rider: hips over the saddle, legs to the pedals, the body leaning to the bars.
    let hip = seat + Vec3::new(0.0, 0.1, 0.0);
    for (z, foot) in [
        (0.1, Vec3::new(0.12, 0.25, 0.1)),
        (-0.1, Vec3::new(-0.2, 0.42, -0.1)),
    ] {
        let knee = Vec3::new(0.15, 0.62, z);
        bar(
            &mut m,
            hip + Vec3::new(0.0, 0.0, z),
            knee,
            0.065 * k,
            BOTTOM,
        );
        bar(&mut m, knee, foot, 0.06 * k, BOTTOM);
    }
    let shoulder = hip + Vec3::new(0.2, 0.55 * k, 0.0);
    let lean = Quat::from_rotation_arc(Vec3::Y, (shoulder - hip).normalize());
    part(
        &mut m,
        (hip + shoulder) * 0.5,
        Vec3::new(0.11, (shoulder - hip).length() * 0.5, 0.18) * Vec3::new(k, 1.0, k),
        lean,
        TOP,
    );
    for z in [0.22, -0.22] {
        bar(
            &mut m,
            shoulder + Vec3::new(0.0, -0.04, z),
            head + Vec3::new(0.05, 0.08, z * 0.9),
            0.045 * k,
            TOP,
        );
    }
    let face = shoulder + Vec3::new(0.06, 0.18 * k, 0.0);
    part(
        &mut m,
        face,
        Vec3::new(0.1, 0.12, 0.085) * k,
        Quat::IDENTITY,
        SKIN,
    );
    part(
        &mut m,
        face + Vec3::new(-0.015, 0.09, 0.0) * k,
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

    #[test]
    fn a_cyclist_rides_on_its_wheels() {
        let m = cyclist(1.72);
        let (lo, hi) = m.positions.iter().fold(
            (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
        );
        assert!(lo.y.abs() < 0.05, "{lo}");
        assert!(hi.y > 1.5 && hi.y < 2.0, "{hi}");
        assert!(lo.x > -1.0 && hi.x < 1.0 && hi.z.abs() < 0.4);
        assert!(
            m.normals
                .iter()
                .all(|n| (Vec3::from(*n).length() - 1.0).abs() < 1e-3)
        );
    }
}
