//! Packed clips (#167's step 5, D-012): ACL's approach (Nicholas Frechette's Animation
//! Compression Library) in its simplest form. Each track's values are reduced to their range
//! and stored on as few bits as keep the skeleton within a tolerance, the bits chosen track by
//! track against the error measured on the posed joints and on a point a little way out from
//! each, as ACL measures it on virtual vertices. A constant track keeps no bits at all.
//!
//! Evenly spaced keys keep their first time, their step and their count, other keys their list
//! (ACL resamples every track evenly: a later step). A packed track interpolates its keys as its
//! clip did ([`Clip::sample`]), the same arithmetic, so a
//! packed clip samples to the bits of its own unpacked values (D-016).

use glam::{Mat4, Quat, Vec3};

use crate::clip::{Interpolation, Key, KeyTimes, sample_keys};
use crate::{Clip, Pose, Skeleton, Track};

/// The fewest and the most bits a component is stored on.
const MIN_BITS: u8 = 3;
const MAX_BITS: u8 = 16;

/// How far out from each joint the error is also measured, metres: a skin's vertices lie
/// about there.
const SHELL: f32 = 0.1;

/// A value a packed track holds: its components as floats.
trait Packable: Key {
    const COMPONENTS: usize;
    fn components(self) -> [f32; 4];
    fn from_components(c: [f32; 4]) -> Self;
}

impl Packable for Vec3 {
    const COMPONENTS: usize = 3;
    fn components(self) -> [f32; 4] {
        [self.x, self.y, self.z, 0.0]
    }
    fn from_components(c: [f32; 4]) -> Self {
        Vec3::new(c[0], c[1], c[2])
    }
}

impl Packable for Quat {
    const COMPONENTS: usize = 4;
    fn components(self) -> [f32; 4] {
        self.to_array()
    }
    fn from_components(c: [f32; 4]) -> Self {
        Quat::from_array(c)
    }
}

/// A packed track's key times: evenly spaced ones by their first, their step and their count
/// (Blender bakes a key a frame), or else the list.
#[derive(Clone, Debug, PartialEq)]
enum Times {
    Even { first: f32, step: f32, count: usize },
    Keys(Vec<f32>),
}

impl Times {
    fn new(times: &[f32]) -> Self {
        if let [first, .., last] = *times {
            let step = (last - first) / (times.len() - 1) as f32;
            let even = |(k, &t): (usize, &f32)| (t - (first + step * k as f32)).abs() <= 1e-5;
            if step > 0.0 && times.iter().enumerate().all(even) {
                return Self::Even {
                    first,
                    step,
                    count: times.len(),
                };
            }
        }
        Self::Keys(times.to_vec())
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Even { .. } => 12,
            Self::Keys(times) => times.len() * 4,
        }
    }
}

impl KeyTimes for Times {
    fn count(&self) -> usize {
        match self {
            Self::Even { count, .. } => *count,
            Self::Keys(times) => times.len(),
        }
    }

    fn at(&self, k: usize) -> f32 {
        match self {
            Self::Even { first, step, .. } => first + step * k as f32,
            Self::Keys(times) => times[k],
        }
    }

    fn after(&self, time: f32) -> usize {
        match self {
            Self::Keys(times) => times.partition_point(|&t| t <= time),
            Self::Even { first, step, count } => {
                // A guess from the step, then to the key the times as computed put it at.
                let guess = ((time - first) / step).floor();
                let mut next = if guess >= 0.0 {
                    (guess as usize + 1).min(*count)
                } else {
                    0
                };
                while next < *count && self.at(next) <= time {
                    next += 1;
                }
                while next > 0 && self.at(next - 1) > time {
                    next -= 1;
                }
                next
            }
        }
    }
}

/// A track's values on `bits` bits a component, each over its range: `min + step × q`.
#[derive(Clone, Debug, PartialEq)]
struct Packed<T> {
    times: Times,
    interpolation: Interpolation,
    /// The values the track holds (one a key, three for a cubic spline).
    count: usize,
    bits: u8,
    min: [f32; 4],
    step: [f32; 4],
    data: Vec<u32>,
    /// The unpacked values, while the bits are chosen.
    source: Vec<T>,
}

impl<T: Packable> Packed<T> {
    fn new(track: &Track<T>, bits: u8) -> Self {
        let (mut min, mut max) = ([f32::MAX; 4], [f32::MIN; 4]);
        for v in &track.values {
            let c = v.components();
            for i in 0..T::COMPONENTS {
                min[i] = min[i].min(c[i]);
                max[i] = max[i].max(c[i]);
            }
        }
        let mut packed = Self {
            times: Times::new(&track.times),
            interpolation: track.interpolation,
            count: track.values.len(),
            bits: 0,
            min,
            step: [0.0; 4],
            data: Vec::new(),
            source: track.values.clone(),
        };
        packed.repack(bits, max);
        packed
    }

    /// Packs the values again on `bits` bits (none for a constant track).
    fn repack(&mut self, bits: u8, max: [f32; 4]) {
        let constant = (0..T::COMPONENTS).all(|i| max[i] <= self.min[i]);
        self.bits = if constant { 0 } else { bits };
        let levels = if self.bits == 0 {
            0.0
        } else {
            ((1u32 << self.bits) - 1) as f32
        };
        self.step = std::array::from_fn(|i| {
            if levels > 0.0 {
                (max[i] - self.min[i]) / levels
            } else {
                0.0
            }
        });
        self.data.clear();
        let mut writer = Bits::default();
        for v in &self.source {
            let c = v.components();
            for (i, &c) in c.iter().enumerate().take(T::COMPONENTS) {
                let q = if self.step[i] > 0.0 {
                    ((c - self.min[i]) / self.step[i])
                        .round()
                        .clamp(0.0, levels) as u32
                } else {
                    0
                };
                writer.push(&mut self.data, q, self.bits);
            }
        }
    }

    /// Value `k`, unpacked.
    fn value(&self, k: usize) -> T {
        let mut c = [0.0; 4];
        let bits = usize::from(self.bits);
        for (i, c) in c.iter_mut().enumerate().take(T::COMPONENTS) {
            let q = read(&self.data, (k * T::COMPONENTS + i) * bits, self.bits);
            *c = self.min[i] + self.step[i] * q as f32;
        }
        T::from_components(c).finish()
    }

    fn sample(&self, time: f32) -> T {
        sample_keys(&self.times, self.interpolation, |k| self.value(k), time)
    }

    /// Its bytes as it would be stored: the packed values, the range and the times, and its bits
    /// in a byte; a constant track its one value.
    fn bytes(&self) -> usize {
        if self.bits == 0 {
            T::COMPONENTS * 4 + 1
        } else {
            self.data.len() * 4 + 2 * T::COMPONENTS * 4 + self.times.bytes() + 2
        }
    }
}

/// Appends values of a few bits to a stream of `u32`s.
#[derive(Default)]
struct Bits {
    used: usize,
}

impl Bits {
    fn push(&mut self, data: &mut Vec<u32>, value: u32, bits: u8) {
        for b in 0..usize::from(bits) {
            let at = self.used + b;
            if at / 32 >= data.len() {
                data.push(0);
            }
            data[at / 32] |= ((value >> b) & 1) << (at % 32);
        }
        self.used += usize::from(bits);
    }
}

/// The value of `bits` bits at bit `at` of `data`.
fn read(data: &[u32], at: usize, bits: u8) -> u32 {
    if bits == 0 {
        return 0;
    }
    // The two words it may span, as one, shifted down and masked.
    let (word, shift) = (at / 32, at % 32);
    let low = u64::from(data[word]);
    let high = data.get(word + 1).map_or(0, |&w| u64::from(w));
    (((low | (high << 32)) >> shift) & ((1u64 << bits) - 1)) as u32
}

/// A joint's packed tracks.
#[derive(Clone, Debug, Default, PartialEq)]
struct PackedJoint {
    translation: Option<Packed<Vec3>>,
    rotation: Option<Packed<Quat>>,
    scale: Option<Packed<Vec3>>,
}

/// A clip packed to a tolerance ([`PackedClip::new`]).
#[derive(Clone, Debug, PartialEq)]
pub struct PackedClip {
    /// Its name.
    pub name: String,
    /// The first key's time, and from the first key to the last, as the clip's.
    pub start: f32,
    /// See `start`.
    pub duration: f32,
    joints: Vec<PackedJoint>,
}

/// A track of a joint, to choose its bits: the joint, and which of its tracks.
#[derive(Clone, Copy, Debug)]
enum Which {
    Translation(usize),
    Rotation(usize),
    Scale(usize),
}

impl PackedClip {
    /// `clip` of `skeleton` packed so that no joint, nor any point `SHELL` metres out from one,
    /// strays more than `tolerance` metres from where the clip puts it, at its keys and between
    /// them.
    ///
    /// # Panics
    ///
    /// When the clip and the skeleton do not have the same number of joints.
    pub fn new(clip: &Clip, skeleton: &Skeleton, tolerance: f32) -> Self {
        assert_eq!(clip.joints.len(), skeleton.len());
        let mut packed = Self {
            name: clip.name.clone(),
            start: clip.start,
            duration: clip.duration,
            joints: clip
                .joints
                .iter()
                .map(|j| PackedJoint {
                    translation: j.translation.as_ref().map(|t| Packed::new(t, MAX_BITS)),
                    rotation: j.rotation.as_ref().map(|t| Packed::new(t, MAX_BITS)),
                    scale: j.scale.as_ref().map(|t| Packed::new(t, MAX_BITS)),
                })
                .collect(),
        };
        let reference = Reference::new(clip, skeleton);
        let tracks: Vec<Which> = (0..clip.joints.len())
            .flat_map(|j| [Which::Translation(j), Which::Rotation(j), Which::Scale(j)])
            .collect();
        for which in tracks {
            if !packed.has(which) {
                continue;
            }
            // The fewest bits that keep the error within the tolerance, the other tracks as
            // they are: the error falls as the bits grow.
            let (mut low, mut high) = (MIN_BITS, MAX_BITS);
            while low < high {
                let mid = (low + high) / 2;
                packed.set_bits(which, mid);
                if reference.error(&packed, skeleton) <= tolerance {
                    high = mid;
                } else {
                    low = mid + 1;
                }
            }
            packed.set_bits(which, low);
        }
        for j in &mut packed.joints {
            for t in j.translation.iter_mut() {
                t.source = Vec::new();
            }
            for t in j.rotation.iter_mut() {
                t.source = Vec::new();
            }
            for t in j.scale.iter_mut() {
                t.source = Vec::new();
            }
        }
        packed
    }

    fn has(&self, which: Which) -> bool {
        match which {
            Which::Translation(j) => self.joints[j].translation.is_some(),
            Which::Rotation(j) => self.joints[j].rotation.is_some(),
            Which::Scale(j) => self.joints[j].scale.is_some(),
        }
    }

    fn set_bits(&mut self, which: Which, bits: u8) {
        match which {
            Which::Translation(j) => {
                if let Some(t) = &mut self.joints[j].translation {
                    let max = t.max_of_source();
                    t.repack(bits, max);
                }
            }
            Which::Rotation(j) => {
                if let Some(t) = &mut self.joints[j].rotation {
                    let max = t.max_of_source();
                    t.repack(bits, max);
                }
            }
            Which::Scale(j) => {
                if let Some(t) = &mut self.joints[j].scale {
                    let max = t.max_of_source();
                    t.repack(bits, max);
                }
            }
        }
    }

    /// Writes the pose `time` seconds into the clip into `out`, as [`Clip::sample`] does.
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

    /// `time` seconds into the clip played in a loop, folded into `0 .. duration`.
    pub fn wrap(&self, time: f32) -> f32 {
        if self.duration > 0.0 {
            time.rem_euclid(self.duration)
        } else {
            0.0
        }
    }

    /// Its bytes.
    pub fn bytes(&self) -> usize {
        self.joints
            .iter()
            .map(|j| {
                j.translation.as_ref().map_or(0, Packed::bytes)
                    + j.rotation.as_ref().map_or(0, Packed::bytes)
                    + j.scale.as_ref().map_or(0, Packed::bytes)
            })
            .sum()
    }

    /// The bits its tracks are stored on, from the fewest to the most, constant tracks at 0.
    pub fn bits(&self) -> Vec<u8> {
        let mut bits: Vec<u8> = self
            .joints
            .iter()
            .flat_map(|j| {
                [
                    j.translation.as_ref().map(|t| t.bits),
                    j.rotation.as_ref().map(|t| t.bits),
                    j.scale.as_ref().map(|t| t.bits),
                ]
            })
            .flatten()
            .collect();
        bits.sort_unstable();
        bits
    }
}

impl<T: Packable> Packed<T> {
    /// The top of the source values' range.
    fn max_of_source(&self) -> [f32; 4] {
        let mut max = [f32::MIN; 4];
        for v in &self.source {
            let c = v.components();
            for i in 0..T::COMPONENTS {
                max[i] = max[i].max(c[i]);
            }
        }
        max
    }
}

/// Where a clip puts the joints and the points out from them, at the times the error is
/// measured: every key and half way between, and every 60th of a second.
struct Reference {
    times: Vec<f32>,
    points: Vec<Vec<Vec3>>,
}

impl Reference {
    fn new(clip: &Clip, skeleton: &Skeleton) -> Self {
        let mut times: Vec<f32> = clip
            .joints
            .iter()
            .flat_map(|j| {
                let t = |track: Option<&Vec<f32>>| track.cloned().unwrap_or_default();
                [
                    t(j.translation.as_ref().map(|t| &t.times)),
                    t(j.rotation.as_ref().map(|t| &t.times)),
                    t(j.scale.as_ref().map(|t| &t.times)),
                ]
            })
            .flatten()
            .map(|t| t - clip.start)
            .collect();
        let steps = (clip.duration * 60.0).ceil() as usize;
        times.extend((0..=steps).map(|k| clip.duration * k as f32 / steps.max(1) as f32));
        times.sort_by(f32::total_cmp);
        times.dedup();
        let halves: Vec<f32> = times.windows(2).map(|w| 0.5 * (w[0] + w[1])).collect();
        times.extend(halves);
        let mut pose = skeleton.rest().clone();
        let points = times
            .iter()
            .map(|&t| {
                clip.sample(skeleton, t, &mut pose);
                points(skeleton, &pose)
            })
            .collect();
        Self { times, points }
    }

    /// The farthest any point of `packed`'s poses is from the clip's.
    fn error(&self, packed: &PackedClip, skeleton: &Skeleton) -> f32 {
        let mut pose = skeleton.rest().clone();
        let mut worst = 0.0_f32;
        for (t, reference) in self.times.iter().zip(&self.points) {
            packed.sample(skeleton, *t, &mut pose);
            for (a, b) in points(skeleton, &pose).iter().zip(reference) {
                worst = worst.max(a.distance(*b));
            }
        }
        worst
    }
}

/// Each joint's origin in the model's frame, and points `SHELL` out along its three axes.
fn points(skeleton: &Skeleton, pose: &Pose) -> Vec<Vec3> {
    let mut model = vec![Mat4::IDENTITY; skeleton.len()];
    skeleton.model_space(pose, &mut model);
    model
        .iter()
        .flat_map(|m| {
            [
                m.transform_point3(Vec3::ZERO),
                m.transform_point3(Vec3::X * SHELL),
                m.transform_point3(Vec3::Y * SHELL),
                m.transform_point3(Vec3::Z * SHELL),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::JointTracks;
    use crate::{Transform, load_rigs};

    fn rigs() -> Vec<crate::Rig> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/models/skinned-creatures.glb"
        );
        load_rigs(&std::fs::read(path).unwrap()).unwrap()
    }

    #[test]
    fn the_lab_s_clips_pack_within_a_tenth_of_a_millimetre_in_a_fraction_of_their_bytes() {
        for rig in rigs() {
            for clip in &rig.clips {
                let packed = PackedClip::new(clip, &rig.skeleton, 1e-4);
                let reference = Reference::new(clip, &rig.skeleton);
                let error = reference.error(&packed, &rig.skeleton);
                assert!(error <= 1e-4, "{}: {error} m", clip.name);
                assert!(
                    packed.bytes() * 2 < clip.bytes(),
                    "{}: {} bytes packed against {}",
                    clip.name,
                    packed.bytes(),
                    clip.bytes()
                );
            }
        }
    }

    #[test]
    fn a_constant_track_keeps_no_bits_and_its_value() {
        let skeleton = Skeleton::new(
            vec!["root".into()],
            vec![None],
            vec![Mat4::IDENTITY],
            &[Transform::IDENTITY],
            vec![Mat4::IDENTITY],
        )
        .unwrap();
        let held = Vec3::new(0.25, -1.5, 3.0);
        let clip = Clip {
            name: String::new(),
            start: 0.0,
            duration: 1.0,
            joints: vec![JointTracks {
                translation: Some(Track {
                    times: vec![0.0, 0.5, 1.0],
                    values: vec![held; 3],
                    interpolation: Interpolation::Linear,
                }),
                rotation: None,
                scale: None,
            }],
        };
        let packed = PackedClip::new(&clip, &skeleton, 1e-4);
        assert_eq!(packed.bits(), [0]);
        let mut pose = skeleton.rest().clone();
        packed.sample(&skeleton, 0.3, &mut pose);
        assert_eq!(pose.translations[0], held);
    }

    #[test]
    fn a_packed_clip_samples_to_the_same_bits_twice() {
        let rig = &rigs()[0];
        let packed = PackedClip::new(&rig.clips[0], &rig.skeleton, 1e-4);
        let (mut a, mut b) = (rig.skeleton.rest().clone(), rig.skeleton.rest().clone());
        for k in 0..50 {
            let t = packed.duration * k as f32 / 50.0;
            packed.sample(&rig.skeleton, t, &mut a);
            packed.sample(&rig.skeleton, t, &mut b);
            assert_eq!(a, b);
        }
    }
}
