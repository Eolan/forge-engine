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

use anyhow::Result;
use forge_anim::Footfall;
use forge_geom::TriMesh;
use forge_physics::deform::{Dig, Layer, Pad, Soft, Tread};
use forge_physics::{BodyDesc, BodyId, Shape, VehicleId, World};
use forge_sim::TICK;
use glam::{DVec2, Vec2, Vec3};

use super::drive::{self, Driver};

/// A bed: its layer, its material's name (the lab's rows'), and its grip when the physics
/// stands on it (the car's, #187: a height field kept to the layer, `Ground`).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Bed {
    pub layer: Layer,
    pub name: &'static str,
    pub grip: Option<f32>,
}

/// The beds' materials (the lab's rows): sand, mud and snow.
pub(super) const SOFT: [&str; 3] = ["lab-sand", "lab-mud", "lab-snow"];
/// A bed as [`BEDS`] gives it: its material and name, metres between two points of its layer,
/// its corners (x, z) from the low to the high, how far in from its border it thins to nothing,
/// and its grip as ground (none: the physics does not see it).
struct BedSpec {
    soft: Soft,
    name: &'static str,
    cell: f32,
    low: [f64; 2],
    high: [f64; 2],
    bevel: f32,
    grip: Option<f32>,
}

const fn spec(soft: Soft, name: &'static str, low: [f64; 2], high: [f64; 2]) -> BedSpec {
    BedSpec {
        soft,
        name,
        cell: 0.01,
        low,
        high,
        bevel: 0.06,
        grip: None,
    }
}

const fn as_ground(spec: BedSpec, grip: f32) -> BedSpec {
    BedSpec {
        cell: 0.02,
        bevel: 0.25,
        grip: Some(grip),
        ..spec
    }
}

/// The car's mud (#187): a puddle of soft mud 15 cm deep, a wheel's 105 kPa sinking it
/// 10.5 cm, wetter than the dogs' (its walls slump to 40°).
const DEEP_MUD: Soft = Soft {
    depth: 0.15,
    stiffness: 1.0e6,
    repose: 0.85,
    packing: 0.5,
    ..Soft::MUD
};
/// The car's sand (#191): a deep bed of it, 12 cm a wheel's 105 kPa sinks 4 cm into, so a
/// wheel spinning there has sand to dig (the dogs' 3 cm, pressed to its least, had none).
const DEEP_SAND: Soft = Soft {
    depth: 0.12,
    stiffness: 2.6e6,
    ..Soft::SAND
};
/// The beds.
/// - The dogs': across both their lanes (x ±0.7) between where they turn about (z −1.6 and
///   2.8), sand, then mud, then snow, the floor showing between them; a point every centimetre,
///   a dog's pad (3 cm by 4 cm) over a dozen. The dogs walk on the floor under them.
/// - The car's: across its lane (x `LANE`), 3 m long each in the same order, which it crosses
///   from the snow; a point every 2 cm, a tyre's patch (20 cm by 14 cm) over seventy. Ground the
///   car stands on, thinning over 25 cm at their edges, gripping as their material does against
///   the floor's 0.2 (Jolt takes the square root of the tyre's and the ground's): damp sand
///   0.4, snow 0.12, the mud a deep puddle at 0.1.
const BEDS: [BedSpec; 6] = [
    spec(Soft::SAND, SOFT[0], [-1.2, -1.3], [1.2, -0.05]),
    spec(Soft::MUD, SOFT[1], [-1.2, 0.05], [1.2, 1.3]),
    spec(Soft::SNOW, SOFT[2], [-1.2, 1.4], [1.2, 2.65]),
    as_ground(spec(DEEP_SAND, SOFT[0], [2.4, -4.6], [4.8, -1.6]), 0.4),
    as_ground(spec(DEEP_MUD, SOFT[1], [2.4, -1.5], [4.8, 1.5]), 0.15),
    as_ground(spec(Soft::SNOW, SOFT[2], [2.4, 1.6], [4.8, 4.6]), 0.2),
];
/// The car's lane (x), where it starts (z, facing −z, rolling on its own), the pace it keeps
/// (m/s) and where it brakes to a stop (z).
pub(super) const LANE: f64 = 3.6;
pub(super) const CAR_START: f64 = 9.0;
const CRUISE: f32 = 3.0;
const STOP_Z: f64 = -7.0;
/// The autopilot's stop in the deep sand (#191): past `HALT` (z) it brakes, standing with all
/// four wheels in the sand, until tick `LAUNCH` (a second or so after); then it pulls away at full
/// throttle until past the sand's far end (z), its driven wheels spinning and digging in. (In
/// the deep mud a standing car would never pull away: its grip moves it less than a quarter of
/// its weight, which ploughing that mud takes.)
const HALT: f64 = -2.55;
const LAUNCH: u64 = 460;
const SAND_END: f64 = -4.6;
/// A tyre's footprint, half its width and half its length along the ground (metres), over
/// which its load presses; and the patch it presses, 2 cm wider each side, so its rut holds the
/// whole tyre and the berms rise beyond it, not under its edges.
const TYRE: Vec2 = Vec2::new(0.1, 0.07);
const PATCH: Vec2 = Vec2::new(0.12, 0.07);
/// The car's tyres' tread (#188): lugs 9 mm deep every 8 cm (four of the car's beds' points
/// a lug, coarse enough not to shimmer), across its 20 cm, in chevrons swept back 0.6 m a metre.
const TREAD: Tread = Tread {
    depth: 0.009,
    half_width: 0.1,
    pitch: 0.08,
    sweep_back: 0.6,
};
/// The yard's sun: low (26°) from beyond the beds and the sand's side, across the dogs' lanes,
/// so the prints' walls and rims stand out in light and shadow.
pub(crate) const SUN: Vec3 = Vec3::new(-0.5, 0.45, -0.75);
/// How far under the floor's top a bed's ground lies at its border, metres: there it is hidden
/// by the floor, which a layer at its least (4 mm) stands over.
pub(super) const UNDER: f32 = 0.001;

/// The beds, untouched.
pub(super) fn beds() -> Vec<Bed> {
    BEDS.iter()
        .map(|b| {
            let size = [
                ((b.high[0] - b.low[0]) / f64::from(b.cell)).round() as u32 + 1,
                ((b.high[1] - b.low[1]) / f64::from(b.cell)).round() as u32 + 1,
            ];
            Bed {
                layer: Layer::with_bevel(b.soft, DVec2::from_array(b.low), b.cell, size, b.bevel),
                name: b.name,
                grip: b.grip,
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
            wheel: 0.0,
            tread: None,
        });
        fall.material = bed.name;
        pressed += 1;
    }
    pressed
}

/// Hands the yard's car its controls for the coming step: the player's while they hold any;
/// else the autopilot's, which keeps it to its lane (against its offset and its heading) at its
/// pace, stops it in its sand (`HALT`) and pulls away hard at tick `LAUNCH` (#191), and brakes
/// it to a stop past the beds. Either way, the soft grounds under its wheels hold it back
/// (#187).
pub(super) fn drive(world: &mut World, driver: &Driver, beds: &[Bed], tick: u64) {
    let Some((chassis, vehicle)) = driver.car else {
        return;
    };
    hold_back(world, (chassis, vehicle), beds);
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
    let z = t[0].position.z;
    if z < STOP_Z || (z < HALT && tick < LAUNCH) {
        world.drive(vehicle, 0.0, steer, 1.0, 0.0);
    } else if tick >= LAUNCH && z > SAND_END {
        // Floored in the mud: its driven wheels spin and dig in (#191).
        world.drive(vehicle, 1.0, steer, 0.0, 0.0);
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
    // Each wheel presses with its share of the car's weight, not the load on it this step: that
    // swings as the car rocks, and its ruts and their berms rose and fell with it in waves
    // (#188).
    let share = drive::CAR_MASS * 9.81 / contacts.iter().flatten().count().max(1) as f32;
    let spins = world.wheel_spins(vehicle);
    for ((c, wheel), spin) in contacts.iter().zip(&wheels).zip(spins) {
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
        // From where it was at the step's start to where it will be at the next one's end: the
        // material gives way before a wheel pushing into it, which would otherwise climb the
        // front of its own rut every step, lifted and held back by it.
        let step = travel.length() * dt;
        // At least `LEAD` ahead even when slow: a wheel left in a trough of its own round, still,
        // rests on its front wall, which the suspension pushes back from, and never climbs out
        // (#189).
        let ahead = step.max(LEAD);
        // Its slip (#191): how much faster its tread runs over the ground than it travels along
        // it (slower, a locked wheel sliding: negative). A slipping tread tears the material from
        // under it and throws it the way it slides, by the distance it slipped, so a wheel
        // spinning where it stands digs itself in; and it smears its lugs, gone past `SMEAR`.
        let forward = Vec2::new(c.forward.x, c.forward.z).normalize_or(heading);
        let slip = spin * WHEEL_RADIUS - travel.dot(forward);
        if slip.abs() > SLIP {
            bed.layer.dig(Dig {
                at,
                throw: -forward * slip.signum(),
                size: TYRE,
                depth: DIG * slip.abs() * dt,
                reach: THROWN,
                wheel: TROUGH * WHEEL_RADIUS,
            });
        }
        let lugs = TREAD.depth * (1.0 - slip.abs() / SMEAR).max(0.0);
        bed.layer.press(Pad {
            at: at + (heading * ahead).as_dvec2(),
            heading,
            size: PATCH,
            pressure: share / (4.0 * TYRE.x * TYRE.y),
            sweep: step + ahead,
            wheel: TROUGH * WHEEL_RADIUS,
            tread: (lugs > 0.0).then_some(Tread {
                depth: lugs,
                ..TREAD
            }),
        });
        pressed += 1;
    }
    pressed
}

/// A bed the physics stands on (#187): its index in the beds, its body (a Jolt height field
/// of `count` × `count` samples, the layer's points and holes past them), and its samples as
/// Jolt was last given them.
#[derive(Clone, Debug)]
pub(super) struct Ground {
    bed: usize,
    body: BodyId,
    count: u32,
    samples: Vec<f32>,
}

impl Ground {
    /// Its samples as Jolt holds them, which lag its layer by up to `SETTLE`: what a save keeps,
    /// so a restored car stands on the same ground to the bit (#191).
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
}

/// Back to saved grounds: each one's samples ([`Ground::samples`], in turn from `saved`, four
/// little-endian bytes each), given to Jolt whole; their beds' changes are theirs already. What
/// of `saved` is left.
pub(super) fn set_grounds<'a>(
    world: &mut World,
    beds: &mut [Bed],
    grounds: &mut [Ground],
    mut saved: &'a [u8],
) -> &'a [u8] {
    for g in grounds {
        beds[g.bed].layer.take_changed();
        let (bytes, rest) = saved.split_at(4 * g.samples.len());
        for (s, b) in g.samples.iter_mut().zip(bytes.as_chunks::<4>().0) {
            *s = f32::from_bits(u32::from_le_bytes(*b));
        }
        world.set_heights(g.body, [0, 0], [g.count, g.count], &g.samples, g.count);
        saved = rest;
    }
    saved
}

/// The highest a ground's samples go, metres: its height field's range, fixed when made.
const GROUND_TOP: f32 = 0.4;
/// How far a sample may stray from the layer before its ground is given it again, metres.
const SETTLE: f32 = 0.001;

/// The ground under each bed with a grip, its height field the layer's, in `world`.
pub(super) fn grounds(world: &mut World, beds: &[Bed]) -> Result<Vec<Ground>> {
    let mut grounds = Vec::new();
    for (k, bed) in beds.iter().enumerate() {
        let Some(grip) = bed.grip else {
            continue;
        };
        let [gx, gz] = ground_size(&bed.layer);
        // Square and even, as Jolt asks (blocks of two samples).
        let count = (gx.max(gz) + 1) & !1;
        let mut samples = vec![f32::MAX; (count * count) as usize];
        for z in 0..gz {
            for x in 0..gx {
                samples[(z * count + x) as usize] = ground_height(&bed.layer, x, z);
            }
        }
        let cell = bed.layer.cell() * GROUND_STRIDE as f32;
        let shape = Shape::height_field_editable(
            &samples,
            count,
            Vec3::ZERO,
            Vec3::new(cell, 1.0, cell),
            (0.0, GROUND_TOP),
        )?;
        let place = place(bed);
        let body = world.add_body(&BodyDesc {
            friction: grip,
            // Felt by the car's wheels alone: no other body tests its many small triangles.
            wheels_only: true,
            ..BodyDesc::fixed(&shape, place.as_dvec3())
        })?;
        grounds.push(Ground {
            bed: k,
            body,
            count,
            samples,
        });
    }
    Ok(grounds)
}

/// A ground takes every `GROUND_STRIDE`-th point of its layer each way (4 cm under the car):
/// each wheel's cylinder is cast against the triangles under it every step, 2 500 of them a
/// step at the layer's 2 cm, which cost 0.4 ms.
const GROUND_STRIDE: u32 = 2;

/// A ground's samples along x and z.
fn ground_size(layer: &Layer) -> [u32; 2] {
    layer.size().map(|n| (n - 1) / GROUND_STRIDE + 1)
}

/// A ground's sample (x, z): its layer's point under it.
fn ground_height(layer: &Layer, x: u32, z: u32) -> f32 {
    let nx = layer.size()[0];
    layer.heights()[(z * GROUND_STRIDE * nx + x * GROUND_STRIDE) as usize]
}

/// Gives each ground what its bed's layer changed since the last call, box by box (`all`: the
/// whole of it,
/// after a restore or a reset, as Jolt's saved state holds no shape), in blocks of two samples.
pub(super) fn settle(world: &mut World, beds: &mut [Bed], grounds: &mut [Ground], all: bool) {
    for g in grounds {
        let layer = &mut beds[g.bed].layer;
        let [nx, nz] = layer.size();
        let [gx, gz] = ground_size(layer);
        let mut boxes = layer.take_changed();
        if all {
            boxes = vec![([0, 0], [nx - 1, nz - 1])];
        }
        for (low, high) in boxes {
            // Only where a sample moved past `SETTLE` from what Jolt holds: the slump's
            // fractions of a millimetre about a rim wait for a later change.
            let (mut l, mut h) = ([u32::MAX; 2], [0u32; 2]);
            let from = low.map(|v| v / GROUND_STRIDE);
            let to = [
                high[0].div_ceil(GROUND_STRIDE).min(gx - 1),
                high[1].div_ceil(GROUND_STRIDE).min(gz - 1),
            ];
            for z in from[1]..=to[1] {
                for x in from[0]..=to[0] {
                    let now = ground_height(layer, x, z);
                    let k = (z * g.count + x) as usize;
                    if all || (now - g.samples[k]).abs() > SETTLE {
                        g.samples[k] = now;
                        l = [l[0].min(x), l[1].min(z)];
                        h = [h[0].max(x), h[1].max(z)];
                    }
                }
            }
            if l[0] > h[0] {
                continue;
            }
            // In blocks of two samples: the box's other samples as Jolt holds them already.
            let x0 = l[0] & !1;
            let z0 = l[1] & !1;
            let x1 = ((h[0] + 2) & !1).min(g.count);
            let z1 = ((h[1] + 2) & !1).min(g.count);
            let first = (z0 * g.count + x0) as usize;
            world.set_heights(
                g.body,
                [x0, z0],
                [x1 - x0, z1 - z0],
                &g.samples[first..],
                g.count,
            );
        }
    }
}

/// How hard the soft ground holds a wheel back, as a share of its load, for each metre of
/// root of its sinkage over its diameter (a rigid wheel's entry angle, `sqrt(z / 2r)`, halved
/// for a tyre that flattens), and the wheels' radius (the drive lab's car's).
const ROLLING: f32 = 0.6;
const WHEEL_RADIUS: f32 = 0.31;
/// Under this pace (m/s) a wheel's hold eases off, so a stopped car is not pushed back.
const ROLLING_PACE: f32 = 0.5;
/// How far ahead of a wheel its press reaches at least, metres, and how much rounder than the
/// wheel the trough it presses is: so the tyre rests on the trough's floor, not on its front
/// wall (#189).
const LEAD: f32 = 0.04;
const TROUGH: f32 = 1.3;
/// A wheel's slip (#191): under `SLIP` (m/s) it grips and digs nothing; past it, its tread
/// tears `DIG` metres of material from under it for each metre it slips and throws it as far as
/// `THROWN` past its patch; its lugs fade as its slip nears `SMEAR` (m/s), smeared past it.
const SLIP: f32 = 0.3;
const DIG: f32 = 0.004;
const THROWN: f32 = 0.6;
const SMEAR: f32 = 1.0;

/// For the coming step: each of the car's wheels sunk into a ground is held back by the soft
/// material it ploughs (#187), against its travel at its contact, by its load times
/// `ROLLING · sqrt(z / 2r)`, z how far it sank below the untouched layer. From the contacts and
/// the beds as the last step left them, so a replay holds it back the same.
fn hold_back(world: &mut World, (chassis, vehicle): (BodyId, VehicleId), beds: &[Bed]) {
    let mut contacts = Vec::new();
    world.wheel_contacts(vehicle, TICK, &mut contacts);
    let (mut v, mut t, mut wheels) = (Vec::new(), Vec::new(), Vec::new());
    world.velocities(&[chassis], &mut v);
    world.transforms(&[chassis], &mut t);
    world.wheels(vehicle, &mut wheels);
    let mut pushes = Vec::new();
    for (c, wheel) in contacts.iter().zip(&wheels) {
        let Some(c) = c else {
            continue;
        };
        let at = DVec2::new(wheel.position.x, wheel.position.z);
        let Some(bed) = beds.iter().find(|b| b.grip.is_some() && b.layer.covers(at)) else {
            continue;
        };
        let sunk = (bed.layer.soft().depth - bed.layer.height_at(at)).max(0.0);
        let r = (wheel.position - t[0].position).as_vec3();
        let moving = v[0].linear + v[0].angular.cross(r);
        let flat = Vec3::new(moving.x, 0.0, moving.z);
        let pace = flat.length();
        let hold = ROLLING * (sunk / (2.0 * WHEEL_RADIUS)).sqrt() * c.load;
        let force = -flat.normalize_or_zero() * hold * (pace / ROLLING_PACE).min(1.0);
        pushes.push((force, c.position, Vec3::ZERO));
    }
    let bodies = vec![chassis; pushes.len()];
    world.push(&bodies, &pushes);
}

/// Every bed's surface as drawn (its thickness and its treads' relief) in turn, as the
/// renderer's height fields take them.
pub(super) fn heights(beds: &[Bed], out: &mut Vec<f32>) {
    out.clear();
    let mut drawn = Vec::new();
    for bed in beds {
        bed.layer.drawn(&mut drawn);
        out.extend_from_slice(&drawn);
    }
}

/// The farthest a bed's ground rises or sinks from flat, metres: how far its clusters' bounds
/// reach. The car through its deep mud heaps berms 27 cm over the floor (12 cm over the mud); as
/// high as its ground goes (`GROUND_TOP`), which a car driven back and forth could reach.
pub(super) const REACH: f32 = GROUND_TOP;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::creatures::{self, Feet, build};
    use crate::lab::drive;
    use forge_physics::{BodyDesc, Shape, WorldDesc};
    use forge_sim::TICK;
    use glam::DVec3;

    /// The car at a tick: where its chassis is along its lane (z) and how high (y), its pace
    /// (m/s), and how fast the faster of its driven wheels' treads runs (m/s, #191).
    type Trip = Vec<(f64, f64, f32, f32)>;

    /// The yard over `seconds`: the dogs pressing their footfalls into the beds and the car on
    /// its autopilot rolling over its own, which it stands on. The beds after, how many
    /// footfalls each material took, how many wheel presses, and the car's trip.
    fn walk_the_yard(seconds: u32) -> (Vec<Bed>, [usize; 3], usize, Trip) {
        let mut world = World::new(&WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(20.0, 0.5, 20.0), 0.05, 0.0).expect("a floor");
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .expect("the floor");
        let field = build(&mut world, 0, 0, creatures::Ground::Yard).expect("the yard");
        let mut beds = beds();
        let mut grounds = grounds(&mut world, &beds).expect("the car's grounds");
        let car =
            drive::car_on(&mut world, DVec3::new(LANE, 0.15, CAR_START), true).expect("the car");
        let driver = Driver {
            car: Some(car),
            ..Driver::default()
        };
        let mut feet = Feet::default();
        let (mut taken, mut rolled, mut trip) = ([0; 3], 0, Vec::new());
        let (mut t, mut v) = (Vec::new(), Vec::new());
        for tick in 0..seconds * 60 {
            let time = f64::from(tick) * f64::from(TICK);
            field.herd.drive(&mut world, time, false);
            drive(&mut world, &driver, &beds, u64::from(tick));
            world.step(TICK, 1).expect("a step");
            let came = field.herd.feel(&world, u64::from(tick), &mut feet);
            press(&mut beds, feet.last_mut(came));
            for fall in feet.last_mut(came) {
                if let Some(k) = SOFT.iter().position(|&s| s == fall.material) {
                    taken[k] += 1;
                }
            }
            rolled += roll(&mut beds, &world, car, TICK);
            settle(&mut world, &mut beds, &mut grounds, false);
            world.transforms(&[car.0], &mut t);
            world.velocities(&[car.0], &mut v);
            let spins = world.wheel_spins(car.1);
            let tread = spins[0].max(spins[1]) * WHEEL_RADIUS;
            trip.push((
                t[0].position.z,
                t[0].position.y,
                v[0].linear.length(),
                tread,
            ));
        }
        (beds, taken, rolled, trip)
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
        let (beds, taken, rolled, trip) = walk_the_yard(26);
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
        // The car stands on its beds (#187): at its pace on the floor before them, slowed in the
        // deep mud (its middle over the mud's; 1.25 m/s, its spinning front wheel tearing at it),
        // up to pace again past its sand, stopped beyond. Its wheels sank 10.5 cm into the mud
        // under their share of its weight.
        let pace = |from: f64, to: f64| {
            trip.iter()
                .filter(|(z, ..)| (from..to).contains(z))
                .map(|&(_, _, p, _)| p)
                .fold((f32::MAX, f32::MIN), |(l, h), p| (l.min(p), h.max(p)))
        };
        let floor = pace(5.6, 6.6).0;
        let mud = pace(-1.0, 1.0).0;
        let after = pace(-6.5, -5.5).1;
        assert!(floor > 2.8, "{floor} m/s on the floor");
        assert!(mud < 0.6 * floor, "{mud} m/s in the mud against {floor}");
        assert!(after > 2.4, "{after} m/s past the beds");
        assert!(
            trip.last()
                .is_some_and(|&(z, _, p, _)| z < STOP_Z && p < 0.1)
        );
        // Its stop in the deep sand (#191): standing there, all four wheels on it, then floored,
        // its driven wheels spinning far faster than it goes (21 m/s past it), and digging 2.4 cm
        // below the ruts their share of its weight presses (8 cm deep) where it bogged down.
        let (z, _, p, _) = trip[LAUNCH as usize - 1];
        assert!(p < 0.05 && (-3.3..-2.8).contains(&z), "{p} m/s at {z}");
        let slip = trip[LAUNCH as usize..]
            .iter()
            .filter(|(z, ..)| *z > SAND_END)
            .map(|&(_, _, p, tread)| tread - p)
            .fold(f32::MIN, f32::max);
        assert!(slip > 5.0, "the treads ran {slip} m/s past the car");
        let sand = &beds[3].layer;
        let pressed = DEEP_SAND.depth
            - drive::CAR_MASS * 9.81 / 4.0 / (4.0 * TYRE.x * TYRE.y) / DEEP_SAND.stiffness;
        for side in [-0.74, 0.74] {
            let dug = (0..250)
                .flat_map(|k| (-8..=8).map(move |j| (k, j)))
                .map(|(k, j)| {
                    let at = DVec2::new(
                        LANE + side + 0.01 * f64::from(j),
                        -4.35 + 0.01 * f64::from(k),
                    );
                    sand.height_at(at)
                })
                .fold(f32::MAX, f32::min);
            assert!(
                dug < pressed - 0.02,
                "{dug} under the {side} wheels, pressed to {pressed}"
            );
        }
        let deep = &beds[4].layer;
        let under = (-30..=30)
            .map(|k| deep.height_at(DVec2::new(LANE - 0.74 + 0.01 * f64::from(k), 0.0)))
            .fold(f32::MAX, f32::min);

        assert!(under < DEEP_MUD.depth - 0.09, "{under} under a wheel");
        // The outer berm of the left wheels' rut: its crest at each z across the mud's middle.
        let crests: Vec<f32> = (0..100)
            .map(|k| {
                let z = -1.0 + 0.02 * f64::from(k);
                (0..40)
                    .map(|j| deep.height_at(DVec2::new(2.45 + 0.01 * f64::from(j), z)))
                    .fold(f32::MIN, f32::max)
            })
            .collect();
        let (lo, hi) = crests
            .iter()
            .fold((f32::MAX, f32::MIN), |(l, h), &c| (l.min(c), h.max(c)));
        let wiggle: f32 =
            crests.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (crests.len() - 1) as f32;
        // Across both ruts' berms proper, outside the ruts' walls and their tops.
        for x in [
            2.42, 2.47, 2.52, 2.57, 2.62, 3.10, 3.15, 3.20, 3.25, 3.30, 3.90, 3.95, 4.00, 4.05,
            4.10, 4.58, 4.63, 4.68, 4.73,
        ] {
            let line: Vec<f32> = (0..100)
                .map(|k| deep.height_at(DVec2::new(x, -1.0 + 0.02 * f64::from(k))))
                .collect();
            let step: f32 = line.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / 99.0;
            // Its flanks as even (#189): they stepped 2.4 mm a point, striped by a low sun.
            assert!(step < 0.001, "the flank at {x}: {step} a point");
        }
        // Low and even (#189): a few centimetres over the mud, its crest stepping under
        // 2 mm a point (it stood 12 cm over it, ridged a step apart).
        assert!(hi < DEEP_MUD.depth + 0.05, "a berm {hi} high");
        assert!(
            hi - lo < 0.03 && wiggle < 0.002,
            "a crest from {lo} to {hi}, {wiggle} a point"
        );
        // The car's snow ruts' walls even along them (#188): the smoothing heaped against their
        // tops once, and the slump poured it into their feet in a sawtooth a low sun dashed.
        let snow = &beds[5].layer;
        for x in [2.73, 2.75, 2.77] {
            let wall: Vec<f32> = (0..100)
                .map(|k| snow.height_at(DVec2::new(x, 2.0 + 0.02 * f64::from(k))))
                .collect();
            let (lo, hi) = wall
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), &w| (l.min(w), h.max(w)));
            assert!(hi - lo < 0.004, "the snow rut's wall at {x}: {lo} to {hi}");
        }
        // The same run presses the same bits.
        let (again, taken_again, rolled_again, _) = walk_the_yard(26);
        assert_eq!((taken, rolled), (taken_again, rolled_again));
        assert!(
            beds.iter()
                .zip(&again)
                .all(|(a, b)| a.layer.heights() == b.layer.heights())
        );
    }
}
