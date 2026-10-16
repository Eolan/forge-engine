//! What the animals see of a place: its buildings and walls, gardens and squares, streets with
//! their limits, the water and its quay, a pasture with its fold, and the trees and shrubs of
//! `forge-flora`'s placement. From those, the places they rest on ([`Site`]: roofs, trees, wall
//! tops, the quay's bollards, nests under the eaves) and feed in ([`Patch`]).
//!
//! The frame is the plan's, as `forge-flora`'s: `x` east, `y` south (north up on the maps), `z`
//! the height above the local ground, metres. The renderer adds the terrain's height.
//!
//! [`Parts::example`] lays out a village by the sea with a hillside behind it; a city or the
//! first map's village fills a [`Parts`] from its own plan (the hook is described in
//! `docs/demos/fauna.md`).

use forge_core::SplitMix64;
use forge_flora::place::{self, Fields, Ground, Layer, Rule};
use forge_flora::species::Species as Plant;
use glam::{DVec2, Vec2, Vec3};

/// A building: its footprint and its height to the eaves.
#[derive(Clone, Debug, PartialEq)]
pub struct Building {
    /// Its footprint, counter-clockwise or clockwise, metres.
    pub footprint: Vec<Vec2>,
    /// Its height to the eaves, metres.
    pub height: f32,
    /// Its door, just outside the footprint on the street.
    pub door: Vec2,
}

/// A wall: a garden's, a fold's, a terrace's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wall {
    /// One end.
    pub a: Vec2,
    /// The other end.
    pub b: Vec2,
    /// Its height, metres.
    pub height: f32,
}

/// A tree of the placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tree {
    /// Its trunk.
    pub pos: Vec2,
    /// Its height, metres.
    pub height: f32,
    /// Its crown's radius, metres.
    pub crown: f32,
}

/// A garden behind a house.
#[derive(Clone, Debug, PartialEq)]
pub struct Garden {
    /// Its outline.
    pub polygon: Vec<Vec2>,
    /// Its house, an index into [`Parts::buildings`].
    pub house: usize,
}

/// A street between two nodes of the street graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Street {
    /// One node.
    pub a: u32,
    /// The other.
    pub b: u32,
    /// Its speed limit for vehicles, km/h; 0 for people only.
    pub limit: f32,
}

/// When the sun rises and sets, seconds since midnight.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sun {
    /// Sunrise.
    pub rise: f32,
    /// Sunset.
    pub set: f32,
}

impl Sun {
    /// A Mediterranean June: up at 6:00, down at 21:15.
    pub const JUNE: Sun = Sun {
        rise: 6.0 * 3600.0,
        set: 21.25 * 3600.0,
    };
}

/// The parts of a place, as its generator gives them.
#[derive(Clone, Debug, PartialEq)]
pub struct Parts {
    /// The plan's side, metres; everything lies in `[0, size]²`.
    pub size: f32,
    /// Whether it is a town or a city: its birds let people closer ([`URBAN`]).
    pub urban: bool,
    /// Its buildings.
    pub buildings: Vec<Building>,
    /// Its walls.
    pub walls: Vec<Wall>,
    /// Its gardens.
    pub gardens: Vec<Garden>,
    /// Its squares' outlines.
    pub squares: Vec<Vec<Vec2>>,
    /// The street graph's nodes.
    pub nodes: Vec<Vec2>,
    /// The streets.
    pub streets: Vec<Street>,
    /// The water's outlines (the sea, a harbour's basin).
    pub water: Vec<Vec<Vec2>>,
    /// The quay's edge, along the water.
    pub quay: Vec<Vec2>,
    /// A pasture's outline (the hillside), or none.
    pub pasture: Vec<Vec2>,
    /// Its fold, sharing an edge with the pasture.
    pub fold: Vec<Vec2>,
    /// The trees.
    pub trees: Vec<Tree>,
    /// The shrubs a herd browses (a subset of the placement's understorey).
    pub shrubs: Vec<Vec2>,
    /// The sun's times.
    pub sun: Sun,
}

/// How much closer than in a village a town's birds let people, vehicles, dogs and cats come:
/// Díaz et al. 2013's urban mean flight distance over its rural one across Europe (7.12 m
/// against 13.34 m).
pub const URBAN: f32 = 0.53;

/// What a resting place is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SiteKind {
    /// A roof's eave.
    Roof,
    /// A tree's crown.
    Tree,
    /// A wall's top.
    Wall,
    /// A bollard on the quay.
    Quay,
    /// A cavity under the eaves.
    Nest,
    /// The sea, off the quay.
    Water,
}

/// A place animals rest on: a group spreads along `along` from `pos`, or about it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Site {
    /// What it is.
    pub kind: SiteKind,
    /// Its middle.
    pub pos: Vec3,
    /// Half its extent, along it.
    pub along: Vec3,
    /// How far about it a group spreads across.
    pub across: f32,
}

impl Site {
    /// Member `k`'s spot of `n` on the site, from `bits`.
    pub fn spot(&self, k: u32, n: u32, bits: u64) -> Vec3 {
        let t = if n <= 1 {
            0.0
        } else {
            2.0 * k as f32 / (n - 1) as f32 - 1.0
        };
        let j = |s: u32| ((bits >> s) & 0xFFFF) as f32 / 65535.0 * 2.0 - 1.0;
        let mut p = self.pos + self.along * t;
        p.x += j(0) * self.across;
        p.y += j(16) * self.across;
        if self.kind == SiteKind::Tree {
            p.z += j(32) * self.across * 0.5;
        }
        p
    }
}

/// What a feeding place is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PatchKind {
    /// A garden.
    Garden,
    /// A square.
    Square,
    /// A street's edge.
    Street,
    /// The quay.
    Quay,
}

/// A place animals feed in: a disc on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    /// What it is.
    pub kind: PatchKind,
    /// Its middle.
    pub pos: Vec2,
    /// Its radius, metres.
    pub radius: f32,
}

/// A place with its resting and feeding places and its lookup grids.
#[derive(Clone, Debug)]
pub struct Plan {
    /// Its parts.
    pub parts: Parts,
    /// Where animals rest.
    pub sites: Vec<Site>,
    /// Where they feed.
    pub patches: Vec<Patch>,
    /// Each building's nest cavities under its two longest free eaves, indices into
    /// [`Plan::sites`] (`u32::MAX` where it has none).
    pub nests: Vec<[u32; 2]>,
    /// Each street node's streets.
    pub adjacent: Vec<Vec<u32>>,
    grid: Grid,
}

/// Lists of buildings and walls per cell.
#[derive(Clone, Debug)]
struct Grid {
    cell: f32,
    side: usize,
    buildings: Vec<Vec<u32>>,
    walls: Vec<Vec<u32>>,
}

const CELL: f32 = 8.0;

impl Grid {
    fn cells(&self, lo: Vec2, hi: Vec2) -> impl Iterator<Item = usize> + '_ {
        let c = |v: f32| ((v / self.cell).floor() as i64).clamp(0, self.side as i64 - 1) as usize;
        let (x0, x1, y0, y1) = (c(lo.x), c(hi.x), c(lo.y), c(hi.y));
        (y0..=y1).flat_map(move |y| (x0..=x1).map(move |x| y * self.side + x))
    }

    fn at(&self, p: Vec2) -> Option<usize> {
        if p.x < 0.0 || p.y < 0.0 {
            return None;
        }
        let (x, y) = ((p.x / self.cell) as usize, (p.y / self.cell) as usize);
        (x < self.side && y < self.side).then_some(y * self.side + x)
    }
}

/// Whether `p` is inside `polygon` (the crossing number).
pub fn inside(p: Vec2, polygon: &[Vec2]) -> bool {
    let mut odd = false;
    let n = polygon.len();
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) {
            let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < x {
                odd = !odd;
            }
        }
    }
    odd
}

/// The point of segment `ab` nearest `p`.
pub fn nearest_on_segment(p: Vec2, a: Vec2, b: Vec2) -> Vec2 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 0.0 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

/// The point of `polygon`'s outline nearest `p`.
pub fn nearest_on_outline(p: Vec2, polygon: &[Vec2]) -> Vec2 {
    let n = polygon.len();
    let mut best = polygon[0];
    let mut d = f32::MAX;
    for i in 0..n {
        let q = nearest_on_segment(p, polygon[i], polygon[(i + 1) % n]);
        let e = (q - p).length_squared();
        if e < d {
            d = e;
            best = q;
        }
    }
    best
}

/// A polygon's centroid (the mean of its corners: the plan's outlines are convex or nearly).
pub fn centroid(polygon: &[Vec2]) -> Vec2 {
    polygon.iter().copied().sum::<Vec2>() / polygon.len().max(1) as f32
}

/// A polygon's area, m².
pub fn area(polygon: &[Vec2]) -> f32 {
    let n = polygon.len();
    let mut s = 0.0;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        s += a.x * b.y - b.x * a.y;
    }
    (s * 0.5).abs()
}

fn bounds(points: &[Vec2]) -> (Vec2, Vec2) {
    points.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    )
}

impl Plan {
    /// A place from its parts: its resting and feeding places, its street graph's adjacency and
    /// its lookup grids.
    pub fn new(parts: Parts) -> Self {
        let side = (parts.size / CELL).ceil() as usize + 1;
        let mut grid = Grid {
            cell: CELL,
            side,
            buildings: vec![Vec::new(); side * side],
            walls: vec![Vec::new(); side * side],
        };
        for (i, b) in parts.buildings.iter().enumerate() {
            let (lo, hi) = bounds(&b.footprint);
            for c in grid.cells(lo, hi).collect::<Vec<_>>() {
                grid.buildings[c].push(i as u32);
            }
        }
        for (i, w) in parts.walls.iter().enumerate() {
            let lo = w.a.min(w.b) - Vec2::splat(0.5);
            let hi = w.a.max(w.b) + Vec2::splat(0.5);
            for c in grid.cells(lo, hi).collect::<Vec<_>>() {
                grid.walls[c].push(i as u32);
            }
        }
        let mut adjacent = vec![Vec::new(); parts.nodes.len()];
        for (e, s) in parts.streets.iter().enumerate() {
            adjacent[s.a as usize].push(e as u32);
            adjacent[s.b as usize].push(e as u32);
        }
        let mut plan = Self {
            parts,
            sites: Vec::new(),
            patches: Vec::new(),
            nests: Vec::new(),
            adjacent,
            grid,
        };
        let (sites, patches, nests) = sites_and_patches(&plan);
        plan.sites = sites;
        plan.patches = patches;
        plan.nests = nests;
        plan
    }

    /// The building whose footprint holds `p`.
    pub fn building_at(&self, p: Vec2) -> Option<usize> {
        let c = self.grid.at(p)?;
        self.grid.buildings[c]
            .iter()
            .map(|&b| b as usize)
            .find(|&b| inside(p, &self.parts.buildings[b].footprint))
    }

    /// Whether `p` is inside a building, within `margin` above its eaves, within `margin` of a
    /// wall below its top, or under the ground.
    pub fn solid(&self, p: Vec3, margin: f32) -> bool {
        if p.z < 0.0 {
            return true;
        }
        let q = p.truncate();
        let Some(c) = self.grid.at(q) else {
            return false;
        };
        for &b in &self.grid.buildings[c] {
            let b = &self.parts.buildings[b as usize];
            if p.z < b.height + margin && inside(q, &b.footprint) {
                return true;
            }
        }
        for &w in &self.grid.walls[c] {
            let w = &self.parts.walls[w as usize];
            if p.z < w.height + margin
                && (nearest_on_segment(q, w.a, w.b) - q).length() < 0.2 + margin
            {
                return true;
            }
        }
        false
    }

    /// The height of the roof under `p`, or 0.
    pub fn roof(&self, p: Vec2) -> f32 {
        self.building_at(p)
            .map_or(0.0, |b| self.parts.buildings[b].height)
    }

    /// `p` moved out of any building to its footprint's nearest edge, 5 cm beyond it.
    pub fn push_out(&self, p: Vec2) -> Vec2 {
        let Some(b) = self.building_at(p) else {
            return p;
        };
        // The nearest point just outside an edge that is not inside a neighbour: a terraced
        // house's party walls lead into the next house.
        let fp = &self.parts.buildings[b].footprint;
        let mid = centroid(fp);
        let n = fp.len();
        let mut best = (f32::MAX, p);
        for i in 0..n {
            let (a, c) = (fp[i], fp[(i + 1) % n]);
            let e = (c - a).normalize_or_zero();
            let mut out = Vec2::new(e.y, -e.x);
            if out.dot((a + c) * 0.5 - mid) < 0.0 {
                out = -out;
            }
            let q = nearest_on_segment(p, a, c) + out * 0.05;
            let d = (q - p).length_squared();
            if d < best.0 && self.building_at(q).is_none() {
                best = (d, q);
            }
        }
        best.1
    }

    /// `p` on open ground: out of any building, and `fallback` if it is then on a wall's line.
    pub fn open_ground(&self, p: Vec2, fallback: Vec2) -> Vec2 {
        let q = self.push_out(p);
        if self.solid(q.extend(0.5), 0.15) {
            fallback
        } else {
            q
        }
    }

    /// Whether `p` is on the water.
    pub fn in_water(&self, p: Vec2) -> bool {
        self.parts.water.iter().any(|w| inside(p, w))
    }

    /// Whether `p` is in the pasture or its fold.
    pub fn in_pasture(&self, p: Vec2) -> bool {
        inside(p, &self.parts.pasture) || inside(p, &self.parts.fold)
    }

    /// How far `p` is from the quay's edge, metres.
    pub fn from_quay(&self, p: Vec2) -> f32 {
        let q = &self.parts.quay;
        (0..q.len().saturating_sub(1))
            .map(|i| (nearest_on_segment(p, q[i], q[i + 1]) - p).length())
            .fold(f32::MAX, f32::min)
    }

    /// The street node nearest `p`.
    pub fn nearest_node(&self, p: Vec2) -> u32 {
        let mut best = (f32::MAX, 0);
        for (i, &n) in self.parts.nodes.iter().enumerate() {
            let d = (n - p).length_squared();
            if d < best.0 {
                best = (d, i as u32);
            }
        }
        best.1
    }

    /// Street `e`'s other end from node `n`.
    pub fn other(&self, e: u32, n: u32) -> u32 {
        let s = self.parts.streets[e as usize];
        if s.a == n { s.b } else { s.a }
    }
}

/// The resting places (eaves, crowns, wall tops, bollards, nests, the sea off the quay) and the
/// feeding places (gardens, squares, streets' edges, the quay) of a place's parts.
fn sites_and_patches(plan: &Plan) -> (Vec<Site>, Vec<Patch>, Vec<[u32; 2]>) {
    let parts = &plan.parts;
    let mut sites = Vec::new();
    let mut nests = Vec::new();
    for (bi, b) in parts.buildings.iter().enumerate() {
        // The longest free edge's eave (not a party wall), and a nest cavity under each of the
        // two longest free edges.
        let n = b.footprint.len();
        let middle = centroid(&b.footprint);
        let mut edges: Vec<(f32, usize)> = (0..n)
            .filter(|&i| {
                let mid = (b.footprint[i] + b.footprint[(i + 1) % n]) * 0.5;
                let out = (mid - middle).normalize_or_zero();
                plan.building_at(mid + out * 0.6).is_none_or(|o| o == bi)
            })
            .map(|i| ((b.footprint[(i + 1) % n] - b.footprint[i]).length(), i))
            .collect();
        let mut cavities = [u32::MAX; 2];
        edges.sort_by(|x, y| y.0.total_cmp(&x.0).then(x.1.cmp(&y.1)));
        for (k, &(len, i)) in edges.iter().take(2).enumerate() {
            let (a, c) = (b.footprint[i], b.footprint[(i + 1) % n]);
            let mid = (a + c) * 0.5;
            let dir = (c - a) / len.max(1e-3);
            let inward = (centroid(&b.footprint) - mid).normalize_or_zero();
            let at = |z: f32, inset: f32| (mid + inward * inset).extend(z);
            if k == 0 {
                sites.push(Site {
                    kind: SiteKind::Roof,
                    pos: at(b.height + 0.15, 0.3),
                    along: (dir * len * 0.4).extend(0.0),
                    across: 0.1,
                });
            }
            cavities[k] = sites.len() as u32;
            sites.push(Site {
                kind: SiteKind::Nest,
                pos: at(b.height - 0.3, -0.3),
                along: (dir * len * 0.35).extend(0.0),
                across: 0.0,
            });
        }
        nests.push(cavities);
    }
    for t in &parts.trees {
        if t.height >= 3.0 && !plan.solid(t.pos.extend(t.height * 0.7), 0.3) {
            sites.push(Site {
                kind: SiteKind::Tree,
                pos: t.pos.extend(t.height * 0.7),
                along: Vec3::ZERO,
                across: t.crown * 0.6,
            });
        }
    }
    for w in &parts.walls {
        if (w.b - w.a).length() >= 4.0 && w.height >= 1.5 {
            sites.push(Site {
                kind: SiteKind::Wall,
                pos: ((w.a + w.b) * 0.5).extend(w.height),
                along: ((w.b - w.a) * 0.4).extend(0.0),
                across: 0.0,
            });
        }
    }
    let q = &parts.quay;
    for i in 0..q.len().saturating_sub(1) {
        let (a, b) = (q[i], q[i + 1]);
        let len = (b - a).length();
        let steps = (len / 12.0).floor() as i32;
        let land = Vec2::new((b - a).y, -(b - a).x).normalize_or_zero();
        for k in 0..steps {
            let p = a + (b - a) * ((k as f32 + 0.5) / steps as f32);
            let toward_land = if parts.water.iter().any(|w| inside(p + land, w)) {
                -land
            } else {
                land
            };
            sites.push(Site {
                kind: SiteKind::Quay,
                pos: (p + toward_land * 0.6).extend(0.6),
                along: ((b - a).normalize_or_zero() * 3.0).extend(0.0),
                across: 0.2,
            });
            sites.push(Site {
                kind: SiteKind::Water,
                pos: (p - toward_land * 40.0).extend(0.0),
                along: ((b - a).normalize_or_zero() * 6.0).extend(0.0),
                across: 4.0,
            });
        }
    }
    let mut patches = Vec::new();
    for g in &parts.gardens {
        let a = area(&g.polygon);
        patches.push(Patch {
            kind: PatchKind::Garden,
            pos: centroid(&g.polygon),
            radius: (a.sqrt() * 0.3).clamp(1.0, 6.0),
        });
    }
    for s in &parts.squares {
        let (lo, hi) = bounds(s);
        let c = centroid(s);
        let r = ((hi - lo).min_element() * 0.18).max(2.0);
        for d in [Vec2::ZERO, Vec2::new(1.0, 1.0), Vec2::new(-1.0, -1.0)] {
            patches.push(Patch {
                kind: PatchKind::Square,
                pos: c + d * r * 1.2,
                radius: r,
            });
        }
    }
    for s in &parts.streets {
        let (a, b) = (parts.nodes[s.a as usize], parts.nodes[s.b as usize]);
        if s.limit <= 30.0 && (b - a).length() > 20.0 {
            patches.push(Patch {
                kind: PatchKind::Street,
                pos: (a + b) * 0.5,
                radius: 2.0,
            });
        }
    }
    for i in 0..q.len().saturating_sub(1) {
        let mid = (q[i] + q[i + 1]) * 0.5;
        let land = Vec2::new((q[i + 1] - q[i]).y, -(q[i + 1] - q[i]).x).normalize_or_zero();
        let toward_land = if parts.water.iter().any(|w| inside(mid + land, w)) {
            -land
        } else {
            land
        };
        patches.push(Patch {
            kind: PatchKind::Quay,
            pos: mid + toward_land * 1.5,
            radius: 1.2,
        });
    }
    (sites, patches, nests)
}

/// The example village's streets: x of its north–south streets, y of its east–west ones.
const XS: [f32; 4] = [110.0, 170.0, 230.0, 290.0];
const YS: [f32; 3] = [190.0, 250.0, 310.0];
/// The coast: the sea south of it.
const COAST: f32 = 340.0;
/// The harbour front's street, and the walk along the quay's edge.
const FRONT: f32 = 330.0;
const EDGE: f32 = COAST - 1.5;

impl Parts {
    /// An example place, 400 m square: a village of five blocks of terraced houses with
    /// gardens behind them and a square of holm oaks, a road through it, a harbour front with
    /// a quay on the sea to the south, and a south-facing hillside to the north with a pasture
    /// and a fold. Its trees and shrubs are `forge-flora`'s placement over the hill's heights,
    /// from `seed`.
    pub fn example(seed: u64) -> Self {
        let size = 400.0;
        let mut rng = SplitMix64::new(seed ^ 0xFA_0A);
        // The street graph: the grid, the road through, the front and the quay's edge.
        let mut nodes = Vec::new();
        let mut node = |p: Vec2| {
            nodes.push(p);
            nodes.len() as u32 - 1
        };
        let grid: Vec<Vec<u32>> = YS
            .iter()
            .map(|&y| XS.iter().map(|&x| node(Vec2::new(x, y))).collect())
            .collect();
        let road_w = node(Vec2::new(0.0, YS[1]));
        let road_e = node(Vec2::new(size, YS[1]));
        let front: Vec<u32> = XS.iter().map(|&x| node(Vec2::new(x, FRONT))).collect();
        let edge: Vec<u32> = XS.iter().map(|&x| node(Vec2::new(x, EDGE))).collect();
        let mut streets = Vec::new();
        let mut street = |a: u32, b: u32, limit: f32| streets.push(Street { a, b, limit });
        for (r, row) in grid.iter().enumerate() {
            for c in 0..row.len() - 1 {
                street(row[c], row[c + 1], if r == 1 { 30.0 } else { 20.0 });
            }
        }
        for c in 0..XS.len() {
            for r in 0..YS.len() - 1 {
                street(grid[r][c], grid[r + 1][c], 20.0);
            }
            street(grid[YS.len() - 1][c], front[c], 20.0);
            street(front[c], edge[c], 0.0);
        }
        street(road_w, grid[1][0], 50.0);
        street(grid[1][XS.len() - 1], road_e, 50.0);
        for c in 0..XS.len() - 1 {
            street(front[c], front[c + 1], 20.0);
            street(edge[c], edge[c + 1], 0.0);
        }
        // The blocks: terraced houses on their north and south streets, gardens behind them
        // with walls between, one block a square.
        let mut buildings = Vec::new();
        let mut walls = Vec::new();
        let mut gardens = Vec::new();
        let mut squares = Vec::new();
        let half = 4.0;
        for by in 0..YS.len() - 1 {
            for bx in 0..XS.len() - 1 {
                let (x0, x1) = (XS[bx] + half, XS[bx + 1] - half);
                let (y0, y1) = (YS[by] + half, YS[by + 1] - half);
                if by == 1 && bx == 1 {
                    squares.push(vec![
                        Vec2::new(x0, y0),
                        Vec2::new(x1, y0),
                        Vec2::new(x1, y1),
                        Vec2::new(x0, y1),
                    ]);
                    continue;
                }
                let mid = (y0 + y1) * 0.5;
                for north in [true, false] {
                    let depth = 10.0;
                    let (hy0, hy1) = if north {
                        (y0, y0 + depth)
                    } else {
                        (y1 - depth, y1)
                    };
                    let (gy0, gy1) = if north { (hy1, mid) } else { (mid, hy0) };
                    let mut x = x0;
                    while x < x1 - 4.0 {
                        let w = rng.range_f32(6.0, 9.5).min(x1 - x);
                        let w = if x1 - (x + w) < 5.0 { x1 - x } else { w };
                        let height = 3.2 * (2.0 + (rng.next_f32() * 1.6).floor()) + 0.4;
                        let door_y = if north { hy0 - 0.5 } else { hy1 + 0.5 };
                        buildings.push(Building {
                            footprint: vec![
                                Vec2::new(x, hy0),
                                Vec2::new(x + w, hy0),
                                Vec2::new(x + w, hy1),
                                Vec2::new(x, hy1),
                            ],
                            height,
                            door: Vec2::new(x + w * 0.5, door_y),
                        });
                        gardens.push(Garden {
                            polygon: vec![
                                Vec2::new(x, gy0),
                                Vec2::new(x + w, gy0),
                                Vec2::new(x + w, gy1),
                                Vec2::new(x, gy1),
                            ],
                            house: buildings.len() - 1,
                        });
                        if x > x0 {
                            walls.push(Wall {
                                a: Vec2::new(x, gy0),
                                b: Vec2::new(x, gy1),
                                height: 2.0,
                            });
                        }
                        x += w;
                    }
                }
                // The middle wall and the gardens' ends on the side streets.
                walls.push(Wall {
                    a: Vec2::new(x0, mid),
                    b: Vec2::new(x1, mid),
                    height: 2.0,
                });
                for xe in [x0, x1] {
                    walls.push(Wall {
                        a: Vec2::new(xe, y0 + 10.0),
                        b: Vec2::new(xe, y1 - 10.0),
                        height: 2.0,
                    });
                }
            }
        }
        let water = vec![vec![
            Vec2::new(0.0, COAST),
            Vec2::new(size, COAST),
            Vec2::new(size, size),
            Vec2::new(0.0, size),
        ]];
        let quay = vec![
            Vec2::new(XS[0] - 10.0, COAST),
            Vec2::new(XS[3] + 10.0, COAST),
        ];
        let pasture = vec![
            Vec2::new(40.0, 30.0),
            Vec2::new(360.0, 30.0),
            Vec2::new(360.0, 161.0),
            Vec2::new(40.0, 161.0),
        ];
        let fold = vec![
            Vec2::new(190.0, 160.0),
            Vec2::new(214.0, 160.0),
            Vec2::new(214.0, 178.0),
            Vec2::new(190.0, 178.0),
        ];
        for (a, b) in [(fold[1], fold[2]), (fold[2], fold[3]), (fold[3], fold[0])] {
            walls.push(Wall { a, b, height: 1.4 });
        }
        let mut parts = Parts {
            size,
            urban: false,
            buildings,
            walls,
            gardens,
            squares,
            nodes,
            streets,
            water,
            quay,
            pasture,
            fold,
            trees: Vec::new(),
            shrubs: Vec::new(),
            sun: Sun::JUNE,
        };
        parts.plant(seed);
        parts
    }

    /// Plants the example's trees and shrubs with `forge-flora`: the hill rising north of the
    /// village (0.3 m a metre), holm oaks on the square, an olive in some gardens, the wild
    /// layers wherever the ground suits them off the buildings, the streets and the water.
    fn plant(&mut self, seed: u64) {
        let cell = 2.0;
        let side = (f64::from(self.size) / cell) as usize + 1;
        let mut heights = Vec::with_capacity(side * side);
        for j in 0..side {
            for _ in 0..side {
                let y = j as f64 * cell;
                heights.push(((YS[0] as f64 - 4.0 - y).max(0.0) * 0.3) as f32);
            }
        }
        let ground = Ground::new(side, cell, heights);
        let fields = Fields::of(&ground);
        let species: Vec<Plant> = forge_flora::mediterranean();
        let index = |id: &str| {
            species
                .iter()
                .position(|s| s.id == id)
                .expect("a village species") as u16
        };
        let poly = |p: &[Vec2]| p.iter().map(|v| v.as_dvec2()).collect::<Vec<DVec2>>();
        let mut rules = Vec::new();
        for s in &self.squares {
            rules.push(Rule::Grove {
                species: index("holm-oak"),
                polygon: poly(s),
                spacing: 17.0,
                fill: 0.8,
                age: [40.0, 90.0],
            });
        }
        for g in &self.gardens {
            rules.push(Rule::Grove {
                species: index("olive"),
                polygon: poly(&g.polygon),
                spacing: 9.0,
                fill: 0.25,
                age: [20.0, 60.0],
            });
        }
        let this = &*self;
        let near_street = |p: Vec2| {
            this.streets.iter().any(|s| {
                let (a, b) = (this.nodes[s.a as usize], this.nodes[s.b as usize]);
                (nearest_on_segment(p, a, b) - p).length() < 5.0
            })
        };
        let keep_out = |d: DVec2| {
            let p = d.as_vec2();
            p.y >= COAST - 2.0
                || this.buildings.iter().any(|b| inside(p, &b.footprint))
                || inside(p, &this.fold)
                || near_street(p)
        };
        let placed = place::place(&ground, &fields, &species, &rules, seed, &keep_out);
        let layer = |s: u16| {
            species[s as usize]
                .habitat
                .as_ref()
                .map_or(Layer::Canopy, |h| h.layer)
        };
        for p in &placed.plants {
            let pos = p.pos.as_vec2();
            match layer(p.species) {
                Layer::Canopy => self.trees.push(Tree {
                    pos,
                    height: p.height,
                    crown: p.crown,
                }),
                Layer::Understorey => {
                    if inside(pos, &self.pasture) {
                        self.shrubs.push(pos);
                    }
                }
            }
        }
        // A herd browses a few thousand shrubs, not every one.
        let stride = self.shrubs.len().div_ceil(3000).max(1);
        self.shrubs = self.shrubs.iter().copied().step_by(stride).collect();
    }
}
