//! The street ground as one map (D-061): every point of the streets is one surface at one level.
//!
//! - **Streets.** Each street is its centreline stroked to its width, and its carriageway to the
//!   carriageway's width. The union of the streets is the street ground; the blocks are what is
//!   left.
//! - **Junctions.** The arms meet at the node with flat ends. Where two driven arms leave a reflex
//!   gap, a mitre fills it. Where they make a corner, the kerb turns on a radius.
//! - **Crossings.** A crossing crosses each driven arm of a junction just past its corners, with a
//!   dropped kerb at either end.
//! - **One walking level.** Pavements, shared and pedestrian streets and paths all lie at
//!   [`WALK_LEVEL`]. A pedestrian way meeting an avenue carries its walking ground across the
//!   avenue's pavement: a continuous footway.
//!
//! Booleans on polygons by `i_overlay`, in its integer mode (exact and the same on every machine,
//! D-016). The map is built tile by tile from the streets near each tile.

use glam::DVec2;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::core::solver::Solver;
use i_overlay::float::overlay::{FloatOverlay, OverlayOptions};
use i_overlay::float::slice::FloatSlice;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{LineCap, LineJoin, StrokeStyle};

use super::geom;
use super::graph::RoadGraph;

/// The carriageway's level over the ground, metres.
pub const ROAD_LEVEL: f64 = 0.16;
/// The walking level over the ground, metres: a kerb of 14 cm over the carriageway.
pub const WALK_LEVEL: f64 = 0.30;
/// A dropped kerb's lip over the carriageway, metres: low enough for a wheelchair, high enough
/// for a cane to find.
pub const DROPPED: f64 = 0.01;
/// A river promenade's width from the water's edge, metres.
pub const PROMENADE: f64 = 13.0;
/// The radius a kerb turns on at a corner between two driven streets, metres: a town street's.
pub const KERB_RADIUS: f64 = 6.0;
/// A dropped kerb's depth into the pavement, metres: 13 cm down over it, 1 in 12 (#263).
pub const RAMP: f64 = 1.6;
/// A crossing's width along the street, metres.
pub const CROSSING: f64 = 4.0;
/// The gap kept between a corner's end and a crossing, metres.
const CROSSING_GAP: f64 = 1.0;
/// Corners sharper than this between two arms' directions get no rounded kerb, radians' cosine:
/// their kerb lines meet too far out (a fork).
const FORK: f64 = 0.985;
/// The arcs' segments, as their length over their radius.
const ARC: f64 = 0.25;
/// How far behind a road's building line a walking way's bollards stand, metres.
const BOLLARD_SET_BACK: f64 = 1.5;
/// The most between two bollards, metres: a wheelchair passes, a car does not.
const BOLLARD_GAP: f64 = 1.5;
/// The radius the inside of a gentle bend's building line turns on, metres.
const BEND_RADIUS: f64 = 40.0;
/// How far the junctions' pieces reach into the strips they join, metres.
const OVERLAP: f64 = 0.3;

/// The map's tiles' side, metres: the city demo's.
pub const TILE: f64 = 250.0;

/// A region of the plan: each shape an outer contour, then its holes.
pub type Shapes = Vec<Vec<Vec<DVec2>>>;

/// A slope between the carriageway and the walking level: a dropped kerb (the pavement's ramp
/// down to a crossing), or a raised table's ramp (the carriageway's up to it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    /// The middle of its foot.
    pub kerb: DVec2,
    /// Across it, unit.
    pub along: DVec2,
    /// Up it, unit.
    pub inward: DVec2,
    /// Half its width, metres.
    pub half: f64,
    /// Its length up the slope, metres.
    pub run: f64,
    /// Its flares' length along its foot each side, metres (0: none).
    pub flare: f64,
    /// Its foot's lip over the carriageway, metres.
    pub lip: f64,
}

impl Ramp {
    /// A dropped kerb: [`RAMP`] long, flared, a centimetre's lip.
    fn dropped(kerb: DVec2, along: DVec2, inward: DVec2, half: f64) -> Self {
        Self {
            kerb,
            along,
            inward,
            half,
            run: RAMP,
            flare: FLARE,
            lip: DROPPED,
        }
    }

    /// A driveway's ramp for vehicles, its foot at the kerb: up the verge, `run` metres long, to
    /// the walking level, flared at its sides; the footway beyond stays level.
    pub fn driveway(kerb: DVec2, along: DVec2, inward: DVec2, half: f64, run: f64) -> Self {
        Self {
            kerb,
            along,
            inward,
            half,
            run,
            flare: FLARE,
            lip: DROPPED,
        }
    }

    /// Its outline, a trapezoid: along its foot (from a little past it, what it stands on trims
    /// it) its width and its flares either side, narrowing to its width `run` up.
    fn outline(&self) -> Vec<DVec2> {
        let flare = if self.flare > 0.0 {
            self.flare + 0.5 * self.flare / self.run
        } else {
            0.0
        };
        let (a, f, n) = (
            self.along * self.half,
            self.along * (self.half + flare),
            self.inward,
        );
        let (near, far) = (self.kerb - n * 0.5, self.kerb + n * self.run);
        vec![near - f, near + f, far + a, far - a]
    }

    /// Whether `p` lies on it (its slopes from its foot up, the flares too).
    pub fn contains(&self, p: DVec2) -> bool {
        let d = p - self.kerb;
        let (s, t) = (d.dot(self.along), d.dot(self.inward));
        (0.0..=self.run).contains(&t) && s.abs() <= self.half + self.flare * (1.0 - t / self.run)
    }

    /// Its level at `p`: from the lip at its foot up to the walking level `run` up, and in each
    /// flare up to it along its foot too, so a dropped kerb's sides slope at 1 in 12 like its
    /// middle. Vertical sides stood at its edges, a trip for anyone and a wall for a wheelchair
    /// (the owner: "gentle slopes all around").
    pub fn level(&self, p: DVec2) -> f64 {
        let d = p - self.kerb;
        let (s, t) = (d.dot(self.along), d.dot(self.inward));
        let sideways = if self.flare > 0.0 {
            (s.abs() - self.half).max(0.0) / self.flare
        } else {
            0.0
        };
        let rise = (t / self.run + sideways).clamp(0.0, 1.0);
        ROAD_LEVEL + self.lip + (WALK_LEVEL - ROAD_LEVEL - self.lip) * rise
    }

    /// Where its middle meets its flares, along its foot: its pieces are planar between them.
    pub fn creases(&self) -> [(DVec2, DVec2); 2] {
        [-1.0, 1.0].map(|side| {
            (
                self.kerb + self.along * (self.half * side),
                self.along * side,
            )
        })
    }
}

/// A dropped kerb's flare along the kerb each side, metres: its 13 cm at 1 in 12.
pub const FLARE: f64 = 1.6;
/// A raised table's ramps' length, metres: 14 cm over 1.5 m, taken at a taxi's walking pace.
pub const TABLE_RAMP: f64 = 1.5;

/// A crossing over a driven street at a junction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Crossing {
    /// The street it crosses.
    pub edge: u32,
    /// The junction it serves.
    pub node: u32,
    /// Its middle, on the street's centreline.
    pub centre: DVec2,
    /// The street's direction there, unit.
    pub along: DVec2,
    /// The carriageway's width it spans, metres.
    pub span: f64,
    /// Whether it is a raised table at the walking level (a taxi road's, D-061), else at the
    /// carriageway's with a dropped kerb at each end (a tram avenue's: rails take no table).
    pub raised: bool,
}

impl Crossing {
    /// Its outline, a metre past either kerb (the carriageway trims it).
    fn outline(&self) -> Vec<DVec2> {
        let (a, n) = (
            self.along * (CROSSING * 0.5),
            DVec2::new(-self.along.y, self.along.x) * (self.span * 0.5 + 1.0),
        );
        vec![
            self.centre - a - n,
            self.centre + a - n,
            self.centre + a + n,
            self.centre - a + n,
        ]
    }

    /// Its painted stripes, half a metre wide a metre apart along the kerbs' direction (a
    /// zebra), as outlines the carriageway trims.
    fn stripes(&self) -> Vec<Vec<DVec2>> {
        let n = DVec2::new(-self.along.y, self.along.x);
        let a = self.along * (CROSSING * 0.5 - 0.25);
        let half = self.span * 0.5;
        let count = (self.span / 1.0).floor() as i32;
        (0..count)
            .map(|k| {
                let c = self.centre + n * (-half + 0.5 + f64::from(k));
                let w = n * 0.25;
                vec![c - a - w, c + a - w, c + a + w, c - a + w]
            })
            .collect()
    }
}

/// One tile of the map, its shapes clipped to it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroundTile {
    /// Its corner nearest the origin.
    pub min: DVec2,
    /// Its far corner.
    pub max: DVec2,
    /// The carriageways at [`ROAD_LEVEL`], the crossings left out.
    pub carriageway: Shapes,
    /// The crossings' paint, at [`ROAD_LEVEL`].
    pub stripes: Shapes,
    /// The crossings between their stripes, at [`ROAD_LEVEL`].
    pub crossings: Shapes,
    /// The raised tables' crossings between their stripes, at [`WALK_LEVEL`].
    pub raised: Shapes,
    /// The raised tables' paint, at [`WALK_LEVEL`].
    pub raised_stripes: Shapes,
    /// The walking ground at [`WALK_LEVEL`], the ramps left out.
    pub walk: Shapes,
    /// The dropped kerbs, each with its ramp's index.
    pub ramps: Vec<(u32, Shapes)>,
    /// The kerbs' segments, the carriageway on the left of each (from first to second point).
    pub kerbs: Vec<[DVec2; 2]>,
    /// The street ground's edge, the street on the left of each segment: where it meets the
    /// blocks, the water or the open ground.
    pub outline: Vec<[DVec2; 2]>,
}

/// The street ground (module docs).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GroundMap {
    /// A tile's side, metres.
    pub tile: f64,
    /// Tiles a row.
    pub side: usize,
    /// The tiles, row by row.
    pub tiles: Vec<GroundTile>,
    /// The crossings.
    pub crossings: Vec<Crossing>,
    /// The bollards across the walking ways' mouths onto the roads.
    pub bollards: Vec<DVec2>,
    /// The dropped kerbs, two a crossing.
    pub ramps: Vec<Ramp>,
    /// Each street's area, stroked to its width.
    streets: Vec<Piece>,
    /// Each street's and walk's centreline and width.
    lines: Vec<(Vec<DVec2>, f64)>,
    /// The rivers' water, as drawn: no ground over it.
    water: Vec<Piece>,
}

/// A polygon with its bounding box.
#[derive(Clone, Debug, Default, PartialEq)]
struct Piece {
    min: DVec2,
    max: DVec2,
    contours: Vec<Vec<[f64; 2]>>,
}

impl Piece {
    fn new(contours: Vec<Vec<[f64; 2]>>) -> Self {
        let (mut min, mut max) = (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN));
        for p in contours.iter().flatten() {
            min = min.min(DVec2::from(*p));
            max = max.max(DVec2::from(*p));
        }
        Self { min, max, contours }
    }

    /// A simple polygon turned counter-clockwise, as the strokes are: under the non-zero rule
    /// a clockwise one would cancel the streets it overlaps.
    fn from_points(points: &[DVec2]) -> Self {
        let mut contour: Vec<[f64; 2]> = points.iter().map(|p| p.to_array()).collect();
        if geom::signed_area(points) < 0.0 {
            contour.reverse();
        }
        Self::new(vec![contour])
    }

    fn meets(&self, min: DVec2, max: DVec2) -> bool {
        self.min.x <= max.x && self.max.x >= min.x && self.min.y <= max.y && self.max.y >= min.y
    }
}

/// i_overlay's shapes.
type Raw = Vec<Vec<Vec<[f64; 2]>>>;

/// The booleans' options: contours under a tenth of a square metre dropped (the slivers two
/// booleans leave along a shared edge, each rounding to its own grid).
fn options() -> OverlayOptions<f64, i32> {
    let mut o = OverlayOptions::default();
    o.min_output_area = 0.1;
    o
}

/// The union of `pieces` meeting the box, as raw shapes.
fn union(pieces: &[Piece], min: DVec2, max: DVec2) -> Raw {
    let contours: Vec<Vec<[f64; 2]>> = pieces
        .iter()
        .filter(|p| p.meets(min, max))
        .flat_map(|p| p.contours.iter().cloned())
        .collect();
    if contours.is_empty() {
        return Vec::new();
    }
    let none: Vec<Vec<[f64; 2]>> = Vec::new();
    FloatOverlay::with_subj_and_clip_custom(&contours, &none, options(), Solver::default())
        .overlay(OverlayRule::Subject, FillRule::NonZero)
}

fn boolean(a: &Raw, b: &Raw, rule: OverlayRule) -> Raw {
    if a.is_empty() {
        return if rule == OverlayRule::Union {
            b.clone()
        } else {
            Vec::new()
        };
    }
    if b.is_empty() {
        return match rule {
            OverlayRule::Intersect => Vec::new(),
            _ => a.clone(),
        };
    }
    FloatOverlay::with_subj_and_clip_custom(a, b, options(), Solver::default())
        .overlay(rule, FillRule::NonZero)
}

fn rect(min: DVec2, max: DVec2) -> Raw {
    vec![vec![vec![
        [min.x, min.y],
        [max.x, min.y],
        [max.x, max.y],
        [min.x, max.y],
    ]]]
}

fn to_shapes(raw: Raw) -> Shapes {
    raw.into_iter()
        .map(|shape| {
            shape
                .into_iter()
                .map(|c| c.into_iter().map(DVec2::from).collect())
                .collect()
        })
        .collect()
}

/// A street's centreline stroked to `width`: flat ends at its junctions, round at a dead end,
/// round where it bends (a mitre at a near reversal shot out as a spike, a walled wedge in the
/// water at a river's mouth, the owner).
fn stroke(points: &[DVec2], width: f64, round_a: bool, round_b: bool) -> Piece {
    let path: Vec<[f64; 2]> = points.iter().map(|p| p.to_array()).collect();
    let cap = |round| {
        if round {
            LineCap::Round(ARC)
        } else {
            LineCap::Butt
        }
    };
    let style = StrokeStyle::new(width)
        .line_join(LineJoin::Round(ARC))
        .start_cap(cap(round_a))
        .end_cap(cap(round_b));
    Piece::new(path.stroke(style, false).into_iter().flatten().collect())
}

/// An arm of a junction: an edge leaving the node.
#[derive(Clone, Copy, Debug)]
struct Arm {
    edge: u32,
    /// Its direction from the node, unit.
    dir: DVec2,
    /// Its angle, for the order round the node.
    angle: f64,
    /// Half its width and half its carriageway's, metres.
    half: f64,
    kerb: f64,
    /// Its length, metres.
    length: f64,
}

/// Where the lines `a + s·u` and `b + t·v` meet, as `(s, t)`.
fn meet(a: DVec2, u: DVec2, b: DVec2, v: DVec2) -> Option<(f64, f64)> {
    let d = geom::cross(u, v);
    if d.abs() < 1e-9 {
        return None;
    }
    let w = b - a;
    Some((geom::cross(w, v) / d, geom::cross(w, u) / d))
}

/// The left normal of `u`.
fn left(u: DVec2) -> DVec2 {
    DVec2::new(-u.y, u.x)
}
/// The piece filling the reflex gap between arms `i` and `j` (in turning order, the gap on `i`'s
/// left) for strips of half widths `hi` and `hj` ending flat at `node`: an arc round the node,
/// its radius going from one half width to the other, so the outside of a bend is round. A
/// mitre came to a point, a kink in the pavement where a road bent at a junction (the owner).
fn mitre(node: DVec2, i: DVec2, hi: f64, j: DVec2, hj: f64) -> Vec<DVec2> {
    let (a, b) = (left(i), -left(j));
    let sweep = forge_core::dmath::atan2(geom::cross(a, b), a.dot(b));
    let start = forge_core::dmath::atan2(a.y, a.x);
    let steps = ((sweep.abs() * hi.max(hj) / 1.0).ceil() as usize).max(1);
    // A little into both strips, so the union leaves no hairline along their ends.
    let (ia, jb) = (i * OVERLAP, j * OVERLAP);
    let mut out = vec![node + ia, node + a * hi + ia];
    for k in 0..=steps {
        let t = k as f64 / steps as f64;
        let (sn, cs) = forge_core::dmath::sin_cos(start + sweep * t);
        out.push(node + DVec2::new(cs, sn) * (hi + (hj - hi) * t));
    }
    out.extend([node + b * hj + jb, node + jb]);
    out
}

/// The wedge rounding the kerb between driven arms `i` and `j` (`i`'s left, `j`'s right), at
/// most `radius`, with how far along each arm it ends; `None` for a fork or a gap.
fn fillet(node: DVec2, i: &Arm, j: &Arm, radius: f64) -> Option<(Vec<DVec2>, f64, f64)> {
    let (u, v) = (i.dir, j.dir);
    // A corner, not a straight run or a reflex gap.
    if geom::cross(u, v) <= 0.0 || u.dot(v) > FORK {
        return None;
    }
    let (s, t) = meet(node + left(u) * i.kerb, u, node - left(v) * j.kerb, v)?;
    let x = node + left(u) * i.kerb + u * s;
    // The half angle between the kerb lines, seen from the corner.
    let cos = u.dot(v).clamp(-1.0, 1.0);
    let half_tan = ((1.0 - cos) / (1.0 + cos)).sqrt();
    // The tangent points at most four fifths along the shorter arm.
    let reach = |d: f64| d.max(0.0);
    let room = 0.8 * i.length.min(j.length) - reach(s).max(reach(t));
    if room <= 0.0 {
        return None;
    }
    let r = radius.min(room * half_tan);
    let tangent = r / half_tan;
    let (ti, tj) = (x + u * tangent, x + v * tangent);
    // From the corner along the bisector to the arc's centre: r / sin(half angle).
    let bisector = (u + v).normalize_or_zero();
    let sin_half = half_tan / (1.0 + half_tan * half_tan).sqrt();
    let centre = x + bisector * (r / sin_half);
    // Its sides a little into both strips: laid edge to edge on the kerb lines, the union left
    // hairlines along them, the old corner's lines on the asphalt (the owner).
    let (iu, iv) = (left(u) * OVERLAP, left(v) * OVERLAP);
    let mut wedge = vec![ti, ti - iu, x - iu + iv, tj + iv, tj];
    // The arc from tj back to ti round the centre, on the corner's side.
    let (a0, a1) = (tj - centre, ti - centre);
    let sweep = forge_core::dmath::atan2(geom::cross(a0, a1), a0.dot(a1));
    let steps = ((sweep.abs() * r / (ARC * r).max(0.5)).ceil() as usize).max(1);
    let start = forge_core::dmath::atan2(a0.y, a0.x);
    for k in 1..steps {
        let (sn, cs) = forge_core::dmath::sin_cos(start + sweep * k as f64 / steps as f64);
        wedge.push(centre + DVec2::new(cs, sn) * r);
    }
    Some((wedge, s + tangent, t + tangent))
}

/// The arms of every node, in turning order.
fn arms(graph: &RoadGraph) -> Vec<Vec<Arm>> {
    let mut arms: Vec<Vec<Arm>> = vec![Vec::new(); graph.nodes.len()];
    for (k, e) in graph.edges.iter().enumerate() {
        let n = e.points.len();
        if n < 2 {
            continue;
        }
        let length = e.length();
        for (node, from, to) in [
            (e.a, e.points[0], e.points[1]),
            (e.b, e.points[n - 1], e.points[n - 2]),
        ] {
            let dir = (to - from).normalize_or_zero();
            if dir == DVec2::ZERO {
                continue;
            }
            arms[node as usize].push(Arm {
                edge: k as u32,
                dir,
                angle: forge_core::dmath::atan2(dir.y, dir.x),
                half: e.profile.width() * 0.5,
                kerb: e.profile.carriageway() * 0.5,
                length,
            });
        }
    }
    for list in &mut arms {
        list.sort_by(|a, b| a.angle.total_cmp(&b.angle).then(a.edge.cmp(&b.edge)));
    }
    arms
}

impl GroundMap {
    /// The map of `graph`'s streets over a frame of side `size`, in tiles of side `tile`; the
    /// carriageways widened to clear the trams' swept paths along `trams`' lines, round the
    /// junctions' corners too; the taxi stands' `laybys` carriageway, and the depots'
    /// `driveways` ramps up the verge.
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        graph: &RoadGraph,
        size: f64,
        tile: f64,
        trams: &[Vec<DVec2>],
        walks: &[(Vec<DVec2>, f64)],
        rivers: &[(Vec<DVec2>, f64)],
        laybys: &[Vec<DVec2>],
        driveways: &[Ramp],
    ) -> Self {
        let arms = arms(graph);
        let driven_degree = |node: u32| arms[node as usize].iter().filter(|a| a.kerb > 0.0).count();
        let degree = |node: u32| arms[node as usize].len();
        // Each street's area and carriageway.
        let mut streets = Vec::with_capacity(graph.edges.len());
        let mut carriageways = Vec::new();
        for e in &graph.edges {
            streets.push(stroke(
                &e.points,
                e.profile.width(),
                degree(e.a) == 1,
                degree(e.b) == 1,
            ));
            if e.profile.driven() {
                carriageways.push(stroke(
                    &e.points,
                    e.profile.carriageway(),
                    driven_degree(e.a) == 1,
                    driven_degree(e.b) == 1,
                ));
            }
        }
        // The walks beside the streets (the rivers' promenades): walking ground, no carriageway.
        for (line, width) in walks.iter().filter(|w| w.0.len() >= 2) {
            streets.push(stroke(line, *width, false, false));
        }
        // Each street's and walk's line and width, for the blocks' forecourts.
        let lines: Vec<(Vec<DVec2>, f64)> = graph
            .edges
            .iter()
            .map(|e| (e.points.clone(), e.profile.width()))
            .chain(walks.iter().cloned())
            .collect();
        let water: Vec<Piece> = rivers
            .iter()
            .filter(|r| r.0.len() >= 2)
            .map(|(line, width)| stroke(line, *width, false, false))
            .collect();
        // The taxi stands' lay-bys, cut into the verge at the carriageway's level.
        for lay_by in laybys {
            carriageways.push(Piece::from_points(lay_by));
        }
        for line in trams.iter().filter(|l| l.len() >= 2) {
            carriageways.push(stroke(line, 2.0 * super::transit::SWEPT, false, false));
        }
        // At each node: the mitres in the reflex gaps, the kerbs' rounded corners, and a
        // crossing on each driven arm of a junction past its corners.
        let mut wedges = Vec::new();
        let mut crossings = Vec::new();
        let mut bollards = Vec::new();
        for (v, list) in arms.iter().enumerate() {
            let node = graph.nodes[v].pos;
            let m = list.len();
            if m < 2 {
                continue;
            }
            // The carriageway's reflex gaps between consecutive driven arms, the walking ways
            // between them left out: a pedestrian street's mouth between an avenue's two arms
            // left a sliver of pavement on the asphalt (the owner).
            let driven: Vec<&Arm> = list.iter().filter(|a| a.kerb > 0.0).collect();
            let md = driven.len();
            for k in 0..md {
                let (i, j) = (driven[k], driven[(k + 1) % md]);
                if md >= 2
                    && (geom::cross(i.dir, j.dir) < 0.0
                        || (geom::cross(i.dir, j.dir) == 0.0 && i.dir.dot(j.dir) < 0.0))
                {
                    wedges.push(Piece::from_points(&mitre(
                        node, i.dir, i.kerb, j.dir, j.kerb,
                    )));
                }
            }
            // How far along each arm its corners reach.
            let mut reach = vec![0.0_f64; m];
            for k in 0..m {
                let (i, j) = (&list[k], &list[(k + 1) % m]);
                // The turn from `i` to `j` is over half a turn: a reflex gap on `i`'s left.
                let gap = geom::cross(i.dir, j.dir) < 0.0
                    || (geom::cross(i.dir, j.dir) == 0.0 && i.dir.dot(j.dir) < 0.0);
                if gap {
                    streets.push(Piece::from_points(&mitre(
                        node, i.dir, i.half, j.dir, j.half,
                    )));
                    continue;
                }
                // The inside of a gentle bend (under 30°): its building line round too, on a long
                // arc. It turned at a point, a kink in the pavement's edge (the owner).
                if i.dir.dot(j.dir) < -(30f64.to_radians().cos()) {
                    let (wi, wj) = (Arm { kerb: i.half, ..*i }, Arm { kerb: j.half, ..*j });
                    if let Some((wedge, _, _)) = fillet(node, &wi, &wj, BEND_RADIUS) {
                        streets.push(Piece::from_points(&wedge));
                    }
                }
                if i.kerb > 0.0 && j.kerb > 0.0 {
                    if let Some((wedge, si, sj)) = fillet(node, i, j, KERB_RADIUS) {
                        wedges.push(Piece::from_points(&wedge));
                        reach[k] = reach[k].max(si);
                        let next = (k + 1) % m;
                        reach[next] = reach[next].max(sj);
                    }
                } else if i.kerb > 0.0 || j.kerb > 0.0 {
                    // A walking way's mouth beside a driven arm: the crossing keeps past its
                    // far edge.
                    let (d, w, k_d, side) = if i.kerb > 0.0 {
                        (i, j, k, 1.0)
                    } else {
                        (j, i, (k + 1) % m, -1.0)
                    };
                    let kerb_line = node + left(d.dir) * d.kerb * side;
                    let edge_line = node - left(w.dir) * w.half * side;
                    if let Some((s, _)) = meet(kerb_line, d.dir, edge_line, w.dir) {
                        reach[k_d] = reach[k_d].max(s);
                    }
                }
            }
            // Bollards across each walking way's mouth where it meets a driven road, just behind
            // the road's pavement, 1.5 m apart at most (a wheelchair passes between): no taxi
            // turns in where people walk (D-061, the owner).
            let road = list
                .iter()
                .filter(|a| a.kerb > 0.0)
                .map(|a| a.half)
                .fold(0.0_f64, f64::max);
            if road > 0.0 {
                for arm in list.iter().filter(|a| a.kerb <= 0.0) {
                    let s = road + BOLLARD_SET_BACK;
                    if arm.length < s + 4.0 {
                        continue;
                    }
                    let e = &graph.edges[arm.edge as usize];
                    let points: Vec<DVec2> = if e.a == v as u32 {
                        e.points.clone()
                    } else {
                        e.points.iter().rev().copied().collect()
                    };
                    let (at, t) = (geom::point_at(&points, s), geom::tangent_at(&points, s));
                    let n = left(t);
                    let span = 2.0 * arm.half - 1.0;
                    let count = (span / BOLLARD_GAP).ceil() as usize + 1;
                    for k in 0..count {
                        let off = -span * 0.5 + span * k as f64 / (count - 1).max(1) as f64;
                        bollards.push(at + n * off);
                    }
                }
            }
            if m < 3 {
                continue;
            }
            for (k, arm) in list.iter().enumerate() {
                if arm.kerb <= 0.0 {
                    continue;
                }
                let s = reach[k] + CROSSING_GAP + CROSSING * 0.5;
                if s + CROSSING * 0.5 > arm.length * 0.5 {
                    continue;
                }
                let e = &graph.edges[arm.edge as usize];
                let forward = e.a == v as u32;
                let points: Vec<DVec2> = if forward {
                    e.points.clone()
                } else {
                    e.points.iter().rev().copied().collect()
                };
                crossings.push(Crossing {
                    edge: arm.edge,
                    node: v as u32,
                    centre: geom::point_at(&points, s),
                    along: geom::tangent_at(&points, s),
                    span: e.profile.carriageway(),
                    raised: e.profile == super::Profile::Avenue,
                });
            }
        }
        // A dropped kerb at each end of a crossing at the carriageway's level; a raised table's
        // two ramps up from the carriageway either side of one at the walking level (they stand
        // on the carriageway, the dropped kerbs on the pavement).
        let mut ramps: Vec<Ramp> = Vec::new();
        let mut on_road: Vec<bool> = Vec::new();
        for c in &crossings {
            let n = left(c.along);
            if c.raised {
                for side in [1.0, -1.0] {
                    ramps.push(Ramp {
                        kerb: c.centre + c.along * (side * (CROSSING * 0.5 + TABLE_RAMP)),
                        along: n,
                        inward: c.along * -side,
                        half: c.span * 0.5 + 1.0,
                        run: TABLE_RAMP,
                        flare: 0.0,
                        lip: 0.0,
                    });
                    on_road.push(true);
                }
            } else {
                for side in [1.0, -1.0] {
                    ramps.push(Ramp::dropped(
                        c.centre + n * (c.span * 0.5 * side),
                        c.along,
                        n * side,
                        CROSSING * 0.5,
                    ));
                    on_road.push(false);
                }
            }
        }
        let crossing_pieces: Vec<Piece> = crossings
            .iter()
            .map(|c| Piece::from_points(&c.outline()))
            .collect();
        let raised_pieces: Vec<Piece> = crossings
            .iter()
            .filter(|c| c.raised)
            .map(|c| Piece::from_points(&c.outline()))
            .collect();
        let stripe_pieces: Vec<Piece> = crossings
            .iter()
            .flat_map(|c| c.stripes())
            .map(|s| Piece::from_points(&s))
            .collect();
        // The depots' driveways up the verges, standing on the walking ground as the dropped
        // kerbs do.
        for r in driveways {
            ramps.push(*r);
            on_road.push(false);
        }
        let ramp_pieces: Vec<Piece> = ramps
            .iter()
            .map(|r| Piece::from_points(&r.outline()))
            .collect();
        let side = (size / tile).ceil().max(1.0) as usize;
        let mut tiles = Vec::with_capacity(side * side);
        let margin = DVec2::splat(1.0);
        for y in 0..side {
            for x in 0..side {
                let min = DVec2::new(x as f64, y as f64) * tile;
                let max = min + DVec2::splat(tile);
                let (lo, hi) = (min - margin, max + margin);
                let square = rect(min, max);
                let street = union(&streets, lo, hi);
                if street.is_empty() {
                    tiles.push(GroundTile {
                        min,
                        max,
                        ..GroundTile::default()
                    });
                    continue;
                }
                let mut road = union(&carriageways, lo, hi);
                let corners = union(&wedges, lo, hi);
                road = boolean(&road, &corners, OverlayRule::Union);
                road = boolean(&road, &street, OverlayRule::Intersect);
                let crossing = boolean(
                    &union(&crossing_pieces, lo, hi),
                    &road,
                    OverlayRule::Intersect,
                );
                let raised = boolean(
                    &union(&raised_pieces, lo, hi),
                    &crossing,
                    OverlayRule::Intersect,
                );
                let stripes = boolean(
                    &union(&stripe_pieces, lo, hi),
                    &crossing,
                    OverlayRule::Intersect,
                );
                let walk_all = boolean(&street, &road, OverlayRule::Difference);
                let open_road = boolean(&road, &crossing, OverlayRule::Difference);
                // The ramps, each kind on what it stands on, cut out of it; each ramp's own
                // piece of them.
                let ramps_on = |kind: bool, ground: &Raw| {
                    let pieces: Vec<Piece> = (0..ramps.len())
                        .filter(|&r| on_road[r] == kind)
                        .map(|r| ramp_pieces[r].clone())
                        .collect();
                    boolean(&union(&pieces, lo, hi), ground, OverlayRule::Intersect)
                };
                let on_walk = ramps_on(false, &walk_all);
                let on_road_ramps = ramps_on(true, &open_road);
                let walk = boolean(&walk_all, &on_walk, OverlayRule::Difference);
                let carriageway = boolean(&open_road, &on_road_ramps, OverlayRule::Difference);
                let mut ramp_shapes = Vec::new();
                for (kind, on) in [(false, &on_walk), (true, &on_road_ramps)] {
                    let on_tile = boolean(on, &square, OverlayRule::Intersect);
                    for r in (0..ramps.len()).filter(|&r| on_road[r] == kind) {
                        if !ramp_pieces[r].meets(lo, hi) {
                            continue;
                        }
                        let outline = vec![ramp_pieces[r].contours.clone()];
                        let piece = boolean(&outline, &on_tile, OverlayRule::Intersect);
                        if !piece.is_empty() {
                            ramp_shapes.push((r as u32, to_shapes(piece)));
                        }
                    }
                }
                let kerbs = segments(&road, min, max);
                let outline = segments(&street, min, max);
                let clip = |r: &Raw| to_shapes(boolean(r, &square, OverlayRule::Intersect));
                let plain = boolean(&crossing, &raised, OverlayRule::Difference);
                tiles.push(GroundTile {
                    min,
                    max,
                    carriageway: clip(&carriageway),
                    stripes: clip(&boolean(&stripes, &raised, OverlayRule::Difference)),
                    crossings: clip(&boolean(&plain, &stripes, OverlayRule::Difference)),
                    raised_stripes: clip(&boolean(&stripes, &raised, OverlayRule::Intersect)),
                    raised: clip(&boolean(&raised, &stripes, OverlayRule::Difference)),
                    walk: clip(&walk),
                    ramps: ramp_shapes,
                    kerbs,
                    outline,
                });
            }
        }
        Self {
            tile,
            side,
            tiles,
            crossings,
            ramps,
            bollards,
            streets,
            water,
            lines,
        }
    }

    /// The tile holding `p`.
    fn tile_at(&self, p: DVec2) -> Option<&GroundTile> {
        if self.tiles.is_empty() || p.x < 0.0 || p.y < 0.0 {
            return None;
        }
        let (x, y) = (
            (p.x / self.tile).floor() as usize,
            (p.y / self.tile).floor() as usize,
        );
        (x < self.side && y < self.side).then(|| &self.tiles[y * self.side + x])
    }

    /// Whether `p` lies on a carriageway (a crossing's included).
    pub fn on_carriageway(&self, p: DVec2) -> bool {
        self.tile_at(p).is_some_and(|t| {
            inside(&t.carriageway, p)
                || inside(&t.crossings, p)
                || inside(&t.stripes, p)
                || inside(&t.raised, p)
                || inside(&t.raised_stripes, p)
        })
    }

    /// The ground's level at `p`: a carriageway's, a dropped kerb's or the walking level (the
    /// blocks lie at it too).
    pub fn level_at(&self, p: DVec2) -> f64 {
        let Some(t) = self.tile_at(p) else {
            return WALK_LEVEL;
        };
        for (r, shapes) in &t.ramps {
            if inside(shapes, p) {
                return self.ramps[*r as usize].level(p);
            }
        }
        if inside(&t.carriageway, p) || inside(&t.crossings, p) || inside(&t.stripes, p) {
            ROAD_LEVEL
        } else {
            WALK_LEVEL
        }
    }

    /// The rivers' water inside the box from `min` to `max`, as drawn: their strokes' union,
    /// which cannot fold where a river turns sharply (strips between two offset lines did, a
    /// light wedge in the water at a mouth, the owner).
    pub fn water_in(&self, min: DVec2, max: DVec2) -> Shapes {
        let water = union(&self.water, min, max);
        to_shapes(boolean(&water, &rect(min, max), OverlayRule::Intersect))
    }

    /// Whether any street, walk or river's water meets the box round `polygon`: if none, it lies
    /// off them whole.
    pub fn touches(&self, polygon: &[DVec2]) -> bool {
        let piece = Piece::from_points(polygon);
        self.streets
            .iter()
            .chain(&self.water)
            .any(|p| p.meets(piece.min, piece.max))
    }

    /// What of `polygon` lies off the streets and the rivers' water (a block's ground).
    pub fn off_streets(&self, polygon: &[DVec2]) -> Shapes {
        let piece = Piece::from_points(polygon);
        let street = union(&self.streets, piece.min, piece.max);
        let water = union(&self.water, piece.min, piece.max);
        let off = boolean(&vec![piece.contours], &street, OverlayRule::Difference);
        to_shapes(boolean(&off, &water, OverlayRule::Difference))
    }

    /// What of `polygon` lies off the streets and the rivers' water, split in two: inside
    /// `inner` and outside it. One level drawn as two surfaces that meet edge to edge (a
    /// square's lawn or water mirror cut out of its paving), where one laid over the other would
    /// fight it.
    pub fn off_streets_split(&self, polygon: &[DVec2], inner: &[DVec2]) -> (Shapes, Shapes) {
        let piece = Piece::from_points(polygon);
        let street = union(&self.streets, piece.min, piece.max);
        let water = union(&self.water, piece.min, piece.max);
        let off = boolean(&vec![piece.contours], &street, OverlayRule::Difference);
        let off = boolean(&off, &water, OverlayRule::Difference);
        let cut = vec![Piece::from_points(inner).contours];
        (
            to_shapes(boolean(&off, &cut, OverlayRule::Intersect)),
            to_shapes(boolean(&off, &cut, OverlayRule::Difference)),
        )
    }

    /// A block's ground off the streets split in two: its forecourt, within `depth` metres of a
    /// street or a walk (paved, as the pavement it meets), and the rest (its gardens). Plain
    /// grass met the pavement in the gaps between the buildings (the owner: they "would either
    /// match the sidewalk or have a decoration"). `paths` (the paths across its green) are paved
    /// with the forecourt, one surface: drawn over it, a path showed as a patch on the paving.
    pub fn forecourt(
        &self,
        polygon: &[DVec2],
        depth: f64,
        paths: &[Vec<DVec2>],
    ) -> (Shapes, Shapes) {
        let piece = Piece::from_points(polygon);
        let (lo, hi) = (
            piece.min - DVec2::splat(depth),
            piece.max + DVec2::splat(depth),
        );
        let street = union(&self.streets, piece.min, piece.max);
        let water = union(&self.water, piece.min, piece.max);
        let off = boolean(&vec![piece.contours], &street, OverlayRule::Difference);
        let off = boolean(&off, &water, OverlayRule::Difference);
        let near: Vec<Piece> = self
            .lines
            .iter()
            .filter(|(line, w)| {
                // Its box, widened by its reach, meets the block's.
                let (a, b) = line.iter().fold(
                    (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                    |(a, b), &p| (a.min(p), b.max(p)),
                );
                let r = w * 0.5 + depth;
                a.x - r <= hi.x && a.y - r <= hi.y && b.x + r >= lo.x && b.y + r >= lo.y
            })
            .map(|(line, w)| stroke(line, w + 2.0 * depth, true, true))
            .collect();
        let mut reach = union(&near, lo, hi);
        let paths: Vec<Piece> = paths.iter().map(|p| Piece::from_points(p)).collect();
        reach = boolean(&reach, &union(&paths, lo, hi), OverlayRule::Union);
        let front = boolean(&off, &reach, OverlayRule::Intersect);
        let rest = boolean(&off, &reach, OverlayRule::Difference);
        (to_shapes(front), to_shapes(rest))
    }
}

/// What of `polygon` lies inside `within`.
pub fn clip(polygon: &[DVec2], within: &[DVec2]) -> Shapes {
    let (a, b) = (Piece::from_points(polygon), Piece::from_points(within));
    to_shapes(boolean(
        &vec![a.contours],
        &vec![b.contours],
        OverlayRule::Intersect,
    ))
}

/// Whether `p` lies in `shapes` (even-odd over all their contours).
pub fn inside(shapes: &Shapes, p: DVec2) -> bool {
    let mut odd = false;
    for contour in shapes.iter().flatten() {
        if geom::contains(contour, p) {
            odd = !odd;
        }
    }
    odd
}

/// The contours' segments inside the box, each clipped to it.
fn segments(raw: &Raw, min: DVec2, max: DVec2) -> Vec<[DVec2; 2]> {
    let mut out = Vec::new();
    for contour in raw.iter().flatten() {
        let n = contour.len();
        for k in 0..n {
            let (a, b) = (DVec2::from(contour[k]), DVec2::from(contour[(k + 1) % n]));
            if let Some(seg) = clip_segment(a, b, min, max) {
                out.push(seg);
            }
        }
    }
    out
}

/// The part of segment `a b` inside the box, or `None`.
fn clip_segment(a: DVec2, b: DVec2, min: DVec2, max: DVec2) -> Option<[DVec2; 2]> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    for (p, q) in [
        (-d.x, a.x - min.x),
        (d.x, max.x - a.x),
        (-d.y, a.y - min.y),
        (d.y, max.y - a.y),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return None;
            }
        } else {
            let r = q / p;
            if p < 0.0 {
                t0 = t0.max(r);
            } else {
                t1 = t1.min(r);
            }
        }
    }
    (t0 < t1).then(|| [a + d * t0, a + d * t1])
}

/// A simple polygon's triangles, whole. Ear clipping first; where it gives up short of the
/// polygon's area (points the booleans left touching), the polygon is sliced across into strips
/// 5 m wide and each strip clipped on its own, then 1 m wide. Ear clipping alone left gaps in the
/// pavement the ground showed through (the owner).
pub fn triangles(polygon: &[DVec2]) -> Vec<[DVec2; 3]> {
    let area = geom::signed_area(polygon).abs();
    let ears = |p: &[DVec2]| -> Vec<[DVec2; 3]> {
        geom::triangulate(p)
            .into_iter()
            .map(|t| t.map(|k| p[k]))
            .collect()
    };
    let covered =
        |tris: &[[DVec2; 3]]| tris.iter().map(|t| geom::signed_area(t).abs()).sum::<f64>();
    let first = ears(polygon);
    if covered(&first) >= area * 0.999 - 0.01 {
        return first;
    }
    let (lo, hi) = polygon.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let mut best = first;
    for step in [5.0, 1.0] {
        let mut out = Vec::new();
        let mut x = lo.x + step;
        let mut strips: Vec<Vec<DVec2>> = vec![polygon.to_vec()];
        while x < hi.x {
            let line = vec![[x, lo.y - 1.0], [x, hi.y + 1.0]];
            strips = strips
                .into_iter()
                .flat_map(|s| {
                    let raw: Vec<[f64; 2]> = s.iter().map(|p| p.to_array()).collect();
                    let pieces = vec![raw].slice_by(&line, FillRule::NonZero);
                    if pieces.is_empty() {
                        vec![s]
                    } else {
                        simple_polygons(&to_shapes(pieces))
                    }
                })
                .collect();
            x += step;
        }
        for strip in &strips {
            out.extend(ears(strip));
        }
        if covered(&out) > covered(&best) {
            best = out;
        }
        if covered(&best) >= area * 0.999 - 0.01 {
            break;
        }
    }
    best
}

/// `shapes` cut into simple polygons with no holes (each hole sliced through by a line across
/// its shape), for triangulating.
pub fn simple_polygons(shapes: &Shapes) -> Vec<Vec<DVec2>> {
    let mut out = Vec::new();
    let mut todo: Vec<Vec<Vec<DVec2>>> = shapes.clone();
    let mut guard = 0;
    while let Some(shape) = todo.pop() {
        if shape.len() <= 1 {
            if let Some(outer) = shape.into_iter().next()
                && outer.len() >= 3
            {
                out.push(outer);
            }
            continue;
        }
        guard += 1;
        if guard > 10_000 {
            // Give up on the holes: the outer contours alone.
            out.extend(shape.into_iter().take(1));
            continue;
        }
        // A vertical line through the first hole's leftmost point, across the whole shape.
        let hole = &shape[1];
        let x = hole.iter().map(|p| p.x).fold(f64::MAX, f64::min);
        let x = x + 1e-3;
        let (lo, hi) = shape[0].iter().fold((f64::MAX, f64::MIN), |(lo, hi), p| {
            (lo.min(p.y), hi.max(p.y))
        });
        let line = vec![[x, lo - 1.0], [x, hi + 1.0]];
        let raw: Vec<Vec<[f64; 2]>> = shape
            .iter()
            .map(|c| c.iter().map(|p| p.to_array()).collect())
            .collect();
        let pieces = raw.slice_by(&line, FillRule::NonZero);
        if pieces.len() <= 1 {
            // The slice missed (a degenerate hole): keep the outer contour.
            out.extend(shape.into_iter().take(1));
            continue;
        }
        todo.extend(to_shapes(pieces));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::Profile;
    use crate::city::graph::{RoadEdge, RoadNode};

    fn area(shapes: &Shapes) -> f64 {
        shapes
            .iter()
            .flatten()
            .map(|c| geom::signed_area(c))
            .sum::<f64>()
            .abs()
    }

    /// A crossroads of two avenues at (100, 100) in a 200 m frame, its arms 100 m long.
    fn crossroads(profile: Profile, side: Profile) -> RoadGraph {
        let node = |x: f64, y: f64| RoadNode {
            pos: DVec2::new(x, y),
        };
        let edge = |a: u32, b: u32, profile: Profile, graph: &[RoadNode]| RoadEdge {
            a,
            b,
            points: vec![graph[a as usize].pos, graph[b as usize].pos],
            class: crate::city::RoadClass::Main,
            profile,
            bridge: false,
        };
        let nodes = vec![
            node(100.0, 100.0),
            node(0.0, 100.0),
            node(200.0, 100.0),
            node(100.0, 0.0),
            node(100.0, 200.0),
        ];
        let edges = vec![
            edge(0, 1, profile, &nodes),
            edge(0, 2, profile, &nodes),
            edge(0, 3, side, &nodes),
            edge(0, 4, side, &nodes),
        ];
        RoadGraph { nodes, edges }
    }

    #[test]
    fn a_tram_crossroads_is_one_ground_with_round_kerbs_and_dropped_kerbs() {
        let graph = crossroads(Profile::TramAvenue, Profile::TramAvenue);
        let map = GroundMap::build(&graph, 200.0, 200.0, &[], &[], &[], &[], &[]);
        let t = &map.tiles[0];
        let (w, c) = (
            Profile::TramAvenue.width(),
            Profile::TramAvenue.carriageway(),
        );
        // The streets cover a cross of their width, no more, no less.
        let street = 2.0 * 200.0 * w - w * w;
        let road = 2.0 * 200.0 * c - c * c;
        // The four rounded corners take the corner's square less the quarter disc from the
        // pavements: (1 - π/4) r² each.
        let rounding = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4) * KERB_RADIUS * KERB_RADIUS;
        let ramps: f64 = t.ramps.iter().map(|(_, s)| area(s)).sum();
        let total = area(&t.carriageway) + area(&t.crossings) + area(&t.stripes);
        assert!(
            (total - (road + rounding)).abs() < 1.0,
            "carriageway {total:.1} m², expected {:.1}",
            road + rounding
        );
        assert!(
            (total + area(&t.walk) + ramps - street).abs() < 1.0,
            "the surfaces add up to the streets: {:.1} + {:.1} + {ramps:.1} against {street:.1}",
            total,
            area(&t.walk)
        );
        assert_eq!(map.crossings.len(), 4, "a crossing on each arm");
        assert_eq!(
            map.ramps.len(),
            8,
            "a dropped kerb at each end of each crossing"
        );
        // Each ramp lies whole on the pavement: 4 m and its flares × 1.6 m.
        assert!(
            (ramps - 8.0 * (CROSSING + FLARE) * RAMP).abs() < 0.5,
            "ramps {ramps:.2} m²"
        );
        // The levels: the carriageway's middle, a pavement, a ramp's foot and top, a block.
        assert_eq!(map.level_at(DVec2::new(130.0, 100.0)), ROAD_LEVEL);
        assert_eq!(map.level_at(DVec2::new(130.0, 109.0)), WALK_LEVEL);
        let r = map.ramps[0];
        let foot = map.level_at(r.kerb + r.inward * 0.05);
        assert!((foot - (ROAD_LEVEL + DROPPED)).abs() < 0.01, "foot {foot}");
        assert_eq!(map.level_at(DVec2::new(150.0, 150.0)), WALK_LEVEL);
        // The kerbs run along the carriageway's outline, the carriageway on their left.
        assert!(!t.kerbs.is_empty());
        for [a, b] in &t.kerbs {
            let mid = (*a + *b) * 0.5;
            let d = (*b - *a).normalize();
            let inner = mid + left(d) * 0.05;
            let outer = mid - left(d) * 0.05;
            if (0.5..199.5).contains(&mid.x) && (0.5..199.5).contains(&mid.y) {
                assert!(
                    map.on_carriageway(inner),
                    "carriageway left of the kerb at {mid}"
                );
                assert!(
                    !map.on_carriageway(outer),
                    "pavement right of the kerb at {mid}"
                );
            }
        }
    }

    #[test]
    fn a_pedestrian_street_meets_an_avenue_on_its_pavement() {
        let graph = crossroads(Profile::Avenue, Profile::Pedestrian);
        let map = GroundMap::build(&graph, 200.0, 200.0, &[], &[], &[], &[], &[]);
        // The avenue's carriageway runs straight through: no corner rounded into the mouth.
        let t = &map.tiles[0];
        let c = Profile::Avenue.carriageway();
        let road_ramps: f64 = t
            .ramps
            .iter()
            .filter(|(r, _)| map.ramps[*r as usize].lip == 0.0)
            .map(|(_, s)| area(s))
            .sum();
        let total = area(&t.carriageway)
            + area(&t.crossings)
            + area(&t.stripes)
            + area(&t.raised)
            + area(&t.raised_stripes)
            + road_ramps;
        assert!((total - 200.0 * c).abs() < 1.0, "carriageway {total:.1} m²");
        // The pedestrian street's mouth lies at the walking level, up to the kerb.
        assert_eq!(
            map.level_at(DVec2::new(100.0, 100.0 + c * 0.5 + 0.2)),
            WALK_LEVEL
        );
        // A crossing on each of the avenue's arms, past the pedestrian street's mouth.
        assert_eq!(map.crossings.len(), 2);
        for x in &map.crossings {
            let off = (x.centre - DVec2::new(100.0, 100.0)).length();
            assert!(
                off >= Profile::Pedestrian.width() * 0.5 + CROSSING * 0.5,
                "{off}"
            );
        }
    }

    #[test]
    fn a_taxi_road_s_crossings_are_raised_tables_flush_with_the_pavements() {
        let graph = crossroads(Profile::Avenue, Profile::Avenue);
        let map = GroundMap::build(&graph, 200.0, 200.0, &[], &[], &[], &[], &[]);
        let t = &map.tiles[0];
        let c = Profile::Avenue.carriageway();
        assert_eq!(map.crossings.len(), 4);
        assert!(map.crossings.iter().all(|x| x.raised));
        // Two ramps each, on the carriageway, up to the table: no dropped kerb on the pavement.
        assert_eq!(map.ramps.len(), 8);
        assert!(map.ramps.iter().all(|r| r.lip == 0.0 && r.flare == 0.0));
        // Each table's ramps lie on the carriageway: its width × their run at least (wider by a
        // rounded corner, on the junction's side).
        let ramps: f64 = t.ramps.iter().map(|(_, s)| area(s)).sum();
        assert!(
            ramps > 8.0 * c * TABLE_RAMP - 1.0 && ramps < 8.0 * (c + 4.0) * TABLE_RAMP,
            "ramps {ramps:.1} m²"
        );
        for x in &map.crossings {
            // The table at the walking level across the whole road, kerb to kerb.
            for f in [-0.45, 0.0, 0.45] {
                let p = x.centre + left(x.along) * (c * f);
                assert_eq!(map.level_at(p), WALK_LEVEL, "table at {p}");
            }
            // Its ramps' middle halfway up, the road beyond at its own level.
            let up = x.centre + x.along * (CROSSING * 0.5 + TABLE_RAMP * 0.5);
            let down = x.centre - x.along * (CROSSING * 0.5 + TABLE_RAMP * 0.5);
            let half = (ROAD_LEVEL + WALK_LEVEL) * 0.5;
            assert!(
                (map.level_at(up) - half).abs() < 0.01,
                "{}",
                map.level_at(up)
            );
            assert!(
                (map.level_at(down) - half).abs() < 0.01,
                "{}",
                map.level_at(down)
            );
            let beyond = x.centre + x.along * (CROSSING * 0.5 + TABLE_RAMP + 1.0);
            assert_eq!(map.level_at(beyond), ROAD_LEVEL);
        }
    }
    #[test]
    fn holes_are_sliced_away_for_triangulating() {
        let shapes: Shapes = vec![vec![
            vec![
                DVec2::new(0.0, 0.0),
                DVec2::new(10.0, 0.0),
                DVec2::new(10.0, 10.0),
                DVec2::new(0.0, 10.0),
            ],
            vec![
                DVec2::new(4.0, 4.0),
                DVec2::new(4.0, 6.0),
                DVec2::new(6.0, 6.0),
                DVec2::new(6.0, 4.0),
            ],
        ]];
        let polygons = simple_polygons(&shapes);
        assert!(polygons.len() >= 2);
        let total: f64 = polygons.iter().map(|p| geom::signed_area(p).abs()).sum();
        assert!((total - 96.0).abs() < 1e-3, "{total}");
    }
}
