//! The yard (#185, `--lab yard`): the start of Phase 3's `materials-yard`. The dogs walk their
//! lanes as on the course, over a bed of damp sand and then a bed of fresh snow and back, and
//! their footfalls (#167's foot-down events) press their paws into the beds: each bed a layer
//! of soft material over the floor (`forge_physics::deform`), drawn as a ground the skin pass
//! raises by the layer's heights (`forge_render::HeightField`), so the prints and their rims
//! take the light and cast shadows.
//!
//! The dogs stand on the floor, under the soft material: their paws go through to it as the
//! layer's least allows, and the beds lie over it, their edges thinning to nothing.

use forge_anim::Footfall;
use forge_geom::TriMesh;
use forge_physics::deform::{Layer, Pad, Soft};
use glam::{DVec2, Vec2, Vec3};

/// A bed: its layer and its material's name (the lab's rows').
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Bed {
    pub layer: Layer,
    pub name: &'static str,
}

/// The beds' materials (the lab's rows), sand and snow.
pub(super) const SOFT: [&str; 2] = ["lab-sand", "lab-snow"];
/// Metres between two points of a bed's layer: a dog's pad (3 cm by 4 cm) over a dozen.
pub(super) const CELL: f32 = 0.01;
/// The beds: material, name, and their corners (x, z) from the low to the high. Across both
/// lanes (x ±0.7) between where the dogs turn about (z −1.6 and 2.8): sand first, then snow,
/// the floor showing between them.
const BEDS: [(Soft, &str, [f64; 2], [f64; 2]); 2] = [
    (Soft::SAND, SOFT[0], [-1.2, -1.05], [1.2, 0.75]),
    (Soft::SNOW, SOFT[1], [-1.2, 0.85], [1.2, 2.65]),
];
/// The yard's sun: low (26°) from beyond the beds and the sand's side, across the dogs' lanes,
/// so the prints' walls and rims stand out in light and shadow.
pub(crate) const SUN: Vec3 = Vec3::new(-0.5, 0.45, -0.75);
/// How far under the floor's top a bed's ground lies at its border, metres: there it is hidden
/// by the floor, which a layer at its least (4 mm) stands over.
pub(super) const UNDER: f32 = 0.001;

/// The beds, untouched.
pub(super) fn beds() -> Vec<Bed> {
    BEDS.iter()
        .map(|&(soft, name, low, high)| {
            let size = [
                ((high[0] - low[0]) / f64::from(CELL)).round() as u32 + 1,
                ((high[1] - low[1]) / f64::from(CELL)).round() as u32 + 1,
            ];
            Bed {
                layer: Layer::new(soft, DVec2::from_array(low), CELL, size),
                name,
            }
        })
        .collect()
}

/// A bed's ground as drawn before the layer raises it: a flat grid of its layer's points, from
/// its first at the origin, two triangles a cell. Its mover places it at [`place`].
pub(super) fn ground(bed: &Bed) -> TriMesh {
    let [nx, nz] = bed.layer.size();
    let mut mesh = TriMesh::default();
    for z in 0..nz {
        for x in 0..nx {
            mesh.positions.push([x as f32 * CELL, 0.0, z as f32 * CELL]);
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
        });
        fall.material = bed.name;
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

/// The highest a bed's ground can rise over its layer's depth (a rim) or sink under it, metres:
/// how far its clusters' bounds reach.
pub(super) const REACH: f32 = 0.12;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::creatures::{Feet, Ground, build};
    use forge_physics::{BodyDesc, Shape, World, WorldDesc};
    use forge_sim::TICK;
    use glam::DVec3;

    /// The yard's dogs over `seconds`, pressing their footfalls into the beds: the beds after,
    /// and how many footfalls each took.
    fn walk_the_yard(seconds: u32) -> (Vec<Bed>, [usize; 2]) {
        let mut world = World::new(&WorldDesc::default());
        let floor = Shape::cuboid(Vec3::new(20.0, 0.5, 20.0), 0.05, 0.0).expect("a floor");
        world
            .add_body(&BodyDesc::fixed(&floor, DVec3::new(0.0, -0.5, 0.0)))
            .expect("the floor");
        let field = build(&mut world, 0, 0, Ground::Yard).expect("the yard");
        let mut beds = beds();
        let mut feet = Feet::default();
        let mut taken = [0; 2];
        for tick in 0..seconds * 60 {
            let time = f64::from(tick) * f64::from(TICK);
            field.herd.drive(&mut world, time, false);
            world.step(TICK, 1).expect("a step");
            let came = field.herd.feel(&world, u64::from(tick), &mut feet);
            press(&mut beds, feet.last_mut(came));
            for fall in feet.last_mut(came) {
                if let Some(k) = SOFT.iter().position(|&s| s == fall.material) {
                    taken[k] += 1;
                }
            }
        }
        (beds, taken)
    }

    #[test]
    fn the_dogs_leave_prints_in_the_sand_and_the_snow_and_replay() {
        let (beds, taken) = walk_the_yard(26);
        // Up both beds and back: a score of footfalls on each or more.
        assert!(taken.iter().all(|&n| n >= 20), "{taken:?}");
        let [sand, snow] = [&beds[0].layer, &beds[1].layer];
        // Over the beds' inside, past their bevelled edges (6 cm).
        let inside = |l: &Layer| -> Vec<f32> {
            let [nx, nz] = l.size();
            let edge = 8;
            (edge..nz - edge)
                .flat_map(|z| (edge..nx - edge).map(move |x| (z * nx + x) as usize))
                .map(|k| l.heights()[k])
                .collect()
        };
        let lowest = |l: &Layer| inside(l).into_iter().fold(f32::MAX, f32::min);
        let highest = |l: &Layer| inside(l).into_iter().fold(f32::MIN, f32::max);
        // Sand: prints over a centimetre deep (1.2 cm), heaped round with a few millimetres (4).
        let depth = Soft::SAND.depth;
        assert!(lowest(sand) < depth - 0.01, "{}", lowest(sand));
        assert!(highest(sand) > depth + 0.002, "{}", highest(sand));
        // Snow: the paws went through to the floor, the least left under them.
        assert!(lowest(snow) < Soft::SNOW.least + 0.002, "{}", lowest(snow));
        // Between the lanes (x 0), untouched.
        for (l, soft) in [(sand, Soft::SAND), (snow, Soft::SNOW)] {
            let middle = l.origin() + DVec2::new(1.2, 0.9);
            assert_eq!(l.height_at(middle), soft.depth);
        }
        // The same walk presses the same bits.
        let (again, taken_again) = walk_the_yard(26);
        assert_eq!(taken, taken_again);
        assert!(
            beds.iter()
                .zip(&again)
                .all(|(a, b)| a.layer.heights() == b.layer.heights())
        );
    }
}
