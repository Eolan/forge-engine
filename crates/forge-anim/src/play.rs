//! Playing clips (#167, D-012's clip layer): a clip's time running on, the switches between
//! clips inertialized (Bollo, "Inertialization: High-Performance Animation Transitions in
//! *Gears of War*", GDC 2016), and a blend space of clips along one parameter, their phases
//! kept together.
//!
//! An inertialized switch evaluates the new clip alone. At the switch it takes the difference
//! between where the old clip had the skeleton and where the new one has it, and how fast that
//! difference was changing, then lets it die away over a fraction of a second along a quintic
//! that ends with no speed and no acceleration. No cross-fade has to sample both clips.
//!
//! Like the rest of the crate, nothing here calls a transcendental function (D-016): a
//! rotation's offset decays as the sine of its half angle (a quaternion's vector part), its
//! cosine recovered by a square root.

use glam::{Quat, Vec3};

use crate::pose::Pose;
use crate::{Clip, Skeleton};

/// Bollo's quintic: an offset `x0` changing at `v0` brought to rest at `t1` seconds with no
/// speed or acceleration left, at `t` seconds. It never overshoots: a speed away from rest is
/// taken as none, and one towards it brings `t1` forward to `−5 x0 / v0` at the latest.
pub fn decay(x0: f32, v0: f32, t1: f32, t: f32) -> f32 {
    if x0 == 0.0 || t1 <= 0.0 {
        return 0.0;
    }
    // In Bollo's terms the offset is positive.
    let sign = if x0 < 0.0 { -1.0 } else { 1.0 };
    let (x0, v0) = (x0 * sign, (v0 * sign).min(0.0));
    let t1 = if v0 < 0.0 { t1.min(-5.0 * x0 / v0) } else { t1 };
    if t >= t1 {
        return 0.0;
    }
    let t1_2 = t1 * t1;
    let (t1_3, t1_4) = (t1_2 * t1, t1_2 * t1_2);
    let t1_5 = t1_4 * t1;
    let a0 = (-8.0 * v0 * t1 - 20.0 * x0) / t1_2;
    let a = -(a0 * t1_2 + 6.0 * v0 * t1 + 12.0 * x0) / (2.0 * t1_5);
    let b = (3.0 * a0 * t1_2 + 16.0 * v0 * t1 + 30.0 * x0) / (2.0 * t1_4);
    let c = -(3.0 * a0 * t1_2 + 12.0 * v0 * t1 + 20.0 * x0) / (2.0 * t1_3);
    let t = t.max(0.0);
    sign * (((((a * t + b) * t + c) * t + 0.5 * a0) * t + v0) * t + x0)
}

/// A joint's offset at a switch: its direction (a unit vector, or a rotation's unit axis), its
/// size (metres, or the sine of the rotation's half angle) and how fast that changed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Offset {
    direction: Vec3,
    size: f32,
    rate: f32,
}

impl Offset {
    /// The offset `now` along its direction, changing from `before` (the same offset a step of
    /// `dt` earlier).
    fn new(now: Vec3, before: Vec3, dt: f32) -> Self {
        let size = now.length();
        if size <= 1e-9 {
            return Self::default();
        }
        let direction = now / size;
        let rate = if dt > 0.0 {
            (size - before.dot(direction)) / dt
        } else {
            0.0
        };
        Self {
            direction,
            size,
            rate,
        }
    }

    /// Its value `t` seconds after the switch, over `duration`.
    fn at(&self, duration: f32, t: f32) -> Vec3 {
        self.direction * decay(self.size, self.rate, duration, t)
    }
}

/// The vector part of `a`'s turn from `b` (`a b⁻¹`), the short way round: its axis times the
/// sine of half its angle.
fn turn(a: Quat, b: Quat) -> Vec3 {
    let q = a * b.conjugate();
    let q = if q.w < 0.0 { -q } else { q };
    Vec3::new(q.x, q.y, q.z)
}

/// The turn whose vector part is `v`: its cosine of half the angle from `|v|`.
fn from_turn(v: Vec3) -> Quat {
    let s2 = v.length_squared().min(1.0);
    Quat::from_xyzw(v.x, v.y, v.z, (1.0 - s2).sqrt())
}

/// A switch between two poses dying away (#167's step 1): set when the source changes
/// ([`Inertializer::start`]), applied to every pose after it ([`Inertializer::apply`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Inertializer {
    translations: Vec<Offset>,
    rotations: Vec<Offset>,
    /// Seconds since the switch, and over how long it dies away.
    time: f32,
    duration: f32,
}

impl Inertializer {
    /// Whether an offset is still dying away.
    pub fn active(&self) -> bool {
        self.time < self.duration
    }

    /// A switch from the pose `from` (and `from_before`, the step of `dt` before it) to `to`
    /// (and `to_before`, where the new source had it a step earlier), dying away over
    /// `duration` seconds.
    ///
    /// # Panics
    ///
    /// When the four poses do not have the same number of joints.
    pub fn start(
        &mut self,
        (from, from_before): (&Pose, &Pose),
        (to, to_before): (&Pose, &Pose),
        dt: f32,
        duration: f32,
    ) {
        let n = from.len();
        assert!(from_before.len() == n && to.len() == n && to_before.len() == n);
        self.translations = (0..n)
            .map(|j| {
                Offset::new(
                    from.translations[j] - to.translations[j],
                    from_before.translations[j] - to_before.translations[j],
                    dt,
                )
            })
            .collect();
        self.rotations = (0..n)
            .map(|j| {
                Offset::new(
                    turn(from.rotations[j], to.rotations[j]),
                    turn(from_before.rotations[j], to_before.rotations[j]),
                    dt,
                )
            })
            .collect();
        self.time = 0.0;
        self.duration = duration;
    }

    /// Adds what is left of the switch's offset to `pose`, then moves on by `dt`.
    pub fn apply(&mut self, pose: &mut Pose, dt: f32) {
        self.apply_at(pose, self.time);
        self.time += dt;
    }

    /// Adds what is left of the switch's offset `time` seconds after it to `pose`, whatever
    /// time it has reached: a pose that is a function of the clock alone (a simulation that
    /// rolls back) evaluates its last switch afresh each tick.
    pub fn apply_at(&self, pose: &mut Pose, time: f32) {
        if time < self.duration && self.translations.len() == pose.len() {
            for j in 0..pose.len() {
                pose.translations[j] += self.translations[j].at(self.duration, time);
                let turn = from_turn(self.rotations[j].at(self.duration, time));
                pose.rotations[j] = (turn * pose.rotations[j]).normalize();
            }
        }
    }
}

/// Clips along one parameter (#167's step 2): an idle at speed 0, a walk at 1.2 m/s, a run at
/// 3 m/s. At a parameter between two entries their poses are blended, both sampled at the same
/// phase (the share of their cycle), so the feet stay in step.
#[derive(Clone, Debug, PartialEq)]
pub struct BlendSpace {
    /// Each entry's clip (an index into the rig's clips) and its parameter, in increasing
    /// order.
    pub entries: Vec<(usize, f32)>,
}

impl BlendSpace {
    /// The two entries around `parameter` and the second's weight (clamped at the ends).
    pub fn around(&self, parameter: f32) -> (usize, usize, f32) {
        let last = self.entries.len() - 1;
        let i = self
            .entries
            .partition_point(|&(_, p)| p <= parameter)
            .clamp(1, last.max(1))
            - 1;
        let j = (i + 1).min(last);
        let (p, q) = (self.entries[i].1, self.entries[j].1);
        let w = if q > p {
            ((parameter - p) / (q - p)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (i, j, w)
    }

    /// Cycles a second at `parameter`: the blended rate of the two clips around it.
    pub fn rate(&self, clips: &[Clip], parameter: f32) -> f32 {
        let (i, j, w) = self.around(parameter);
        let rate = |k: usize| {
            let d = clips[self.entries[k].0].duration;
            if d > 0.0 { 1.0 / d } else { 0.0 }
        };
        rate(i) + (rate(j) - rate(i)) * w
    }

    /// Writes the pose at `parameter` and `phase` (0..1 of a cycle) into `out`, with `scratch`
    /// for the second clip.
    pub fn sample(
        &self,
        clips: &[Clip],
        skeleton: &Skeleton,
        (parameter, phase): (f32, f32),
        out: &mut Pose,
        scratch: &mut Pose,
    ) {
        let (i, j, w) = self.around(parameter);
        let at = |k: usize| {
            let clip = &clips[self.entries[k].0];
            phase.rem_euclid(1.0) * clip.duration
        };
        clips[self.entries[i].0].sample(skeleton, at(i), out);
        if w > 0.0 && j != i {
            clips[self.entries[j].0].sample(skeleton, at(j), scratch);

            out.mix(scratch, w);
        }
    }
}

/// What a [`Player`] plays.
#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    /// One clip, looped.
    Clip(usize),
    /// A blend space, at the player's parameter.
    Blend(BlendSpace),
}

/// A creature's animation (#167): its source, the phase it has reached, the parameter a blend
/// space reads, and the last switch dying away. Each [`Player::update`] writes the pose.
#[derive(Clone, Debug, PartialEq)]
pub struct Player {
    source: Source,
    /// The share of the source's cycle reached, 0..1.
    phase: f32,
    /// What a blend space is read at (a speed, say).
    pub parameter: f32,
    /// The playback rate: 1 as authored.
    pub speed: f32,
    inertializer: Inertializer,
    /// The last two poses written, for the next switch's offset and its rate.
    last: Pose,
    before: Pose,
    last_dt: f32,
    scratch: Pose,
}

impl Player {
    /// A player of `source` from its start, for a skeleton of `joints` joints.
    pub fn new(source: Source, joints: usize) -> Self {
        let empty = Pose {
            translations: vec![Vec3::ZERO; joints],
            rotations: vec![Quat::IDENTITY; joints],
            scales: vec![Vec3::ONE; joints],
        };
        Self {
            source,
            phase: 0.0,
            parameter: 0.0,
            speed: 1.0,
            inertializer: Inertializer::default(),
            last: empty.clone(),
            before: empty.clone(),
            last_dt: 0.0,
            scratch: empty,
        }
    }

    /// What it plays.
    pub fn source(&self) -> &Source {
        &self.source
    }

    /// The share of its cycle reached.
    pub fn phase(&self) -> f32 {
        self.phase
    }

    /// Switches to `source` from its start, the change dying away over `duration` seconds.
    /// Takes effect at the next [`Player::update`]; before any update, it just sets the source.
    pub fn play(&mut self, source: Source, duration: f32, clips: &[Clip], skeleton: &Skeleton) {
        let first = self.last_dt == 0.0;
        self.source = source;
        self.phase = 0.0;
        if first {
            return;
        }
        // Where the new source has the skeleton at its start, and a step before.
        let mut to = self.last.clone();
        let mut to_before = self.last.clone();
        let dt = self.last_dt;
        let rate = self.rate(clips);
        self.sample(clips, skeleton, 0.0, &mut to);
        self.sample(clips, skeleton, -dt * rate * self.speed, &mut to_before);
        // The offset is the one from where it was drawn, the last switch's included.
        self.inertializer
            .start((&self.last, &self.before), (&to, &to_before), dt, duration);
    }

    /// Cycles a second of the source.
    fn rate(&self, clips: &[Clip]) -> f32 {
        match &self.source {
            Source::Clip(c) => {
                let d = clips[*c].duration;
                if d > 0.0 { 1.0 / d } else { 0.0 }
            }
            Source::Blend(space) => space.rate(clips, self.parameter),
        }
    }

    /// The source's pose at `phase` into `out`.
    fn sample(&mut self, clips: &[Clip], skeleton: &Skeleton, phase: f32, out: &mut Pose) {
        match &self.source {
            Source::Clip(c) => {
                let clip = &clips[*c];
                clip.sample(skeleton, phase.rem_euclid(1.0) * clip.duration, out);
            }
            Source::Blend(space) => {
                let mut scratch = std::mem::take(&mut self.scratch);
                space.sample(clips, skeleton, (self.parameter, phase), out, &mut scratch);
                self.scratch = scratch;
            }
        }
    }

    /// Moves on by `dt` seconds and writes the pose into `out`: the source's at the new phase,
    /// with what is left of the last switch.
    pub fn update(&mut self, clips: &[Clip], skeleton: &Skeleton, dt: f32, out: &mut Pose) {
        let rate = self.rate(clips);
        if self.last_dt != 0.0 {
            self.phase = (self.phase + dt * rate * self.speed).rem_euclid(1.0);
        }
        let phase = self.phase;
        self.sample(clips, skeleton, phase, out);
        self.inertializer.apply(out, dt);
        std::mem::swap(&mut self.before, &mut self.last);
        self.last.clone_from(out);
        self.last_dt = dt;
    }
}

#[cfg(test)]
mod tests {
    use glam::Mat4;

    use super::*;
    use crate::clip::{Interpolation, JointTracks, Track};
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

    /// A clip of `duration` seconds: the root sliding along x by `slide` and back, the arm
    /// turning about z from `turn.0` to `turn.1` and back, linear keys.
    fn clip(duration: f32, slide: f32, turn: (f32, f32)) -> Clip {
        let times = vec![0.0, 0.5 * duration, duration];
        Clip {
            name: String::new(),
            start: 0.0,
            duration,
            joints: vec![
                JointTracks {
                    translation: Some(Track {
                        times: times.clone(),
                        values: vec![Vec3::ZERO, Vec3::X * slide, Vec3::ZERO],
                        interpolation: Interpolation::Linear,
                    }),
                    rotation: None,
                    scale: None,
                },
                JointTracks {
                    translation: None,
                    rotation: Some(Track {
                        times,
                        values: [turn.0, turn.1, turn.0].map(Quat::from_rotation_z).to_vec(),
                        interpolation: Interpolation::Linear,
                    }),
                    scale: None,
                },
            ],
        }
    }

    fn empty() -> Pose {
        Pose {
            translations: vec![Vec3::ZERO; 2],
            rotations: vec![Quat::IDENTITY; 2],
            scales: vec![Vec3::ONE; 2],
        }
    }

    /// The largest change between two poses: metres, or the sine of a half turn.
    fn change(a: &Pose, b: &Pose) -> f32 {
        (0..a.len())
            .map(|j| {
                let t = (a.translations[j] - b.translations[j]).length();
                t.max(turn(a.rotations[j], b.rotations[j]).length())
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_decay_starts_at_the_offset_and_ends_at_rest_without_overshooting() {
        for (x0, v0) in [
            (1.0, 0.0),
            (1.0, -1.0),
            (1.0, -20.0),
            (-0.5, 2.0),
            (0.3, 4.0),
        ] {
            let t1 = 0.4;
            assert!((decay(x0, v0, t1, 0.0) - x0).abs() < 1e-6, "{x0} {v0}");
            assert_eq!(decay(x0, v0, t1, t1), 0.0);
            let (mut last, h) = (x0, 1e-3);
            for k in 1..=400 {
                let x = decay(x0, v0, t1, k as f32 * h);
                // On the offset's side of rest, never further out than it started, to the
                // rounding where its terms cancel near the end (micrometres a metre).
                assert!(
                    x * x0 >= -1e-5 && x.abs() <= x0.abs() + 1e-5,
                    "{x0} {v0}: {x}"
                );
                last = x;
            }
            assert!(last.abs() < 1e-6);
            // At rest at the end: no speed left.
            let end = decay(x0, v0, t1, t1 - h);
            assert!(
                end.abs() < 1e-5,
                "{x0} {v0}: {end} a millisecond before the end"
            );
        }
    }

    #[test]
    fn a_switch_carries_on_from_the_old_clip_and_ends_on_the_new_one() {
        let skeleton = skeleton();
        let clips = [clip(1.0, 2.0, (0.0, 1.0)), clip(0.5, -0.4, (-0.6, -0.2))];
        let mut player = Player::new(Source::Clip(0), 2);
        let (dt, mut pose, mut previous) = (1.0 / 60.0, empty(), empty());
        for _ in 0..20 {
            previous.clone_from(&pose);
            player.update(&clips, &skeleton, dt, &mut pose);
        }
        // The old clip's step a frame: what a frame may change by across the switch.
        let step = change(&pose, &previous);
        player.play(Source::Clip(1), 0.3, &clips, &skeleton);
        let mut new = empty();
        let mut biggest = 0.0_f32;
        for frame in 0..30 {
            previous.clone_from(&pose);
            player.update(&clips, &skeleton, dt, &mut pose);
            biggest = biggest.max(change(&pose, &previous));
            if frame == 0 {
                // The new clip alone would jump by far more.
                clips[1].sample(&skeleton, 0.0, &mut new);
                assert!(change(&new, &previous) > 10.0 * step);
            }
        }
        assert!(
            biggest < 3.0 * step,
            "a frame changed by {biggest} against {step}"
        );
        // Past the 0.3 s, the new clip's pose exactly.
        clips[1].sample(&skeleton, player.phase() * clips[1].duration, &mut new);
        assert_eq!(pose, new);
    }

    #[test]
    fn a_blend_space_mixes_the_clips_around_its_parameter_in_phase() {
        let skeleton = skeleton();
        let clips = [clip(1.0, 1.0, (0.0, 0.5)), clip(2.0, 3.0, (0.0, 1.5))];
        let space = BlendSpace {
            entries: vec![(0, 0.0), (1, 2.0)],
        };
        let (mut out, mut scratch, mut a, mut b) = (empty(), empty(), empty(), empty());
        // At an entry, its clip; past the ends, the end's.
        space.sample(&clips, &skeleton, (0.0, 0.25), &mut out, &mut scratch);
        clips[0].sample(&skeleton, 0.25, &mut a);
        assert_eq!(out, a);
        space.sample(&clips, &skeleton, (5.0, 0.25), &mut out, &mut scratch);
        clips[1].sample(&skeleton, 0.5, &mut b);
        assert_eq!(out, b);
        // A quarter of the way: both at the same share of their cycles, mixed 3 to 1.
        space.sample(&clips, &skeleton, (0.5, 0.25), &mut out, &mut scratch);
        let mut mixed = empty();
        Pose::blend(&a, &b, 0.25, &mut mixed);
        assert!(change(&out, &mixed) < 1e-6);
        // Its rate between the clips' cycles a second.
        assert!((space.rate(&clips, 1.0) - 0.75).abs() < 1e-6);
    }

    #[test]
    fn the_same_playing_twice_gives_the_same_bits() {
        let skeleton = skeleton();
        let clips = [clip(1.0, 2.0, (0.0, 1.0)), clip(0.5, -0.4, (-0.6, -0.2))];
        let run = || {
            let mut player = Player::new(Source::Clip(0), 2);
            let mut pose = empty();
            for frame in 0..90 {
                if frame == 40 {
                    player.play(Source::Clip(1), 0.25, &clips, &skeleton);
                }
                player.update(&clips, &skeleton, 1.0 / 60.0, &mut pose);
            }
            pose
        };
        assert_eq!(run(), run());
    }
}
