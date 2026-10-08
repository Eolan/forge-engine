//! The yard (#185, #186, `--lab yard`): the start of Phase 3's `materials-yard`. The dogs walk
//! their lanes as on the course, over beds of damp sand, mud and fresh snow and back, and their
//! footfalls (#167's foot-down events) press their paws into the beds. Beside them the drive
//! lab's car crosses its own beds of the three on an autopilot (the arrow keys take over), each
//! wheel on a bed pressing its contact patch every tick, so it ploughs its ruts. Each bed is a
//! layer of soft material over the floor (`forge_physics::deform`), drawn as a ground the skin
//! pass raises by the layer's heights (`forge_render::HeightField`), so the prints, the ruts and
//! their rims take the light and cast shadows.
//!
//! The dogs and the car stand on the floor, under the soft material: their paws and wheels go
//! through to it as the layer's least allows, and the beds lie over it, their edges thinning to
//! nothing.

use forge_anim::Footfall;
use forge_geom::TriMesh;
use forge_physics::deform::{Layer, Pad, Soft};
use forge_physics::{BodyId, VehicleId, World};
use glam::{DVec2, Vec2, Vec3};

use super::drive::Driver;

/// A bed: its layer and its material's name (the lab's rows').
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Bed {
    pub layer: Layer,
    pub name: &'static str,
}

/// The beds' materials (the lab's rows): sand, mud and snow.
pub(super) const SOFT: [&str; 3] = ["lab-sand", "lab-mud", "lab-snow"];
/// A bed as [`BEDS`] gives it.
type BedSpec = (Soft, &'static str, f32, [f64; 2], [f64; 2]);
/// The beds: material, name, metres between two points of its layer, and its corners (x, z)
/// from the low to the high.
/// - The dogs': across both their lanes (x ±0.7) between where they turn about (z −1.6 and
///   2.8), sand, then mud, then snow, the floor showing between them; a point every centimetre,
///   a dog's pad (3 cm by 4 cm) over a dozen.
/// - The car's: across its lane (x `LANE`), 3 m long each in the same order, which it crosses
///   from the snow; a point every 2 cm, a tyre's patch (20 cm by 14 cm) over seventy.
const BEDS: [BedSpec; 6] = [
    (Soft::SAND, SOFT[0], 0.01, [-1.2, -1.3], [1.2, -0.05]),
    (Soft::MUD, SOFT[1], 0.01, [-1.2, 0.05], [1.2, 1.3]),
    (Soft::SNOW, SOFT[2], 0.01, [-1.2, 1.4], [1.2, 2.65]),
    (Soft::SAND, SOFT[0], 0.02, [2.4, -4.6], [4.8, -1.6]),
    (Soft::MUD, SOFT[1], 0.02, [2.4, -1.5], [4.8, 1.5]),
    (Soft::SNOW, SOFT[2], 0.02, [2.4, 1.6], [4.8, 4.6]),
];
/// The car's lane (x), where it starts (z, facing −z, rolling on its own), the pace it keeps
/// (m/s) and where it brakes to a stop (z).
pub(super) const LANE: f64 = 3.6;
pub(super) const CAR_START: f64 = 9.0;
const CRUISE: f32 = 3.0;
const STOP_Z: f64 = -7.0;
/// A tyre's contact patch: half its width and half its length along the ground, metres.
const PATCH: Vec2 = Vec2::new(0.1, 0.07);
/// The yard's sun: low (26°) from beyond the beds and the sand's side, across the dogs' lanes,
/// so the prints' walls and rims stand out in light and shadow.
pub(crate) const SUN: Vec3 = Vec3::new(-0.5, 0.45, -0.75);
/// How far under the floor's top a bed's ground lies at its border, metres: there it is hidden
/// by the floor, which a layer at its least (4 mm) stands over.
pub(super) const UNDER: f32 = 0.001;

/// The beds, untouched.
pub(super) fn beds() -> Vec<Bed> {
    BEDS.iter()
        .map(|&(soft, name, cell, low, high)| {
            let size = [
                ((high[0] - low[0]) / f64::from(cell)).round() as u32 + 1,
                ((high[1] - low[1]) / f64::from(cell)).round() as u32 + 1,
            ];
            Bed {
                layer: Layer::new(soft, DVec2::from_array(low), cell, size),
                name,
            }
        })
        .collect()
}

/// A bed's ground as drawn before the layer raises it: a flat grid of its layer's points, from
/// its first at the origin, two triangles a cell. Its mover places it at [`place`].
pub(super) fn ground(bed: &Bed) -> TriMesh {
    let [nx, nz] = bed.layer.size();
    let cell = bed.layer.cell();
    let mut mesh = TriMesh::default();
    for z in 0..nz {
        for x in 0..nx {
            mesh.positions.push([x as f32 * cell, 0.0, z as f32 * cell]);
            mesh.normals.push([0.0, 1.0, 0.0]);
        }
    }
    for z in 0..nz - 1 {
        for x in 0..nx - 1 {
            let v = z * nx + x;
            // Counter-clockwise seen from above (+y).
            mesh.indices
                .extend_from_slice(&[v, v + nx, v + 1, v + 1, v + nx, v + nx + 1]);
        }
    }
    mesh
}

/// Where a bed's ground is drawn: its first point, `UNDER` the floor.
pub(super) fn place(bed: &Bed) -> Vec3 {
    let o = bed.layer.origin();
    Vec3::new(o.x as f32, -UNDER, o.y as f32)
}

/// The footfalls `falls` that came down on a bed pressed into it, each taking the bed's
/// material. How many did.
pub(super) fn press<'a>(
    beds: &mut [Bed],
    falls: impl IntoIterator<Item = &'a mut Footfall<&'static str>>,
) -> usize {
    let mut pressed = 0;
    for fall in falls {
        let at = DVec2::new(fall.position.x, fall.position.z);
        let Some(bed) = beds.iter_mut().find(|b| b.layer.covers(at)) else {
            continue;
        };
        let heading = Vec2::new(fall.heading.x, fall.heading.z);
        bed.layer.press(Pad {
            at,
            heading,
            size: fall.size,
            pressure: fall.pressure,
            sweep: 0.0,
        });
        fall.material = bed.name;
        pressed += 1;
    }
    pressed
}

/// Hands the yard's car its controls for the coming step: the player's while they hold any;
/// else the autopilot's, which keeps it to its lane (against its offset and its heading) at its
/// pace, and brakes it to a stop past the beds.
pub(super) fn drive(world: &mut World, driver: &Driver) {
    let Some((chassis, vehicle)) = driver.car else {
        return;
    };
    if driver.throttle != 0.0 || driver.steer != 0.0 || driver.handbrake != 0.0 {
        driver.tick(world);
        return;
    }
    let (mut t, mut v) = (Vec::new(), Vec::new());
    world.transforms(&[chassis], &mut t);
    world.velocities(&[chassis], &mut v);
    let forward = t[0].rotation * Vec3::NEG_Z;
    let ahead = v[0].linear.dot(forward);
    let off = (t[0].position.x - LANE) as f32;
    let steer = (-0.3 * off - 1.5 * forward.x).clamp(-1.0, 1.0);
    if t[0].position.z < STOP_Z {
        world.drive(vehicle, 0.0, steer, 1.0, 0.0);
    } else {
        let throttle = (0.3 * (CRUISE - ahead)).clamp(0.0, 0.6);
        world.drive(vehicle, throttle, steer, 0.0, 0.0);
    }
}

/// After a step of `dt`: each of the car's wheels touching a bed presses its contact patch into
/// it at the pressure of the load on it, over the whole stretch it rolled across in the step
/// (along its middle's own travel), so its rut runs on unbroken. How many did.
pub(super) fn roll(
    beds: &mut [Bed],
    world: &World,
    (chassis, vehicle): (BodyId, VehicleId),
    dt: f32,
) -> usize {
    let mut contacts = Vec::new();
    world.wheel_contacts(vehicle, dt, &mut contacts);
    let mut v = Vec::new();
    world.velocities(&[chassis], &mut v);
    let mut t = Vec::new();
    world.transforms(&[chassis], &mut t);
    let mut pressed = 0;
    // Under each wheel's middle: the contact Jolt finds may lie anywhere across a flat tyre's
    // width, 18 cm from one step to the next.
    let mut wheels = Vec::new();
    world.wheels(vehicle, &mut wheels);
    for (c, wheel) in contacts.iter().zip(&wheels) {
        let Some(c) = c else {
            continue;
        };
        let at = DVec2::new(wheel.position.x, wheel.position.z);
        let Some(bed) = beds.iter_mut().find(|b| b.layer.covers(at)) else {
            continue;
        };
        // The way it rolled over the step, and how far: its middle's own velocity (the car's,
        // and its turn's about it), so the stretch runs back to where it was, drift and all.
        let r = (wheel.position - t[0].position).as_vec3();
        let moving = v[0].linear + v[0].angular.cross(r);
        let travel = Vec2::new(moving.x, moving.z);
        let heading = travel.normalize_or(Vec2::new(c.forward.x, c.forward.z));
        bed.layer.press(Pad {
            at,
            heading,
            size: PATCH,
            pressure: c.load / (std::f32::consts::PI * PATCH.x * PATCH.y),
            sweep: travel.length() * dt,
        });
        pressed += 1;
    }
    pressed
}

/// Every bed's heights in turn, as the renderer's height fields take them.
pub(super) fn heights(beds: &[Bed], out: &mut Vec<f32>) {
    out.clear();
    for bed in beds {
        out.extend_from_slice(bed.layer.heights());
    }
}

/// The farthest a bed's ground rises or sinks from flat, metres: how far its clusters' bounds
/// reach. A car through the mud heaps berms 14 cm over the floor (9 cm over the mud); a car
/// driven back and forth could heap more, past it.
pub(super) const REACH: f32 = 0.25;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::creatures::{Feet, Ground, build};
    use crate::lab::drive;
    use forge_physics::{BodyDesc, Shape, WorldDesc};
    use forge_sim::TICK;
    use glam::DVec3;

    /// The yard over `seconds`: the dogs pressing their footfalls into the beds and the car on
    /// its autopilot rolling over its own. The beds after, how many footfalls each material
    /// took, and how many wheel presses.
    fn walk_the_yard(seconds: u32) -> (Vec<Bed>, [usize; 3], usize) {
        let mut world = World::new(&WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(20.0, 0.5, 20.0), 0.05, 0.0).expect("a floor");
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .expect("the floor");
        let field = build(&mut world, 0, 0, Ground::Yard).expect("the yard");
        let car = drive::car(&mut world, DVec3::new(LANE, 0.15, CAR_START)).expect("the car");
        let driver = Driver {
            car: Some(car),
            ..Driver::default()
        };
        let mut beds = beds();
        let mut feet = Feet::default();
        let (mut taken, mut rolled) = ([0; 3], 0);
        for tick in 0..seconds * 60 {
            let time = f64::from(tick) * f64::from(TICK);
            field.herd.drive(&mut world, time, false);
            drive(&mut world, &driver);
            world.step(TICK, 1).expect("a step");
            let came = field.herd.feel(&world, u64::from(tick), &mut feet);
            press(&mut beds, feet.last_mut(came));
            for fall in feet.last_mut(came) {
                if let Some(k) = SOFT.iter().position(|&s| s == fall.material) {
                    taken[k] += 1;
                }
            }
            rolled += roll(&mut beds, &world, car, TICK);
        }
        (beds, taken, rolled)
    }

    /// A layer's heights inside its bevelled edges (6 cm, eight points in from the border).
    fn inside(l: &Layer) -> Vec<f32> {
        let [nx, nz] = l.size();
        let edge = (0.08 / l.cell()).round() as u32;
        (edge..nz - edge)
            .flat_map(|z| (edge..nx - edge).map(move |x| (z * nx + x) as usize))
            .map(|k| l.heights()[k])
            .collect()
    }

    fn lowest(l: &Layer) -> f32 {
        inside(l).into_iter().fold(f32::MAX, f32::min)
    }

    fn highest(l: &Layer) -> f32 {
        inside(l).into_iter().fold(f32::MIN, f32::max)
    }

    #[test]
    fn the_dogs_print_and_the_car_ruts_sand_mud_and_snow_and_replay() {
        let (beds, taken, rolled) = walk_the_yard(26);
        // Up the dogs' three beds and back: a dozen footfalls on each or more.
        assert!(taken.iter().all(|&n| n >= 12), "{taken:?}");
        let [sand, mud, snow] = [&beds[0].layer, &beds[1].layer, &beds[2].layer];
        // Sand: prints over a centimetre deep (1.2 cm), heaped round with a few millimetres (4).
        let depth = Soft::SAND.depth;
        assert!(lowest(sand) < depth - 0.01, "{}", lowest(sand));
        assert!(highest(sand) > depth + 0.002, "{}", highest(sand));
        // Mud: deeper (1.9 cm, as deep as its 56° walls let a small pad's print be), and heaped
        // higher, nearly nothing packed.
        assert!(lowest(mud) < Soft::MUD.depth - 0.015, "{}", lowest(mud));
        assert!(
            highest(mud) - Soft::MUD.depth > highest(sand) - depth,
            "{} {}",
            highest(mud),
            highest(sand)
        );
        // Snow: the paws went through to the floor, the least left under them.
        assert!(lowest(snow) < Soft::SNOW.least + 0.002, "{}", lowest(snow));
        // Between the lanes (x 0), untouched.
        for bed in &beds[..3] {
            let middle = bed.layer.origin() + DVec2::new(1.2, 0.6);
            assert_eq!(bed.layer.height_at(middle), bed.layer.soft().depth);
        }
        // The car crossed its three beds, every tick a few wheels on them: two ruts in each,
        // under each pair of wheels (0.74 m either side of its lane), and none between them.
        assert!(rolled > 200, "{rolled} wheel presses");
        for bed in &beds[3..] {
            let l = &bed.layer;
            let soft = l.soft();
            let across = |x: f64| {
                let z = l.origin().y + 1.5;
                l.height_at(DVec2::new(x, z))
            };
            // The autopilot keeps within 15 cm of its lane: the deepest point within 25 cm of
            // each wheel's line.
            for side in [-0.74, 0.74] {
                let rut = (-25..=25)
                    .map(|k| across(LANE + side + 0.01 * f64::from(k)))
                    .fold(f32::MAX, f32::min);
                assert!(rut < soft.depth - 0.01, "{}: {rut} under a wheel", bed.name);
            }
            assert_eq!(across(LANE), soft.depth, "{} between the wheels", bed.name);
            // Ploughed: the berms beside the ruts stand over the untouched layer.
            assert!(
                highest(l) > soft.depth + 0.002,
                "{}: {}",
                bed.name,
                highest(l)
            );
        }
        // The same run presses the same bits.
        let (again, taken_again, rolled_again) = walk_the_yard(26);
        assert_eq!((taken, rolled), (taken_again, rolled_again));
        assert!(
            beds.iter()
                .zip(&again)
                .all(|(a, b)| a.layer.heights() == b.layer.heights())
        );
    }
}
