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
use std::collections::{BTreeMap, BinaryHeap};

use glam::{Vec2, Vec3};

use crate::{DIRECTIONS, END_REACH, Field, FieldDesc, Link, NONE, Path, Walker};

/// A tile's own columns along each side.
pub const TILE_CELLS: u32 = 256;
/// The columns round a tile's own that its field covers too.
pub const TILE_MARGIN: u32 = 8;

/// A place on the tiles: its tile and its index in that tile's field.
pub type TilePlace = ((i32, i32), u32);
type Node = TilePlace;
/// A place in the search: its tile's slot among those built, and its index in that field.
type Slot = (usize, u32);

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
        self.fields
            .insert(t, Field::build(self.desc(t), self.walker, triangles));
    }

    /// What tile `t`'s field is built with: its own columns and the margin round them. A host
    /// may build it on any thread, `Field::build(tiles.desc(t), tiles.walker(), triangles)`, and
    /// hand it over ([`Self::insert`]) without the tiles going with it.
    pub fn desc(&self, t: (i32, i32)) -> FieldDesc {
        FieldDesc {
            corner: self.bounds(t).0,
            cells: TILE_CELLS + 2 * TILE_MARGIN,
            cell: self.cell,
        }
    }

    /// The walker the tiles' fields are built for.
    pub fn walker(&self) -> Walker {
        self.walker
    }

    /// Makes `field` tile `t`'s, replacing the one it had: a field built elsewhere with
    /// [`Self::desc`] and [`Self::walker`]. A field built for another tile or walker is given
    /// back.
    pub fn insert(&mut self, t: (i32, i32), field: Field) -> Result<(), Field> {
        if field.desc() != self.desc(t) || field.walker() != self.walker {
            return Err(field);
        }
        self.fields.insert(t, field);
        Ok(())
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

    /// The field of the tile under `p` (x and z), if built: where an agent there walks
    /// ([`crate::Crowd::step_tiled`], [`crate::Follow::preferred`]).
    pub fn field_at(&self, p: Vec3) -> Option<&Field> {
        self.fields.get(&self.tile_of(p))
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
        // The tiles built as slots, in their order: the search's places are (slot, place), its
        // costs dense arrays per slot made when the search first reaches the slot.
        let tiles: Vec<(i32, i32)> = self.fields.keys().copied().collect();
        let fields: Vec<&Field> = self.fields.values().collect();
        let slot_of = |t: (i32, i32)| tiles.binary_search(&t).ok();
        let to_slot = |n: Node| (slot_of(n.0).expect("a built tile"), n.1);
        let own = TILE_MARGIN..TILE_MARGIN + TILE_CELLS;
        // As `owned`, by slots.
        let owned = |sl: usize, s: u32| -> Option<(usize, u32)> {
            let (i, j) = fields[sl].at[s as usize];
            if own.contains(&i) && own.contains(&j) {
                return Some((sl, s));
            }
            let p = fields[sl].point(s);
            let other = slot_of(self.tile_of(p))?;
            Some((other, fields[other].place(p)?))
        };
        let (c, m) = (i64::from(TILE_CELLS), i64::from(TILE_MARGIN));
        let column = |(sl, s): (usize, u32)| {
            let (i, j) = fields[sl].at[s as usize];
            let t = tiles[sl];
            (
                i64::from(t.0) * c + i64::from(i) - m,
                i64::from(t.1) * c + i64::from(j) - m,
            )
        };
        let target = (
            (to.x / self.cell).floor() as i64,
            (to.z / self.cell).floor() as i64,
        );
        let h = |n: (usize, u32)| Self::heuristic(column(n), target);
        // The links' ends as places, both ways where they lead back.
        let mut jumps: BTreeMap<Slot, Vec<(Slot, u32, usize)>> = BTreeMap::new();
        for (k, l) in links.iter().enumerate().filter(|(_, l)| l.enabled) {
            let (Some(a), Some(b)) = (self.place(l.a), self.place(l.b)) else {
                continue;
            };
            let (a, b) = (to_slot(a), to_slot(b));
            let cost = ((l.cost.max(0.0) * 1.4 / self.cell * 10.0).round() as u32).max(1);
            jumps.entry(a).or_default().push((b, cost, k));
            if l.both_ways {
                jumps.entry(b).or_default().push((a, cost, k));
            }
        }
        let mut cost: Vec<Vec<u32>> = vec![Vec::new(); tiles.len()];
        let mut came: Vec<Vec<(u32, u32, u32)>> = vec![Vec::new(); tiles.len()];
        let reached_at = |cost: &[Vec<u32>], (sl, s): (usize, u32)| {
            cost[sl].get(s as usize).copied().unwrap_or(u32::MAX)
        };
        let (start, goal) = (to_slot(start), goal.map(to_slot));
        cost[start.0] = vec![u32::MAX; fields[start.0].spans.len()];
        came[start.0] = vec![(NONE, NONE, NONE); fields[start.0].spans.len()];
        cost[start.0][start.1 as usize] = 0;
        let mut open = BinaryHeap::new();
        open.push(Reverse((h(start), start.0, start.1)));
        let mut best = (h(start), start);
        let (mut expanded, mut reached) = (0, false);
        while let Some(Reverse((f, sl, s))) = open.pop() {
            let n = (sl, s);
            let here = reached_at(&cost, n);
            if f > here + h(n) {
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
            if (h(n), n) < best {
                best = (h(n), n);
            }
            let mut steps: Vec<((usize, u32), u32, u32)> = Vec::with_capacity(8);
            for (d, &o) in fields[sl].spans[s as usize].next.iter().enumerate() {
                if o == NONE {
                    continue;
                }
                if let Some(o) = owned(sl, o) {
                    steps.push((o, if d < 4 { 10 } else { 14 }, NONE));
                }
            }
            for &(b, c, k) in jumps.get(&n).map_or(&[][..], |v| &v[..]) {
                steps.push((b, c, k as u32));
            }
            for (o, step, link) in steps {
                if !fields[o.0].fits(o.1) {
                    continue;
                }
                if cost[o.0].is_empty() {
                    cost[o.0] = vec![u32::MAX; fields[o.0].spans.len()];
                    came[o.0] = vec![(NONE, NONE, NONE); fields[o.0].spans.len()];
                }
                let c = here + step;
                if c < cost[o.0][o.1 as usize] {
                    cost[o.0][o.1 as usize] = c;
                    came[o.0][o.1 as usize] = (sl as u32, s, link);
                    open.push(Reverse((c + h(o), o.0, o.1)));
                }
            }
        }
        let end = if reached {
            goal.unwrap_or(start)
        } else {
            best.1
        };
        let node = |(sl, s): (usize, u32)| (tiles[sl], s);
        let mut steps = vec![(node(end), NONE)];
        let mut n = end;
        while n != start {
            let (psl, ps, link) = came[n.0][n.1 as usize];
            if psl == NONE {
                break;
            }
            let prev = (psl as usize, ps);
            steps.push((node(prev), link));
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
    fn a_crowd_walks_across_tiles_on_each_ones_field() {
        // Two people crossing from the first tile into the second, and back, past the wall's end
        // with room to spare.
        crowd_across(23.0);
    }

    #[test]
    fn a_crowd_rounds_a_walls_end_tightly() {
        // The same from z = 10, round the wall's end at z = 20: one passed the two corners there
        // within reach, cut the end and stood wedged against it (#317's known case).
        crowd_across(10.0);
    }

    /// Two people from (20, `z`) to (45, `z`) and back across the two tiles, each on its tile's
    /// field: both arrive within a minute, never through the wall.
    fn crowd_across(z: f32) {
        let t = ground();
        let mut tiles = Tiles::new(PERSON, 0.1);
        tiles.build((0, 0), &t);
        tiles.build((1, 0), &t);
        let (a, b) = (Vec3::new(20.0, 0.0, z), Vec3::new(45.0, 0.0, z));
        let mut crowd = crate::Crowd::new(crate::CrowdSettings::default());
        let mut follows = Vec::new();
        for (from, to) in [(a, b), (b, a)] {
            let path = tiles.path(from, to, &[], 1 << 22).unwrap();
            assert!(path.complete);
            crowd.add(crate::Agent::new(path.points[0], 0.3, 1.4));
            follows.push(crate::Follow::new(&path));
        }
        for _ in 0..60 * 60 {
            for (agent, f) in crowd.agents.iter_mut().zip(&mut follows) {
                agent.preferred = f.preferred(
                    agent.position,
                    agent.max_speed,
                    tiles.field_at(agent.position),
                );
            }
            crowd.step_tiled(1.0 / 60.0, &tiles);
            // Never through the wall: on its far side only past its end.
            for agent in &crowd.agents {
                let p = agent.position;
                assert!(!(p.x > 37.6 && p.x < 38.4 && p.z < 20.0), "{p}");
            }
        }
        assert!(
            follows.iter().all(crate::Follow::arrived),
            "{:?}",
            crowd.agents
        );
    }

    #[test]
    fn a_tile_built_on_another_thread_is_handed_over() {
        let t = ground();
        let mut tiles = Tiles::new(PERSON, 0.1);
        tiles.build((0, 0), &t);
        // The second tile built on a thread of its own, with only its description and walker.
        let (desc, walker) = (tiles.desc((1, 0)), tiles.walker());
        let triangles = t.clone();
        let field = std::thread::spawn(move || Field::build(desc, walker, &triangles))
            .join()
            .unwrap();
        // Not the tile it was built for: given back.
        let field = tiles.insert((2, 0), field).unwrap_err();
        tiles.insert((1, 0), field).unwrap();
        let (a, b) = (Vec3::new(5.0, 0.0, 10.0), Vec3::new(45.0, 0.0, 10.0));
        let handed = tiles.path(a, b, &[], 1 << 22).unwrap();
        tiles.build((1, 0), &t);
        let built = tiles.path(a, b, &[], 1 << 22).unwrap();
        assert!(handed.complete);
        assert_eq!(handed, built);
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
