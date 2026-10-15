//! Tram depots (the owner, 2026-10-11: one or more depots for maintenance and stabling;
//! `research/transit-and-vehicles.md` §10): a yard of parallel stabling tracks behind the
//! building line of a tram avenue near a line's end, a maintenance hall over its deepest tracks,
//! and a single track linking it to the line.
//!
//! The depots follow the research's numbers:
//! - a stabling place is a tram and 5 m;
//! - tracks are 4.6 m apart (an estimate: 4.5–5 m with walkways);
//! - curves are 20 m in the yard and 25 m on the link (Alstom Citadis: 18 m in depots);
//! - there are 2–4 depots for a network of 5–8 lines, near line ends, on industrial land first.
//!
//! The link leaves the line's near track on an S-curve, rises up the verge from the
//! carriageway to the walking level (the footway it crosses stays level, wheelchair first), and
//! becomes the yard's first track. The others branch from it on S-curves of their own.
//!
//! A depot is laid out in its own frame: `x` along the street, from where the link becomes the
//! first track, `y` into the block from the building line.

use glam::DVec2;

use super::blocks::{Block, BlockUse, edge_half_width, street_grid};
use super::geom;
use super::graph::RoadGraph;
use super::ground::Ramp;
use super::transit::{TRACK_OFFSET, TramLine};
use super::{LAND, Lane, Profile, Site};

/// A stabling place's length along its track, metres: a tram (33 m) and 5 m.
pub const PLACE: f64 = 38.0;
/// The yard's tracks' spacing, metres.
pub const SPACING: f64 = 4.6;
/// How far the yard's first track lies behind the building line, metres.
const FIRST: f64 = 5.0;
/// The tightest curve in the yard and on the link, metres.
const YARD_RADIUS: f64 = 20.0;
const LINK_RADIUS: f64 = 25.0;
/// How far apart along the first track the next tracks branch off, metres.
const STAGGER: f64 = 4.0;
/// The first track that leaves the ladder: the nearer ones lie too close to the first track for
/// the ladder's two curves, and leave it on S-curves.
const LADDER_FROM: usize = 3;
/// The maintenance hall: its length over the tracks' ends, its height, and how many of the
/// deepest tracks it covers (its pit tracks).
pub const HALL: (f64, f64, usize) = (50.0, 9.0, 3);
/// The yard's margin round its tracks, metres.
const MARGIN: f64 = 4.0;
/// The yard sizes tried, the largest first: its tracks, and its length from where the link
/// becomes its first track, metres.
const SIZES: [(usize, f64); 12] = [
    (8, 190.0),
    (8, 160.0),
    (6, 190.0),
    (6, 160.0),
    (6, 130.0),
    (5, 130.0),
    (5, 110.0),
    (4, 130.0),
    (4, 110.0),
    (4, 90.0),
    (3, 110.0),
    (3, 90.0),
];
/// The fewest trams a depot holds.
const LEAST: usize = 6;
/// How far from a line's end a depot may stand, metres along it.
const REACH: f64 = 800.0;
/// The least distance between two depots, metres.
const APART: f64 = 1500.0;

/// A tram depot.
#[derive(Clone, Debug, PartialEq)]
pub struct TramDepot {
    /// The line it serves, an index into the plan's tram lines.
    pub line: usize,
    /// The block it stands in.
    pub block: u32,
    /// Its frame's origin on the building line, where its link becomes its first track.
    pub origin: DVec2,
    /// Its frame's `x`: along the street, the way its tracks run from the link.
    pub along: DVec2,
    /// Its frame's `y`: into the block.
    pub into: DVec2,
    /// The street's half width: the line's middle lies this far out from the building line.
    pub half: f64,
    /// The street's footway's and verge's widths, metres.
    pub footway: f64,
    /// Its verge's width, metres.
    pub verge: f64,
    /// Its stabling tracks.
    pub tracks: usize,
    /// Its tracks' far end, metres along `x`.
    pub length: f64,
}

/// The rise of an S-curve at `u` of its length, from 0 to 1 (smoothstep: no kink at its ends).
fn rise(u: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// An S-curve's length for a shift of `d` metres with no curve tighter than `radius`.
fn s_length(d: f64, radius: f64) -> f64 {
    (6.0 * d.abs() * radius).sqrt().ceil()
}

impl TramDepot {
    /// A point of its frame on the plan.
    pub fn at(&self, x: f64, y: f64) -> DVec2 {
        self.origin + self.along * x + self.into * y
    }

    /// Its link's shift: from the line's near track out on the street to its first track.
    fn link_shift(&self) -> f64 {
        FIRST + self.half - TRACK_OFFSET
    }

    /// Its link's length along the street, metres.
    pub fn link_length(&self) -> f64 {
        s_length(self.link_shift(), LINK_RADIUS)
    }

    /// Its link's middle line on the plan, every metre: from the line's near track to its
    /// first track at the frame's origin.
    pub fn link(&self) -> Vec<DVec2> {
        let l = self.link_length();
        let y0 = TRACK_OFFSET - self.half;
        let d = self.link_shift();
        (0..=l as usize)
            .map(|k| {
                let x = -l + k as f64;
                self.at(x, y0 + d * rise(k as f64 / l))
            })
            .collect()
    }

    /// How far up from the carriageway to the walking level the ground under `p` stands, 0 to
    /// 1: the link rises up the verge.
    pub fn level_at(&self, p: DVec2) -> f64 {
        let y = (p - self.origin).dot(self.into);
        let kerb = -(self.footway + self.verge);
        ((y - kerb) / self.verge).clamp(0.0, 1.0)
    }

    /// The ramp up the verge where the link crosses it, for the street's ground.
    pub fn ramp(&self) -> Ramp {
        let link = self.link();
        let kerb = -(self.footway + self.verge);
        let cross = |y: f64| {
            link.iter()
                .map(|&p| (p, (p - self.origin).dot(self.into)))
                .find(|&(_, py)| py >= y)
                .map_or(self.origin, |(p, _)| p)
        };
        let (a, b) = (cross(kerb), cross(-self.footway));
        let (xa, xb) = (
            (a - self.origin).dot(self.along),
            (b - self.origin).dot(self.along),
        );
        let middle = (xa + xb) * 0.5;
        Ramp::driveway(
            self.at(middle, kerb),
            self.along,
            self.into,
            (xb - xa).abs() * 0.5 + 2.5,
            self.verge,
        )
    }

    /// Track `k`'s distance into the block from the building line, metres.
    fn y(k: usize) -> f64 {
        FIRST + k as f64 * SPACING
    }

    /// Where track `k` (from [`LADDER_FROM`] on) leaves the ladder's diagonal, in the frame: the
    /// diagonal runs at 45° from the end of the ladder's first curve.
    fn on_ladder(k: usize) -> (f64, f64) {
        let (c, r) = (std::f64::consts::FRAC_1_SQRT_2, YARD_RADIUS);
        let (x1, y1) = (r * c, FIRST + r * (1.0 - c));
        let y = Self::y(k) - r * (1.0 - c);
        (x1 + (y - y1), y)
    }

    /// Where track `k`'s straight begins, metres along `x`: the first track's at the frame's
    /// origin, the next ones' past their S-curves off it, the deeper ones' past their curves off
    /// the ladder.
    fn straight(&self, k: usize) -> f64 {
        match k {
            0 => 0.0,
            _ if k < LADDER_FROM => {
                (k - 1) as f64 * STAGGER + s_length(k as f64 * SPACING, YARD_RADIUS)
            }
            _ => Self::on_ladder(k).0 + YARD_RADIUS * std::f64::consts::FRAC_1_SQRT_2,
        }
    }

    /// Its tracks' middle lines on the plan, every metre or so on their curves, each piece of
    /// track once: the first track; the next ones from their S-curves off it; the ladder (a
    /// curve off the first track to 45° and a diagonal); the deeper ones from their curves off
    /// the ladder back along the street; each to the yard's far end.
    pub fn tracks(&self) -> Vec<Vec<DVec2>> {
        let (c, r) = (std::f64::consts::FRAC_1_SQRT_2, YARD_RADIUS);
        // A circle's points by its rational parameter (exact on every machine, D-016): `s` the
        // tangent of half the angle turned.
        let turn = |s: f64| (2.0 * s / (1.0 + s * s), (1.0 - s * s) / (1.0 + s * s));
        let quarter = std::f64::consts::SQRT_2 - 1.0;
        let steps = (r * std::f64::consts::FRAC_PI_4).ceil() as usize;
        let mut out = vec![vec![self.at(0.0, FIRST), self.at(self.length, FIRST)]];
        for k in 1..self.tracks.min(LADDER_FROM) {
            let from = (k - 1) as f64 * STAGGER;
            let l = s_length(k as f64 * SPACING, YARD_RADIUS);
            let mut line: Vec<DVec2> = (0..=l as usize)
                .map(|j| {
                    self.at(
                        from + j as f64,
                        FIRST + k as f64 * SPACING * rise(j as f64 / l),
                    )
                })
                .collect();
            line.push(self.at(self.length, Self::y(k)));
            out.push(line);
        }
        if self.tracks > LADDER_FROM {
            // The ladder: a left curve to 45°, then the diagonal to the last track's branch.
            let mut ladder: Vec<DVec2> = (0..=steps)
                .map(|j| {
                    let (sin, cos) = turn(quarter * j as f64 / steps as f64);
                    self.at(r * sin, FIRST + r * (1.0 - cos))
                })
                .collect();
            let (x, y) = Self::on_ladder(self.tracks - 1);
            ladder.push(self.at(x, y));
            out.push(ladder);
            for k in LADDER_FROM..self.tracks {
                // A right curve from 45° back along the street, its centre to the diagonal's
                // right.
                let (qx, qy) = Self::on_ladder(k);
                let (cx, cy) = (qx + r * c, qy - r * c);
                let mut line: Vec<DVec2> = (0..=steps)
                    .map(|j| {
                        let (sin, cos) = turn(quarter * (steps - j) as f64 / steps as f64);
                        self.at(cx - r * sin, cy + r * cos)
                    })
                    .collect();
                line.push(self.at(self.length, Self::y(k)));
                out.push(line);
            }
        }
        out
    }

    /// Its stabling places: each tram's middle and the way its track runs, the far ends first.
    pub fn places(&self) -> Vec<(DVec2, DVec2)> {
        let mut out = Vec::new();
        for k in 0..self.tracks {
            let start = self.straight(k);
            let mut end = self.length - 2.0;
            while end - PLACE >= start {
                out.push((
                    self.at(end - PLACE * 0.5, FIRST + k as f64 * SPACING),
                    self.along,
                ));
                end -= PLACE;
            }
        }
        out
    }

    /// The maintenance hall's outline over its pit tracks' far ends, counter-clockwise.
    pub fn hall(&self) -> Vec<DVec2> {
        let (long, _, pits) = HALL;
        let last = (self.tracks - 1) as f64 * SPACING + FIRST;
        let first = (self.tracks - pits.min(self.tracks)) as f64 * SPACING + FIRST;
        ccw(vec![
            self.at(self.length - long, first - 2.4),
            self.at(self.length + 2.0, first - 2.4),
            self.at(self.length + 2.0, last + 2.4),
            self.at(self.length - long, last + 2.4),
        ])
    }

    /// Where the link crosses the building line, metres along `x`.
    fn entry(&self) -> f64 {
        let l = self.link_length();
        let y0 = TRACK_OFFSET - self.half;
        let d = self.link_shift();
        (0..=l as usize)
            .map(|k| (-l + k as f64, y0 + d * rise(k as f64 / l)))
            .find(|&(_, y)| y >= 0.0)
            .map_or(0.0, |(x, _)| x)
    }

    /// The yard's outline, counter-clockwise: behind the building line, from where the link
    /// comes in to the tracks' far end and round them by [`MARGIN`].
    pub fn yard(&self) -> Vec<DVec2> {
        let x0 = self.entry() - MARGIN;
        let deep = FIRST + (self.tracks - 1) as f64 * SPACING + MARGIN;
        ccw(vec![
            self.at(x0, 0.0),
            self.at(self.length + MARGIN, 0.0),
            self.at(self.length + MARGIN, deep),
            self.at(x0, deep),
        ])
    }

    /// The yard's front along the building line either side of the link's opening, as
    /// segments: a fence or a hedge stands there.
    pub fn front(&self) -> Vec<(DVec2, DVec2)> {
        let x0 = self.entry() - MARGIN;
        let gap = self.entry();
        // The link's own width at the building line, crossing it at a slant.
        let open = (gap - 4.0, gap + 6.0);
        [(x0, open.0), (open.1, self.length + MARGIN)]
            .into_iter()
            .filter(|(a, b)| b - a > 1.0)
            .map(|(a, b)| (self.at(a, 0.0), self.at(b, 0.0)))
            .collect()
    }

    /// Its stabling places' number.
    pub fn capacity(&self) -> usize {
        self.places().len()
    }

    /// Whether `p` lies in its yard, `margin` metres round it.
    pub fn on_yard(&self, p: DVec2, margin: f64) -> bool {
        let d = p - self.origin;
        let (x, y) = (d.dot(self.along), d.dot(self.into));
        let deep = FIRST + (self.tracks - 1) as f64 * SPACING + MARGIN;
        (self.entry() - MARGIN - margin..=self.length + MARGIN + margin).contains(&x)
            && (-margin..=deep + margin).contains(&y)
    }

    /// Whether `polygon` (a lot) reaches into its yard or within 2 m of it.
    pub fn covers(&self, polygon: &[DVec2]) -> bool {
        let x0 = self.entry() - MARGIN - 2.0;
        let deep = FIRST + (self.tracks - 1) as f64 * SPACING + MARGIN + 2.0;
        let grown = [
            self.at(x0, -2.0),
            self.at(self.length + MARGIN + 2.0, -2.0),
            self.at(self.length + MARGIN + 2.0, deep),
            self.at(x0, deep),
        ];
        geom::overlaps(&grown, polygon)
    }

    /// Whether `p` lies on its link out on the street, `margin` metres either side of its rails.
    pub fn on_link(&self, p: DVec2, margin: f64) -> bool {
        self.link()
            .windows(2)
            .any(|w| geom::distance_to_segment(p, w[0], w[1]) < 1.5 + margin)
    }
}

/// An outline turned counter-clockwise.
fn ccw(mut outline: Vec<DVec2>) -> Vec<DVec2> {
    if geom::signed_area(&outline) < 0.0 {
        outline.reverse();
    }
    outline
}

/// What a depot must keep clear of.
pub struct Avoid<'a> {
    /// The tram lines' planned stops.
    pub stops: &'a [DVec2],
    /// The junctions.
    pub junctions: &'a [DVec2],
    /// The taxi stands' kerbs and the taxi depots' blocks and kerbs.
    pub stands: &'a [DVec2],
    /// Blocks taken.
    pub blocks: &'a [u32],
}

/// The network's depots: about one for three lines (2–4 for a network of 5–8), each near one
/// of a line's ends where a tram avenue's block behind its building line holds a yard of
/// [`LEAST`] trams or more, the largest yard first and industrial land first; [`APART`] apart.
pub fn depots(
    roads: &RoadGraph,
    site: &Site,
    blocks: &[Block],
    industrial: &[bool],
    lines: &[TramLine],
    avoid: &Avoid,
) -> Vec<TramDepot> {
    if lines.is_empty() {
        return Vec::new();
    }
    let grid = street_grid(roads, site.size);
    // Each block's edges' half widths: its outline lies on the streets' middles.
    let halves: Vec<Vec<f64>> = blocks
        .iter()
        .map(|b| {
            let n = b.polygon.len();
            (0..n)
                .map(|i| edge_half_width(&grid, roads, b.polygon[i], b.polygon[(i + 1) % n]))
                .collect()
        })
        .collect();
    let block_at = |p: DVec2| -> Option<usize> {
        blocks
            .iter()
            .position(|b| b.use_ == BlockUse::Built && geom::contains(&b.polygon, p))
    };
    // Whether `p` lies in block `b` behind its building lines.
    let behind = |b: usize, p: DVec2| {
        let poly = &blocks[b].polygon;
        let m = poly.len();
        geom::contains(poly, p)
            && site.water_at(p) == LAND
            && (0..m).all(|k| {
                geom::distance_to_segment(p, poly[k], poly[(k + 1) % m]) >= halves[b][k] + 0.5
            })
    };
    let mut candidates: Vec<(f64, TramDepot)> = Vec::new();
    for (l, line) in lines.iter().enumerate() {
        let total = geom::polyline_length(&line.points);
        if total < 600.0 {
            continue;
        }
        // Inward from each end, every 10 m, as far as the line's middle.
        for (end, dir) in [(0.0, 1.0), (total, -1.0)] {
            let mut gone = 60.0;
            while gone <= REACH.min(total * 0.5) {
                let s = end + dir * gone;
                gone += 10.0;
                let p = geom::point_at(&line.points, s);
                let t = geom::tangent_at(&line.points, s) * dir;
                let Some((_, e)) = grid.nearest(p, 20.0) else {
                    continue;
                };
                let profile = roads.edges[e as usize].profile;
                if profile != Profile::TramAvenue {
                    continue;
                }
                let strip = |lane: Lane| {
                    profile
                        .section()
                        .iter()
                        .find(|s| s.lane == lane)
                        .map_or(0.0, |s| s.width)
                };
                for side in [1.0, -1.0] {
                    let into = DVec2::new(-t.y, t.x) * side;
                    let mut depot = TramDepot {
                        line: l,
                        block: u32::MAX,
                        origin: DVec2::ZERO,
                        along: t,
                        into,
                        half: profile.width() * 0.5,
                        footway: strip(Lane::Footway),
                        verge: strip(Lane::Verge),
                        tracks: 0,
                        length: 0.0,
                    };
                    // The link leaves the line at `s`, the yard begins its length on.
                    let link = depot.link_length();
                    let at = geom::point_at(&line.points, s + dir * link);
                    depot.origin = at + into * depot.half;
                    // Straight street over the link and the yard's first stretch.
                    let straight = (0..=4).all(|k| {
                        let q = s + dir * (link + 20.0) * k as f64 / 4.0;
                        geom::tangent_at(&line.points, q.clamp(0.0, total)).dot(t * dir) > 0.9994
                    });
                    if !straight {
                        continue;
                    }
                    let path = depot.link();
                    let near = |q: DVec2, r: f64| {
                        path.windows(2)
                            .any(|w| geom::distance_to_segment(q, w[0], w[1]) < r)
                    };
                    if avoid.stops.iter().any(|&q| near(q, 60.0))
                        || avoid.junctions.iter().any(|&q| near(q, 25.0))
                        || avoid.stands.iter().any(|&q| near(q, 30.0))
                    {
                        continue;
                    }
                    let Some(b) = block_at(depot.at(10.0, FIRST)) else {
                        continue;
                    };
                    if avoid.blocks.contains(&(b as u32)) {
                        continue;
                    }
                    // The largest yard that fits behind the building line.
                    let fitted = SIZES.iter().find_map(|&(tracks, length)| {
                        let d = TramDepot {
                            block: b as u32,
                            tracks,
                            length,
                            ..depot.clone()
                        };
                        // Its front stands on the building line: probed just behind it.
                        let (x0, x1) = (d.entry() - MARGIN, d.length + MARGIN);
                        let deep = FIRST + (d.tracks - 1) as f64 * SPACING + MARGIN;
                        let fits = [x0, (x0 + x1) * 0.5, x1].iter().all(|&x| {
                            [0.6, deep * 0.5, deep]
                                .iter()
                                .all(|&y| behind(b, d.at(x, y)))
                        }) && d.capacity() >= LEAST;
                        fits.then_some(d)
                    });
                    if let Some(d) = fitted {
                        let bonus = if industrial.get(b).copied().unwrap_or(false) {
                            1.5
                        } else {
                            1.0
                        };
                        let score = d.capacity() as f64 * bonus - (gone - 10.0) / 100.0;
                        candidates.push((score, d));
                    }
                }
            }
        }
    }
    // The best first, spread apart, one for each three lines (one at least, four at most).
    candidates.sort_by(|a, b| b.0.total_cmp(&a.0));
    let wanted = lines.len().div_ceil(3).clamp(1, 4);
    let mut out: Vec<TramDepot> = Vec::new();
    for (_, d) in candidates {
        if out.len() >= wanted {
            break;
        }
        let c = geom::centroid(&d.yard());
        if out
            .iter()
            .all(|o| o.block != d.block && geom::centroid(&o.yard()).distance(c) >= APART)
        {
            out.push(d);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depot() -> TramDepot {
        TramDepot {
            line: 0,
            block: 0,
            origin: DVec2::new(100.0, 100.0),
            along: DVec2::X,
            into: DVec2::Y,
            half: 12.0,
            footway: 3.5,
            verge: 2.5,
            tracks: 6,
            length: 160.0,
        }
    }

    #[test]
    fn a_depot_s_tracks_curve_no_tighter_than_a_tram_turns_and_hold_its_places() {
        let d = depot();
        // Every curve's radius from three points a metre apart.
        let tightest = |line: &[DVec2]| {
            line.windows(3)
                .filter(|w| (w[1] - w[0]).length() < 1.5 && (w[2] - w[1]).length() < 1.5)
                .map(|w| {
                    let (a, b, c) = (
                        w[0].distance(w[1]),
                        w[1].distance(w[2]),
                        w[0].distance(w[2]),
                    );
                    let area = geom::cross(w[1] - w[0], w[2] - w[0]).abs() * 0.5;
                    if area < 1e-9 {
                        f64::MAX
                    } else {
                        a * b * c / (4.0 * area)
                    }
                })
                .fold(f64::MAX, f64::min)
        };
        let link = d.link();
        assert!(
            tightest(&link) >= LINK_RADIUS * 0.95,
            "link {:.1}",
            tightest(&link)
        );
        assert!(link[0].distance(DVec2::new(link[0].x, 100.0 - 12.0 + TRACK_OFFSET)) < 1e-9);
        assert!(link[link.len() - 1].distance(d.at(0.0, FIRST)) < 1e-9);
        for (k, t) in d.tracks().iter().enumerate() {
            assert!(
                tightest(t) >= YARD_RADIUS * 0.95,
                "track {k}: {:.1}",
                tightest(t)
            );
        }
        // Each place on a straight, inside the yard, none over another.
        let places = d.places();
        assert!(places.len() >= 12, "{} places", places.len());
        for (i, &(p, _)) in places.iter().enumerate() {
            assert!(d.on_yard(p, 0.0));
            for &(q, _) in &places[i + 1..] {
                assert!(
                    (p - q).dot(d.into).abs() > 1.0 || (p - q).dot(d.along).abs() >= PLACE - 1e-9
                );
            }
        }
        // The verge's ramp from the kerb up to the footway.
        assert!((d.level_at(d.at(-20.0, -6.0)) - 0.0).abs() < 1e-9);
        assert!((d.level_at(d.at(-20.0, -3.5)) - 1.0).abs() < 1e-9);
        // The hall over the deepest tracks' far ends.
        let hall = d.hall();
        assert!(geom::contains(
            &hall,
            d.at(d.length - 10.0, FIRST + 5.0 * SPACING)
        ));
        assert!(!geom::contains(&hall, d.at(d.length - 10.0, FIRST)));
    }
}
