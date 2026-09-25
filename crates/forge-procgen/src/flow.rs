//! Water over a heightfield. D8 receivers take the steepest of the eight neighbours (ties to
//! the lowest index); the stack orders the cells downstream first (Braun & Willett 2013), so
//! an implicit erosion step visits a cell after its receiver; and the drainage area is an
//! integer count of the cells upstream. Depressions are resolved two ways: [`priority_flood`]
//! (Barnes, Lehman & Mullen 2014) fills every one to its spill level with an ε slope towards
//! the outlet, a heap over every cell, which is the reference and the last step's lakes; and
//! [`drain`] builds the basin graph of Cordonnier, Bovy & Braun (2019) on the raw field and
//! carves each closed basin's outlet path through its lowest pass, linear in the cells, which
//! is what the erosion runs every step. Every choice is pinned, so the same field gives the
//! same bytes on every machine and with any number of threads (D-016).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use forge_task::TaskPool;

use crate::field::Field2;

/// The eight neighbours, `(dx, dy)`, and their distance in cells.
const NEIGHBOURS: [(i32, i32, f32); 8] = [
    (1, 0, 1.0),
    (-1, 0, 1.0),
    (0, 1, 1.0),
    (0, -1, 1.0),
    (1, 1, std::f32::consts::SQRT_2),
    (-1, 1, std::f32::consts::SQRT_2),
    (1, -1, std::f32::consts::SQRT_2),
    (-1, -1, std::f32::consts::SQRT_2),
];

/// Cells per segment of the stack when the trees are grouped for parallel work.
const SEGMENT_GRAIN: usize = 1 << 15;

/// Where the water goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    /// Per cell, the cell it drains into (itself for an outlet: the sea and the border).
    pub receiver: Vec<u32>,
    /// Every land cell (the outlets left out: they never move), each after its receiver,
    /// from the outlets upstream; each outlet's tree is one contiguous run.
    pub stack: Vec<u32>,
    /// Per cell, its index in `stack` (`u32::MAX` for an outlet).
    pub position: Vec<u32>,
    /// Boundaries of the stack's segments, `0` first and the stack's length last: every
    /// segment `stack[segments[s]..segments[s + 1]]` holds whole trees, so a cell's receiver
    /// is an outlet or lies in the same segment before it, and the segments can be worked on
    /// in parallel.
    pub segments: Vec<u32>,
    /// Per cell, how many cells drain through it, itself included.
    pub area: Vec<u32>,
    /// Per cell, the distance to its receiver, metres (0 for an outlet).
    pub distance: Vec<f32>,
}

/// A heap key ordered by height then index, lowest first through `Reverse`.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Key(u32, u32);

/// The bits of a float in height order (negative heights are ordered too, by flipping them
/// below the positives).
fn ordered(h: f32) -> u32 {
    let bits = h.to_bits();
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}

/// Priority flood: the field with every depression filled to its spill level plus an ε rise
/// per cell towards the outlet. Outlets are the cells at or below `sea_level` and the border
/// cells; they keep their height.
pub fn priority_flood(height: &Field2<f32>, sea_level: f32) -> Field2<f32> {
    let n = height.size;
    let mut filled = height.clone();
    let mut seen = vec![false; height.len()];
    let mut heap = BinaryHeap::new();
    for (i, (&h, visited)) in height.data.iter().zip(seen.iter_mut()).enumerate() {
        let (x, y) = height.coords(i);
        if h <= sea_level || height.on_border(x, y) {
            *visited = true;
            heap.push(Reverse(Key(ordered(h), i as u32)));
        }
    }
    while let Some(Reverse(Key(_, i))) = heap.pop() {
        let (x, y) = height.coords(i as usize);
        let h = filled.data[i as usize];
        for (dx, dy, _) in NEIGHBOURS {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                continue;
            }
            let j = height.index(nx as u32, ny as u32);
            if seen[j] {
                continue;
            }
            seen[j] = true;
            if filled.data[j] <= h {
                filled.data[j] = h.next_up();
            }
            heap.push(Reverse(Key(ordered(filled.data[j]), j as u32)));
        }
    }
    filled
}

/// The D8 receiver of cell `i`: the steepest lower neighbour, the lowest index at equal
/// slope; `i` itself for an outlet (at or below the sea, or on the border) and for a pit.
#[inline]
fn d8_receiver(field: &Field2<f32>, i: usize, sea_level: f32) -> u32 {
    let n = field.size;
    let (x, y) = field.coords(i);
    let h = field.data[i];
    if h <= sea_level || field.on_border(x, y) {
        return i as u32;
    }
    let mut receiver = i as u32;
    let mut best_slope = 0.0_f32;
    for (dx, dy, d) in NEIGHBOURS {
        let (nx, ny) = (x as i32 + dx, y as i32 + dy);
        if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
            continue;
        }
        let j = field.index(nx as u32, ny as u32);
        let slope = (h - field.data[j]) / d;
        // The steepest; at equal slope the lowest index (the neighbour order is fixed).
        if slope > best_slope || (slope == best_slope && slope > 0.0 && (j as u32) < receiver) {
            best_slope = slope;
            receiver = j as u32;
        }
    }
    receiver
}

/// The D8 receivers of every cell, rows in parallel.
fn d8_receivers(field: &Field2<f32>, sea_level: f32, pool: &TaskPool) -> Vec<u32> {
    let n = field.size as usize;
    let mut receiver = vec![0_u32; field.len()];
    pool.par_chunks_mut(&mut receiver, n, |row, chunk| {
        for (x, r) in chunk.iter_mut().enumerate() {
            *r = d8_receiver(field, row * n + x, sea_level);
        }
    });
    receiver
}

/// The distance from each cell to its receiver, metres, from their coordinates.
fn distances(field: &Field2<f32>, receiver: &[u32], pool: &TaskPool) -> Vec<f32> {
    let n = field.size as usize;
    let spacing = field.spacing as f32;
    let mut distance = vec![0.0_f32; field.len()];
    pool.par_chunks_mut(&mut distance, n, |row, chunk| {
        for (x, d) in chunk.iter_mut().enumerate() {
            let i = row * n + x;
            let r = receiver[i] as usize;
            if r != i {
                let diagonal = r % n != x && r / n != row;
                *d = if diagonal {
                    std::f32::consts::SQRT_2 * spacing
                } else {
                    spacing
                };
            }
        }
    });
    distance
}

/// D8 routing over a flooded field: receivers, the downstream-first stack, drainage areas.
/// The reference routing; the erosion uses [`drain`].
pub fn route(filled: &Field2<f32>, sea_level: f32) -> Flow {
    let pool = TaskPool::new(forge_task::PoolConfig::with_workers(0));
    let receiver = d8_receivers(filled, sea_level, &pool);
    let distance = distances(filled, &receiver, &pool);
    Flow::from_receivers(receiver, distance)
}

/// A pass between two basins: the lowest pair of adjacent cells, one in each.
#[derive(Clone, Copy, Debug)]
struct Pass {
    /// The pass height's ordered bits (`max` of the two cells).
    height: u32,
    /// The basin the pass was found from (a pit's) and the one across.
    from: u32,
    to: u32,
    /// The cell in `from` and the cell in `to`.
    cell: u32,
    across: u32,
}

impl Pass {
    /// A total order: by height, then the basin pair, then the cell pair, then the side the
    /// pass was found from, so a sort gives the same sequence everywhere.
    fn key(&self) -> (u32, u32, u32, u32, u32, u32) {
        (
            self.height,
            self.from.min(self.to),
            self.from.max(self.to),
            self.cell.min(self.across),
            self.cell.max(self.across),
            self.from,
        )
    }
}

/// Union-find over the basins.
struct Basins(Vec<u32>);

impl Basins {
    fn find(&mut self, mut b: u32) -> u32 {
        while self.0[b as usize] != b {
            let parent = self.0[b as usize];
            self.0[b as usize] = self.0[parent as usize];
            b = parent;
        }
        b
    }

    /// Joins the sets of `a` and `b`; `false` when they were one already.
    fn union(&mut self, a: u32, b: u32) -> bool {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra == rb {
            return false;
        }
        self.0[ra.max(rb) as usize] = ra.min(rb);
        true
    }
}

/// Routing over the raw field with depressions resolved by the basin graph (Cordonnier, Bovy
/// & Braun 2019, carve mode): D8 receivers, every pit's basin labelled, the lowest pass between
/// each pair of adjacent basins, the minimum spanning tree of those passes rooted at the sea,
/// and each pit's path to its pass reversed so its water leaves through it. Linear in the
/// cells, where a flood is a heap over all of them.
pub fn drain(height: &Field2<f32>, sea_level: f32, pool: &TaskPool) -> Flow {
    let count = height.len();
    let n = height.size as usize;
    let mut receiver = d8_receivers(height, sea_level, pool);
    // Basins: 0 for the sea and the border (every outlet), 1.. for the pits, in the order the
    // cells meet them. Each cell walks to its root once; the walked cells are labelled.
    let mut basin = vec![u32::MAX; count];
    let mut pits: Vec<u32> = Vec::new();
    let mut path = Vec::new();
    for i in 0..count {
        if basin[i] != u32::MAX {
            continue;
        }
        let mut c = i;
        while basin[c] == u32::MAX && receiver[c] as usize != c {
            path.push(c);
            c = receiver[c] as usize;
        }
        let b = if basin[c] != u32::MAX {
            basin[c]
        } else {
            let (x, y) = height.coords(c);
            let b = if height.data[c] <= sea_level || height.on_border(x, y) {
                0
            } else {
                pits.push(c as u32);
                pits.len() as u32
            };
            basin[c] = b;
            b
        };
        for &p in &path {
            basin[p] = b;
        }
        path.clear();
    }
    if !pits.is_empty() {
        // The passes: from every cell of a pit's basin, the neighbours in another basin,
        // rows in parallel; sorted, the first of each basin pair is its lowest pass.
        let mut rows: Vec<Vec<Pass>> = vec![Vec::new(); n];
        pool.par_map_into(&mut rows, 1, |y| {
            let mut passes = Vec::new();
            for x in 0..n {
                let i = y * n + x;
                let from = basin[i];
                if from == 0 {
                    continue;
                }
                for (dx, dy, _) in NEIGHBOURS {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                        continue;
                    }
                    let j = ny as usize * n + nx as usize;
                    if basin[j] != from {
                        passes.push(Pass {
                            height: ordered(height.data[i].max(height.data[j])),
                            from,
                            to: basin[j],
                            cell: i as u32,
                            across: j as u32,
                        });
                    }
                }
            }
            passes
        });
        let mut passes: Vec<Pass> = rows.into_iter().flatten().collect();
        passes.sort_unstable_by_key(Pass::key);
        // Kruskal: the lowest passes that join basins form the spanning tree.
        let mut sets = Basins((0..=pits.len() as u32).collect());
        let mut tree: Vec<Vec<usize>> = vec![Vec::new(); pits.len() + 1];
        let mut edges = Vec::with_capacity(pits.len());
        for (k, pass) in passes.iter().enumerate() {
            if sets.union(pass.from, pass.to) {
                tree[pass.from as usize].push(k);
                tree[pass.to as usize].push(k);
                edges.push(k);
                if edges.len() == pits.len() {
                    break;
                }
            }
        }
        // From the sea outwards: each basin's path from its pass cell down to its pit is
        // reversed, and the pass cell drains across.
        let mut visited = vec![false; pits.len() + 1];
        visited[0] = true;
        let mut queue = std::collections::VecDeque::from([0_u32]);
        while let Some(parent) = queue.pop_front() {
            for &k in &tree[parent as usize] {
                let pass = passes[k];
                let (child, cell, across) = if pass.from == parent {
                    (pass.to, pass.across, pass.cell)
                } else {
                    (pass.from, pass.cell, pass.across)
                };
                if visited[child as usize] {
                    continue;
                }
                visited[child as usize] = true;
                queue.push_back(child);
                let mut c = cell as usize;
                path.clear();
                loop {
                    path.push(c);
                    let r = receiver[c] as usize;
                    if r == c {
                        break;
                    }
                    c = r;
                }
                receiver[cell as usize] = across;
                for w in path.windows(2) {
                    receiver[w[1]] = w[0] as u32;
                }
            }
        }
        debug_assert!(visited.iter().all(|&v| v), "a basin without a pass");
    }
    let distance = distances(height, &receiver, pool);
    Flow::from_receivers(receiver, distance)
}

impl Flow {
    /// The stack, positions, segments and areas of a receiver graph.
    fn from_receivers(receiver: Vec<u32>, distance: Vec<f32>) -> Self {
        let count = receiver.len();
        // Donors by counting: the cells that drain into each cell.
        let mut donor_count = vec![0_u32; count + 1];
        for (i, &r) in receiver.iter().enumerate() {
            if r as usize != i {
                donor_count[r as usize + 1] += 1;
            }
        }
        for i in 0..count {
            donor_count[i + 1] += donor_count[i];
        }
        let mut donors = vec![0_u32; count];
        let mut fill = donor_count.clone();
        for (i, &r) in receiver.iter().enumerate() {
            if r as usize != i {
                donors[fill[r as usize] as usize] = i as u32;
                fill[r as usize] += 1;
            }
        }
        // The stack: from every outlet, its donors, theirs, and so on (depth first, index
        // order), the outlet itself left out; a segment closes when it holds a grain of cells.
        let mut stack = Vec::with_capacity(count);
        let mut position = vec![u32::MAX; count];
        let mut segments = vec![0_u32];
        let mut pending = Vec::new();
        let donors_of = |c: usize| &donors[donor_count[c] as usize..donor_count[c + 1] as usize];
        for (i, &r) in receiver.iter().enumerate() {
            if r as usize != i || donor_count[i] == donor_count[i + 1] {
                continue;
            }
            pending.extend(donors_of(i).iter().rev());
            while let Some(c) = pending.pop() {
                position[c as usize] = stack.len() as u32;
                stack.push(c);
                pending.extend(donors_of(c as usize).iter().rev());
            }
            if stack.len() - *segments.last().expect("a start") as usize >= SEGMENT_GRAIN {
                segments.push(stack.len() as u32);
            }
        }
        if *segments.last().expect("a start") as usize != stack.len() {
            segments.push(stack.len() as u32);
        }
        // Areas: upstream first, each cell adds itself to its receiver.
        let mut area = vec![1_u32; count];
        for &c in stack.iter().rev() {
            let r = receiver[c as usize] as usize;
            if r != c as usize {
                area[r] += area[c as usize];
            }
        }
        Flow {
            receiver,
            stack,
            position,
            segments,
            area,
            distance,
        }
    }

    /// Whether cell `i` is an outlet (drains nowhere: the sea or the border).
    pub fn is_outlet(&self, i: usize) -> bool {
        self.receiver[i] as usize == i
    }

    /// Runs `f(segment, first, slice)` over the segments of the stack in parallel, with
    /// `out[first..]` the segment's slice of `out`, a value per stack position; inside a
    /// segment a cell's receiver is an outlet or has a lower position, so `f` can read what
    /// it wrote for it.
    pub fn par_segments<T: Send>(
        &self,
        pool: &TaskPool,
        out: &mut [T],
        f: impl Fn(usize, usize, &mut [T]) + Sync,
    ) {
        assert_eq!(out.len(), self.stack.len(), "a value per stack position");
        pool.scope(|scope| {
            let f = &f;
            let mut rest = out;
            for (s, bounds) in self.segments.windows(2).enumerate() {
                let (first, end) = (bounds[0] as usize, bounds[1] as usize);
                let (slice, tail) = rest.split_at_mut(end - first);
                rest = tail;
                scope.spawn(move |_| f(s, first, slice));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::noise::fbm;

    /// A 7 × 7 cone rising to the centre, with a pit dug next to the peak.
    fn cone_with_pit() -> Field2<f32> {
        let mut f = Field2::from_fn(7, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 3.0, y as f32 - 3.0);
            30.0 - 5.0 * (dx * dx + dy * dy).sqrt()
        });
        f.set(4, 3, 5.0); // a pit: lower than every neighbour
        f
    }

    /// Every cell reaches an outlet, the stack visits a receiver before its donors and its
    /// segments hold whole trees, and the outlets' areas sum to the cell count.
    fn check_invariants(field: &Field2<f32>, flow: &Flow) {
        let count = field.len();
        let land = (0..count).filter(|&i| !flow.is_outlet(i)).count();
        assert_eq!(flow.stack.len(), land);
        for i in 0..count {
            let mut c = i;
            let mut hops = 0;
            while !flow.is_outlet(c) {
                c = flow.receiver[c] as usize;
                hops += 1;
                assert!(hops <= count, "cell {i} never reaches an outlet");
            }
            let (x, y) = field.coords(i);
            let outlet = field.data[i] <= 0.0 || field.on_border(x, y);
            assert_eq!(flow.is_outlet(i), outlet, "cell {i}");
            if outlet {
                assert_eq!(flow.position[i], u32::MAX);
            } else {
                assert_eq!(flow.stack[flow.position[i] as usize] as usize, i);
                let r = flow.receiver[i] as usize;
                assert!(flow.is_outlet(r) || flow.position[r] < flow.position[i]);
            }
        }
        for bounds in flow.segments.windows(2) {
            for k in bounds[0]..bounds[1] {
                let c = flow.stack[k as usize] as usize;
                let r = flow.receiver[c] as usize;
                assert!(flow.is_outlet(r) || flow.position[r] >= bounds[0]);
            }
        }
        assert_eq!(flow.segments[0], 0);
        assert_eq!(*flow.segments.last().unwrap() as usize, land);
        let outlets: u32 = (0..count)
            .filter(|&i| flow.is_outlet(i))
            .map(|i| flow.area[i])
            .sum();
        assert_eq!(outlets as usize, count);
    }

    #[test]
    fn the_flood_fills_the_pit_to_its_spill_level_and_the_water_reaches_the_sea() {
        let field = cone_with_pit();
        let filled = priority_flood(&field, 0.0);
        // The pit rises to just over its lowest neighbour; nothing else moves.
        let pit = filled.get(4, 3);
        let spill = NEIGHBOURS
            .iter()
            .map(|(dx, dy, _)| field.get((4 + dx) as u32, (3 + dy) as u32))
            .fold(f32::MAX, f32::min);
        assert!(pit > spill && pit < spill + 1e-3, "{pit} vs {spill}");
        for (i, (&a, &b)) in field.data.iter().zip(&filled.data).enumerate() {
            if i != field.index(4, 3) {
                assert_eq!(a, b);
            }
        }
        let flow = route(&filled, 0.0);
        check_invariants(&field, &flow);
        assert!(flow.area[field.index(3, 3)] == 1);
        // The same field gives the same flow.
        assert_eq!(route(&filled, 0.0), flow);
    }

    #[test]
    fn the_basin_graph_drains_the_pit_through_its_lowest_pass() {
        let field = cone_with_pit();
        let pool = TaskPool::new(forge_task::PoolConfig::with_workers(0));
        let flow = drain(&field, 0.0, &pool);
        check_invariants(&field, &flow);
        // The pit is no outlet: its water climbs out over the lowest cell of its basin's rim,
        // the cell the flood spilled over, and reaches the border.
        let pit = field.index(4, 3);
        assert!(!flow.is_outlet(pit));
        let reference = route(&priority_flood(&field, 0.0), 0.0);
        let pass = |flow: &Flow| {
            let mut c = pit;
            let mut highest = (field.data[c], c);
            while !flow.is_outlet(c) {
                c = flow.receiver[c] as usize;
                if field.data[c] > highest.0 {
                    highest = (field.data[c], c);
                }
            }
            highest.1
        };
        assert_eq!(pass(&flow), pass(&reference));
        // The whole basin (the pit, its ring and the peak that drains into it) leaves there.
        assert_eq!(flow.area[pass(&flow)], 9);
    }

    #[test]
    fn a_noisy_field_drains_the_same_with_any_number_of_threads() {
        // Hills with many closed depressions, and a sea in one corner.
        let field = Field2::from_fn(48, 10.0, |x, y| {
            let n = fbm(99, f64::from(x) / 6.0, f64::from(y) / 6.0, 3, 2.0, 0.5) as f32;
            let sea = ((x as f32 - 8.0).powi(2) + (y as f32 - 8.0).powi(2)).sqrt() - 6.0;
            20.0 * n + 10.0 + sea.min(0.0) * 5.0
        });
        let serial = TaskPool::new(forge_task::PoolConfig::with_workers(0));
        let parallel = TaskPool::new(forge_task::PoolConfig::with_workers(3));
        let flow = drain(&field, 0.0, &serial);
        check_invariants(&field, &flow);
        assert_eq!(drain(&field, 0.0, &parallel), flow);
        // The raw D8 leaves pits; the basin graph leaves none (every cell reaches an outlet,
        // checked above) and no cell drains into the sea from the border's inside twice.
        let pits = (0..field.len())
            .filter(|&i| d8_receiver(&field, i, 0.0) as usize == i && !flow.is_outlet(i))
            .count();
        assert!(pits > 3, "{pits} pits: the field should have depressions");
        // The flood's routing sends the same amount of water to the sea and the border in
        // total (every cell drains out either way), and the segments split the stack.
        let reference = route(&priority_flood(&field, 0.0), 0.0);
        let total = |f: &Flow| -> u32 {
            (0..field.len())
                .filter(|&i| f.is_outlet(i))
                .map(|i| f.area[i])
                .sum()
        };
        assert_eq!(total(&flow), total(&reference));
        // par_segments hands every position to exactly one task, receivers first.
        let mut seen = vec![0_u32; flow.stack.len()];
        flow.par_segments(&parallel, &mut seen, |_, first, slice| {
            for k in 0..slice.len() {
                let c = flow.stack[first + k] as usize;
                let r = flow.receiver[c] as usize;
                assert!(flow.is_outlet(r) || slice[flow.position[r] as usize - first] == 1);
                slice[k] = 1;
            }
        });
        assert!(seen.iter().all(|&v| v == 1));
    }
}
