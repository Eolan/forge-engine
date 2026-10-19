//! `forge-nav` — navigation for characters (D-076, #317): a walk field over what was built, a
//! path search over it and the links between its places, then the path pulled straight.
//!
//! 1. **The walk field** ([`Field::build`]): the solid triangles of a tile (floors, walls,
//!    furniture, flights) rasterised into columns of [`FieldDesc::cell`] metres, each column's
//!    solids as spans (Recast's heightfield, D-076). A span's top is walkable where a triangle
//!    facing up within the walker's slope made it and the walker stands under the next span
//!    above. Neighbouring spans join where the step between them is the walker's to climb, and
//!    each span keeps its clearance: how far, in cells, the nearest edge of the walkable lies.
//! 2. **Links** ([`Link`]): a door that is not open, a window, a low fence, a gap. Each joins the
//!    place at one end to the place at the other, with a cost (the crossing's seconds) and a kind
//!    the game reads when an agent reaches it (#316). The field never detects them from its
//!    geometry: they come with the pieces' data.
//! 3. **The search** ([`Field::path`]): A* over the walkable spans the walker's radius fits and
//!    over the links, integer costs (10 straight, 14 across), ties by place, a budget of nodes.
//!    With no path, the reachable place nearest the goal (an agent goes there and waits; it never
//!    leaps at what it cannot reach).
//! 4. **Pulling it straight:** from each corner the path goes on to the farthest of its places in
//!    a straight line over walkable ground the radius fits; a link's ends stay corners.
//!
//! Space games ahead (the owner, 2026-10-19): the links and the path's shape do not depend on
//! the field, so other representations can answer the same questions later (polygons outdoors,
//! a voxel octree for flight), and a field is built in the frame of what it stands on (a ship's
//! deck). Nothing here calls a transcendental function (D-016): the same triangles give the same
//! spans and paths on every machine.

#![forbid(unsafe_code)]

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use glam::{Vec2, Vec3};

/// A walker's shape for the field: an upright capsule of `radius`, `height` tall, stepping over
/// `step`, walking up slopes whose normal's y is at least `slope_cos`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Walker {
    /// Half its width, metres.
    pub radius: f32,
    /// Its height, metres: the head room a place needs.
    pub height: f32,
    /// The rise it steps over, metres.
    pub step: f32,
    /// The steepest ground it walks: the least y of a walkable triangle's unit normal.
    pub slope_cos: f32,
}

/// A person (D-076's second question): the controller's step (`forge-physics`, 0.4 m) and the
/// capsule sweep's top (1.95 m, a stride's bob), shoulders 0.6 m across, ground up to 45°.
pub const PERSON: Walker = Walker {
    radius: 0.3,
    height: 1.95,
    step: 0.4,
    slope_cos: std::f32::consts::FRAC_1_SQRT_2,
};

/// Where a field lies and how fine it is: `cells` × `cells` columns of `cell` metres from
/// `corner` (the least x and z, in the frame of what it stands on).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FieldDesc {
    /// The least x and z, metres.
    pub corner: Vec2,
    /// Columns along each side.
    pub cells: u32,
    /// A column's side, metres (D-076: 0.10, a third of a person's radius).
    pub cell: f32,
}

impl FieldDesc {
    /// D-076's tile: 256 columns of 0.10 m, 25.6 m a side, from `corner`.
    pub fn tile(corner: Vec2) -> Self {
        Self {
            corner,
            cells: 256,
            cell: 0.1,
        }
    }

    /// The column holding `p` (x and z), if the field does.
    pub fn column(&self, p: Vec2) -> Option<(u32, u32)> {
        let q = (p - self.corner) / self.cell;
        let n = self.cells as f32;
        (q.x >= 0.0 && q.y >= 0.0 && q.x < n && q.y < n).then_some((q.x as u32, q.y as u32))
    }

    /// The middle of column (i, j), x and z.
    pub fn centre(&self, i: u32, j: u32) -> Vec2 {
        self.corner + (Vec2::new(i as f32, j as f32) + 0.5) * self.cell
    }
}

/// A place a walker stands: a walkable span's top.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Span {
    /// The floor's height, metres.
    pub floor: f32,
    /// The underside of the next solid above, metres (`f32::MAX`: open sky).
    pub ceiling: f32,
    /// How far the nearest edge of the walkable lies, in half cells (`chamfer` 2 along a side, 3
    /// across a corner), from 0 at a place that misses a neighbour ([`Field::fits`]).
    pub clearance: u16,
    /// Its neighbours' spans, one a direction ([`DIRECTIONS`]), `u32::MAX` for none.
    pub next: [u32; 8],
}

/// The eight directions, (di, dj): the four sides first, then the corners.
pub const DIRECTIONS: [(i32, i32); 8] = [
    (1, 0),
    (0, 1),
    (-1, 0),
    (0, -1),
    (1, 1),
    (-1, 1),
    (-1, -1),
    (1, -1),
];

const NONE: u32 = u32::MAX;

/// How far from a path's or a link's end, along the ground, its place may stand, metres: an end
/// in a bed or against a wall takes the nearest place the walker fits.
pub const END_REACH: f32 = 1.0;

/// A link between two places the field does not join (#316's passage, D-076): an agent at `a`
/// reaches `b` (and back with `both_ways`) at `cost` seconds, and the game plays the crossing of
/// `kind` there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    /// One end, metres.
    pub a: Vec3,
    /// The other.
    pub b: Vec3,
    /// The crossing's time, seconds (as a path's metres at walking pace: `cost × 1.4 m/s`).
    pub cost: f32,
    /// What the game plays there (a door, a window, a low fence…), the game's own number.
    pub kind: u16,
    /// Whether it leads back from `b` to `a`.
    pub both_ways: bool,
    /// Whether it is open to this search (a shut window for an archetype that breaks no glass is
    /// not).
    pub enabled: bool,
}

/// A path ([`Field::path`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    /// Its corners, from the start to the end, on the floors.
    pub points: Vec<Vec3>,
    /// The links it takes: the index of the corner it starts at (the next corner is its other
    /// end) and the link's index.
    pub links: Vec<(usize, usize)>,
    /// Whether it reaches the goal; else it ends at the reachable place nearest it.
    pub complete: bool,
    /// The places the search expanded.
    pub expanded: u32,
}

/// A walk field (D-076): its columns' walkable spans and how they join.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    desc: FieldDesc,
    walker: Walker,
    /// Per column, its first span in `spans`, then the end (`cells² + 1` entries).
    first: Vec<u32>,
    spans: Vec<Span>,
    /// Per span, its column (i, j).
    at: Vec<(u32, u32)>,
}

/// A solid interval of a column, and whether its top is walkable.
#[derive(Clone, Copy, Debug)]
struct Solid {
    bottom: f32,
    top: f32,
    walkable: bool,
}

/// Clips polygon `poly` (x, y, z) to `axis` (0: x, 2: z) ≥ `at` (`keep_above`) or < it: a
/// column holds [x0, x1), so a face standing on a column's side belongs to one column alone.
fn clip(poly: &[Vec3], axis: usize, at: f32, keep_above: bool) -> Vec<Vec3> {
    let inside = |p: &Vec3| {
        let v = p[axis];
        if keep_above { v >= at } else { v < at }
    };
    let mut out = Vec::with_capacity(poly.len() + 2);
    for k in 0..poly.len() {
        let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
        let (pi, qi) = (inside(&p), inside(&q));
        if pi {
            out.push(p);
        }
        if pi != qi {
            let t = (at - p[axis]) / (q[axis] - p[axis]);
            out.push(p + (q - p) * t);
        }
    }
    out
}

impl Field {
    /// The field of `triangles` (world metres, in the frame of what they stand on) over `desc`
    /// for `walker`. Every triangle is solid; those facing up within the walker's slope make
    /// walkable tops. Their order does not change the field.
    pub fn build(desc: FieldDesc, walker: Walker, triangles: &[[Vec3; 3]]) -> Field {
        let n = desc.cells as usize;
        let mut columns: Vec<Vec<Solid>> = vec![Vec::new(); n * n];
        for t in triangles {
            let normal = (t[1] - t[0]).cross(t[2] - t[0]);
            let length = normal.length();
            if length <= 1e-12 {
                continue;
            }
            let walkable = normal.y / length >= walker.slope_cos;
            let lo = t[0].min(t[1]).min(t[2]);
            let hi = t[0].max(t[1]).max(t[2]);
            let to_cell = |v: f32, c: f32| ((v - c) / desc.cell).floor();
            let (i0, i1) = (to_cell(lo.x, desc.corner.x), to_cell(hi.x, desc.corner.x));
            let (j0, j1) = (to_cell(lo.z, desc.corner.y), to_cell(hi.z, desc.corner.y));
            let max = desc.cells as f32 - 1.0;
            if i1 < 0.0 || j1 < 0.0 || i0 > max || j0 > max {
                continue;
            }
            let (i0, i1) = (i0.max(0.0) as u32, i1.min(max) as u32);
            let (j0, j1) = (j0.max(0.0) as u32, j1.min(max) as u32);
            for j in j0..=j1 {
                let z0 = desc.corner.y + j as f32 * desc.cell;
                let row = clip(&clip(t, 2, z0, true), 2, z0 + desc.cell, false);
                if row.len() < 3 {
                    continue;
                }
                for i in i0..=i1 {
                    let x0 = desc.corner.x + i as f32 * desc.cell;
                    let cell = clip(&clip(&row, 0, x0, true), 0, x0 + desc.cell, false);
                    if cell.len() < 3 {
                        continue;
                    }
                    let (bottom, top) = cell
                        .iter()
                        .fold((f32::MAX, f32::MIN), |(b, t), p| (b.min(p.y), t.max(p.y)));
                    columns[j as usize * n + i as usize].push(Solid {
                        bottom,
                        top,
                        walkable,
                    });
                }
            }
        }
        // Each column's solids merged where they overlap, the top's walkability the highest
        // top's (Recast's rule), in height order; the places are the walkable tops with the
        // walker's height free above them.
        let mut first = Vec::with_capacity(n * n + 1);
        let mut spans: Vec<Span> = Vec::new();
        let mut at = Vec::new();
        for (c, solids) in columns.iter_mut().enumerate() {
            first.push(spans.len() as u32);
            solids.sort_by(|a, b| {
                a.bottom
                    .total_cmp(&b.bottom)
                    .then(a.top.total_cmp(&b.top))
                    .then(a.walkable.cmp(&b.walkable))
            });
            let mut merged: Vec<Solid> = Vec::with_capacity(solids.len());
            for s in solids.iter() {
                match merged.last_mut() {
                    Some(m) if s.bottom <= m.top => {
                        if s.top > m.top {
                            m.top = s.top;
                            m.walkable = s.walkable;
                        } else if s.top == m.top {
                            m.walkable |= s.walkable;
                        }
                    }
                    _ => merged.push(*s),
                }
            }
            for (k, s) in merged.iter().enumerate() {
                let ceiling = merged.get(k + 1).map_or(f32::MAX, |above| above.bottom);
                if s.walkable && ceiling - s.top >= walker.height {
                    spans.push(Span {
                        floor: s.top,
                        ceiling,
                        clearance: 0,
                        next: [NONE; 8],
                    });
                    at.push(((c % n) as u32, (c / n) as u32));
                }
            }
        }
        first.push(spans.len() as u32);
        let mut field = Field {
            desc,
            walker,
            first,
            spans,
            at,
        };
        field.join();
        field.clearance();
        field
    }

    /// Its extent and resolution.
    pub fn desc(&self) -> FieldDesc {
        self.desc
    }

    /// The walker it was built for.
    pub fn walker(&self) -> Walker {
        self.walker
    }

    /// Its places.
    pub fn spans(&self) -> &[Span] {
        &self.spans
    }

    /// The places of column (i, j), as indices into [`Self::spans`].
    pub fn column(&self, i: u32, j: u32) -> std::ops::Range<u32> {
        let c = (j * self.desc.cells + i) as usize;
        self.first[c]..self.first[c + 1]
    }

    /// Place `s` as a point: its column's middle on its floor.
    pub fn point(&self, s: u32) -> Vec3 {
        let (i, j) = self.at[s as usize];
        let c = self.desc.centre(i, j);
        Vec3::new(c.x, self.spans[s as usize].floor, c.y)
    }

    /// Joins each place to its neighbours': the step between their floors the walker's to
    /// climb, and its height free over both. A corner joins only where both sides it passes do.
    fn join(&mut self) {
        let cells = self.desc.cells as i32;
        let (step, height) = (self.walker.step, self.walker.height);
        for s in 0..self.spans.len() {
            let (i, j) = self.at[s];
            let me = self.spans[s];
            for (d, &(di, dj)) in DIRECTIONS.iter().enumerate().take(4) {
                let (ni, nj) = (i as i32 + di, j as i32 + dj);
                if ni < 0 || nj < 0 || ni >= cells || nj >= cells {
                    continue;
                }
                let found = self.column(ni as u32, nj as u32).find(|&o| {
                    let other = &self.spans[o as usize];
                    (other.floor - me.floor).abs() <= step
                        && me.ceiling.min(other.ceiling) - me.floor.max(other.floor) >= height
                });
                if let Some(o) = found {
                    self.spans[s].next[d] = o;
                }
            }
        }
        // The corners: through the side neighbours, both ways round agreeing.
        for s in 0..self.spans.len() {
            for (d, &(di, dj)) in DIRECTIONS.iter().enumerate().skip(4) {
                let side = |dx: i32, dz: i32| {
                    DIRECTIONS
                        .iter()
                        .position(|&v| v == (dx, dz))
                        .expect("a side")
                };
                let (a, b) = (side(di, 0), side(0, dj));
                let via_a = self.spans[s].next[a];
                let via_b = self.spans[s].next[b];
                if via_a == NONE || via_b == NONE {
                    continue;
                }
                let corner_a = self.spans[via_a as usize].next[b];
                let corner_b = self.spans[via_b as usize].next[a];
                if corner_a != NONE && corner_a == corner_b {
                    self.spans[s].next[d] = corner_a;
                }
            }
        }
    }

    /// Each place's clearance: a two-pass chamfer distance (2 along a side, 3 across a corner)
    /// from the places missing a side neighbour, which stand at the walkable's edge.
    fn clearance(&mut self) {
        let count = self.spans.len();
        let mut dist: Vec<u16> = (0..count)
            .map(|s| {
                if self.spans[s].next[..4].contains(&NONE) {
                    0
                } else {
                    u16::MAX
                }
            })
            .collect();
        // Forward over the columns in order, then backward, each place taking its neighbours'
        // distance plus the step's cost where it is smaller (Recast's distance field).
        for pass in 0..2 {
            let order: Box<dyn Iterator<Item = usize>> = if pass == 0 {
                Box::new(0..count)
            } else {
                Box::new((0..count).rev())
            };
            for s in order {
                for (d, &o) in self.spans[s].next.iter().enumerate() {
                    if o == NONE {
                        continue;
                    }
                    let cost = if d < 4 { 2 } else { 3 };
                    let via = dist[o as usize].saturating_add(cost);
                    if via < dist[s] {
                        dist[s] = via;
                    }
                }
            }
        }
        for (span, d) in self.spans.iter_mut().zip(dist) {
            span.clearance = d;
        }
    }

    /// Whether the walker fits at place `s`: the nearest solid lies at least half a cell beyond
    /// the edge place the clearance counts from, so the walker fits where
    /// `(clearance + 1) × cell / 2 ≥ radius`.
    pub fn fits(&self, s: u32) -> bool {
        (f32::from(self.spans[s as usize].clearance) + 1.0) * self.desc.cell * 0.5
            >= self.walker.radius
    }

    /// The place under `p` the walker stands on: the column's highest floor within the
    /// walker's step above `p` and up to its height below, of those it fits.
    pub fn place(&self, p: Vec3) -> Option<u32> {
        let (i, j) = self.desc.column(Vec2::new(p.x, p.z))?;
        self.column(i, j)
            .filter(|&s| {
                let f = self.spans[s as usize].floor;
                f <= p.y + self.walker.step && f >= p.y - self.walker.height
            })
            .filter(|&s| self.fits(s))
            .min_by(|&a, &b| {
                let (fa, fb) = (self.spans[a as usize].floor, self.spans[b as usize].floor);
                (fa - p.y)
                    .abs()
                    .total_cmp(&(fb - p.y).abs())
                    .then(a.cmp(&b))
            })
    }

    /// The place nearest `p` (along the ground) within `reach` metres the walker fits, on a floor
    /// within its step above `p` and its height below: where an agent stands when `p` is in a
    /// bed, a cupboard or a wall's thickness. Ties go to the lower index.
    pub fn place_near(&self, p: Vec3, reach: f32) -> Option<u32> {
        if let Some(s) = self.place(p) {
            return Some(s);
        }
        let d = self.desc;
        let r = (reach / d.cell).ceil() as i64;
        let q = (Vec2::new(p.x, p.z) - d.corner) / d.cell;
        let (ci, cj) = (q.x.floor() as i64, q.y.floor() as i64);
        let n = i64::from(d.cells);
        let mut best: Option<(f32, u32)> = None;
        for j in (cj - r).max(0)..=(cj + r).min(n - 1) {
            for i in (ci - r).max(0)..=(ci + r).min(n - 1) {
                let c = d.centre(i as u32, j as u32);
                let along = (c - Vec2::new(p.x, p.z)).length();
                if along > reach {
                    continue;
                }
                for s in self.column(i as u32, j as u32) {
                    let f = self.spans[s as usize].floor;
                    if f > p.y + self.walker.step || f < p.y - self.walker.height || !self.fits(s) {
                        continue;
                    }
                    if best.is_none_or(|(b, bs)| (along, s) < (b, bs)) {
                        best = Some((along, s));
                    }
                }
            }
        }
        best.map(|(_, s)| s)
    }

    /// Which places the walker reaches from place `from` on the field alone (no link): one flag
    /// a place, by [`Self::spans`]' order. A check of what was built (a room no one reaches) more
    /// than a path's question.
    pub fn reach(&self, from: u32) -> Vec<bool> {
        let mut seen = vec![false; self.spans.len()];
        if !self.fits(from) {
            return seen;
        }
        let mut stack = vec![from];
        seen[from as usize] = true;
        while let Some(s) = stack.pop() {
            for &o in &self.spans[s as usize].next {
                if o != NONE && !seen[o as usize] && self.fits(o) {
                    seen[o as usize] = true;
                    stack.push(o);
                }
            }
        }
        seen
    }

    /// The octile distance from place `a`'s column to column `to`, in the search's units.
    fn heuristic(&self, a: u32, to: (u32, u32)) -> u32 {
        let (ai, aj) = self.at[a as usize];
        let (dx, dz) = (ai.abs_diff(to.0), aj.abs_diff(to.1));
        let (lo, hi) = (dx.min(dz), dx.max(dz));
        14 * lo + 10 * (hi - lo)
    }

    /// A path for the walker from `from` to `to` (points on or over the floors) over the field
    /// and `links`, expanding at most `budget` places. `None` when `from` stands on no place the
    /// walker fits; else a path, complete or to the reachable place nearest `to`.
    pub fn path(&self, from: Vec3, to: Vec3, links: &[Link], budget: u32) -> Option<Path> {
        let start = self.place_near(from, END_REACH)?;
        let goal = self.place_near(to, END_REACH);
        // The links' ends as places, both ways where they lead back.
        let mut jumps: Vec<(u32, u32, u32, usize)> = Vec::new(); // (from, to, cost, link)
        for (k, l) in links.iter().enumerate().filter(|(_, l)| l.enabled) {
            let (Some(a), Some(b)) = (
                self.place_near(l.a, END_REACH),
                self.place_near(l.b, END_REACH),
            ) else {
                continue;
            };
            // Seconds at walking pace (1.4 m/s) in the search's units (10 a cell).
            let cost = (l.cost.max(0.0) * 1.4 / self.desc.cell * 10.0).round() as u32;
            jumps.push((a, b, cost.max(1), k));
            if l.both_ways {
                jumps.push((b, a, cost.max(1), k));
            }
        }
        jumps.sort_unstable();
        let count = self.spans.len();
        let mut cost = vec![u32::MAX; count];
        let mut came: Vec<(u32, u32)> = vec![(NONE, NONE); count]; // (place, link or NONE)
        let mut open = BinaryHeap::new();
        // Towards the goal's column (the field's nearest when it lies outside).
        let last = self.desc.cells - 1;
        let q = (Vec2::new(to.x, to.z) - self.desc.corner) / self.desc.cell;
        let target = (
            (q.x.max(0.0) as u32).min(last),
            (q.y.max(0.0) as u32).min(last),
        );
        cost[start as usize] = 0;
        open.push(Reverse((self.heuristic(start, target), start)));
        let mut best = (self.heuristic(start, target), start);
        let mut expanded = 0;
        let mut reached = false;
        while let Some(Reverse((_, s))) = open.pop() {
            if Some(s) == goal {
                reached = true;
                break;
            }
            if expanded >= budget {
                break;
            }
            expanded += 1;
            let here = cost[s as usize];
            let h = self.heuristic(s, target);
            if (h, s) < best {
                best = (h, s);
            }
            let mut relax = |o: u32, step: u32, link: u32| {
                if !self.fits(o) {
                    return;
                }
                let c = here + step;
                if c < cost[o as usize] {
                    cost[o as usize] = c;
                    came[o as usize] = (s, link);
                    open.push(Reverse((c + self.heuristic(o, target), o)));
                }
            };
            for (d, &o) in self.spans[s as usize].next.iter().enumerate() {
                if o != NONE {
                    relax(o, if d < 4 { 10 } else { 14 }, NONE);
                }
            }
            let at = jumps.partition_point(|j| j.0 < s);
            for &(_, b, c, k) in jumps[at..].iter().take_while(|j| j.0 == s) {
                relax(b, c, k as u32);
            }
        }
        let end = if reached {
            goal.unwrap_or(start)
        } else {
            best.1
        };
        // The places back from the end, then forward, with the links taken.
        let mut places = vec![(end, NONE)];
        let mut s = end;
        while s != start {
            let (prev, link) = came[s as usize];
            if prev == NONE {
                break;
            }
            places.push((prev, link));
            s = prev;
        }
        // Each place with the link that leaves it (the one `came` named for the place after).
        places.reverse();
        let mut path = self.pulled(&places);
        path.complete = reached;
        path.expanded = expanded;
        Some(path)
    }

    /// The path through `steps` (places, each with the link leaving it or NONE) pulled
    /// straight: from each corner to the farthest place in a straight line the walker walks
    /// ([`Self::walkable_line`]); the ends of a link stay corners.
    fn pulled(&self, steps: &[(u32, u32)]) -> Path {
        let mut path = Path::default();
        if steps.is_empty() {
            return path;
        }
        let mut k = 0;
        path.points.push(self.point(steps[0].0));
        while k + 1 < steps.len() {
            if steps[k].1 != NONE {
                // A link: its other end is the next corner.
                path.links
                    .push((path.points.len() - 1, steps[k].1 as usize));
                k += 1;
                path.points.push(self.point(steps[k].0));
                continue;
            }
            // On along the path while the line from here still reaches, never past a link.
            let mut far = k + 1;
            while far + 1 < steps.len()
                && steps[far].1 == NONE
                && self.walkable_line(steps[k].0, steps[far + 1].0)
            {
                far += 1;
            }
            k = far;
            path.points.push(self.point(steps[k].0));
        }
        path
    }

    /// Whether the walker walks straight from place `a` to place `b`: the cells the segment
    /// between their middles crosses each hold a place it fits, joined to the one before (a
    /// supercover walk, the corners stepped through their sides).
    pub fn walkable_line(&self, a: u32, b: u32) -> bool {
        let (ai, aj) = self.at[a as usize];
        let (bi, bj) = self.at[b as usize];
        let (dx, dz) = (bi as i64 - ai as i64, bj as i64 - aj as i64);
        let (nx, nz) = (dx.unsigned_abs(), dz.unsigned_abs());
        let (sx, sz) = (dx.signum() as i32, dz.signum() as i32);
        let (mut ix, mut iz) = (0u64, 0u64);
        let mut s = a;
        while ix < nx || iz < nz {
            // Which side the segment leaves the current cell by: compare (ix + ½)/nx with
            // (iz + ½)/nz in integers.
            let lhs = (2 * ix + 1) * nz;
            let rhs = (2 * iz + 1) * nx;
            let d = if nz == 0 || (nx != 0 && lhs < rhs) {
                ix += 1;
                (sx, 0)
            } else if nx == 0 || lhs > rhs {
                iz += 1;
                (0, sz)
            } else {
                ix += 1;
                iz += 1;
                (sx, sz)
            };
            let dir = DIRECTIONS
                .iter()
                .position(|&v| v == d)
                .expect("a direction");
            let o = self.spans[s as usize].next[dir];
            if o == NONE || !self.fits(o) {
                return false;
            }
            s = o;
        }
        s == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A horizontal quad from (x0, z0) to (x1, z1) at height y, facing up.
    fn floor(out: &mut Vec<[Vec3; 3]>, x0: f32, z0: f32, x1: f32, z1: f32, y: f32) {
        let (a, b) = (Vec3::new(x0, y, z0), Vec3::new(x1, y, z0));
        let (c, d) = (Vec3::new(x1, y, z1), Vec3::new(x0, y, z1));
        out.push([a, c, b]);
        out.push([a, d, c]);
    }

    /// A box from `lo` to `hi`: its twelve triangles, the top facing up.
    fn block(out: &mut Vec<[Vec3; 3]>, lo: Vec3, hi: Vec3) {
        floor(out, lo.x, lo.z, hi.x, hi.z, hi.y);
        // The sides, upright (never walkable), and the bottom.
        let p = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let quads = [
            [
                p(lo.x, lo.y, lo.z),
                p(hi.x, lo.y, lo.z),
                p(hi.x, hi.y, lo.z),
                p(lo.x, hi.y, lo.z),
            ],
            [
                p(lo.x, lo.y, hi.z),
                p(lo.x, hi.y, hi.z),
                p(hi.x, hi.y, hi.z),
                p(hi.x, lo.y, hi.z),
            ],
            [
                p(lo.x, lo.y, lo.z),
                p(lo.x, hi.y, lo.z),
                p(lo.x, hi.y, hi.z),
                p(lo.x, lo.y, hi.z),
            ],
            [
                p(hi.x, lo.y, lo.z),
                p(hi.x, lo.y, hi.z),
                p(hi.x, hi.y, hi.z),
                p(hi.x, hi.y, lo.z),
            ],
            [
                p(lo.x, lo.y, lo.z),
                p(lo.x, lo.y, hi.z),
                p(hi.x, lo.y, hi.z),
                p(hi.x, lo.y, lo.z),
            ],
        ];
        for q in quads {
            out.push([q[0], q[1], q[2]]);
            out.push([q[0], q[2], q[3]]);
        }
    }

    /// A 10 m square of ground with a wall across it at x = 5 (0.2 m thick, 3 m high) from z = 0
    /// to `door_from` and from `door_to` to 10: a doorway between them when they differ.
    fn walled(door_from: f32, door_to: f32) -> Vec<[Vec3; 3]> {
        let mut t = Vec::new();
        floor(&mut t, 0.0, 0.0, 10.0, 10.0, 0.0);
        block(
            &mut t,
            Vec3::new(4.9, 0.0, 0.0),
            Vec3::new(5.1, 3.0, door_from),
        );
        block(
            &mut t,
            Vec3::new(4.9, 0.0, door_to),
            Vec3::new(5.1, 3.0, 10.0),
        );
        t
    }

    fn desc() -> FieldDesc {
        FieldDesc {
            corner: Vec2::ZERO,
            cells: 100,
            cell: 0.1,
        }
    }

    #[test]
    fn open_ground_is_one_straight_line() {
        let mut t = Vec::new();
        floor(&mut t, 0.0, 0.0, 10.0, 10.0, 0.0);
        let field = Field::build(desc(), PERSON, &t);
        assert_eq!(field.spans().len(), 100 * 100);
        let path = field
            .path(
                Vec3::new(1.0, 0.0, 1.0),
                Vec3::new(8.0, 0.0, 6.0),
                &[],
                100_000,
            )
            .unwrap();
        assert!(path.complete);
        assert_eq!(path.points.len(), 2, "{:?}", path.points);
        // Away from the edge by the radius: the edge's places do not fit.
        let edge = field.place(Vec3::new(0.05, 0.0, 5.0));
        assert!(edge.is_none());
    }

    #[test]
    fn a_wall_is_passed_by_its_doorway_and_a_narrow_gap_is_not() {
        let field = Field::build(desc(), PERSON, &walled(4.5, 5.4));
        let (from, to) = (Vec3::new(2.0, 0.0, 1.0), Vec3::new(8.0, 0.0, 1.0));
        let path = field.path(from, to, &[], 100_000).unwrap();
        assert!(path.complete);
        // Through the 0.9 m doorway between z = 4.5 and 5.4.
        assert!(
            path.points
                .iter()
                .any(|p| (p.x - 5.0).abs() < 0.3 && p.z > 4.5 && p.z < 5.4)
        );
        // A 0.5 m gap is narrower than a person.
        let narrow = Field::build(desc(), PERSON, &walled(4.5, 5.0));
        let path = narrow.path(from, to, &[], 100_000).unwrap();
        assert!(!path.complete);
        // It ends at the reachable place nearest the goal, on this side of the wall.
        let end = *path.points.last().unwrap();
        assert!(end.x < 5.0 && (end.z - 1.0).abs() < 4.0, "{end}");
    }

    #[test]
    fn a_link_crosses_a_wall_and_a_disabled_one_does_not() {
        let field = Field::build(desc(), PERSON, &walled(10.0, 10.0));
        let (from, to) = (Vec3::new(2.0, 0.0, 5.0), Vec3::new(8.0, 0.0, 5.0));
        let mut window = Link {
            a: Vec3::new(4.5, 0.0, 5.0),
            b: Vec3::new(5.5, 0.0, 5.0),
            cost: 1.5,
            kind: 1,
            both_ways: true,
            enabled: true,
        };
        let path = field.path(from, to, &[window], 100_000).unwrap();
        assert!(path.complete);
        assert_eq!(path.links.len(), 1);
        let (corner, link) = path.links[0];
        assert_eq!(link, 0);
        assert!((path.points[corner].x - 4.5).abs() < 0.1);
        assert!((path.points[corner + 1].x - 5.5).abs() < 0.1);
        // Back the other way too; and not at all when the window is shut.
        assert!(field.path(to, from, &[window], 100_000).unwrap().complete);
        window.enabled = false;
        assert!(!field.path(from, to, &[window], 100_000).unwrap().complete);
    }

    #[test]
    fn steps_are_climbed_and_ledges_and_low_ceilings_are_not() {
        let mut t = Vec::new();
        floor(&mut t, 0.0, 0.0, 10.0, 10.0, 0.0);
        // Three steps of 0.17 m up to a landing, x 3–4, then on at 0.51 m.
        for k in 0..3 {
            let x = 3.0 + k as f32 * 0.3;
            block(
                &mut t,
                Vec3::new(x, 0.0, 0.0),
                Vec3::new(10.0, 0.17 * (k + 1) as f32, 4.0),
            );
        }
        // A ledge of 0.6 m, z 6–10.
        block(&mut t, Vec3::new(3.0, 0.0, 6.0), Vec3::new(10.0, 0.6, 10.0));
        let field = Field::build(desc(), PERSON, &t);
        let up = field
            .path(
                Vec3::new(1.0, 0.0, 2.0),
                Vec3::new(8.0, 0.51, 2.0),
                &[],
                100_000,
            )
            .unwrap();
        assert!(up.complete);
        let ledge = field
            .path(
                Vec3::new(1.0, 0.0, 8.0),
                Vec3::new(8.0, 0.6, 8.0),
                &[],
                100_000,
            )
            .unwrap();
        assert!(!ledge.complete);
        // Under a slab at 1.5 m nobody stands.
        let mut low = Vec::new();
        floor(&mut low, 0.0, 0.0, 10.0, 10.0, 0.0);
        block(
            &mut low,
            Vec3::new(4.0, 1.5, 0.0),
            Vec3::new(6.0, 1.7, 10.0),
        );
        let field = Field::build(desc(), PERSON, &low);
        assert!(field.place(Vec3::new(5.0, 0.0, 5.0)).is_none());
        assert!(
            field.place(Vec3::new(5.0, 1.7, 5.0)).is_some(),
            "on top of the slab"
        );
        let path = field
            .path(
                Vec3::new(1.0, 0.0, 5.0),
                Vec3::new(8.0, 0.0, 5.0),
                &[],
                100_000,
            )
            .unwrap();
        assert!(!path.complete);
    }

    #[test]
    fn the_same_triangles_in_any_order_make_the_same_field_and_paths() {
        let t = walled(4.5, 5.4);
        let mut reversed = t.clone();
        reversed.reverse();
        let (a, b) = (
            Field::build(desc(), PERSON, &t),
            Field::build(desc(), PERSON, &reversed),
        );
        assert_eq!(a, b);
        let (from, to) = (Vec3::new(2.0, 0.0, 1.0), Vec3::new(8.0, 0.0, 9.0));
        assert_eq!(
            a.path(from, to, &[], 100_000),
            b.path(from, to, &[], 100_000)
        );
    }

    #[test]
    fn a_budget_bounds_the_search() {
        let field = Field::build(desc(), PERSON, &walled(4.5, 5.4));
        let path = field
            .path(Vec3::new(2.0, 0.0, 1.0), Vec3::new(8.0, 0.0, 1.0), &[], 50)
            .unwrap();
        assert!(!path.complete);
        assert!(path.expanded <= 50);
    }
}
