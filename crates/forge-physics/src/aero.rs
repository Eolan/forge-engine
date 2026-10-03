//! Lift and drag on flying surfaces (Phase 3's step 5, issue #141): each surface of an aeroplane
//! (a wing's half, the tailplane, the fin) is a flat plate in its body's frame with a span
//! direction and a normal; the air past it, from the body's motion and the wind, gives it an
//! angle of attack, and the classic thin-wing law gives its lift (2π per radian of incidence,
//! falling off past the stall like a flat plate) and its drag (parasitic, induced by the lift,
//! and the plate's broadside drag at high incidence). A control surface (an elevator, an
//! aileron, a rudder) deflected by δ adds to the incidence.
//!
//! As in the buoyancy, no transcendental function: the incidence is carried by its sine and
//! cosine, taken from the airflow's components, so the forces are the same bits everywhere.

use glam::{DVec3, Vec3};

use crate::{Transform, Velocity};

/// One flying surface in its body's frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Surface {
    /// Where its forces act: the centre of its quarter chord, metres.
    pub at: Vec3,
    /// Its normal (unit): the way positive lift points at zero incidence.
    pub normal: Vec3,
    /// Its chord's way forward (unit, across the normal).
    pub forward: Vec3,
    /// Its area, m².
    pub area: f32,
    /// Its aspect ratio (span² over area), for the drag its lift induces.
    pub aspect: f32,
    /// Its incidence at rest in its body (a wing's rigging), as the sine of the angle.
    pub rigging: f32,
}

/// The air.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Air {
    /// kg/m³: 1.225 at the sea's level.
    pub density: f32,
    /// The wind, m/s.
    pub wind: Vec3,
}

impl Air {
    /// Still air at the sea's level.
    pub const STILL: Self = Self {
        density: 1.225,
        wind: Vec3::ZERO,
    };
}

/// A surface's lift and drag coefficients at an incidence given by its sine and cosine.
pub fn coefficients(sin: f32, cos: f32, aspect: f32) -> (f32, f32) {
    // Up to the stall (about 14°) the thin-wing law, 2π a radian; past it the flat plate's
    // 2 sin cos, which the lift falls to by 20°.
    const STALL: f32 = 0.24;
    const FALLEN: f32 = 0.34;
    let thin = 2.0 * std::f32::consts::PI * sin;
    let plate = 2.0 * sin * cos;
    let s = sin.abs();
    let lift = if s <= STALL {
        thin
    } else if s >= FALLEN {
        plate
    } else {
        let t = (s - STALL) / (FALLEN - STALL);
        thin + (plate - thin) * t
    };
    // Parasitic drag, the drag the lift induces (Oswald efficiency 0.8), and the plate's.
    let induced = lift * lift / (std::f32::consts::PI * 0.8 * aspect.max(0.5));
    let drag = 0.03 + induced + 1.2 * s * s;
    (lift, drag)
}

/// The air's push on a body of `surfaces`, each deflected by its `deflections` (the sine of a
/// control surface's angle, 0 for none), moving at `velocity` about `center_of_mass`: a force
/// and a torque about the body's origin.
pub fn push(
    surfaces: &[Surface],
    deflections: &[f32],
    at: Transform,
    velocity: Velocity,
    center_of_mass: DVec3,
    air: &Air,
) -> (Vec3, Vec3) {
    let com = (center_of_mass - at.position).as_vec3();
    let (mut force, mut torque) = (Vec3::ZERO, Vec3::ZERO);
    for (k, s) in surfaces.iter().enumerate() {
        let r = at.rotation * s.at;
        let (normal, forward) = (at.rotation * s.normal, at.rotation * s.forward);
        // The air past the surface: against its motion, with the wind.
        let flow = air.wind - (velocity.linear + velocity.angular.cross(r - com));
        // Only the flow in the surface's own plane of action (normal and chord) lifts it; the
        // span's share slides along it.
        let span = normal.cross(forward);
        let flow = flow - span * flow.dot(span);
        let speed2 = flow.length_squared();
        if speed2 < 1e-4 {
            continue;
        }
        let speed = speed2.sqrt();
        // The incidence: the flow coming up into the surface from under (along its normal) and
        // back along its chord.
        let (up, back) = (flow.dot(normal) / speed, -flow.dot(forward) / speed);
        // The rigging and the deflection add their angles: sin(a + b) = sa cb + ca sb.
        let extra = (s.rigging + deflections.get(k).copied().unwrap_or(0.0)).clamp(-0.5, 0.5);
        let extra_cos = (1.0 - extra * extra).sqrt();
        let (sin, cos) = (up * extra_cos + back * extra, back * extra_cos - up * extra);
        let (cl, cd) = coefficients(sin, cos, s.aspect);
        let q = 0.5 * air.density * speed2 * s.area;
        // Drag along the flow; lift across it, in the plane of the normal and the flow.
        let along = flow / speed;
        let across = span.cross(along).normalize_or_zero();
        let f = q * (cd * along + cl * across);
        force += f;
        torque += r.cross(f);
    }
    (force, torque)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    /// A wing of 16 m², its normal up and its chord forward along −z.
    fn wing() -> Surface {
        Surface {
            at: Vec3::ZERO,
            normal: Vec3::Y,
            forward: Vec3::NEG_Z,
            area: 16.0,
            aspect: 6.25,
            rigging: 0.0,
        }
    }

    fn flying(speed: f32, sin_pitch: f32) -> (Transform, Velocity) {
        let cos = (1.0 - sin_pitch * sin_pitch).sqrt();
        // Nose up by the angle: a turn about +x (the half angle's sine and cosine).
        let (hs, hc) = (((1.0 - cos) / 2.0).sqrt(), ((1.0 + cos) / 2.0).sqrt());
        let at = Transform {
            position: DVec3::ZERO,
            rotation: Quat::from_xyzw(hs * sin_pitch.signum(), 0.0, 0.0, hc),
        };
        let v = Velocity {
            linear: Vec3::new(0.0, 0.0, -speed),
            angular: Vec3::ZERO,
        };
        (at, v)
    }

    #[test]
    fn a_wing_lifts_with_its_incidence_and_stalls() {
        let lift = |sin: f32| {
            let (at, v) = flying(40.0, sin);
            push(&[wing()], &[], at, v, DVec3::ZERO, &Air::STILL).0.y
        };
        assert!(
            lift(0.0).abs() < 1.0,
            "no incidence, no lift: {}",
            lift(0.0)
        );
        // At 5° and 40 m/s: q S 2π sin, about 8.6 kN.
        let five = lift(0.087);
        assert!((7800.0..9400.0).contains(&five), "{five}");
        assert!(lift(0.17) > five, "more incidence, more lift");
        assert!(lift(0.45) < lift(0.2), "past the stall the lift falls");
        assert!(lift(-0.087) < -0.9 * five, "negative incidence pushes down");
    }

    #[test]
    fn drag_opposes_the_flight_and_grows_with_the_lift() {
        let (at, v) = flying(40.0, 0.0);
        let level = push(&[wing()], &[], at, v, DVec3::ZERO, &Air::STILL).0;
        assert!(level.z > 0.0, "drag backwards: {level}");
        let (at, v) = flying(40.0, 0.15);
        let lifting = push(&[wing()], &[], at, v, DVec3::ZERO, &Air::STILL).0;
        assert!(lifting.z > level.z, "induced drag");
    }

    #[test]
    fn a_tailplane_behind_the_centre_of_mass_steadies_the_pitch() {
        let tail = Surface {
            at: Vec3::new(0.0, 0.0, 4.5),
            area: 3.0,
            aspect: 4.0,
            ..wing()
        };
        // Nose up: the tail meets the air at an incidence and lifts, pushing the nose down.
        let (at, v) = flying(40.0, 0.1);
        let (_, torque) = push(&[tail], &[], at, v, DVec3::ZERO, &Air::STILL);
        assert!(torque.x < 0.0, "a nose-down torque: {torque}");
        // An elevator up (negative deflection) pulls the tail down: nose up.
        let (at, v) = flying(40.0, 0.0);
        let (_, torque) = push(&[tail], &[-0.2], at, v, DVec3::ZERO, &Air::STILL);
        assert!(torque.x > 0.0, "a nose-up torque: {torque}");
    }
}
