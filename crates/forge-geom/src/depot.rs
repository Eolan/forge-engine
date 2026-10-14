//! A taxi depot (D-061): a split-level parking deck for the city's automated taxis, and the ways
//! its cabs drive in and out.
//!
//! The owner asked whether the cabs could drive in at all (the first deck had no ramps, a column
//! in its way in and 2.1 m under its lintel) and how car parks keep the flows in and out apart.
//! The layout follows the usual practice (`docs/demos/city-plan.md`, "Depots"):
//! - **Split levels:** the front half's floors at 0, 3 and 6 m, the back half's 1.5 m higher;
//!   short ramps (1.5 m over 10 m) in the side strips join each half to the other's next floors.
//! - **One way round:** the front aisle runs east, the back aisle west; on each side an up lane and
//!   a down lane, so no cab meets another head-on on a ramp.
//! - **In and out apart:** the way in at the front's middle passes every floor's bays on the way
//!   up; the way out at the front's west corner, at the foot of the last down lane.
//! - **People:** a stair and a lift in the front's east corner with a door onto the pavement, for
//!   the staff who keep the chargers and the cabs (the fleet stops its cabs on a floor they walk).
//! - **Clear ways:** columns 7.5 m apart only between bays, at the strips' edges and in the
//!   facades; 2.7 m under each slab, 2.3 m under the way in's fascia.
//!
//! The deck's frame: `x` along the street, its middle at 0; `y` up from the pavement; `+z` out
//! of its front (on `z = 0`) towards the street, its inside at `z < 0`.

use glam::Vec3;

use crate::modules::{ALL, FRAME, GLASS, METAL, NEG_Y, POS_Y, STONE, WOOD, cuboid, quad};
use crate::procedural::TriMesh;

/// Along the street, metres.
pub const WIDTH: f32 = 48.0;
/// From the front to the back, metres.
pub const DEPTH: f32 = 32.0;
/// A level, floor to floor, metres.
pub const STOREY: f32 = 3.0;
/// How much higher the back half's floors stand than the front's, metres.
pub const HALF: f32 = 1.5;
/// Parking floors in each half.
pub const LEVELS: u32 = 3;
/// A slab's thickness, metres.
const SLAB: f32 = 0.3;
/// A bay's width, metres; the bays run from `-BAYS_HALF` to `BAYS_HALF` across.
pub const BAY: f32 = 2.5;
/// Half the width the bays take, metres: 15 a row.
pub const BAYS_HALF: f32 = 18.75;
/// The rows' middles, metres in from the front: two in the front half, two in the back.
pub const ROWS: [f32; 4] = [2.5, 13.5, 18.5, 29.5];
/// The aisles' middles, metres in from the front: the front half's, then the back half's.
pub const AISLES: [f32; 2] = [8.0, 24.0];
/// Where the halves meet, metres in from the front.
pub const SPLIT: f32 = 16.0;
/// Where the ramps start and end, metres in from the front.
pub const RAMPS: (f32, f32) = (11.0, 21.0);
/// The side strips, either side, from the bays to the facade (`|x|`), metres.
pub const STRIP: (f32, f32) = (18.95, 23.6);
/// The up lane's and the down lane's middles in each strip (`|x|`), metres.
pub const LANES: (f32, f32) = (20.1, 22.4);
/// Half the way in's width at the front's middle, metres.
pub const ENTRANCE_HALF: f32 = 3.55;
/// The way out's middle at the front's west corner (`x`), metres.
pub const EXIT_X: f32 = -21.3;
/// The staff's core in the front's east corner: `x` from, to; in from the front from, to.
pub const CORE: (f32, f32, f32, f32) = (19.0, 23.6, 0.2, 4.8);
/// The deck's top: the back half's roof, metres.
pub const TOP: f32 = LEVELS as f32 * STOREY + HALF;

/// The height of half-level `h`: the front half's floors at even `h`, the back half's at odd.
pub fn floor(h: u32) -> f32 {
    (h / 2) as f32 * STOREY + if !h.is_multiple_of(2) { HALF } else { 0.0 }
}

/// A bay of the deck.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bay {
    /// Its half-level ([`floor`]).
    pub h: u32,
    /// Its row ([`ROWS`]).
    pub row: usize,
    /// Its middle across.
    pub x: f32,
}

impl Bay {
    /// Its middle on its floor.
    pub fn at(&self) -> Vec3 {
        Vec3::new(self.x, floor(self.h), -ROWS[self.row])
    }

    /// The aisle it opens onto ([`AISLES`]).
    fn aisle(&self) -> f32 {
        AISLES[self.row / 2]
    }

    /// The way a cab in it faces: nose in, from its aisle.
    pub fn facing(&self) -> Vec3 {
        Vec3::new(
            0.0,
            0.0,
            if ROWS[self.row] < self.aisle() {
                1.0
            } else {
                -1.0
            },
        )
    }
}

/// The deck's bays, floor after floor from the ground, row after row, west to east: 15 a row,
/// four rows a level of both halves, less the three before the way in.
pub fn bays() -> Vec<Bay> {
    let mut out = Vec::new();
    for h in 0..2 * LEVELS {
        let rows = if h.is_multiple_of(2) { [0, 1] } else { [2, 3] };
        for row in rows {
            for b in 0..15 {
                let x = -BAYS_HALF + BAY * (b as f32 + 0.5);
                if h == 0 && row == 0 && x.abs() < ENTRANCE_HALF + 0.2 {
                    continue;
                }
                out.push(Bay { h, row, x });
            }
        }
    }
    out
}

/// The way from the street into `bay`: from the lane before the way in (`lane` in front of the
/// deck's front), up the driveway, round the floors the one way they run, to the bay, nose in.
pub fn way_in(bay: &Bay, lane: f32, road: f32) -> Vec<Vec3> {
    let mut pts = vec![Vec3::new(0.0, road, lane), Vec3::new(0.0, 0.0, 0.0)];
    pts.push(Vec3::new(0.0, 0.0, -AISLES[0]));
    let (mut h, mut x) = (0_u32, 0.0_f32);
    loop {
        let front = h.is_multiple_of(2);
        if h == bay.h && (front && bay.x >= x - 0.01 || !front && bay.x <= x + 0.01) {
            pts.push(Vec3::new(bay.x, floor(h), -bay.aisle()));
            break;
        }
        // Round to the strip at the aisle's end, then the lane towards the bay's floor: up if
        // above, down if below, and round again (up from the front, down from the back) if it
        // was passed on this floor.
        let up = if h == bay.h { front } else { bay.h > h };
        let next = if up { h + 1 } else { h - 1 };
        ramp(&mut pts, h, next, up);
        h = next;
        x = if front { LANES.0 } else { -LANES.0 };
        x = if up { x } else { x.signum() * LANES.1 };
    }
    pts.push(bay.at());
    pts
}

/// The way from `bay` out to the street: backing out to its aisle, round and down the floors the
/// one way they run, out by the way out at the front's west corner to the lane before it.
pub fn way_out(bay: &Bay, lane: f32, road: f32) -> Vec<Vec3> {
    let mut pts = vec![bay.at(), Vec3::new(bay.x, floor(bay.h), -bay.aisle())];
    let mut h = bay.h;
    loop {
        if h == 1 {
            // The last down lane, then on along the strip to the way out.
            ramp(&mut pts, 1, 0, false);
            pts.push(Vec3::new(EXIT_X, 0.0, 0.0));
            pts.push(Vec3::new(EXIT_X, road, lane));
            break;
        }
        // From the ground's front half there is no lower floor: round by the back half.
        let up = h == 0;
        let next = if up { 1 } else { h - 1 };
        ramp(&mut pts, h, next, up);
        h = next;
    }
    pts
}

/// The points of a ramp from half-level `h` to `next` (`up` or down), from the end of `h`'s aisle
/// to the start of `next`'s: the east strip from the front half, the west from the back.
fn ramp(pts: &mut Vec<Vec3>, h: u32, next: u32, up: bool) {
    let lane = if up { LANES.0 } else { LANES.1 };
    let (y0, y1) = (floor(h), floor(next));
    if h.is_multiple_of(2) {
        pts.extend([
            Vec3::new(lane, y0, -AISLES[0]),
            Vec3::new(lane, y0, -RAMPS.0),
            Vec3::new(lane, y1, -RAMPS.1),
            Vec3::new(lane, y1, -AISLES[1]),
        ]);
    } else {
        pts.extend([
            Vec3::new(-lane, y0, -AISLES[1]),
            Vec3::new(-lane, y0, -RAMPS.1),
            Vec3::new(-lane, y1, -RAMPS.0),
            Vec3::new(-lane, y1, -AISLES[0]),
        ]);
    }
}

/// The ramps' lanes: (the strip's side, the lane's middle `|x|`, the half-levels it joins, its
/// low and high ends in from the front).
fn ramps() -> Vec<(f32, f32, u32, u32)> {
    let mut out = Vec::new();
    for f in 0..LEVELS {
        let h = 2 * f;
        // From the front half: east, up to the back half's floor; down to the one under.
        out.push((1.0, LANES.0, h, h + 1));
        if f > 0 {
            out.push((1.0, LANES.1, h, h - 1));
        }
        // From the back half: west, up to the front's next floor; down to its own.
        if f + 1 < LEVELS {
            out.push((-1.0, LANES.0, h + 1, h + 2));
        }
        out.push((-1.0, LANES.1, h + 1, h));
    }
    out
}

/// A face of a sloped slab, turned to face `out`.
fn face(m: &mut TriMesh, mut p: [Vec3; 4], out: Vec3, section: u8) {
    let n = (p[2] - p[0]).cross(p[3] - p[1]);
    if n.dot(out) < 0.0 {
        p.reverse();
    }
    quad(m, p, section);
}

/// A slab `t` thick from `x0` to `x1` whose top runs from `(z0, y0)` to `(z1, y1)`.
#[allow(clippy::too_many_arguments)]
fn sloped(m: &mut TriMesh, x0: f32, x1: f32, z0: f32, y0: f32, z1: f32, y1: f32, t: f32) {
    let v = Vec3::new;
    let top = [v(x0, y0, z0), v(x1, y0, z0), v(x1, y1, z1), v(x0, y1, z1)];
    let bottom = top.map(|p| p - Vec3::Y * t);
    let along = Vec3::new(0.0, y1 - y0, z1 - z0).normalize();
    let up = Vec3::X.cross(along).normalize()
        * if Vec3::X.cross(along).y < 0.0 {
            -1.0
        } else {
            1.0
        };
    face(m, top, up, STONE);
    face(m, bottom, -up, STONE);
    face(m, [top[0], top[3], bottom[3], bottom[0]], -Vec3::X, STONE);
    face(m, [top[1], top[2], bottom[2], bottom[1]], Vec3::X, STONE);
    face(m, [top[0], top[1], bottom[1], bottom[0]], -along, STONE);
    face(m, [top[3], top[2], bottom[2], bottom[3]], along, STONE);
}

/// A slab whose top is at `y` over `x` from `x0` to `x1` and in from the front from `z0` to `z1`.
fn slab(m: &mut TriMesh, x: (f32, f32), z: (f32, f32), y: f32) {
    cuboid(
        m,
        Vec3::new(x.0, y - SLAB, -z.1),
        Vec3::new(x.1, y, -z.0),
        STONE,
        ALL,
    );
}

/// The deck (see the module's notes), its parts in the kit's sections: concrete [`STONE`],
/// timber fins [`WOOD`], railings and posts [`METAL`], solar panels and the staff's door's glass
/// [`GLASS`], its frame [`FRAME`].
pub fn taxi_depot() -> TriMesh {
    let mut m = TriMesh::default();
    let v = Vec3::new;
    let (hx, d) = (WIDTH * 0.5, DEPTH);
    let (s0, s1) = STRIP;
    let mid = (-s0, s0);
    let full = (-hx, hx);
    // The front half's ground floor; the back half's first floor on a plinth.
    cuboid(&mut m, v(-hx, 0.0, -SPLIT), v(hx, 0.04, 0.0), STONE, POS_Y);
    cuboid(
        &mut m,
        v(-s0, 0.0, -RAMPS.1),
        v(s0, HALF, -SPLIT),
        STONE,
        ALL & !NEG_Y,
    );
    cuboid(
        &mut m,
        v(-hx, 0.0, -d),
        v(hx, HALF, -RAMPS.1),
        STONE,
        ALL & !NEG_Y,
    );
    // The slabs: the front half's over the front strips and the middle, up to the split; the
    // back half's from the split over the middle, and the whole width behind the ramps. The
    // strips between the ramps' ends are the ramps'.
    for f in 1..=LEVELS {
        let y = f as f32 * STOREY;
        slab(&mut m, full, (0.0, RAMPS.0), y);
        slab(&mut m, mid, (RAMPS.0, SPLIT), y);
        let yb = y + HALF;
        slab(&mut m, mid, (SPLIT, RAMPS.1), yb);
        slab(&mut m, full, (RAMPS.1, d), yb);
    }
    // The ramps, and an upstand along each one's inner side and between its two lanes.
    let wall = 0.15;
    for (side, lane, from, to) in ramps() {
        let half_lane = (s1 - s0 - wall) * 0.25;
        let (x0, x1) = (side * (lane - half_lane), side * (lane + half_lane));
        let (x0, x1) = (x0.min(x1), x0.max(x1));
        let (y0, y1) = (floor(from), floor(to));
        // In from the front: the front half's end at the ramps' near end.
        let (zf, zb) = (-RAMPS.0, -RAMPS.1);
        let (yf, yb) = if from.is_multiple_of(2) {
            (y0, y1)
        } else {
            (y1, y0)
        };
        sloped(&mut m, x0, x1, zf, yf, zb, yb, 0.25);
        // Its inner side's upstand, a metre over it.
        let xi = if side > 0.0 { x0 - wall } else { x1 };
        if lane == LANES.0 {
            sloped(&mut m, xi, xi + wall, zf, yf + 1.0, zb, yb + 1.0, 1.0);
        }
        // Between the lanes.
        if lane == LANES.1 {
            let xw = if side > 0.0 { x0 - wall } else { x1 };
            sloped(&mut m, xw, xw + wall, zf, yf + 1.0, zb, yb + 1.0, 1.0);
        }
    }
    // The upstands round the voids: along the strips beside the slabs' edges, and where the back
    // half's floors stand over the front's.
    for f in 0..=LEVELS {
        for side in [-1.0_f32, 1.0] {
            let x = side * s0;
            let (xa, xb) = (x.min(x - side * wall), x.max(x - side * wall));
            if f > 0 {
                let y = f as f32 * STOREY;
                cuboid(
                    &mut m,
                    v(xa, y, -SPLIT),
                    v(xb, y + 1.0, -RAMPS.0),
                    STONE,
                    ALL & !NEG_Y,
                );
            }
            let yb = f as f32 * STOREY + HALF;
            cuboid(
                &mut m,
                v(xa, yb, -RAMPS.1),
                v(xb, yb + 1.0, -SPLIT),
                STONE,
                ALL & !NEG_Y,
            );
        }
        let yb = f as f32 * STOREY + HALF;
        cuboid(
            &mut m,
            v(-s0, yb, -SPLIT - wall),
            v(s0, yb + 1.0, -SPLIT),
            STONE,
            ALL & !NEG_Y,
        );
    }
    // The columns: 7.5 m apart across, between the bays, at the strips' edges and in the
    // facades; at the front, the split and the back.
    let xs = [
        -hx + 0.2,
        -18.75,
        -11.25,
        -3.75,
        3.75,
        11.25,
        18.75,
        hx - 0.2,
    ];
    for x in xs {
        for (z, top) in [
            (-0.2, LEVELS as f32 * STOREY),
            (-SPLIT, TOP),
            (-d + 0.2, TOP),
        ] {
            cuboid(
                &mut m,
                v(x - 0.2, 0.0, z - 0.2),
                v(x + 0.2, top, z + 0.2),
                STONE,
                ALL & !NEG_Y,
            );
        }
    }
    // The fins: every half metre round the floors, 6 cm thick, 25 cm deep, standing outside
    // the slabs' edges; none across the ways in and out or before the staff's core.
    let fin = |m: &mut TriMesh, lo: Vec3, hi: Vec3| cuboid(m, lo, hi, WOOD, ALL & !NEG_Y & !POS_Y);
    for f in 0..LEVELS {
        let (y0, y1) = (f as f32 * STOREY, (f + 1) as f32 * STOREY - SLAB);
        let n = (WIDTH / 0.5) as u32;
        for i in 0..=n {
            let x = -hx + WIDTH * i as f32 / n as f32;
            let open = f == 0 && (x.abs() < ENTRANCE_HALF + 0.2 || (x - EXIT_X).abs() < 2.5);
            if !open && !(CORE.0 - 0.2..=CORE.1 + 0.2).contains(&x) {
                fin(&mut m, v(x - 0.03, y0, 0.02), v(x + 0.03, y1, 0.27));
            }
            let (yb0, yb1) = (y0 + HALF, y1 + HALF);
            fin(
                &mut m,
                v(x - 0.03, yb0, -d - 0.27),
                v(x + 0.03, yb1, -d - 0.02),
            );
        }
    }
    for side in [-1.0_f32, 1.0] {
        let n = (d / 0.5) as u32;
        for i in 0..=n {
            let z = -d * i as f32 / n as f32;
            if side > 0.0 && -z < CORE.3 + 0.2 {
                continue;
            }
            let x = side * (hx + 0.145);
            let top = if -z < SPLIT {
                LEVELS as f32 * STOREY
            } else {
                TOP
            } - SLAB;
            let base = if -z < SPLIT { 0.0 } else { HALF };
            fin(
                &mut m,
                v(x - 0.125, base, z - 0.03),
                v(x + 0.125, top, z + 0.03),
            );
        }
    }
    // A fascia over the way in, 2.3 m clear under it.
    cuboid(
        &mut m,
        v(-ENTRANCE_HALF - 0.2, 2.3, 0.0),
        v(ENTRANCE_HALF + 0.2, STOREY - SLAB, 0.3),
        STONE,
        ALL,
    );
    // The staff's core: walls round a stair and a lift, a glazed door onto the pavement, its
    // threshold at the walking level.
    let (cx0, cx1, cz0, cz1) = CORE;
    let ctop = TOP + 1.2;
    for (lo, hi) in [
        (v(cx0, 0.0, -cz1), v(cx1, ctop, -cz1 + 0.2)),
        (v(cx0, 0.0, -cz1), v(cx0 + 0.2, ctop, -cz0)),
        (v(cx1 - 0.2, 0.0, -cz1), v(cx1, ctop, -cz0)),
        (v(cx0, 2.4, -cz0 - 0.2), v(cx1, ctop, -cz0)),
        (v(cx0, 0.0, -cz0 - 0.2), v(cx0 + 1.4, 2.4, -cz0)),
        (v(cx0 + 2.6, 0.0, -cz0 - 0.2), v(cx1, 2.4, -cz0)),
    ] {
        cuboid(&mut m, lo, hi, STONE, ALL & !NEG_Y);
    }
    cuboid(
        &mut m,
        v(cx0, ctop, -cz1),
        v(cx1, ctop + 0.2, -cz0),
        STONE,
        ALL,
    );
    cuboid(
        &mut m,
        v(cx0 + 1.4, 0.0, -cz0 - 0.15),
        v(cx0 + 2.6, 2.4, -cz0 - 0.05),
        FRAME,
        ALL,
    );
    quad(
        &mut m,
        [
            v(cx0 + 1.5, 0.05, -cz0 - 0.04),
            v(cx0 + 2.5, 0.05, -cz0 - 0.04),
            v(cx0 + 2.5, 2.3, -cz0 - 0.04),
            v(cx0 + 1.5, 2.3, -cz0 - 0.04),
        ],
        GLASS,
    );
    // The roofs: a parapet round each half's, solar panels over them on posts 2.4 m up.
    for (z0, z1, y) in [(0.0, SPLIT, LEVELS as f32 * STOREY), (SPLIT, d, TOP)] {
        for (lo, hi) in [
            (v(-hx, y, -z0 - 0.2), v(hx, y + 1.1, -z0)),
            (v(-hx, y, -z1), v(hx, y + 1.1, -z1 + 0.2)),
            (v(-hx, y, -z1), v(-hx + 0.2, y + 1.1, -z0)),
            (v(hx - 0.2, y, -z1), v(hx, y + 1.1, -z0)),
        ] {
            cuboid(&mut m, lo, hi, STONE, ALL & !NEG_Y);
        }
        let rows = ((z1 - z0 - 2.0) / 5.0).floor().max(1.0) as u32;
        for r in 0..rows {
            let za = -z0 - 1.0 - r as f32 * 5.0;
            let zb = za - 4.4;
            let x1 = if z0 == 0.0 { cx0 - 0.5 } else { hx - 1.0 };
            cuboid(
                &mut m,
                v(-hx + 1.0, y + 2.4, zb),
                v(x1, y + 2.45, za),
                GLASS,
                ALL,
            );
            for x in [-hx + 1.5, 0.0, x1 - 0.5] {
                let z = (za + zb) * 0.5;
                cuboid(
                    &mut m,
                    v(x - 0.06, y, z - 0.06),
                    v(x + 0.06, y + 2.4, z + 0.06),
                    METAL,
                    ALL & !NEG_Y,
                );
            }
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deck_holds_177_cabs_in_bays_clear_of_its_columns() {
        let bays = bays();
        assert_eq!(bays.len(), 177);
        let columns = [-23.8, -18.75, -11.25, -3.75, 3.75, 11.25, 18.75, 23.8];
        for b in &bays {
            // A 2.5 m bay holds a 1.8 m cab: a column stands only on a bay's edge, at the split.
            for c in columns {
                assert!((b.x - c).abs() >= BAY * 0.5 - 0.01, "a column in bay {b:?}");
            }
            assert!(b.x.abs() < STRIP.0, "{b:?} in a strip");
        }
    }

    #[test]
    fn every_bay_is_reached_one_way_round_and_left_by_the_way_out() {
        for b in bays() {
            let way = way_in(&b, 7.5, -0.14);
            assert_eq!(*way.last().unwrap(), b.at());
            // Floor by floor, never more than a half-level per ramp.
            for w in way.windows(2) {
                let rise = (w[1].y - w[0].y).abs();
                let run = Vec3::new(w[1].x - w[0].x, 0.0, w[1].z - w[0].z).length();
                assert!(rise <= HALF + 0.01, "{b:?}: a step of {rise} m");
                assert!(
                    rise <= run * 0.16,
                    "{b:?}: a slope of {rise} m over {run} m"
                );
            }
            let out = way_out(&b, 7.5, -0.14);
            assert_eq!(out[0], b.at());
            assert!((out.last().unwrap().x - EXIT_X).abs() < 0.01);
            // Every aisle crossed the one way it runs: the front east, the back west.
            for way in [&way, &out] {
                for w in way.windows(2) {
                    let flat = (w[1].y - w[0].y).abs() < 0.01 && (w[1].z - w[0].z).abs() < 0.01;
                    if flat && (w[0].z + AISLES[0]).abs() < 0.01 && w[0].y < TOP {
                        assert!(w[1].x >= w[0].x - 0.01, "{b:?}: west along the front aisle");
                    }
                    if flat && (w[0].z + AISLES[1]).abs() < 0.01 {
                        assert!(w[1].x <= w[0].x + 0.01, "{b:?}: east along the back aisle");
                    }
                }
            }
        }
    }
}
