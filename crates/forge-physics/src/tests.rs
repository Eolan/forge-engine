//! The binding's checks, and the determinism tests of `docs/research/physics-fluids.md` §6: the
//! same run twice, at several thread counts, replayed from a saved state, and on Windows and
//! Linux (CI runs both against one constant).

use super::*;

/// A pile to drop: 256 bodies (boxes, balls, cylinders, hulls) in four layers of 8 × 8 over a
/// 100 m ground, each turned by a rotation made without trigonometry (the platforms' `sin`
/// differ in their last bits; a square root does not).
struct Pile {
    world: World,
    bodies: Vec<BodyId>,
    _shapes: Vec<Shape>,
}

fn mix(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// A number in [-1, 1) from `x`, exactly.
fn unit(x: u64) -> f32 {
    (mix(x) >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

fn pile(threads: u32) -> Pile {
    let mut world = World::new(&WorldDesc {
        threads,
        ..WorldDesc::default()
    });
    let ground = Shape::cuboid(Vec3::new(50.0, 0.5, 50.0), 0.05, 0.0).unwrap();
    world
        .add_body(&BodyDesc::fixed(&ground, DVec3::new(0.0, -0.5, 0.0)))
        .unwrap();
    // A rock: twelve points about their mean, so the body's origin lies inside it.
    let points: Vec<Vec3> = (0..12)
        .map(|k| Vec3::new(unit(100 + 3 * k), unit(101 + 3 * k), unit(102 + 3 * k)) * 0.4)
        .collect();
    let mean = points.iter().sum::<Vec3>() / points.len() as f32;
    let rock: Vec<Vec3> = points.iter().map(|&p| p - mean).collect();
    let shapes = vec![
        Shape::cuboid(Vec3::splat(0.4), 0.05, 600.0).unwrap(),
        Shape::sphere(0.35, 1000.0).unwrap(),
        Shape::cylinder(0.44, 0.3, 0.05, 400.0).unwrap(),
        Shape::convex_hull(&rock, 0.05, 2600.0).unwrap(),
        ground,
    ];
    let mut bodies = Vec::new();
    for layer in 0..4u64 {
        for j in 0..8u64 {
            for i in 0..8u64 {
                let k = (layer * 8 + j) * 8 + i;
                let rotation = Quat::from_xyzw(
                    unit(4 * k),
                    unit(4 * k + 1),
                    unit(4 * k + 2),
                    unit(4 * k + 3),
                )
                .normalize();
                let desc = BodyDesc {
                    rotation,
                    friction: 0.5,
                    restitution: 0.1,
                    // Rolling resistance, roughly: no ball rolls on for ever.
                    angular_damping: 0.3,
                    ..BodyDesc::dynamic(
                        &shapes[(k % 4) as usize],
                        DVec3::new(
                            (i as f64 - 3.5) * 1.3,
                            2.0 + layer as f64 * 1.6,
                            (j as f64 - 3.5) * 1.3,
                        ),
                    )
                };
                bodies.push(world.add_body(&desc).unwrap());
            }
        }
    }
    world.optimize_broad_phase();
    Pile {
        world,
        bodies,
        _shapes: shapes,
    }
}

impl Pile {
    fn run(&mut self, steps: u32) {
        for _ in 0..steps {
            self.world.step(1.0 / 60.0, 1).unwrap();
        }
    }

    fn hash(&self) -> u64 {
        let mut transforms = Vec::new();
        self.world.transforms(&self.bodies, &mut transforms);
        state_hash(&transforms)
    }
}

/// The pile's hash after five seconds, on every platform (CI's Windows and Linux runners).
const PILE_HASH_300: u64 = 0x3e65_693b_8892_8ad9;

#[test]
fn the_c_structs_and_their_rust_twins_agree() {
    // SAFETY: no arguments.
    let c = unsafe { ffi::fj_layout() };
    assert_eq!(
        c,
        ffi::FjLayout {
            world_desc: size_of::<ffi::FjWorldDesc>() as u32,
            body_desc: size_of::<ffi::FjBodyDesc>() as u32,
            ray_hit: size_of::<ffi::FjRayHit>() as u32,
            character_desc: size_of::<ffi::FjCharacterDesc>() as u32,
            character_state: size_of::<ffi::FjCharacterState>() as u32,
            vehicle_desc: size_of::<ffi::FjVehicleDesc>() as u32,
            ragdoll_part: size_of::<ffi::FjRagdollPart>() as u32,
        }
    );
}

#[test]
fn the_same_run_twice_gives_the_same_hash() {
    let (mut a, mut b) = (pile(3), pile(3));
    a.run(300);
    b.run(300);
    assert_eq!(a.hash(), b.hash());
}

#[test]
fn the_hash_does_not_depend_on_the_threads() {
    let hashes: Vec<u64> = [0, 1, 3, 7]
        .iter()
        .map(|&threads| {
            let mut p = pile(threads);
            p.run(300);
            p.hash()
        })
        .collect();
    assert!(hashes.iter().all(|&h| h == hashes[0]), "{hashes:x?}");
}

#[test]
fn the_hash_is_the_same_on_every_platform() {
    let mut p = pile(3);
    p.run(300);
    assert_eq!(p.hash(), PILE_HASH_300, "{:#x}", p.hash());
}

#[test]
fn a_restored_state_replays_the_same_steps() {
    let mut p = pile(3);
    p.run(100);
    let saved = p.world.save_state();
    p.run(200);
    let straight = p.hash();
    p.world.restore_state(&saved).unwrap();
    p.run(200);
    assert_eq!(p.hash(), straight);
}

#[test]
fn the_pile_comes_to_rest_on_the_ground() {
    let mut p = pile(3);
    p.run(1800);
    let mut transforms = Vec::new();
    p.world.transforms(&p.bodies, &mut transforms);
    let lowest = transforms
        .iter()
        .map(|t| t.position.y)
        .fold(f64::INFINITY, f64::min);
    assert!(lowest > 0.0, "a body sank to {lowest}");
    // The boxes and the rocks sleep; a ball or a cylinder may still roll, slowly (Jolt has no
    // rolling resistance: its spin's damping stands in for it).
    let mut awake = Vec::new();
    p.world.awake(&p.bodies, &mut awake);
    let flat_awake = awake
        .iter()
        .enumerate()
        .filter(|&(k, &a)| a && (k % 4 == 0 || k % 4 == 3))
        .count();
    assert_eq!(flat_awake, 0, "boxes and rocks asleep after 30 s");
    assert!(p.world.active_bodies() < 32, "{}", p.world.active_bodies());
}

#[test]
fn a_ray_down_meets_the_ground() {
    let p = pile(0);
    let hit = p
        .world
        .cast_ray(DVec3::new(30.0, 10.0, 30.0), Vec3::new(0.0, -20.0, 0.0))
        .expect("the ground");
    assert!((hit.fraction - 0.5).abs() < 1e-4, "{}", hit.fraction);
    assert!(hit.normal.abs_diff_eq(Vec3::Y, 1e-4), "{}", hit.normal);
}

#[test]
fn a_hull_of_one_point_is_refused() {
    let point = [Vec3::ONE; 4];
    assert_eq!(
        Shape::convex_hull(&point, 0.0, 1000.0).err(),
        Some(PhysicsError::ShapeRefused)
    );
}

#[test]
fn a_raft_dropped_in_still_water_floats_at_its_draft() {
    use crate::buoyancy::{Fluid, Hull, Level, push};
    let mut world = World::new(&WorldDesc::default());
    // A raft of 500 kg/m³, 2 m square and 0.2 m thick, dropped from 2 m; in the sea it floats
    // with 500/1025 of its thickness under.
    let half = Vec3::new(1.0, 0.1, 1.0);
    let shape = Shape::cuboid(half, 0.02, 500.0).unwrap();
    let raft = world
        .add_body(&BodyDesc::dynamic(&shape, DVec3::new(0.0, 2.0, 0.0)))
        .unwrap();
    let hull = Hull::cuboid(half, 4);
    let (mut transforms, mut velocities, mut centers) = (Vec::new(), Vec::new(), Vec::new());
    for _ in 0..600 {
        world.transforms(&[raft], &mut transforms);
        world.velocities(&[raft], &mut velocities);
        world.centers_of_mass(&[raft], &mut centers);
        let p = push(
            &hull,
            transforms[0],
            velocities[0],
            centers[0],
            &Level(0.0),
            &Fluid::SEA,
        );
        world.push(&[raft], &[(p.force, transforms[0].position, p.torque)]);
        world.step(1.0 / 60.0, 1).unwrap();
    }
    world.transforms(&[raft], &mut transforms);
    world.velocities(&[raft], &mut velocities);
    let expected = 0.1 - 0.2 * 500.0 / 1025.0;
    let y = transforms[0].position.y;
    assert!((y - expected).abs() < 0.005, "at {y}, not {expected}");
    assert!(velocities[0].linear.length() < 0.01, "{:?}", velocities[0]);
    // And a stone the same size sinks.
    let stone_shape = Shape::cuboid(half, 0.02, 2600.0).unwrap();
    let mut world = World::new(&WorldDesc::default());
    let stone = world
        .add_body(&BodyDesc::dynamic(&stone_shape, DVec3::new(0.0, 0.0, 0.0)))
        .unwrap();
    for _ in 0..60 {
        world.transforms(&[stone], &mut transforms);
        world.velocities(&[stone], &mut velocities);
        world.centers_of_mass(&[stone], &mut centers);
        let p = push(
            &hull,
            transforms[0],
            velocities[0],
            centers[0],
            &Level(0.0),
            &Fluid::SEA,
        );
        world.push(&[stone], &[(p.force, transforms[0].position, p.torque)]);
        world.step(1.0 / 60.0, 1).unwrap();
    }
    world.transforms(&[stone], &mut transforms);
    assert!(
        transforms[0].position.y < -1.0,
        "{}",
        transforms[0].position.y
    );
}

/// A floor, a flight of five steps of 20 cm rising along +x from x = 2, a ramp of 50° rising
/// along −x from x = −3, and a platform 2 m to +z moving along +z at 1 m/s.
fn playground() -> (World, BodyId) {
    let mut world = World::new(&WorldDesc::default());
    let floor = Shape::cuboid(Vec3::new(50.0, 0.5, 50.0), 0.05, 0.0).unwrap();
    world
        .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
        .unwrap();
    for k in 0..5 {
        let rise = 0.2 * (k + 1) as f32;
        let step = Shape::cuboid(Vec3::new(0.2, 0.5 * rise, 1.0), 0.01, 0.0).unwrap();
        let at = DVec3::new(2.2 + 0.4 * f64::from(k), 0.5 * f64::from(rise), 0.0);
        world.add_body(&BodyDesc::fixed(&step, at)).unwrap();
    }
    // sin and cos of 25°, as numbers: the half angle of the ramp's 50°.
    let ramp = Shape::cuboid(Vec3::new(3.0, 0.1, 1.0), 0.01, 0.0).unwrap();
    let turn = Quat::from_xyzw(0.0, 0.0, -0.422_618_26, 0.906_307_8);
    world
        .add_body(&BodyDesc {
            rotation: turn,
            ..BodyDesc::fixed(&ramp, DVec3::new(-5.0, 2.0, 0.0))
        })
        .unwrap();
    let deck = Shape::cuboid(Vec3::new(1.0, 0.1, 1.0), 0.01, 0.0).unwrap();
    let platform = world
        .add_body(&BodyDesc {
            motion: Motion::Kinematic,
            linear_velocity: Vec3::new(0.0, 0.0, 1.0),
            ..BodyDesc::fixed(&deck, DVec3::new(0.0, 0.1, 4.0))
        })
        .unwrap();
    (world, platform)
}

/// Walks `c` at `wish` (m/s, along the ground) for `ticks`, as a game would: on firm ground
/// it takes the ground's velocity, in the air it keeps its fall; gravity each tick.
fn walk(world: &mut World, c: CharacterId, wish: Vec3, ticks: u32) {
    let dt = 1.0 / 60.0;
    for _ in 0..ticks {
        let s = world.character(c);
        let mut v = wish;
        if s.ground == Ground::Firm {
            v += s.ground_velocity;
        } else {
            v.y = s.velocity.y;
        }
        v.y -= 9.81 * dt;
        world.move_character(c, dt, v);
        world.step(dt, 1).unwrap();
    }
}

#[test]
fn a_character_climbs_the_stairs_and_not_a_steep_ramp() {
    let (mut world, _) = playground();
    let up = world.add_character(&CharacterDesc::default());
    walk(&mut world, up, Vec3::ZERO, 30);
    assert_eq!(world.character(up).ground, Ground::Firm);
    // 3.8 m of walking: onto the top step (x from 3.6 to 4), each step up costing a little.
    walk(&mut world, up, Vec3::new(2.0, 0.0, 0.0), 114);
    let top = world.character(up);
    assert!(
        (top.position.y - 1.0).abs() < 0.05,
        "on the top step: {}",
        top.position
    );
    // Its capsule's edge on the step: the feet a little short of it.
    assert!((3.3..4.0).contains(&top.position.x), "{}", top.position);
    // The ramp of 50° stops a second one walking at it.
    let blocked = world.add_character(&CharacterDesc {
        position: DVec3::new(-1.0, 0.0, 0.0),
        ..CharacterDesc::default()
    });
    walk(&mut world, blocked, Vec3::new(-2.0, 0.0, 0.0), 150);
    let s = world.character(blocked);
    assert!(s.position.y < 0.6, "it climbed to {}", s.position);
}

#[test]
fn a_platform_carries_a_character_and_a_saved_world_replays_it() {
    let (mut world, _) = playground();
    let rider = world.add_character(&CharacterDesc {
        position: DVec3::new(0.0, 0.3, 4.0),
        ..CharacterDesc::default()
    });
    walk(&mut world, rider, Vec3::ZERO, 30);
    let start = world.character(rider).position;
    let saved = world.save_state();
    walk(&mut world, rider, Vec3::ZERO, 60);
    let after = world.character(rider);
    assert!(after.ground_body.is_some());
    assert!(
        (after.position.z - start.z - 1.0).abs() < 0.05,
        "carried {} m",
        after.position.z - start.z
    );
    // Back to the saved world, the same second again: the same place to the bit.
    world.restore_state(&saved).unwrap();
    walk(&mut world, rider, Vec3::ZERO, 60);
    assert_eq!(world.character(rider).position, after.position);
}

/// A car: a 1 200 kg box chassis on four wheels, on a floor.
fn car() -> (World, BodyId, VehicleId) {
    let mut world = World::new(&WorldDesc::default());
    let floor = Shape::cuboid(Vec3::new(200.0, 0.5, 200.0), 0.05, 0.0).unwrap();
    world
        .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
        .unwrap();
    let body = Shape::cuboid(Vec3::new(0.85, 0.35, 1.9), 0.05, 0.0)
        .unwrap()
        .offset(Vec3::new(0.0, 0.6, 0.0), Quat::IDENTITY)
        .unwrap();
    let chassis = world
        .add_body(&BodyDesc {
            mass: Some(1200.0),
            friction: 0.5,
            ..BodyDesc::dynamic(&body, DVec3::new(0.0, 0.1, 0.0))
        })
        .unwrap();
    let vehicle = world
        .add_vehicle(
            chassis,
            &VehicleDesc {
                half_track: 0.74,
                half_wheelbase: 1.225,
                attach_y: 0.55,
                suspension: (0.05, 0.35),
                spring: (1.5, 0.5),
                wheel: (0.31, 0.2),
                max_steer: 0.6,
                engine: (300.0, 6000.0),
                brakes: (1500.0, 4000.0),
            },
        )
        .unwrap();
    (world, chassis, vehicle)
}

#[test]
fn a_car_drives_off_turns_and_replays() {
    let (mut world, chassis, car) = car();
    let mut t = Vec::new();
    let run = |world: &mut World, ticks: u32| {
        for _ in 0..ticks {
            world.step(1.0 / 60.0, 1).unwrap();
        }
    };
    run(&mut world, 60);
    world.drive(car, 1.0, 0.0, 0.0, 0.0);
    run(&mut world, 180);
    world.transforms(&[chassis], &mut t);
    let ahead = t[0];
    assert!(
        ahead.position.z < -6.0,
        "3 s of throttle: {}",
        ahead.position
    );
    // Upright: its up still up.
    assert!((ahead.rotation * Vec3::Y).y > 0.95);
    let mut wheels = Vec::new();
    world.wheels(car, &mut wheels);
    assert_eq!(wheels.len(), 4);
    assert!(
        wheels.iter().all(|w| (w.position.y - 0.31).abs() < 0.08),
        "{wheels:?}"
    );
    let (rpm, gear) = world.engine(car);
    assert!(rpm > 1000.0 && gear >= 1, "{rpm} rpm in gear {gear}");
    // Steering right turns it towards +x; saved first, and replayed to the bit.
    world.drive(car, 0.6, 1.0, 0.0, 0.0);
    let saved = world.save_state();
    run(&mut world, 120);
    world.transforms(&[chassis], &mut t);
    let turned = t[0];
    assert!(
        turned.position.x > ahead.position.x + 2.0,
        "{}",
        turned.position
    );
    world.restore_state(&saved).unwrap();
    run(&mut world, 120);
    world.transforms(&[chassis], &mut t);
    assert_eq!(t[0], turned);
}

/// Two boxes side by side over the ground, the left one held to the world and the right one
/// to the left one, like a beam out of a wall.
fn beam() -> (World, [BodyId; 2], [JointId; 2]) {
    let mut world = World::new(&WorldDesc::default());
    let ground = Shape::cuboid(Vec3::new(10.0, 0.5, 10.0), 0.05, 0.0).unwrap();
    world
        .add_body(&BodyDesc::fixed(&ground, DVec3::new(0.0, -0.5, 0.0)))
        .unwrap();
    // 1 m boxes of 100 kg, 3 m up.
    let block = Shape::cuboid(Vec3::splat(0.5), 0.05, 100.0).unwrap();
    let left = world
        .add_body(&BodyDesc::dynamic(&block, DVec3::new(0.0, 3.0, 0.0)))
        .unwrap();
    let right = world
        .add_body(&BodyDesc::dynamic(&block, DVec3::new(1.0, 3.0, 0.0)))
        .unwrap();
    let wall = world.join_fixed(None, left, (0, 0));
    let joint = world.join_fixed(Some(left), right, (0, 0));
    (world, [left, right], [wall, joint])
}

#[test]
fn a_joint_carries_its_load_and_lets_go_when_broken() {
    let (mut world, [left, right], joints) = beam();
    let dt = 1.0 / 60.0;
    for _ in 0..60 {
        world.step(dt, 1).unwrap();
    }
    // The beam holds: the right box sags by less than a centimetre.
    let mut t = Vec::new();
    world.transforms(&[left, right], &mut t);
    assert!((t[1].position.y - 3.0).abs() < 0.01, "{}", t[1].position);
    // The wall's joint carries both boxes' weight, and the turn of the right one's 1 m out;
    // the middle one the right box's weight, and its turn 0.5 m out.
    let mut loads = Vec::new();
    world.joint_loads(&joints, &mut loads);
    let weight = 100.0 * 9.81 * dt;
    let near = |load: f32, expected: f32| (load / expected - 1.0).abs() < 0.05;
    assert!(near(loads[0].position, 2.0 * weight), "{loads:?}");
    assert!(near(loads[1].position, weight), "{loads:?}");
    assert!(near(loads[0].rotation, weight), "{loads:?}");
    assert!(near(loads[1].rotation, 0.5 * weight), "{loads:?}");
    // Broken, the right box falls to the ground; the left one stays.
    world.set_holding(&joints[1..], false);
    let mut holding = Vec::new();
    world.holding(&joints, &mut holding);
    assert_eq!(holding, [true, false]);
    for _ in 0..120 {
        world.step(dt, 1).unwrap();
    }
    world.transforms(&[left, right], &mut t);
    assert!((t[0].position.y - 3.0).abs() < 0.01, "{}", t[0].position);
    assert!(t[1].position.y < 0.6, "{}", t[1].position);
}

#[test]
fn a_broken_joint_is_mended_by_a_restored_state() {
    let (mut world, bodies, joints) = beam();
    let run = |world: &mut World| {
        for _ in 0..90 {
            world.step(1.0 / 60.0, 1).unwrap();
        }
    };
    let saved = world.save_state();
    run(&mut world);
    let mut t = Vec::new();
    world.transforms(&bodies, &mut t);
    let held = t.clone();
    // Broken and run: the right box falls. Restored: the joint holds again and the run ends
    // where the first did, to the bit.
    world.restore_state(&saved).unwrap();
    world.set_holding(&joints[1..], false);
    run(&mut world);
    world.transforms(&bodies, &mut t);
    assert!(t[1].position.y < held[1].position.y - 1.0);
    world.restore_state(&saved).unwrap();
    let mut holding = Vec::new();
    world.holding(&joints, &mut holding);
    assert_eq!(holding, [true, true]);
    run(&mut world);
    world.transforms(&bodies, &mut t);
    assert_eq!(t, held);
}

#[test]
fn a_ball_on_a_chain_swings_at_its_length() {
    let mut world = World::new(&WorldDesc::default());
    let ball = Shape::sphere(0.3, 7800.0).unwrap();
    // Hung from 6 m up, pulled 4 m aside, let go.
    let pivot = DVec3::new(0.0, 6.0, 0.0);
    let start = DVec3::new(4.0, 6.0 - 20.0_f64.sqrt(), 0.0);
    let body = world.add_body(&BodyDesc::dynamic(&ball, start)).unwrap();
    let chain = world.join_distance(None, body, pivot, start, (0.0, 6.0));
    let mut t = Vec::new();
    let mut lowest = f64::MAX;
    for _ in 0..240 {
        world.step(1.0 / 60.0, 1).unwrap();
        world.transforms(&[body], &mut t);
        lowest = lowest.min(t[0].position.y);
        assert!(t[0].position.distance(pivot) < 6.02, "{}", t[0].position);
    }
    // It swung through the bottom, the chain taut there.
    assert!(lowest < 0.05, "{lowest}");
    let mut loads = Vec::new();
    world.joint_loads(&[chain], &mut loads);
    assert!(loads[0].position > 0.0);
}

/// A shoulder fixed to the world, an upper arm out along +x on a ball joint and a forearm on a
/// hinge after it, both 40 cm capsules of 4 and 3 kg-ish.
fn arm() -> (World, RagdollId, Vec<BodyId>) {
    let mut world = World::new(&WorldDesc::default());
    let block = Shape::cuboid(Vec3::splat(0.1), 0.02, 500.0).unwrap();
    let limb = Shape::capsule(0.15, 0.05, 1000.0)
        .unwrap()
        .offset(Vec3::ZERO, Quat::from_rotation_arc(Vec3::Y, Vec3::X))
        .unwrap();
    let at = |x: f64| Transform {
        position: DVec3::new(x, 2.0, 0.0),
        rotation: Quat::IDENTITY,
    };
    let part = |at: Transform, parent: Option<usize>, joint, pivot: f64| RagdollPart {
        shape: if parent.is_none() { &block } else { &limb },
        at,
        parent,
        joint,
        pivot: DVec3::new(pivot, 2.0, 0.0),
        twist_axis: Vec3::X,
        plane_axis: Vec3::Z,
        friction: 0.5,
    };
    let ball = RagdollJoint::SwingTwist {
        cone: (1.4, 1.4),
        twist: (-0.5, 0.5),
    };
    let hinge = RagdollJoint::Hinge { range: (-2.0, 2.0) };
    let parts = [
        part(at(0.0), None, ball, 0.0),
        part(at(0.3), Some(0), ball, 0.1),
        part(at(0.7), Some(1), hinge, 0.5),
    ];
    let (ragdoll, bodies) = world.add_ragdoll(&parts).unwrap();
    world.join_fixed(None, bodies[0], (0, 0));
    (world, ragdoll, bodies)
}

#[test]
fn a_ragdoll_holds_its_pose_on_its_motors_and_falls_limp() {
    let (mut world, ragdoll, bodies) = arm();
    let still = [[0.0, 0.0, 0.0, 1.0]; 3];
    let strong = Motors {
        stiffness: 2000.0,
        damping: 60.0,
        torque: 200.0,
    };
    let run = |world: &mut World, ticks: u32| {
        for _ in 0..ticks {
            world.step(1.0 / 60.0, 1).unwrap();
        }
    };
    world.drive_ragdoll(ragdoll, &still, strong);
    run(&mut world, 60);
    let mut t = Vec::new();
    world.transforms(&bodies, &mut t);
    // Held out: the hand's end within a few centimetres of where it was built.
    assert!((t[2].position.y - 2.0).abs() < 0.05, "{}", t[2].position);
    // Saved, then the motors let go: the arm hangs; restored and run again, the same bits.
    let saved = world.save_state();
    world.drive_ragdoll(
        ragdoll,
        &still,
        Motors {
            torque: 0.0,
            ..strong
        },
    );
    run(&mut world, 120);
    world.transforms(&bodies, &mut t);
    assert!(t[2].position.y < 1.6, "limp: {}", t[2].position);
    let hung = t.clone();
    world.restore_state(&saved).unwrap();
    world.drive_ragdoll(
        ragdoll,
        &still,
        Motors {
            torque: 0.0,
            ..strong
        },
    );
    run(&mut world, 120);
    world.transforms(&bodies, &mut t);
    assert_eq!(t, hung);
    // Driven to a bent elbow, the forearm turns up.
    world.restore_state(&saved).unwrap();
    let mut bent = still;
    bent[2] = [1.2, 0.0, 0.0, 0.0];
    world.drive_ragdoll(ragdoll, &bent, strong);
    run(&mut world, 60);
    world.transforms(&bodies, &mut t);
    assert!(
        (t[2].position - DVec3::new(0.7, 2.0, 0.0)).length() > 0.1,
        "{}",
        t[2].position
    );
}
