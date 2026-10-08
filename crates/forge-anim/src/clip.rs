//! Clips: keys per joint, sampled into poses.

use std::ops::{Add, Mul, Sub};

use glam::{Quat, Vec3};

use crate::pose::{Pose, lerp3, nlerp};
use crate::skeleton::Skeleton;

/// How a track goes from one key to the next (glTF's sampler interpolations).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interpolation {
    /// Holds each key until the next.
    Step,
    /// Straight (normalised for rotations).
    Linear,
    /// Hermite splines through the keys, with an in and an out tangent at each.
    CubicSpline,
}

/// One property of one joint over time.
#[derive(Clone, Debug, PartialEq)]
pub struct Track<T> {
    /// Increasing, in seconds.
    pub times: Vec<f32>,
    /// One per time, or three per time for [`Interpolation::CubicSpline`] (in tangent, value,
    /// out tangent).
    pub values: Vec<T>,
    /// Between keys.
    pub interpolation: Interpolation,
}

/// What a track's values need: a vector space, and a last step (normalising a rotation).
pub(crate) trait Key:
    Copy + Add<Output = Self> + Sub<Output = Self> + Mul<f32, Output = Self>
{
    fn lerp(a: Self, b: Self, s: f32) -> Self;
    fn finish(self) -> Self;
}

impl Key for Vec3 {
    fn lerp(a: Self, b: Self, s: f32) -> Self {
        lerp3(a, b, s)
    }
    fn finish(self) -> Self {
        self
    }
}

impl Key for Quat {
    fn lerp(a: Self, b: Self, s: f32) -> Self {
        nlerp(a, b, s)
    }
    fn finish(self) -> Self {
        self.normalize()
    }
}

#[allow(private_bounds)]
impl<T: Key> Track<T> {
    /// The value at `time`: the first key's before it, the last key's after.
    pub fn sample(&self, time: f32) -> T {
        sample_keys(
            self.times.as_slice(),
            self.interpolation,
            |k| self.values[k],
            time,
        )
    }
}

/// A track's key times as [`sample_keys`] reads them: a list, or a packed track's even steps.
pub(crate) trait KeyTimes {
    /// How many keys.
    fn count(&self) -> usize;
    /// Key `k`'s time.
    fn at(&self, k: usize) -> f32;
    /// The first key after `time` (`count` when none is).
    fn after(&self, time: f32) -> usize;
}

impl KeyTimes for [f32] {
    fn count(&self) -> usize {
        self.len()
    }
    fn at(&self, k: usize) -> f32 {
        self[k]
    }
    fn after(&self, time: f32) -> usize {
        self.partition_point(|&t| t <= time)
    }
}

/// A track's value at `time` from its key `times`, its `interpolation` and `value`, its values
/// by index (one a key, or in tangent, value and out tangent for a cubic spline): what
/// [`Track::sample`] and a packed track ([`crate::PackedClip`]) both do, the same arithmetic.
pub(crate) fn sample_keys<T: Key>(
    times: &(impl KeyTimes + ?Sized),
    interpolation: Interpolation,
    value: impl Fn(usize) -> T,
    time: f32,
) -> T {
    let cubic = interpolation == Interpolation::CubicSpline;
    let key = |k: usize| if cubic { value(3 * k + 1) } else { value(k) };
    // The first key after `time`.
    let next = times.after(time);
    if next == 0 {
        return key(0);
    }
    if next == times.count() {
        return key(next - 1);
    }
    let k = next - 1;
    let span = times.at(next) - times.at(k);
    let s = (time - times.at(k)) / span;
    // At a key, that key, to the bit.
    if s == 0.0 {
        return key(k);
    }
    match interpolation {
        Interpolation::Step => key(k),
        Interpolation::Linear => T::lerp(key(k), key(next), s),
        Interpolation::CubicSpline => {
            // glTF 2.0's appendix C: p0 + its out tangent, p1 + its in tangent, both scaled by
            // the span.
            let (p0, m0) = (key(k), value(3 * k + 2) * span);
            let (p1, m1) = (key(next), value(3 * next) * span);
            let (s2, s3) = (s * s, s * s * s);
            (p0 * (2.0 * s3 - 3.0 * s2 + 1.0)
                + m0 * (s3 - 2.0 * s2 + s)
                + p1 * (-2.0 * s3 + 3.0 * s2)
                + m1 * (s3 - s2))
                .finish()
        }
    }
}

/// A joint's tracks in a clip; a missing one keeps the rest pose's value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct JointTracks {
    /// Its translation.
    pub translation: Option<Track<Vec3>>,
    /// Its rotation, unit quaternions.
    pub rotation: Option<Track<Quat>>,
    /// Its scale.
    pub scale: Option<Track<Vec3>>,
}

/// An animation: tracks for some of a skeleton's joints, over `start .. start + duration`
/// seconds of its keys' times.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    /// Its name ("" when unnamed).
    pub name: String,
    /// The first key's time.
    pub start: f32,
    /// From the first key to the last.
    pub duration: f32,
    /// One per joint of its skeleton, in the skin's order.
    pub joints: Vec<JointTracks>,
}

impl Clip {
    /// Its bytes as it holds them: each track's values and times, four bytes a float.
    pub fn bytes(&self) -> usize {
        fn track<T>(t: &Option<Track<T>>, components: usize) -> usize {
            t.as_ref()
                .map_or(0, |t| (t.values.len() * components + t.times.len()) * 4)
        }
        self.joints
            .iter()
            .map(|j| track(&j.translation, 3) + track(&j.rotation, 4) + track(&j.scale, 3))
            .sum()
    }

    /// `time` seconds into the clip played in a loop, folded into `0 .. duration`.
    pub fn wrap(&self, time: f32) -> f32 {
        if self.duration > 0.0 {
            time.rem_euclid(self.duration)
        } else {
            0.0
        }
    }

    /// Writes the pose `time` seconds into the clip (held at its ends) into `out`; joints
    /// without a track keep `skeleton`'s rest pose.
    ///
    /// # Panics
    ///
    /// When the clip, the skeleton and `out` do not have the same number of joints.
    pub fn sample(&self, skeleton: &Skeleton, time: f32, out: &mut Pose) {
        assert!(self.joints.len() == skeleton.len() && out.len() == skeleton.len());
        let rest = skeleton.rest();
        let time = self.start + time;
        for (j, tracks) in self.joints.iter().enumerate() {
            out.translations[j] = match &tracks.translation {
                Some(track) => track.sample(time),
                None => rest.translations[j],
            };
            out.rotations[j] = match &tracks.rotation {
                Some(track) => track.sample(time),
                None => rest.rotations[j],
            };
            out.scales[j] = match &tracks.scale {
                Some(track) => track.sample(time),
                None => rest.scales[j],
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::Mat4;

    use super::*;
    use crate::pose::Transform;

    fn skeleton() -> Skeleton {
        Skeleton::new(
            vec!["root".into(), "arm".into()],
            vec![None, Some(0)],
            vec![Mat4::IDENTITY; 2],
            &[
                Transform::IDENTITY,
                Transform {
                    translation: Vec3::X,
                    ..Transform::IDENTITY
                },
            ],
            vec![Mat4::IDENTITY; 2],
        )
        .unwrap()
    }

    /// The root slides and turns over two seconds; the arm has no tracks.
    fn clip(interpolation: Interpolation) -> Clip {
        let times = vec![0.0, 0.5, 2.0];
        let positions = [
            Vec3::ZERO,
            Vec3::new(1.0, 0.25, 0.0),
            Vec3::new(3.0, 0.0, -1.0),
        ];
        let turns = [0.0, 1.2, -0.4].map(Quat::from_rotation_y);
        // Cubic keys with flat tangents.
        fn spread<T: Key>(keys: &[T], interpolation: Interpolation) -> Vec<T> {
            match interpolation {
                Interpolation::CubicSpline => {
                    keys.iter().flat_map(|&k| [k * 0.0, k, k * 0.0]).collect()
                }
                _ => keys.to_vec(),
            }
        }
        Clip {
            name: "slide".into(),
            start: 0.0,
            duration: 2.0,
            joints: vec![
                JointTracks {
                    translation: Some(Track {
                        times: times.clone(),
                        values: spread(&positions, interpolation),
                        interpolation,
                    }),
                    rotation: Some(Track {
                        times,
                        values: spread(&turns, interpolation),
                        interpolation,
                    }),
                    scale: None,
                },
                JointTracks::default(),
            ],
        }
    }

    #[test]
    fn a_clip_sampled_at_its_keys_gives_the_keys() {
        let skeleton = skeleton();
        for interpolation in [
            Interpolation::Step,
            Interpolation::Linear,
            Interpolation::CubicSpline,
        ] {
            let clip = clip(interpolation);
            let track = clip.joints[0].translation.as_ref().unwrap();
            let turns = clip.joints[0].rotation.as_ref().unwrap();
            let mut pose = skeleton.rest().clone();
            for (k, &t) in track.times.iter().enumerate() {
                clip.sample(&skeleton, t, &mut pose);
                let i = if interpolation == Interpolation::CubicSpline {
                    3 * k + 1
                } else {
                    k
                };
                assert_eq!(
                    pose.translations[0], track.values[i],
                    "{interpolation:?} at {t}"
                );
                assert_eq!(
                    pose.rotations[0], turns.values[i],
                    "{interpolation:?} at {t}"
                );
                // No track: the rest pose.
                assert_eq!(pose.transform(1), skeleton.rest().transform(1));
            }
        }
    }

    #[test]
    fn keys_are_held_before_the_first_and_after_the_last() {
        let skeleton = skeleton();
        let clip = clip(Interpolation::Linear);
        let mut pose = skeleton.rest().clone();
        clip.sample(&skeleton, -1.0, &mut pose);
        assert_eq!(pose.translations[0], Vec3::ZERO);
        clip.sample(&skeleton, 5.0, &mut pose);
        assert_eq!(pose.translations[0], Vec3::new(3.0, 0.0, -1.0));
    }

    #[test]
    fn interpolations_between_keys() {
        let skeleton = skeleton();
        let mut pose = skeleton.rest().clone();
        clip(Interpolation::Step).sample(&skeleton, 0.25, &mut pose);
        assert_eq!(pose.translations[0], Vec3::ZERO);
        clip(Interpolation::Linear).sample(&skeleton, 1.25, &mut pose);
        assert!(pose.translations[0].abs_diff_eq(Vec3::new(2.0, 0.125, -0.5), 1e-6));
        // Flat tangents: half way between two keys is their mean too, but a quarter of the
        // way is eased (5/32 of the step, not 1/4).
        clip(Interpolation::CubicSpline).sample(&skeleton, 1.25, &mut pose);
        assert!(pose.translations[0].abs_diff_eq(Vec3::new(2.0, 0.125, -0.5), 1e-6));
        clip(Interpolation::CubicSpline).sample(&skeleton, 0.125, &mut pose);
        let eased = 5.0 / 32.0;
        assert!(
            pose.translations[0].abs_diff_eq(Vec3::new(eased, 0.25 * eased, 0.0), 1e-6),
            "{}",
            pose.translations[0]
        );
        assert!(pose.rotations[0].is_normalized());
    }

    #[test]
    fn a_loop_wraps_its_time() {
        let clip = clip(Interpolation::Linear);
        assert_eq!(clip.wrap(2.5), 0.5);
        assert_eq!(clip.wrap(-0.5), 1.5);
    }

    #[test]
    fn the_same_inputs_give_the_same_bits() {
        let skeleton = skeleton();
        let (walk, slide) = (
            clip(Interpolation::CubicSpline),
            clip(Interpolation::Linear),
        );
        let run = || {
            let mut a = skeleton.rest().clone();
            let mut b = a.clone();
            let mut mixed = a.clone();
            let mut model = vec![Mat4::IDENTITY; 2];
            let mut skin = vec![Mat4::IDENTITY; 2];
            let mut bits = Vec::new();
            for frame in 0..240 {
                let t = frame as f32 / 60.0;
                walk.sample(&skeleton, walk.wrap(t), &mut a);
                slide.sample(&skeleton, slide.wrap(t * 1.3), &mut b);
                Pose::blend(&a, &b, (frame % 17) as f32 / 16.0, &mut mixed);
                skeleton.model_space(&mixed, &mut model);
                skeleton.skinning_matrices(&model, &mut skin);
                bits.extend(
                    skin.iter()
                        .flat_map(|m| m.to_cols_array())
                        .map(f32::to_bits),
                );
            }
            bits
        };
        let first = run();
        let second = std::thread::scope(|s| s.spawn(run).join().unwrap());
        assert_eq!(first, second);
    }
}
