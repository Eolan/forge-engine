//! Inverse kinematics (#167's step 4): a chain of two bones reaching a target (a leg putting
//! its foot on the ground), a joint turned to look at a point, and when a foot comes down.
//! Like the rest of the crate, without transcendental functions (D-016): the middle joint is
//! placed by the law of cosines with a square root, and the joints turned by the arcs between
//! directions (`Quat::from_rotation_arc`).

use glam::{Mat4, Quat, Vec3};

use crate::{Pose, Skeleton};

/// Two bones: a hip, a knee and the foot at the shin's tip (or a shoulder, an elbow and a
/// hand). The tip is a point in the middle joint's frame: the next joint's place there, or
/// the bone's end where the skeleton has no joint for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chain {
    /// The first joint, which turns the whole chain.
    pub root: usize,
    /// The joint between the two bones, its parent the root, which bends.
    pub middle: usize,
    /// Where the chain ends, in the middle joint's frame: what is put on the target.
    pub tip: Vec3,
}

/// A joint's place and turn in the model's frame, from its matrix.
fn place(m: &Mat4) -> (Vec3, Quat) {
    let (_, rotation, translation) = m.to_scale_rotation_translation();
    (translation, rotation)
}

/// Turns joint `joint` of `pose` by `turn`, a rotation in the model's frame about the joint,
/// given its parent's turn in the model's frame (`parent`).
fn turn_joint(pose: &mut Pose, joint: usize, parent: Quat, turn: Quat) {
    pose.rotations[joint] = (parent.inverse() * turn * parent * pose.rotations[joint]).normalize();
}

/// Turns `chain`'s root and middle in `pose` so its end reaches `target` (in the model's
/// frame), bending its middle on the side it is posed on. Out of reach the chain stretches
/// straight towards the target; too near, it folds as far as its bones allow. Returns how far
/// the end is left from the target, metres. `model` is scratch for the model's matrices, a
/// joint each.
///
/// # Panics
///
/// When `pose` or `model` does not have one entry per joint of `skeleton`.
pub fn two_bone(
    skeleton: &Skeleton,
    pose: &mut Pose,
    chain: Chain,
    target: Vec3,
    model: &mut [Mat4],
) -> f32 {
    reach(skeleton, pose, chain, target, None, model)
}

/// [`two_bone`], its middle bent towards `pole` (a direction in the model's frame) whatever
/// side it is posed on: a joint that bends one way only (a knee, a dog's hock), whose side a
/// nearly straight pose leaves to chance. Across the line from the root to the target, its
/// middle goes where `pole` points.
///
/// # Panics
///
/// When `pose` or `model` does not have one entry per joint of `skeleton`.
pub fn two_bone_toward(
    skeleton: &Skeleton,
    pose: &mut Pose,
    chain: Chain,
    target: Vec3,
    pole: Vec3,
    model: &mut [Mat4],
) -> f32 {
    reach(skeleton, pose, chain, target, Some(pole), model)
}

fn reach(
    skeleton: &Skeleton,
    pose: &mut Pose,
    chain: Chain,
    target: Vec3,
    pole: Option<Vec3>,
    model: &mut [Mat4],
) -> f32 {
    skeleton.model_space(pose, model);
    let (hip, hip_turn) = place(&model[chain.root]);
    let (knee, _) = place(&model[chain.middle]);
    let ankle = model[chain.middle].transform_point3(chain.tip);
    let (thigh, shin) = (knee.distance(hip), ankle.distance(knee));
    let to_target = target - hip;
    let reach = to_target.length();
    if thigh <= 1e-6 || shin <= 1e-6 || reach <= 1e-6 {
        return ankle.distance(target);
    }
    let along = to_target / reach;
    // Within what the two bones can span.
    let d = reach.clamp((thigh - shin).abs() + 1e-5, thigh + shin - 1e-5);
    // The knee's side: across the line from the hip to the target, where the pole points, or
    // where it is now (any side across when the leg is straight).
    let towards = pole.unwrap_or(knee - hip);
    let mut side = towards - along * towards.dot(along);
    if side.length_squared() <= 1e-12 {
        side = along.any_orthonormal_vector();
    }
    let side = side.normalize();
    // The knee where the two bones meet: `x` along, `h` across (the law of cosines).
    let x = (thigh * thigh - shin * shin + d * d) / (2.0 * d);
    let h = (thigh * thigh - x * x).max(0.0).sqrt();
    let new_knee = hip + along * x + side * h;
    let new_ankle = hip + along * d;
    // The thigh turned onto its new direction about the hip, then the shin about the knee.
    let first = Quat::from_rotation_arc((knee - hip) / thigh, (new_knee - hip) / thigh);
    let hip_parent = skeleton
        .parent(chain.root)
        .map_or(Quat::IDENTITY, |p| place(&model[p]).1);
    turn_joint(pose, chain.root, hip_parent, first);
    let shin_now = (first * (ankle - knee)) / shin;
    let second = Quat::from_rotation_arc(shin_now, (new_ankle - new_knee) / shin);
    // The knee's parent (the hip) is now turned by `first` in the model's frame.
    let knee_parent = first * hip_turn;
    turn_joint(pose, chain.middle, knee_parent, second);
    new_ankle.distance(target)
}

/// Turns `joint` in `pose` so its `forward` axis (in its own frame) points at `target` (in the
/// model's frame), by at most the turn whose cosine is `limit` (−1 for no limit): a head
/// looking at something. `model` is scratch for the model's matrices, a joint each.
///
/// # Panics
///
/// When `pose` or `model` does not have one entry per joint of `skeleton`.
pub fn look_at(
    skeleton: &Skeleton,
    pose: &mut Pose,
    joint: usize,
    forward: Vec3,
    target: Vec3,
    limit: f32,
    model: &mut [Mat4],
) {
    skeleton.model_space(pose, model);
    let (at, turn) = place(&model[joint]);
    let now = (turn * forward).normalize_or_zero();
    let mut wanted = (target - at).normalize_or_zero();
    if now == Vec3::ZERO || wanted == Vec3::ZERO {
        return;
    }
    let cos = now.dot(wanted);
    if cos < limit {
        // As far towards it as the limit allows, in the plane of the two.
        let across = (wanted - now * cos).normalize_or(now.any_orthonormal_vector());
        let sin = (1.0 - limit * limit).max(0.0).sqrt();
        wanted = now * limit + across * sin;
    }
    let parent = skeleton
        .parent(joint)
        .map_or(Quat::IDENTITY, |p| place(&model[p]).1);
    turn_joint(pose, joint, parent, Quat::from_rotation_arc(now, wanted));
}

/// When a foot comes down (#167's foot-down events, for D-007's prints later): it is down
/// within `height` metres of the ground, and comes down again only after it has been up
/// twice as high.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FootDown {
    /// Metres over the ground within which the foot is down.
    pub height: f32,
    down: bool,
}

impl FootDown {
    /// A foot that starts up, down within `height` metres.
    pub fn new(height: f32) -> Self {
        Self {
            height,
            down: false,
        }
    }

    /// Whether the foot, `above` metres over the ground now, has just come down.
    pub fn update(&mut self, above: f32) -> bool {
        if self.down {
            if above > 2.0 * self.height {
                self.down = false;
            }
            false
        } else if above <= self.height {
            self.down = true;
            true
        } else {
            false
        }
    }

    /// Whether it is down.
    pub fn is_down(&self) -> bool {
        self.down
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Transform;

    /// A leg hanging from a hip at the origin: a thigh and a shin of 0.45 m each down −y, the
    /// thigh turned a little forward (+z) and the shin back, the knee ahead of the line from the
    /// hip to the ankle, under a root.
    fn leg() -> (Skeleton, Pose) {
        let down = |y: f32| Transform {
            translation: Vec3::new(0.0, -y, 0.0),
            ..Transform::IDENTITY
        };
        let skeleton = Skeleton::new(
            vec!["root".into(), "hip".into(), "knee".into(), "ankle".into()],
            vec![None, Some(0), Some(1), Some(2)],
            vec![Mat4::IDENTITY; 4],
            &[
                Transform::IDENTITY,
                Transform {
                    rotation: Quat::from_rotation_x(-0.2),
                    ..Transform::IDENTITY
                },
                Transform {
                    rotation: Quat::from_rotation_x(0.4),
                    ..down(0.45)
                },
                down(0.45),
            ],
            vec![Mat4::IDENTITY; 4],
        )
        .unwrap();
        let pose = skeleton.rest().clone();
        (skeleton, pose)
    }

    const CHAIN: Chain = Chain {
        root: 1,
        middle: 2,
        tip: Vec3::new(0.0, -0.45, 0.0),
    };

    fn ends(skeleton: &Skeleton, pose: &Pose) -> [Vec3; 4] {
        let mut model = vec![Mat4::IDENTITY; skeleton.len()];
        skeleton.model_space(pose, &mut model);
        std::array::from_fn(|j| model[j].w_axis.truncate())
    }

    #[test]
    fn a_leg_reaches_a_target_in_reach_its_bones_kept() {
        let (skeleton, mut pose) = leg();
        let mut model = vec![Mat4::IDENTITY; 4];
        for target in [
            Vec3::new(0.0, -0.7, 0.1),
            Vec3::new(0.2, -0.6, -0.1),
            Vec3::new(-0.1, -0.85, 0.05),
        ] {
            let left = two_bone(&skeleton, &mut pose, CHAIN, target, &mut model);
            let [_, hip, knee, ankle] = ends(&skeleton, &pose);
            assert!(
                left < 1e-4 && ankle.distance(target) < 1e-4,
                "{target}: {ankle}"
            );
            assert!((knee.distance(hip) - 0.45).abs() < 1e-4);
            assert!((ankle.distance(knee) - 0.45).abs() < 1e-4);
        }
    }

    #[test]
    fn a_leg_bends_its_knee_on_the_side_it_was_bent_and_stretches_out_of_reach() {
        let (skeleton, mut pose) = leg();
        let mut model = vec![Mat4::IDENTITY; 4];
        // Pulled up: the knee comes forward (+z), as it was bent.
        two_bone(
            &skeleton,
            &mut pose,
            CHAIN,
            Vec3::new(0.0, -0.5, 0.0),
            &mut model,
        );
        let [_, _, knee, _] = ends(&skeleton, &pose);
        assert!(knee.z > 0.2, "{knee}");
        // Out of reach: straight towards the target, 0.1 m short.
        let left = two_bone(
            &skeleton,
            &mut pose,
            CHAIN,
            Vec3::new(0.0, -1.0, 0.0),
            &mut model,
        );
        let [_, hip, knee, ankle] = ends(&skeleton, &pose);
        assert!((left - 0.1).abs() < 1e-3, "{left}");
        assert!(knee.z.abs() < 1e-2 && (ankle - hip).normalize().abs_diff_eq(Vec3::NEG_Y, 1e-3));
    }

    #[test]
    fn a_pole_bends_the_knee_its_way_whatever_side_it_was_on() {
        let (skeleton, mut pose) = leg();
        let mut model = vec![Mat4::IDENTITY; 4];
        // Posed bent forward (+z), pulled up with a pole behind: the knee goes back.
        let target = Vec3::new(0.0, -0.5, 0.0);
        let left = two_bone_toward(&skeleton, &mut pose, CHAIN, target, Vec3::NEG_Z, &mut model);
        let [_, hip, knee, ankle] = ends(&skeleton, &pose);
        assert!(left < 1e-4 && ankle.distance(target) < 1e-4, "{ankle}");
        assert!(knee.z < -0.2, "{knee}");
        assert!((knee.distance(hip) - 0.45).abs() < 1e-4);
        // And forward again with a pole ahead.
        two_bone_toward(&skeleton, &mut pose, CHAIN, target, Vec3::Z, &mut model);
        let [_, _, knee, _] = ends(&skeleton, &pose);
        assert!(knee.z > 0.2, "{knee}");
    }

    #[test]
    fn a_head_looks_at_a_point_as_far_as_its_limit() {
        let (skeleton, mut pose) = leg();
        let mut model = vec![Mat4::IDENTITY; 4];
        // The root looks along −z; turned to look at a point to its side.
        look_at(
            &skeleton,
            &mut pose,
            0,
            Vec3::NEG_Z,
            Vec3::new(1.0, 0.0, -1.0),
            -1.0,
            &mut model,
        );
        assert!(
            (pose.rotations[0] * Vec3::NEG_Z)
                .abs_diff_eq(Vec3::new(1.0, 0.0, -1.0).normalize(), 1e-5)
        );
        // Limited to a turn of cosine 0.9.
        let (skeleton, mut pose) = leg();
        look_at(
            &skeleton,
            &mut pose,
            0,
            Vec3::NEG_Z,
            Vec3::X,
            0.9,
            &mut model,
        );
        let cos = (pose.rotations[0] * Vec3::NEG_Z).dot(Vec3::NEG_Z);
        assert!((cos - 0.9).abs() < 1e-5, "{cos}");
    }

    #[test]
    fn a_foot_comes_down_once_a_step() {
        let mut foot = FootDown::new(0.02);
        let heights = [
            0.2, 0.1, 0.03, 0.015, 0.0, 0.01, 0.03, 0.05, 0.2, 0.05, 0.01, 0.0,
        ];
        let downs: Vec<usize> = heights
            .iter()
            .enumerate()
            .filter(|(_, h)| foot.update(**h))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(downs, [3, 10]);
    }
}
