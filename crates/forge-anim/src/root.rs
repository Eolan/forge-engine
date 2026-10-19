//! Root motion (#311): how a clip carries its character, for a controller to apply, and the pose
//! held in place while it does.
//!
//! A clip's travel and turn are its root joint's: its place on the ground and its turn about +Y
//! in the model's frame (a root frame above the pelvis that is not a joint is baked into the
//! root by the import, #310). [`Clip::root_motion`] gives how far and how much it moved between
//! two times of the clip, in the character's frame at the first; the controller moves and turns
//! the character by it, and [`lock_root`] takes the same out of the pose, so the body is drawn
//! where the controller has it. Applying the whole motion and locking all of it reproduces the
//! clip's own path, the hips' sway included (the controller's capsule follows it by a few
//! centimetres).
//!
//! The turn is a quaternion, the root rotation's twist about +Y, with `+ − × ÷` and `sqrt` only
//! (D-016: no transcendental function).

use glam::{Mat4, Quat, Vec3};

use crate::clip::Clip;
use crate::pose::Pose;
use crate::skeleton::Skeleton;

/// How a clip's root moved between two of its times: what a character controller applies.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RootMotion {
    /// The root's move, in the character's frame at the first time (its turn about +Y taken
    /// out): `x` and `z` along the ground, `y` up.
    pub translation: Vec3,
    /// Its turn about +Y over the interval, a unit quaternion.
    pub turn: Quat,
}

impl RootMotion {
    /// No motion.
    pub const NONE: Self = Self {
        translation: Vec3::ZERO,
        turn: Quat::IDENTITY,
    };

    /// This motion, then `next` (given in the frame this one ends in), in this one's frame: a
    /// clip played over its end and on from its start.
    pub fn then(self, next: RootMotion) -> RootMotion {
        RootMotion {
            translation: self.translation + self.turn * next.translation,
            turn: (next.turn * self.turn).normalize(),
        }
    }
}

/// The twist of `q` about +Y: the turn about the vertical it holds, a unit quaternion (the
/// identity when it holds none, as for a root lying on its side and turned a half round).
pub fn yaw_of(q: Quat) -> Quat {
    let twist = Quat::from_xyzw(0.0, q.y, 0.0, q.w);
    let length = twist.length();
    if length < 1e-6 {
        Quat::IDENTITY
    } else {
        twist / length
    }
}

impl Clip {
    /// Root joint `root`'s place and rotation in the model's frame `time` seconds into the clip
    /// (held at its ends).
    pub fn root_at(&self, skeleton: &Skeleton, root: usize, time: f32) -> (Vec3, Quat) {
        let rest = skeleton.rest();
        let tracks = &self.joints[root];
        let time = self.start + time;
        let translation = tracks
            .translation
            .as_ref()
            .map_or(rest.translations[root], |t| t.sample(time));
        let rotation = tracks
            .rotation
            .as_ref()
            .map_or(rest.rotations[root], |t| t.sample(time));
        let scale = tracks
            .scale
            .as_ref()
            .map_or(rest.scales[root], |t| t.sample(time));
        let model = skeleton.frame(root)
            * Mat4::from_scale_rotation_translation(scale, rotation, translation);
        let (_, rotation, translation) = model.to_scale_rotation_translation();
        (translation, rotation)
    }

    /// How root joint `root` moved from `from` to `to` seconds into the clip (each held at its
    /// ends), in the root's own frame at `from`: its turn about +Y then taken out. With `from`
    /// after `to` the clip is taken as looping: to its end, then on from its start.
    ///
    /// The root's frame is the character's when its rest faces the way the character does. Some
    /// rigs carry the travel on a root frame turned differently from clip to clip (along its `x`
    /// in one, its `z` in the next): there, use the ground move's length along the way the
    /// controller already knows (`translation.xz().length()`), or [`Clip::natural_speed`].
    pub fn root_motion(&self, skeleton: &Skeleton, root: usize, from: f32, to: f32) -> RootMotion {
        if from > to {
            let end = self.root_motion(skeleton, root, from, self.duration);
            return end.then(self.root_motion(skeleton, root, 0.0, to));
        }
        let (p0, r0) = self.root_at(skeleton, root, from);
        let (p1, r1) = self.root_at(skeleton, root, to);
        let (y0, y1) = (yaw_of(r0), yaw_of(r1));
        RootMotion {
            translation: y0.inverse() * (p1 - p0),
            turn: (y1 * y0.inverse()).normalize(),
        }
    }

    /// The speed root joint `root` travels at over the whole clip, metres a second along the
    /// ground: its natural speed, which a host playing it at a controller's speed divides by
    /// (a walk's loop played at the speed over it keeps its stride). 0 for a clip without
    /// duration.
    pub fn natural_speed(&self, skeleton: &Skeleton, root: usize) -> f32 {
        if self.duration <= 0.0 {
            return 0.0;
        }
        let travel = self
            .root_motion(skeleton, root, 0.0, self.duration)
            .translation;
        Vec3::new(travel.x, 0.0, travel.z).length() / self.duration
    }
}

/// Holds root joint `root` of `pose` where the controller draws the character: its place along
/// the ground at `anchor`'s and its turn about +Y at `anchor`'s (a place and a rotation in the
/// model's frame, as [`Clip::root_at`] gives them, usually the clip's start), its height and its
/// other turns kept. The motion taken out is what [`Clip::root_motion`] gives the controller.
pub fn lock_root(skeleton: &Skeleton, root: usize, pose: &mut Pose, anchor: (Vec3, Quat)) {
    let frame = skeleton.frame(root);
    let local = pose.transform(root);
    let model = frame * local.matrix();
    let (scale, rotation, translation) = model.to_scale_rotation_translation();
    let turn = yaw_of(anchor.1) * yaw_of(rotation).inverse();
    let held = Mat4::from_scale_rotation_translation(
        scale,
        (turn * rotation).normalize(),
        Vec3::new(anchor.0.x, translation.y, anchor.0.z),
    );
    let (scale, rotation, translation) = (frame.inverse() * held).to_scale_rotation_translation();
    pose.translations[root] = translation;
    pose.rotations[root] = rotation;
    pose.scales[root] = scale;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Interpolation, JointTracks, Track};
    use crate::pose::Transform;

    /// One joint 1 m up under a frame, walking 2 m along +z and turning a quarter round +Y
    /// over its second.
    fn walker() -> (Skeleton, Clip) {
        let rest = [Transform::IDENTITY];
        let skeleton = Skeleton::new(
            vec!["pelvis".into()],
            vec![None],
            vec![Mat4::from_translation(Vec3::new(0.0, 1.0, 0.0))],
            &rest,
            vec![Mat4::IDENTITY],
        )
        .unwrap();
        let quarter = Quat::from_xyzw(
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
            0.0,
            std::f32::consts::FRAC_1_SQRT_2,
        );
        let clip = Clip {
            name: "walk".into(),
            start: 0.0,
            duration: 1.0,
            joints: vec![JointTracks {
                translation: Some(Track {
                    times: vec![0.0, 1.0],
                    values: vec![Vec3::ZERO, Vec3::new(0.0, 0.0, 2.0)],
                    interpolation: Interpolation::Linear,
                }),
                rotation: Some(Track {
                    times: vec![0.0, 1.0],
                    values: vec![Quat::IDENTITY, quarter],
                    interpolation: Interpolation::Linear,
                }),
                scale: None,
            }],
        };
        (skeleton, clip)
    }

    #[test]
    fn a_clip_gives_its_travel_and_turn() {
        let (skeleton, clip) = walker();
        let m = clip.root_motion(&skeleton, 0, 0.0, 1.0);
        assert!(
            m.translation.abs_diff_eq(Vec3::new(0.0, 0.0, 2.0), 1e-5),
            "{}",
            m.translation
        );
        let quarter = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!(m.turn.abs_diff_eq(quarter, 1e-5), "{}", m.turn);
        // From halfway, the move is in the frame turned an eighth: 1 m along +z of the world
        // is along the turned frame's +z and −x.
        let half = clip.root_motion(&skeleton, 0, 0.5, 1.0);
        assert!((half.translation.length() - 1.0).abs() < 1e-5);
        assert!((half.translation.y).abs() < 1e-6);
        // The two halves make the whole.
        let whole = clip
            .root_motion(&skeleton, 0, 0.0, 0.5)
            .then(clip.root_motion(&skeleton, 0, 0.5, 1.0));
        assert!(whole.translation.abs_diff_eq(m.translation, 1e-5));
        assert!(whole.turn.abs_diff_eq(m.turn, 1e-5));
        // Over the loop's end: from 0.75 to the end, then the start to 0.25.
        let looped = clip.root_motion(&skeleton, 0, 0.75, 0.25);
        let parts = clip
            .root_motion(&skeleton, 0, 0.75, 1.0)
            .then(clip.root_motion(&skeleton, 0, 0.0, 0.25));
        assert!(looped.translation.abs_diff_eq(parts.translation, 1e-6));
    }

    #[test]
    fn a_locked_root_stays_at_its_anchor_with_its_height() {
        let (skeleton, clip) = walker();
        let anchor = clip.root_at(&skeleton, 0, 0.0);
        let mut pose = Pose::clone(skeleton.rest());
        clip.sample(&skeleton, 1.0, &mut pose);
        lock_root(&skeleton, 0, &mut pose, anchor);
        let mut model = vec![Mat4::IDENTITY];
        skeleton.model_space(&pose, &mut model);
        let (_, rotation, translation) = model[0].to_scale_rotation_translation();
        assert!(
            translation.abs_diff_eq(Vec3::new(0.0, 1.0, 0.0), 1e-5),
            "{translation}"
        );
        assert!(
            yaw_of(rotation).abs_diff_eq(Quat::IDENTITY, 1e-5),
            "{rotation}"
        );
    }

    #[test]
    fn a_clip_s_natural_speed_is_its_ground_travel_over_its_duration() {
        let (skeleton, clip) = walker();
        assert!((clip.natural_speed(&skeleton, 0) - 2.0).abs() < 1e-5);
    }
    #[test]
    fn the_twist_keeps_the_turn_about_the_vertical_only() {
        let yaw = Quat::from_rotation_y(0.7);
        let tilt = Quat::from_rotation_x(0.3);
        assert!(yaw_of(yaw * tilt).abs_diff_eq(yaw, 1e-5));
        assert!(yaw_of(tilt).abs_diff_eq(Quat::IDENTITY, 1e-6));
    }
}
