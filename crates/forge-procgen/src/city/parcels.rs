//! Lots that fit the houses (`research/parcels-and-rooms.md`, D-062's first map): a block's
//! street sides cut into strips of one depth (the block inset as [`geom::inset`] does, mitred
//! at the corners, near enough the straight skeleton's), each strip into lots by cuts square
//! to its street at widths a type draws, the corner lots taking the corners, a strip's end too
//! short for a lot merged into the last one. What a cadastre does, and what the city's lots
//! ([`super::blocks::subdivide`], split across the longer side) do not: party walls square to
//! the street, no slivers, no wedge for no reason (the owner, 2026-10-15). The first map's
//! village takes it first; the city, block by block.

use forge_core::SplitMix64;
use glam::DVec2;

use super::geom;

/// How a block's street sides are cut into lots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParcelType {
    /// The lots' depth from the street, metres: the strip's.
    pub depth: f64,
    /// The lots' widths along the street, metres, drawn between these two.
    pub width: (f64, f64),
    /// How far behind the street line the house stands, metres: none in a core, a front
    /// garden in a lotissement.
    pub setback: f64,
    /// What the house leaves to each side of its lot, metres: none between party walls, a
    /// passage round a free-standing house.
    pub sides: f64,
    /// What the house leaves at the back of its lot at least, metres: a yard, a garden.
    pub yard: f64,
}

impl ParcelType {
    /// A village house's lot: 5 to 8 m wide on a 14 m strip, a yard behind.
    pub const VILLAGE: Self = Self {
        depth: 14.0,
        width: (5.0, 8.0),
        setback: 0.0,
        sides: 0.0,
        yard: 1.5,
    };
    /// A perimeter block's building: 12 to 25 m wide on an 18 m strip.
    pub const BLOCK: Self = Self {
        depth: 18.0,
        width: (12.0, 25.0),
        setback: 0.0,
        sides: 0.0,
        yard: 0.0,
    };
    /// An old core's lot (`research/cadastre-villages-and-towns.md` §2): 5 to 9 m wide on a
    /// 12 m strip, the house the whole lot, a court only where the rooms leave one.
    pub const CORE: Self = Self {
        depth: 12.0,
        width: (5.0, 9.0),
        setback: 0.0,
        sides: 0.0,
        yard: 0.0,
    };
    /// A faubourg's lanière: 6 to 12 m wide on a 30 m strip, the house on the street line, a
    /// garden of 12 m at least behind.
    pub const FAUBOURG: Self = Self {
        depth: 30.0,
        width: (6.0, 12.0),
        setback: 0.0,
        sides: 0.0,
        yard: 12.0,
    };
    /// A lotissement's lot: 15 to 25 m wide on a 35 m strip (500 to 900 m²), the house free on
    /// four sides, 5 m back from the street, 3 m off the sides, a garden of 8 m behind.
    pub const LOTISSEMENT: Self = Self {
        depth: 35.0,
        width: (15.0, 25.0),
        setback: 5.0,
        sides: 3.0,
        yard: 8.0,
    };
}

/// A block's lots and its court. The block is a polygon of positive area; `street[i]` says
/// whether its edge `i` (from vertex `i` to `i + 1`) is a street side, the others party sides
/// (a lot there reaches the court). `None` where the block is too thin for a strip of the
/// type's depth all round: the caller falls back on another cut.
pub fn strips(
    block: &[DVec2],
    street: &[bool],
    ty: &ParcelType,
    seed: u64,
) -> Option<(Vec<Vec<DVec2>>, Vec<DVec2>)> {
    let mut rng = SplitMix64::new(seed);
    strips_with(block, street, ty, |_| rng.range_f64(ty.width.0, ty.width.1))
        .map(|(lots, court, _)| (lots, court))
}

/// A block cut into strips: its lots (each its street side first), its court, and each lot's
/// key, the cut it was planned as ([`strips_with`]).
pub type Cut = (Vec<Vec<DVec2>>, Vec<DVec2>, Vec<usize>);

/// [`strips`] with each lot's width asked of `width` by a key counting the cuts along the
/// block's street sides: the lots cut to fit their houses, the owner's way round (the rooms
/// first). The keys come back with the lots: a lot that took a strip's remainder or swallowed
/// a corner sliver keeps the key of its planned part, so the keys skip where lots merged.
pub fn strips_with(
    block: &[DVec2],
    street: &[bool],
    ty: &ParcelType,
    mut width: impl FnMut(usize) -> f64,
) -> Option<Cut> {
    let n = block.len();
    if n < 3 || street.len() != n || geom::signed_area(block) <= 0.0 {
        return None;
    }
    // The court: the block inset by the depth. A block too thin for it takes no strips; one
    // whose inset loses a corner (a short edge gone, the straight skeleton's edge event) still
    // takes them, each strip bounded by its depth line and its corners' bisectors.
    let court = geom::inset_by_collapsing(block, &vec![ty.depth; n])
        .filter(|c| c.len() >= 3 && geom::signed_area(c) > 1.0)?;
    // Each edge's inward normal, and each corner's bisector (its two edges' normals' sum, which
    // also halves a reflex corner's inner angle).
    let normal = |i: usize| {
        let t = (block[(i + 1) % n] - block[i]).normalize_or_zero();
        DVec2::new(-t.y, t.x)
    };
    let bisector = |i: usize| {
        let d = normal((i + n - 1) % n) + normal(i);
        if d.length() < 1e-6 {
            normal(i)
        } else {
            d.normalize()
        }
    };
    let mut lots: Vec<Vec<DVec2>> = Vec::new();
    let mut keys: Vec<usize> = Vec::new();
    for i in 0..n {
        if !street[i] {
            continue;
        }
        let j = (i + 1) % n;
        let (a, b) = (block[i], block[j]);
        let length = a.distance(b);
        if length < 0.5 {
            continue;
        }
        let t = (b - a) / length;
        let inward = DVec2::new(-t.y, t.x);
        // The strip: the block within the depth of its street side, between the bisectors at
        // the side's two corners, so the first and last lots take the corners.
        let mut strip = geom::clip_half_plane(block, a + inward * ty.depth, -t);
        for (corner, d, along) in [(a, bisector(i), t), (b, bisector(j), -t)] {
            let keep = if d.perp_dot(along) >= 0.0 { d } else { -d };
            strip = geom::clip_half_plane(&strip, corner, keep);
        }
        // The clips repeat a corner they pass through.
        let strip = geom::clean(&strip, 1e-6);
        if strip.len() < 3 || geom::signed_area(&strip) <= 1.0 {
            continue;
        }
        // The cuts along the street: a width drawn each time; the last lot takes what is left
        // when that is short of the least width, so no sliver is cut.
        let mut cuts = vec![0.0];
        let mut s = 0.0;
        let mut k = lots.len();
        loop {
            let w = width(k).max(0.5);
            if s + w + ty.width.0 > length {
                break;
            }
            s += w;
            cuts.push(s);
            k += 1;
        }
        cuts.push(length);
        // The keys the lots were cut for, one a cut.
        let first = lots.len();
        let mut ks: Vec<usize> = (first..first + cuts.len() - 1).collect();
        let cut = |cuts: &[f64], ks: &[usize]| -> Vec<(usize, Vec<DVec2>)> {
            let mut out = Vec::new();
            for k in 0..cuts.len() - 1 {
                let (s0, s1) = (cuts[k], cuts[k + 1]);
                let mut lot = strip.clone();
                if k > 0 {
                    // After the cut at s0: the right of the inward line through it.
                    lot = geom::clip_half_plane(&lot, a + t * s0, -inward);
                }
                if k + 2 < cuts.len() {
                    // Before the cut at s1: its left.
                    lot = geom::clip_half_plane(&lot, a + t * s1, inward);
                }
                let lot = geom::clean(&lot, 1e-6);
                if lot.len() >= 3 && geom::signed_area(&lot).abs() > 1.0 {
                    out.push((ks[k], from_the_street(lot, a, b)));
                }
            }
            out
        };
        let mut here = cut(&cuts, &ks);
        // A corner lot cut to a sliver by an acute corner's bisector joins its neighbour: the
        // cut between them goes (a lot under half the least lot's area is a sliver), and the
        // merged lot keeps the key of the part that was planned (the first's, or the last but
        // one's: the last took the remainder).
        let sliver = ty.width.0 * ty.depth * 0.5;
        for _ in 0..2 {
            let m = here.len();
            if m < 2 || cuts.len() < 3 {
                break;
            }
            if geom::signed_area(&here[0].1).abs() < sliver {
                cuts.remove(1);
                ks.remove(1);
            } else if geom::signed_area(&here[m - 1].1).abs() < sliver {
                cuts.remove(cuts.len() - 2);
                ks.pop();
            } else {
                break;
            }
            here = cut(&cuts, &ks);
        }
        for (key, lot) in here {
            keys.push(key);
            lots.push(lot);
        }
    }
    Some((lots, court, keys))
}

/// `lot` turned to start with its street side (its edge along `a b`), so its first two vertices
/// are its frontage and the next edge and the last its party sides; the clip leaves the vertices
/// starting wherever they fell.
fn from_the_street(mut lot: Vec<DVec2>, a: DVec2, b: DVec2) -> Vec<DVec2> {
    let n = lot.len();
    let on = |p: DVec2| geom::distance_to_segment(p, a, b) < 1e-6;
    if let Some(k) = (0..n).find(|&k| on(lot[k]) && on(lot[(k + 1) % n])) {
        lot.rotate_left(k);
    }
    lot
}

/// A lot's width along its street side (the edge from its first to its second vertex, as
/// [`strips`] lays it), metres.
pub fn frontage(lot: &[DVec2]) -> f64 {
    if lot.len() < 2 {
        return 0.0;
    }
    lot[0].distance(lot[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle(w: f64, h: f64) -> Vec<DVec2> {
        vec![
            DVec2::ZERO,
            DVec2::new(w, 0.0),
            DVec2::new(w, h),
            DVec2::new(0.0, h),
        ]
    }

    /// The lots and the court cover the block, every lot has its street side on the block's
    /// outline, and no lot is narrower than the type's least width.
    fn check(block: &[DVec2], street: &[bool], ty: &ParcelType, seed: u64) -> usize {
        let (lots, court) = strips(block, street, ty, seed).expect("a block that takes strips");
        let block_area = geom::signed_area(block).abs();
        let covered: f64 = lots.iter().map(|l| geom::signed_area(l).abs()).sum::<f64>()
            + geom::signed_area(&court).abs();
        let party: f64 = (0..block.len())
            .filter(|&i| !street[i])
            .map(|i| block[i].distance(block[(i + 1) % block.len()]) * ty.depth)
            .sum();
        // The party sides' strips are left out; the rest covers the block.
        assert!(
            (block_area - covered - party).abs() < block_area * 0.02 + party * 0.6,
            "block {block_area:.0}, covered {covered:.0}, party strips about {party:.0}"
        );
        let n = block.len();
        for lot in &lots {
            let (a, b) = (lot[0], lot[1]);
            let on = |p: DVec2| {
                (0..n).any(|i| geom::distance_to_segment(p, block[i], block[(i + 1) % n]) < 1e-6)
            };
            assert!(
                on(a) && on(b),
                "a lot's street side off the outline: {a} {b}"
            );
            assert!(
                frontage(lot) >= ty.width.0 - 1e-6,
                "a lot {:.1} m wide under {:.1}",
                frontage(lot),
                ty.width.0
            );
            // The last lot may take a short remainder, a corner lot its sliver neighbour.
            assert!(frontage(lot) <= 2.0 * ty.width.1 + ty.width.0 + 1e-6);
            for k in 0..lot.len() {
                let (p, q, r) = (
                    lot[(k + lot.len() - 1) % lot.len()],
                    lot[k],
                    lot[(k + 1) % lot.len()],
                );
                let (u, v) = ((p - q).normalize(), (r - q).normalize());
                let angle = u.dot(v).clamp(-1.0, 1.0).acos().to_degrees();
                assert!(angle > 30.0, "a lot with a {angle:.0}° corner");
            }
        }
        lots.len()
    }

    #[test]
    fn a_rectangle_cuts_into_strips_of_village_lots() {
        let block = rectangle(100.0, 60.0);
        let n = check(&block, &[true; 4], &ParcelType::VILLAGE, 7);
        // About 320 m of street at 6.5 m a lot.
        assert!((40..=60).contains(&n), "{n} lots");
    }

    #[test]
    fn a_trapezoid_keeps_its_party_walls_square_to_the_street() {
        let block = vec![
            DVec2::ZERO,
            DVec2::new(120.0, 0.0),
            DVec2::new(100.0, 70.0),
            DVec2::new(10.0, 70.0),
        ];
        let n = check(&block, &[true, true, true, true], &ParcelType::BLOCK, 11);
        assert!((14..=30).contains(&n), "{n} lots");
        let (lots, _) = strips(&block, &[true; 4], &ParcelType::BLOCK, 11).expect("strips");
        // Each lot's two party sides run square to its street, but the corner lots', which
        // take the corners' bisectors (an acute corner gives a flatiron, as in a town).
        let corner = |lot: &Vec<DVec2>| {
            lot.iter()
                .any(|&p| block.iter().any(|&q| p.distance(q) < 1e-6))
        };
        let mut inner = 0;
        for lot in lots.iter().filter(|l| !corner(l)) {
            let t = (lot[1] - lot[0]).normalize();
            for side in [
                (lot[2] - lot[1]).normalize(),
                (lot[0] - lot[lot.len() - 1]).normalize(),
            ] {
                assert!(
                    t.dot(side).abs() < 0.02,
                    "a party side {:.1}° off the square",
                    t.dot(side).acos().to_degrees() - 90.0
                );
            }
            inner += 1;
        }
        assert!(inner >= 6, "{inner} inner lots");
    }

    /// With `PARCELS_PNG=path`, the trapezoid's lots drawn there for a look.
    #[test]
    fn the_trapezoid_is_drawn_on_request() {
        let Ok(path) = std::env::var("PARCELS_PNG") else {
            return;
        };
        let block = vec![
            DVec2::ZERO,
            DVec2::new(120.0, 0.0),
            DVec2::new(100.0, 70.0),
            DVec2::new(10.0, 70.0),
        ];
        let (lots, court) = strips(&block, &[true; 4], &ParcelType::VILLAGE, 11).expect("strips");
        super::super::map::write_parcels(&block, &lots, &court, std::path::Path::new(&path), 0.1)
            .expect("the drawing");
    }

    #[test]
    fn a_party_side_is_left_to_the_court() {
        let block = rectangle(80.0, 50.0);
        let (lots, _) =
            strips(&block, &[true, false, true, false], &ParcelType::VILLAGE, 3).expect("strips");
        assert!(
            lots.iter()
                .all(|l| l[0].y.abs() < 1e-9 || (l[0].y - 50.0).abs() < 1e-9)
        );
    }

    #[test]
    fn a_block_too_thin_for_the_depth_takes_no_strips() {
        assert!(strips(&rectangle(100.0, 20.0), &[true; 4], &ParcelType::VILLAGE, 1).is_none());
    }
}
