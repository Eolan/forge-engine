//! `physics-lab --lab flyer` (#184, Phase 3's step 7's flyer): gulls flying on their wings. A
//! gull modelled in Blender (`assets/blender/bird.py`): one skinned body on seven bones (the
//! body, the head, the tail, each wing's arm and hand) with two clips, a flap and a glide. Each
//! bird is one Jolt body; each tick its clip's pose places its wings' panels (each arm, each
//! hand, the tail), and `forge_physics::aero` gives each panel its lift and drag from the air
//! past it, the wing's own beat included: a beating wing pulls the bird on, a held one carries
//! it. It beats below its height or its cruising speed and glides above them, and is held to
//! the attitude of its circuit (level, banked into the turn) by a balance torque, as the dogs
//! are held up (D-012's guided physics layer).

use std::sync::OnceLock;

use anyhow::Result;
use forge_anim::{Inertializer, Pose, Rig, load_rigs};
use forge_core::dmath::{atan2, sin_cos};
use forge_geom::model::{Model, load_glb};
use forge_physics::aero::{Air, Surface, push};
use forge_physics::{BodyDesc, BodyId, Shape, Velocity, World};
use forge_sim::TICK;
use glam::{DVec3, Mat4, Quat, Vec3};

/// The gull's model and rig, read once from `assets/models/bird.glb`.
pub(super) fn model() -> &'static (Model, Rig) {
    static MODEL: OnceLock<(Model, Rig)> = OnceLock::new();
    MODEL.get_or_init(|| {
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let path = root.join("assets/models/bird.glb");
        let bytes = std::fs::read(&path)
            .unwrap_or_else(|e| panic!("the bird's model {}: {e}", path.display()));
        let model = load_glb(&bytes).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let rig = load_rigs(&bytes)
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
            .into_iter()
            .find(|r| r.name == "bird")
            .unwrap_or_else(|| panic!("{} has no bird rig", path.display()));
        (model, rig)
    })
}

/// How far from a gull's origin its skinned mesh reaches in any pose, metres: its wing tips
/// (0.66 m) beaten up or folded.
pub(super) const REACH: f32 = 0.8;

/// How a gull is drawn: its joints' matrices for `pose`, turned by its body's `rotation`, in
/// its mover's frame (at its body's place, not turned) into `out`.
pub(super) fn skin(pose: &Pose, rotation: Quat, out: &mut Vec<Mat4>) {
    let (_, rig) = model();
    let skeleton = &rig.skeleton;
    let mut model = vec![Mat4::IDENTITY; skeleton.len()];
    skeleton.model_space(pose, &mut model);
    out.clear();
    out.resize(skeleton.len(), Mat4::IDENTITY);
    skeleton.skinning_matrices(&model, out);
    let turn = Mat4::from_quat(rotation);
    for m in out.iter_mut() {
        *m = turn * *m;
    }
}

/// A flying panel: its joint, its span's ends in the joint's frame, its chord (metres), and
/// which way is up and forward in the joint's frame at rest.
struct Panel {
    joint: usize,
    ends: [Vec3; 2],
    chord: f32,
    up: Vec3,
    forward: Vec3,
}

/// The gull's clips and its panels.
struct Wings {
    flap: usize,
    glide: usize,
    panels: Vec<Panel>,
}

/// The panels' aspect ratio: the whole wing's (1.3 m over about 0.17 m²).
const ASPECT: f32 = 8.0;
/// The wings' incidence on the body at rest, as a sine (3°).
const RIGGING: f32 = 0.05;

fn wings() -> &'static Wings {
    static WINGS: OnceLock<Wings> = OnceLock::new();
    WINGS.get_or_init(|| {
        let (_, rig) = model();
        let skeleton = &rig.skeleton;
        let mut rest = vec![Mat4::IDENTITY; skeleton.len()];
        skeleton.model_space(skeleton.rest(), &mut rest);
        let joint = |name: &str| {
            skeleton
                .joint(name)
                .unwrap_or_else(|| panic!("the bird has no {name}"))
        };
        let clip = |name: &str| {
            rig.clips
                .iter()
                .position(|c| c.name == name)
                .unwrap_or_else(|| panic!("the bird has no clip {name}"))
        };
        // In the model's frame (Blender's x, z, −y): the shoulders, elbows and tips as
        // `bird.py` places them; the tail's plate from its root to its end.
        let mut panels = Vec::new();
        let mut add = |name: &str, a: Vec3, b: Vec3, chord: f32| {
            let j = joint(name);
            let into = rest[j].inverse();
            let turn = rest[j].to_scale_rotation_translation().1.inverse();
            panels.push(Panel {
                joint: j,
                ends: [into.transform_point3(a), into.transform_point3(b)],
                chord,
                up: turn * Vec3::Y,
                forward: turn * Vec3::NEG_Z,
            });
        };
        for (side, s) in [("l", -1.0), ("r", 1.0)] {
            let shoulder = Vec3::new(s * 0.05, 0.025, -0.03);
            let elbow = Vec3::new(s * 0.31, 0.025, 0.0);
            let tip = Vec3::new(s * 0.65, 0.025, 0.08);
            add(&format!("wing-{side}"), shoulder, elbow, 0.16);
            add(&format!("hand-{side}"), elbow, tip, 0.12);
        }
        add(
            "tail",
            Vec3::new(-0.06, 0.0, 0.24),
            Vec3::new(0.06, 0.0, 0.24),
            0.13,
        );
        Wings {
            flap: clip("bird-flap"),
            glide: clip("bird-glide"),
            panels,
        }
    })
}

/// The panels as `pose` places them, in the model's (the body's) frame.
fn surfaces(pose: &Pose, model: &mut [Mat4]) -> Vec<Surface> {
    let (_, rig) = self::model();
    rig.skeleton.model_space(pose, model);
    wings()
        .panels
        .iter()
        .map(|p| {
            let m = model[p.joint];
            let (a, b) = (m.transform_point3(p.ends[0]), m.transform_point3(p.ends[1]));
            let turn = m.to_scale_rotation_translation().1;
            let normal = (turn * p.up).normalize();
            let forward = turn * p.forward;
            let forward = (forward - normal * forward.dot(normal)).normalize();
            let span = a.distance(b);
            Surface {
                at: 0.5 * (a + b),
                normal,
                forward,
                area: span * p.chord,
                aspect: if p.chord > 0.125 && span < 0.2 {
                    1.5
                } else {
                    ASPECT
                },
                rigging: if p.chord > 0.125 && span < 0.2 {
                    0.0
                } else {
                    RIGGING
                },
            }
        })
        .collect()
}

/// A gull's mass, kg, and its body's radius as Jolt holds it (a ball that gives it the
/// turning inertia of its spread wings, not its trunk's).
const MASS: f32 = 0.75;
const RADIUS: f32 = 0.3;
/// Its circuit: a circle about the lab's middle at this height and radius, flown anticlockwise
/// seen from above; it beats its wings below the height or the cruising speed (less a margin)
/// and glides above them (more a margin), metres and m/s.
const HEIGHT: f64 = 8.0;
const CIRCUIT: f64 = 25.0;
const CRUISE: f32 = 11.0;
const MARGIN: (f64, f32) = (0.5, 1.0);
/// The angle at which its wings meet the air over their rigging, radians, and how much more a
/// metre under its height (divided by the bank's cosine: a banked wing lifts that much less).
const INCIDENCE: (f64, f64) = (0.1, 0.02);
/// How far it banks to turn: for its circuit's curve at its speed, and so much more a radian
/// off the way it wants, at most `BANK`; it wants the circuit's tangent, turned in by `STEER`
/// radians a metre outside it.
const BANK_PER_RADIAN: f64 = 1.0;
const BANK: f64 = 0.7;
const STEER: f64 = 0.15;
/// The balance holding it to its attitude (N·m a radian, N·m·s a radian).
const BALANCE: (f32, f32) = (20.0, 1.6);
/// How long a switch between its clips takes to die away, seconds.
const SWITCH: f32 = 0.3;

/// One gull: its body and its clips' schedule: the
/// clip it plays and the tick it started, and the one before with its start.
#[derive(Clone, Copy, Debug)]
pub(super) struct Bird {
    pub body: BodyId,
    clip: usize,
    since: u64,
    before: Option<(usize, u64)>,
}

/// The gulls, on a circuit each a third of a turn apart.
pub(super) const BIRDS: usize = 3;

/// Where on its circuit a gull at angle `a` is, and the way it flies there.
fn on_circuit(a: f64) -> (DVec3, DVec3) {
    let (s, c) = sin_cos(a);
    (
        DVec3::new(CIRCUIT * c, HEIGHT, CIRCUIT * s),
        DVec3::new(s, 0.0, -c),
    )
}

/// The turn that faces a gull (its model faces −z) along `way`, banked by `bank` radians
/// (positive: its left wing down) and pitched up by `pitch`.
fn attitude(way: DVec3, bank: f64, pitch: f64) -> Quat {
    let yaw = atan2(-way.x, -way.z);
    let half = |a: f64| {
        let (s, c) = sin_cos(0.5 * a);
        (s as f32, c as f32)
    };
    let (ys, yc) = half(yaw);
    let (ps, pc) = half(pitch);
    let (bs, bc) = half(bank);
    Quat::from_xyzw(0.0, ys, 0.0, yc)
        * Quat::from_xyzw(ps, 0.0, 0.0, pc)
        * Quat::from_xyzw(0.0, 0.0, bs, bc)
}

/// Adds the gulls to `world`, flying their circuit at their cruising speed.
pub(super) fn build(world: &mut World) -> Result<Vec<Bird>> {
    let shape = Shape::sphere(RADIUS, 100.0)?;
    let w = wings();
    (0..BIRDS)
        .map(|k| {
            let start = std::f64::consts::TAU * k as f64 / BIRDS as f64;
            let (at, way) = on_circuit(start);
            let body = world.add_body(&BodyDesc {
                rotation: attitude(way, 0.0, INCIDENCE.0),
                mass: Some(MASS),
                ..BodyDesc::dynamic(&shape, at)
            })?;
            world.set_velocity(
                body,
                Velocity {
                    linear: (way * f64::from(CRUISE)).as_vec3(),
                    angular: Vec3::ZERO,
                },
            );
            Ok(Bird {
                body,
                clip: w.glide,
                since: 0,
                before: None,
            })
        })
        .collect()
}

impl Bird {
    /// What a saved state keeps of it: its clip and its start, the one before and its start
    /// (`u64::MAX` for none).
    pub(super) fn state(&self) -> [u64; 4] {
        let (before, from) = self.before.map_or((u64::MAX, 0), |(c, s)| (c as u64, s));
        [self.clip as u64, self.since, before, from]
    }

    /// Back to a saved state.
    pub(super) fn set_state(&mut self, [clip, since, before, from]: [u64; 4]) {
        self.clip = clip as usize;
        self.since = since;
        self.before = (before != u64::MAX).then_some((before as usize, from));
    }

    /// Its pose at tick `tick`: its clip from its start, a switch dying away over `SWITCH`.
    pub(super) fn pose_at(&self, tick: f64, out: &mut Pose) {
        let (_, rig) = model();
        let skeleton = &rig.skeleton;
        let sample = |clip: usize, ticks: f64, out: &mut Pose| {
            let c = &rig.clips[clip];
            c.sample(skeleton, c.wrap((ticks * f64::from(TICK)) as f32), out);
        };
        let into = tick - self.since as f64;
        sample(self.clip, into, out);
        if let Some((before, from)) = self.before {
            let seconds = (into * f64::from(TICK)) as f32;
            if seconds < SWITCH {
                let ran = (self.since - from) as f64;
                let [mut a, mut a_before, mut b, mut b_before] =
                    std::array::from_fn(|_| out.clone());
                sample(before, ran, &mut a);
                sample(before, ran - 1.0, &mut a_before);
                sample(self.clip, 0.0, &mut b);
                sample(self.clip, -1.0, &mut b_before);
                let mut inertia = Inertializer::default();
                inertia.start((&a, &a_before), (&b, &b_before), TICK, SWITCH);
                inertia.apply_at(out, seconds);
            }
        }
    }
}

/// Before the step at `tick`: each gull's wings' lift and drag, its balance, and its choice of
/// clip for the ticks after.
pub(super) fn drive(world: &mut World, birds: &mut [Bird], tick: u64) {
    let w = wings();
    let (_, rig) = model();
    let mut model = vec![Mat4::IDENTITY; rig.skeleton.len()];
    let (mut pose, mut before) = (rig.skeleton.rest().clone(), rig.skeleton.rest().clone());
    let (mut t, mut v, mut c) = (Vec::new(), Vec::new(), Vec::new());
    for bird in birds.iter_mut() {
        world.transforms(&[bird.body], &mut t);
        world.velocities(&[bird.body], &mut v);
        world.centers_of_mass(&[bird.body], &mut c);
        let (at, moving, com) = (t[0], v[0], c[0]);
        bird.pose_at(tick as f64, &mut pose);
        bird.pose_at(tick as f64 - 1.0, &mut before);
        let now = surfaces(&pose, &mut model);
        let was = surfaces(&before, &mut model);
        // Each panel in the air past it, its own beat added to the body's motion.
        let (mut force, mut torque) = (Vec3::ZERO, Vec3::ZERO);
        for (s, b) in now.iter().zip(&was) {
            let beat = at.rotation * ((s.at - b.at) / TICK);
            let (f, m) = push(
                std::slice::from_ref(s),
                &[],
                at,
                Velocity {
                    linear: moving.linear + beat,
                    ..moving
                },
                com,
                &Air::STILL,
            );
            force += f;
            torque += m;
        }
        // Its attitude: facing the way it flies, its wings meeting the air at the angle its
        // height asks, banked to turn onto its circuit (lift turns it, as it turns a bird).
        let speed = moving.linear.length();
        let v = moving.linear.as_dvec3();
        let level = (v.x * v.x + v.z * v.z).sqrt().max(1e-3);
        let way = DVec3::new(v.x, 0.0, v.z) / level;
        let path = atan2(v.y, level);
        let below = HEIGHT - at.position.y;
        let flat = DVec3::new(at.position.x, 0.0, at.position.z);
        let (_, tangent) = on_circuit(atan2(flat.z, flat.x));
        let out = flat.normalize_or(DVec3::X);
        let wanted = (tangent - out * (STEER * (flat.length() - CIRCUIT))).normalize();
        // The way to turn: its left (up × way) towards the wanted way, a bank to the left.
        let left = DVec3::Y.cross(way);
        let error = atan2(wanted.dot(left), wanted.dot(way));
        let turn = atan2(f64::from(speed * speed), 9.81 * CIRCUIT);
        let bank = (turn + BANK_PER_RADIAN * error).clamp(-BANK, BANK);
        let (_, c) = sin_cos(bank);
        let incidence = (INCIDENCE.0 + INCIDENCE.1 * below.clamp(-3.0, 3.0)) / c.max(0.5);
        let mut q = attitude(way, bank, path + incidence) * at.rotation.inverse();
        if q.w < 0.0 {
            q = -q;
        }
        let axis = Vec3::new(q.x, q.y, q.z);
        let sine = axis.length();
        let back = if sine > 1e-6 {
            axis * (2.0 * atan2(f64::from(sine), f64::from(q.w)) as f32 / sine)
        } else {
            Vec3::ZERO
        };
        torque += BALANCE.0 * back - BALANCE.1 * moving.angular;
        world.push(&[bird.body], &[(force, at.position, torque)]);
        let flapping = bird.clip == w.flap;
        // Beat below its height or speed, glide above; a switch waits for the beat's top.
        let want = if below > MARGIN.0 || speed < CRUISE - MARGIN.1 {
            w.flap
        } else if below < -MARGIN.0 && speed > CRUISE + MARGIN.1 {
            w.glide
        } else {
            bird.clip
        };
        let beat_ticks = (rig.clips[w.flap].duration / TICK).round() as u64;
        let top = !flapping || (tick + 1 - bird.since).is_multiple_of(beat_ticks);
        if want != bird.clip && top && tick + 1 - bird.since > beat_ticks {
            bird.before = Some((bird.clip, bird.since));
            bird.clip = want;
            bird.since = tick + 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use forge_physics::Transform;

    use super::*;

    /// The mean force over a beat of `clip` (or its first pose held), flying 10 m/s level along
    /// −z with its wings meeting the air at `incidence` over their rigging.
    fn mean_force(clip: usize, incidence: f64, beating: bool) -> Vec3 {
        let (_, rig) = model();
        let mut m = vec![Mat4::IDENTITY; rig.skeleton.len()];
        let (mut pose, mut before) = (rig.skeleton.rest().clone(), rig.skeleton.rest().clone());
        let clip = &rig.clips[clip];
        let at = Transform {
            position: DVec3::ZERO,
            rotation: attitude(DVec3::NEG_Z, 0.0, incidence),
        };
        let n = 24;
        let mut sum = Vec3::ZERO;
        for k in 0..n {
            let t = if beating {
                clip.duration * k as f32 / n as f32
            } else {
                0.0
            };
            clip.sample(&rig.skeleton, t, &mut pose);
            clip.sample(&rig.skeleton, clip.wrap(t - TICK), &mut before);
            let now = surfaces(&pose, &mut m);
            let was = surfaces(&before, &mut m);
            for (s, b) in now.iter().zip(&was) {
                let beat = if beating {
                    at.rotation * ((s.at - b.at) / TICK)
                } else {
                    Vec3::ZERO
                };
                let v = Velocity {
                    linear: Vec3::new(0.0, 0.0, -10.0) + beat,
                    angular: Vec3::ZERO,
                };
                sum += push(
                    std::slice::from_ref(s),
                    &[],
                    at,
                    v,
                    DVec3::ZERO,
                    &Air::STILL,
                )
                .0;
            }
        }
        sum / n as f32
    }

    #[test]
    fn a_beating_wing_pulls_the_gull_on_and_a_held_one_carries_it() {
        let w = wings();
        // Beating at 0.1 rad: lift and a pull forward (−z), from the wings' own motion alone.
        let beat = mean_force(w.flap, INCIDENCE.0, true);
        assert!(beat.y > 6.0 && beat.z < -0.1, "{beat}");
        // The same pose held still: the pull is gone, drag holds it back.
        let held = mean_force(w.flap, INCIDENCE.0, false);
        assert!(held.z > 0.0, "{held}");
        // Gliding: about its weight (0.75 kg) in lift, a little drag.
        let glide = mean_force(w.glide, INCIDENCE.0, false);
        assert!(glide.y > 0.8 * MASS * 9.81 && glide.z > 0.0, "{glide}");
    }

    /// The gulls over `seconds`: per gull its lowest and highest height and its nearest and
    /// furthest from the circuit's middle after the first 10 s, the ticks it beat its wings;
    /// and the final transforms.
    fn fly(seconds: u64) -> (Vec<[f64; 5]>, Vec<Transform>) {
        let mut world = World::new(&forge_physics::WorldDesc::default());
        let mut birds = build(&mut world).expect("the gulls");
        let bodies: Vec<BodyId> = birds.iter().map(|b| b.body).collect();
        let mut seen = vec![[f64::MAX, f64::MIN, f64::MAX, f64::MIN, 0.0]; BIRDS];
        let mut t = Vec::new();
        for tick in 0..seconds * 60 {
            drive(&mut world, &mut birds, tick);
            world.step(TICK, 1).expect("a step");
            world.transforms(&bodies, &mut t);
            for ((s, at), bird) in seen.iter_mut().zip(&t).zip(&birds) {
                if bird.clip == wings().flap {
                    s[4] += 1.0;
                }
                if tick < 600 {
                    continue;
                }
                let out = DVec3::new(at.position.x, 0.0, at.position.z).length();
                s[0] = s[0].min(at.position.y);
                s[1] = s[1].max(at.position.y);
                s[2] = s[2].min(out);
                s[3] = s[3].max(out);
            }
        }
        (seen, t)
    }

    #[test]
    fn the_gulls_fly_their_circuit_on_their_wings_and_replay() {
        let (seen, last) = fly(60);
        for (k, [low, high, near, far, beats]) in seen.iter().enumerate() {
            eprintln!(
                "gull {k}: {low:.2}..{high:.2} m up, {near:.1}..{far:.1} m out, {beats} ticks beating"
            );
            assert!(*low > 4.0 && *high < 12.0, "gull {k}: {low}..{high} m up");
            assert!(*near > 10.0 && *far < 50.0, "gull {k}: {near}..{far} m out");
            // It beats its wings and it glides.
            assert!(
                *beats > 600.0 && *beats < 3400.0,
                "gull {k}: {beats} ticks beating"
            );
        }
        assert_eq!(last, fly(60).1);
    }
}
