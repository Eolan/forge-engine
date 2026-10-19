//! Field tiles (#317, D-076): the walk field in square tiles of [`TILE_CELLS`] columns, built
//! where the agents need them and rebuilt alone when what stands on one changes.
//!
//! - **A tile's field** covers its own columns and a margin of [`TILE_MARGIN`] round them, so
//!   the clearance near its edge sees what stands beyond it.
//! - **A place** belongs to the tile whose own columns hold it. A step into a tile's margin
//!   goes on in the tile that owns the column, through the same point on the same floor. A tile
//!   not built yet is not walked: a path towards it ends at the reachable place nearest its
//!   goal, as on one field.
//! - **The search** is [`Field::path`]'s over (tile, place) nodes: A* with integer octile costs
//!   on the tiles' common grid, ties by tile and place (D-016), a budget, and the links.
//! - **Pulling straight** checks a line across tiles as on one field ([`Tiles::walkable_line`]).

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

use glam::{Vec2, Vec3};

use crate::{DIRECTIONS, END_REACH, Field, FieldDesc, Link, NONE, Path, Walker};

/// A tile's own columns along each side.
pub const TILE_CELLS: u32 = 256;
/// The columns round a tile's own that its field covers too.
pub const TILE_MARGIN: u32 = 8;

/// A place on the tiles: its tile and its index in that tile's field.
pub type TilePlace = ((i32, i32), u32);
type Node = TilePlace;

/// The walk field in tiles.
#[derive(Clone, Debug)]
pub struct Tiles {
    walker: Walker,
    cell: f32,
    fields: BTreeMap<(i32, i32), Field>,
}

impl Tiles {
    /// No tile yet, for `walker`, in columns of `cell` metres (D-076: 0.10).
    pub fn new(walker: Walker, cell: f32) -> Self {
        Self {
            walker,
            cell,
            fields: BTreeMap::new(),
        }
    }

    /// A tile's side, metres.
    pub fn size(&self) -> f32 {
        TILE_CELLS as f32 * self.cell
    }

    /// The tile holding `p` (x and z).
    pub fn tile_of(&self, p: Vec3) -> (i32, i32) {
        let s = self.size();
        ((p.x / s).floor() as i32, (p.z / s).floor() as i32)
    }

    /// The tiles within `r` metres of `p` (x and z): those an agent there needs, in order.
    pub fn round(&self, p: Vec3, r: f32) -> Vec<(i32, i32)> {
        let (lo, hi) = (
            self.tile_of(p - Vec3::new(r, 0.0, r)),
            self.tile_of(p + Vec3::new(r, 0.0, r)),
        );
        (lo.1..=hi.1)
            .flat_map(|z| (lo.0..=hi.0).map(move |x| (x, z)))
            .collect()
    }

    /// What tile `t`'s field covers: its least x and z, and its side, metres. The host gives
    /// the triangles of the solids over it to [`Self::build`].
    pub fn bounds(&self, t: (i32, i32)) -> (Vec2, f32) {
        let margin = TILE_MARGIN as f32 * self.cell;
        let corner = Vec2::new(t.0 as f32, t.1 as f32) * self.size() - Vec2::splat(margin);
        (corner, self.size() + 2.0 * margin)
    }

    /// Builds tile `t`'s field from `triangles` (the solids over [`Self::bounds`]; others are
    /// clipped away), replacing the one it had.
    pub fn build(&mut self, t: (i32, i32), triangles: &[[Vec3; 3]]) {
        let desc = FieldDesc {
            corner: self.bounds(t).0,
            cells: TILE_CELLS + 2 * TILE_MARGIN,
            cell: self.cell,
        };
        self.fields
            .insert(t, Field::build(desc, self.walker, triangles));
    }

    /// Drops tile `t`'s field: no agent needs it.
    pub fn drop_tile(&mut self, t: (i32, i32)) {
        self.fields.remove(&t);
    }

    /// Whether tile `t` is built.
    pub fn has(&self, t: (i32, i32)) -> bool {
        self.fields.contains_key(&t)
    }

    /// Tile `t`'s field, if built.
    pub fn field(&self, t: (i32, i32)) -> Option<&Field> {
        self.fields.get(&t)
    }

    /// The tiles built, in order.
    pub fn built(&self) -> impl Iterator<Item = (i32, i32)> + '_ {
        self.fields.keys().copied()
    }

    /// The place nearest `p` the walker fits, within [`END_REACH`], in the tile that owns it.
    pub fn place(&self, p: Vec3) -> Option<TilePlace> {
        let t = self.tile_of(p);
        let s = self.fields.get(&t)?.place_near(p, END_REACH)?;
        self.owned(t, s)
    }

    /// A place as a point: its column's middle on its floor.
    pub fn point(&self, n: TilePlace) -> Vec3 {
        self.fields[&n.0].point(n.1)
    }

    /// Place `s` of tile `t`'s field in the tile that owns its column: itself among the tile's
    /// own columns, else the same floor in the neighbour's field (None when that tile is not
    /// built or the walker does not fit there).
    fn owned(&self, t: (i32, i32), s: u32) -> Option<Node> {
        let field = &self.fields[&t];
        let (i, j) = field.at[s as usize];
        let own = TILE_MARGIN..TILE_MARGIN + TILE_CELLS;
        if own.contains(&i) && own.contains(&j) {
            return Some((t, s));
        }
        let p = field.point(s);
        let owner = self.tile_of(p);
        let other = self.fields.get(&owner)?;
        let o = other.place(p)?;
        Some((owner, o))
    }

    /// Whether the walker fits at a node.
    fn fits(&self, n: Node) -> bool {
        self.fields[&n.0].fits(n.1)
    }

    /// A node's column on the tiles' common grid.
    fn column(&self, n: Node) -> (i64, i64) {
        let (i, j) = self.fields[&n.0].at[n.1 as usize];
        let c = i64::from(TILE_CELLS);
        let m = i64::from(TILE_MARGIN);
        (
            i64::from(n.0.0) * c + i64::from(i) - m,
            i64::from(n.0.1) * c + i64::from(j) - m,
        )
    }

    /// The octile distance between two columns, in the search's units.
    fn heuristic(a: (i64, i64), b: (i64, i64)) -> u32 {
        let (dx, dz) = (a.0.abs_diff(b.0), a.1.abs_diff(b.1));
        let (lo, hi) = (dx.min(dz), dx.max(dz));
        (14 * lo + 10 * (hi - lo)).min(u64::from(u32::MAX)) as u32
    }

    /// A path for the walker from `from` to `to` over the tiles built and `links`, expanding at
    /// most `budget` places: as [`Field::path`], its corners in the tiles' fields.
    pub fn path(&self, from: Vec3, to: Vec3, links: &[Link], budget: u32) -> Option<Path> {
        let start = self.place(from)?;
        let goal = self.place(to);
        let target = (
            (to.x / self.cell).floor() as i64,
            (to.z / self.cell).floor() as i64,
        );
        // The links' ends as nodes, both ways where they lead back.
        let mut jumps: BTreeMap<Node, Vec<(Node, u32, usize)>> = BTreeMap::new();
        for (k, l) in links.iter().enumerate().filter(|(_, l)| l.enabled) {
            let (Some(a), Some(b)) = (self.place(l.a), self.place(l.b)) else {
                continue;
            };
            let cost = ((l.cost.max(0.0) * 1.4 / self.cell * 10.0).round() as u32).max(1);
            jumps.entry(a).or_default().push((b, cost, k));
            if l.both_ways {
                jumps.entry(b).or_default().push((a, cost, k));
            }
        }
        let mut cost: HashMap<Node, u32> = HashMap::new();
        let mut came: HashMap<Node, (Node, u32)> = HashMap::new();
        let mut open = BinaryHeap::new();
        cost.insert(start, 0);
        let h0 = Self::heuristic(self.column(start), target);
        open.push(Reverse((h0, start)));
        let mut best = (h0, start);
        let (mut expanded, mut reached) = (0, false);
        while let Some(Reverse((f, n))) = open.pop() {
            let here = cost[&n];
            if f > here + Self::heuristic(self.column(n), target) {
                // An older entry, since reached more cheaply.
                continue;
            }
            if Some(n) == goal {
                reached = true;
                break;
            }
            if expanded >= budget {
                break;
            }
            expanded += 1;
            let h = Self::heuristic(self.column(n), target);
            if (h, n) < best {
                best = (h, n);
            }
            let mut relax = |o: Node, step: u32, link: u32| {
                if !self.fits(o) {
                    return;
                }
                let c = here + step;
                if cost.get(&o).is_none_or(|&old| c < old) {
                    cost.insert(o, c);
                    came.insert(o, (n, link));
                    open.push(Reverse((c + Self::heuristic(self.column(o), target), o)));
                }
            };
            let field = &self.fields[&n.0];
            for (d, &o) in field.spans[n.1 as usize].next.iter().enumerate() {
                if o == NONE {
                    continue;
                }
                if let Some(o) = self.owned(n.0, o) {
                    relax(o, if d < 4 { 10 } else { 14 }, NONE);
                }
            }
            for &(b, c, k) in jumps.get(&n).map_or(&[][..], |v| &v[..]) {
                relax(b, c, k as u32);
            }
        }
        let end = if reached {
            goal.unwrap_or(start)
        } else {
            best.1
        };
        let mut steps = vec![(end, NONE)];
        let mut n = end;
        while n != start {
            let Some(&(prev, link)) = came.get(&n) else {
                break;
            };
            steps.push((prev, link));
            n = prev;
        }
        steps.reverse();
        let mut path = self.pulled(&steps);
        path.complete = reached;
        path.expanded = expanded;
        Some(path)
    }

    /// As [`Field::walkable_line`] over the tiles: from node `a` to node `b`, each column the
    /// segment crosses holding a place the walker fits, joined to the one before, a step into a
    /// tile's margin going on in the tile that owns the column.
    pub fn walkable_line(&self, a: TilePlace, b: TilePlace) -> bool {
        let (ca, cb) = (self.column(a), self.column(b));
        let (dx, dz) = (cb.0 - ca.0, cb.1 - ca.1);
        let (nx, nz) = (dx.unsigned_abs(), dz.unsigned_abs());
        let (sx, sz) = (dx.signum() as i32, dz.signum() as i32);
        let (mut ix, mut iz) = (0u64, 0u64);
        let mut n = a;
        while ix < nx || iz < nz {
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
            let o = self.fields[&n.0].spans[n.1 as usize].next[dir];
            match (o != NONE).then(|| self.owned(n.0, o)).flatten() {
                Some(o) if self.fits(o) => n = o,
                _ => return false,
            }
        }
        n == b
    }

    /// As [`Field::pulled`], over the tiles.
    fn pulled(&self, steps: &[(Node, u32)]) -> Path {
        let mut path = Path::default();
        if steps.is_empty() {
            return path;
        }
        let line = |a: Node, b: Node| self.walkable_line(a, b);
        let mut k = 0;
        path.points.push(self.point(steps[0].0));
        while k + 1 < steps.len() {
            if steps[k].1 != NONE {
                path.links
                    .push((path.points.len() - 1, steps[k].1 as usize));
                k += 1;
                path.points.push(self.point(steps[k].0));
                continue;
            }
            let mut far = k + 1;
            while far + 1 < steps.len()
                && steps[far].1 == NONE
                && line(steps[k].0, steps[far + 1].0)
            {
                far += 1;
            }
            k = far;
            path.points.push(self.point(steps[k].0));
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PERSON;
    use crate::tests::{block, floor};

    /// Ground over two tiles side by side (x from 0 to 51.2) and their margins, with a wall
    /// 3 m high across the second at x = 38 from z = −1 to 20: the way past it round its end.
    fn ground() -> Vec<[Vec3; 3]> {
        let mut t = Vec::new();
        floor(&mut t, -1.0, -1.0, 53.0, 27.0, 0.0);
        block(
            &mut t,
            Vec3::new(37.9, 0.0, -1.0),
            Vec3::new(38.1, 3.0, 20.0),
        );
        t
    }

    #[test]
    fn a_path_crosses_from_tile_to_tile_as_on_one_field() {
        let t = ground();
        let mut tiles = Tiles::new(PERSON, 0.1);
        let clock = std::time::Instant::now();
        for tile in tiles.round(Vec3::new(25.6, 0.0, 10.0), 1.0) {
            tiles.build(tile, &t);
        }
        assert!(tiles.has((0, 0)) && tiles.has((1, 0)));
        let (a, b) = (Vec3::new(5.0, 0.0, 10.0), Vec3::new(45.0, 0.0, 10.0));
        let built = clock.elapsed();
        let clock = std::time::Instant::now();
        let path = tiles.path(a, b, &[], 1 << 22).unwrap();
        let searched = clock.elapsed();
        assert!(path.complete);
        // Round the wall's end at z = 20.
        assert!(path.points.iter().any(|p| p.z > 20.0), "{:?}", path.points);
        let length = |p: &Path| -> f32 { p.points.windows(2).map(|w| w[0].distance(w[1])).sum() };
        // One field over both tiles finds the same way, within a few per cent.
        let whole = Field::build(
            FieldDesc {
                corner: Vec2::new(-0.8, -0.8),
                cells: 540,
                cell: 0.1,
            },
            PERSON,
            &t,
        );
        let one = whole.path(a, b, &[], 1 << 22).unwrap();
        assert!(one.complete);
        let (tiled, single) = (length(&path), length(&one));
        eprintln!(
            "two tiles built in {built:?}, a path over both in {searched:?} ({} places): {tiled:.2} m, {single:.2} m on one field",
            path.expanded
        );
        assert!((tiled - single).abs() < 0.03 * single, "{tiled} {single}");
    }

    #[test]
    fn a_tile_not_built_is_not_walked() {
        let t = ground();
        let mut tiles = Tiles::new(PERSON, 0.1);
        tiles.build((0, 0), &t);
        let path = tiles
            .path(
                Vec3::new(5.0, 0.0, 10.0),
                Vec3::new(45.0, 0.0, 10.0),
                &[],
                1 << 22,
            )
            .unwrap();
        // It ends at the edge of the tile built, nearest the goal.
        assert!(!path.complete);
        let end = path.points[path.points.len() - 1];
        assert!(end.x > 25.0 && end.x < 25.6, "{end}");
        // Building the next tile opens the way; dropping it closes it again.
        tiles.build((1, 0), &t);
        assert!(
            tiles
                .path(
                    Vec3::new(5.0, 0.0, 10.0),
                    Vec3::new(45.0, 0.0, 10.0),
                    &[],
                    1 << 22
                )
                .unwrap()
                .complete
        );
        tiles.drop_tile((1, 0));
        assert!(!tiles.has((1, 0)));
    }
}
