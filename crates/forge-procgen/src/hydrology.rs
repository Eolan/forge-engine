//! Stage 4 of the terrain pipeline: the river network as polylines. River cells are the cells
//! whose catchment reaches a threshold; from every mouth (a river cell draining into an
//! outlet) the trunk follows the largest tributary upstream to its head, and every other river
//! donor along it starts a tributary of its own, traced the same way; so each [`River`] runs
//! from a head to an outlet or to its junction with a larger river. Strahler orders (Strahler
//! 1957) come bottom-up: a head is 1, a junction of two equals is one more. Widths follow the
//! hydraulic geometry of Leopold & Maddock (1953), `w ∝ √Q` with the catchment as the
//! discharge. Lakes ([`trace_lakes`]) are the connected patches where the final flood stands
//! over the eroded field, each with its level, its outlet and its cells. Everything is a pure
//! function of the flow and the flood, in index order (D-016).

use crate::field::Field2;
use crate::flow::Flow;

/// Where a river ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mouth {
    /// It reaches an outlet: the sea or the border, at this cell.
    Outlet(u32),
    /// It joins the river at this index, at that point of it.
    Junction {
        /// The larger river.
        river: u32,
        /// The index of the point of `river` the water joins at.
        point: u32,
    },
}

/// One river, from its head to its mouth.
#[derive(Clone, Debug, PartialEq)]
pub struct River {
    /// The cells, head first.
    pub cells: Vec<u32>,
    /// `(x, y, height)` per cell, metres in the field's frame (`x` along columns, `y` along
    /// rows), head first.
    pub points: Vec<[f32; 3]>,
    /// The catchment at each cell, cells (times the cell area for m²), head first.
    pub area: Vec<u32>,
    /// The Strahler order at the mouth.
    pub order: u8,
    /// Where it ends.
    pub mouth: Mouth,
}

impl River {
    /// The river's length along its cells, metres.
    pub fn length(&self) -> f32 {
        self.points
            .windows(2)
            .map(|w| {
                let (dx, dy) = (w[1][0] - w[0][0], w[1][1] - w[0][1]);
                (dx * dx + dy * dy).sqrt()
            })
            .sum()
    }
}

/// The network.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rivers {
    /// Every river, the trunks (mouth at an outlet) before their tributaries.
    pub rivers: Vec<River>,
    /// Per cell, the river through it (`u32::MAX` for none).
    pub river_of: Vec<u32>,
    /// Per cell, the Strahler order (0 off the network).
    pub order: Vec<u8>,
}

/// The width of a river with `area_m2` of catchment, metres: 5 m at a square kilometre,
/// 35 m at fifty (`w = 0.005 √A`, Leopold & Maddock's exponent with a coefficient for a
/// temperate island).
pub fn width(area_m2: f64) -> f64 {
    0.005 * area_m2.sqrt()
}

/// The rivers of `flow` over `height`: cells with at least `min_area` cells of catchment.
pub fn trace_rivers(height: &Field2<f32>, flow: &Flow, min_area: u32) -> Rivers {
    let count = height.len();
    let spacing = height.spacing as f32;
    let is_river = |i: usize| !flow.is_outlet(i) && flow.area[i] >= min_area;
    // The river cells get a slot each (the network is a small part of the field).
    let mut slot = vec![u32::MAX; count];
    let mut slots = 0_u32;
    for (i, s) in slot.iter_mut().enumerate() {
        if is_river(i) {
            *s = slots;
            slots += 1;
        }
    }
    // Per river cell, its river donors: the largest first (ties to the lowest index), which
    // is the trunk's way upstream; the Strahler order from the donors', upstream first.
    let mut donors: Vec<Vec<u32>> = vec![Vec::new(); slots as usize];
    let mut order = vec![0_u8; count];
    for &c in flow.stack.iter().rev() {
        let i = c as usize;
        if slot[i] == u32::MAX {
            continue;
        }
        let mine = &mut donors[slot[i] as usize];
        mine.sort_by_key(|&d| (std::cmp::Reverse(flow.area[d as usize]), d));
        order[i] = match mine.as_slice() {
            [] => 1,
            [one] => order[*one as usize],
            [first, second, ..] => {
                let (a, b) = (order[*first as usize], order[*second as usize]);
                if a == b { a + 1 } else { a.max(b) }
            }
        };
        let r = flow.receiver[i] as usize;
        if slot[r] != u32::MAX {
            donors[slot[r] as usize].push(c);
        }
    }
    let donors_of = |i: usize| -> &[u32] { &donors[slot[i] as usize] };
    // From every mouth, in index order, the trunk upstream along the largest donors; each
    // other donor met on the way starts a tributary (a worklist, so the order is fixed).
    let mut rivers = Vec::new();
    let mut river_of = vec![u32::MAX; count];
    let mut pending: Vec<(u32, Mouth)> = Vec::new();
    for (i, &s) in slot.iter().enumerate() {
        if s != u32::MAX && flow.is_outlet(flow.receiver[i] as usize) {
            pending.push((i as u32, Mouth::Outlet(flow.receiver[i])));
        }
        while let Some((start, mouth)) = pending.pop() {
            let index = rivers.len() as u32;
            let mut cells = vec![start];
            let mut c = start as usize;
            while let Some(&main) = donors_of(c).first() {
                cells.push(main);
                c = main as usize;
            }
            cells.reverse();
            for (k, &cell) in cells.iter().enumerate() {
                river_of[cell as usize] = index;
                // Tributaries join at this point; the last pushed is traced first, so they
                // are pushed in reverse to come out in donor order.
                for &tributary in donors_of(cell as usize).iter().skip(1).rev() {
                    pending.push((
                        tributary,
                        Mouth::Junction {
                            river: index,
                            point: k as u32,
                        },
                    ));
                }
            }
            let points = cells
                .iter()
                .map(|&cell| {
                    let (x, y) = height.coords(cell as usize);
                    [
                        x as f32 * spacing,
                        y as f32 * spacing,
                        height.data[cell as usize],
                    ]
                })
                .collect();
            let area = cells.iter().map(|&cell| flow.area[cell as usize]).collect();
            rivers.push(River {
                order: order[start as usize],
                cells,
                points,
                area,
                mouth,
            });
        }
    }
    Rivers {
        rivers,
        river_of,
        order,
    }
}

impl Rivers {
    /// The total length of the network, metres.
    pub fn total_length(&self) -> f32 {
        self.rivers.iter().map(River::length).sum()
    }

    /// The highest Strahler order in the network.
    pub fn max_order(&self) -> u8 {
        self.rivers.iter().map(|r| r.order).max().unwrap_or(0)
    }
}

/// One lake: a flooded depression.
#[derive(Clone, Debug, PartialEq)]
pub struct Lake {
    /// The water's level, metres (the flood's height over the depression).
    pub level: f32,
    /// The cells under water, in index order.
    pub cells: Vec<u32>,
    /// The deepest point below the level, metres.
    pub depth: f32,
    /// The cell the water leaves by: the lake cell whose receiver is outside the lake and
    /// lowest, or the lake's lowest-index cell when none drains out (the border).
    pub outlet: u32,
}

impl Lake {
    /// The lake's area, m², for a field `spacing` metres apart.
    pub fn area(&self, spacing: f64) -> f64 {
        self.cells.len() as f64 * spacing * spacing
    }
}

/// The lakes of a field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lakes {
    /// Every lake, in the index order of their first cell.
    pub lakes: Vec<Lake>,
    /// Per cell, the lake it is under (`u32::MAX` for none).
    pub lake_of: Vec<u32>,
}

/// The lakes: the 4-connected patches of cells where `filled` (a priority flood of `height`)
/// stands more than `min_depth` over `height`, each with the flood's level there (the same
/// across the patch up to the flood's ε), its deepest point and its outlet along `flow`.
pub fn trace_lakes(
    height: &Field2<f32>,
    filled: &Field2<f32>,
    flow: &Flow,
    min_depth: f32,
) -> Lakes {
    let (n, count) = (height.size as usize, height.len());
    let under = |i: usize| filled.data[i] - height.data[i] > min_depth;
    let mut lake_of = vec![u32::MAX; count];
    let mut lakes = Vec::new();
    let mut queue = std::collections::VecDeque::new();
    for start in 0..count {
        if lake_of[start] != u32::MAX || !under(start) {
            continue;
        }
        let id = lakes.len() as u32;
        lake_of[start] = id;
        queue.push_back(start);
        let mut cells = Vec::new();
        while let Some(i) = queue.pop_front() {
            cells.push(i as u32);
            let (x, y) = (i % n, i / n);
            let neighbours = [
                (x > 0).then(|| i - 1),
                (x + 1 < n).then(|| i + 1),
                (y > 0).then(|| i - n),
                (y + 1 < n).then(|| i + n),
            ];
            for j in neighbours.into_iter().flatten() {
                if lake_of[j] == u32::MAX && under(j) {
                    lake_of[j] = id;
                    queue.push_back(j);
                }
            }
        }
        cells.sort_unstable();
        let level = cells
            .iter()
            .map(|&c| filled.data[c as usize])
            .fold(f32::MIN, f32::max);
        let depth = cells
            .iter()
            .map(|&c| level - height.data[c as usize])
            .fold(0.0, f32::max);
        let outlet = cells
            .iter()
            .copied()
            .filter(|&c| lake_of[flow.receiver[c as usize] as usize] != id)
            .min_by(|&a, &b| {
                filled.data[a as usize]
                    .total_cmp(&filled.data[b as usize])
                    .then(a.cmp(&b))
            })
            .unwrap_or(cells[0]);
        lakes.push(Lake {
            level,
            cells,
            depth,
            outlet,
        });
    }
    Lakes { lakes, lake_of }
}

impl Lakes {
    /// The largest lake's area, m².
    pub fn largest_area(&self, spacing: f64) -> f64 {
        self.lakes
            .iter()
            .map(|l| l.area(spacing))
            .fold(0.0, f64::max)
    }

    /// The deepest lake's depth, metres.
    pub fn deepest(&self) -> f32 {
        self.lakes.iter().map(|l| l.depth).fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::drain;
    use forge_task::{PoolConfig, TaskPool};

    #[test]
    fn a_valley_gives_one_river_to_the_border_and_a_fork_gives_orders() {
        // A V-shaped valley along x = 10 falling towards y = 0, the border and the outlet.
        let valley = Field2::from_fn(21, 10.0, |x, y| {
            2.0 * (x as f32 - 10.0).abs() + y as f32 + 1.0
        });
        let pool = TaskPool::new(PoolConfig::with_workers(0));
        let flow = drain(&valley, 0.0, &pool);
        let rivers = trace_rivers(&valley, &flow, 15);
        assert_eq!(rivers.rivers.len(), 1);
        let river = &rivers.rivers[0];
        assert_eq!(river.order, 1);
        assert!(matches!(river.mouth, Mouth::Outlet(_)));
        assert!(
            river
                .cells
                .iter()
                .all(|&c| valley.coords(c as usize).0 == 10)
        );
        // Head first: the rows decrease towards the border, the area grows.
        assert!(river.points.windows(2).all(|w| w[1][1] < w[0][1]));
        assert!(river.area.windows(2).all(|w| w[1] > w[0]));
        assert!(river.length() > 100.0);
        assert!(width(1.0e6) > 4.9 && width(1.0e6) < 5.1);
        // Two valleys meeting: a Y. The trunk takes the larger branch, the other is a
        // tributary joining it, and the junction of two firsts is a second.
        let fork = Field2::from_fn(41, 10.0, |x, y| {
            let (fx, fy) = (x as f32, y as f32);
            let spread = (fy - 20.0).max(0.0) * 0.5;
            let branch = (fx - (20.0 - spread))
                .abs()
                .min((fx - (20.0 + spread)).abs());
            2.0 * branch + fy + 1.0
        });
        let flow = drain(&fork, 0.0, &pool);
        let rivers = trace_rivers(&fork, &flow, 30);
        assert!(rivers.rivers.len() >= 2, "{} rivers", rivers.rivers.len());
        let trunks = rivers
            .rivers
            .iter()
            .filter(|r| matches!(r.mouth, Mouth::Outlet(_)))
            .count();
        assert_eq!(trunks, 1);
        assert!(rivers.max_order() >= 2);
        for (k, river) in rivers.rivers.iter().enumerate() {
            assert!(
                river
                    .cells
                    .iter()
                    .all(|&c| rivers.river_of[c as usize] == k as u32)
            );
            if let Mouth::Junction { river: into, point } = river.mouth {
                let into = &rivers.rivers[into as usize];
                assert!((into.order >= river.order) && (point as usize) < into.cells.len());
                // The tributary's last cell drains into the junction point.
                let last = *river.cells.last().unwrap() as usize;
                assert_eq!(flow.receiver[last], into.cells[point as usize]);
            }
        }
        // Deterministic.
        assert_eq!(trace_rivers(&fork, &flow, 30), rivers);
    }

    #[test]
    fn a_pit_next_to_the_peak_is_one_lake_at_its_spill_level() {
        use crate::flow::{priority_flood, route};
        let mut cone = Field2::from_fn(7, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 3.0, y as f32 - 3.0);
            30.0 - 5.0 * (dx * dx + dy * dy).sqrt()
        });
        cone.set(4, 3, 5.0);
        let filled = priority_flood(&cone, 0.0);
        let flow = route(&filled, 0.0);
        let lakes = trace_lakes(&cone, &filled, &flow, 0.5);
        assert_eq!(lakes.lakes.len(), 1);
        let lake = &lakes.lakes[0];
        assert_eq!(lake.cells, vec![cone.index(4, 3) as u32]);
        assert_eq!(lake.outlet, cone.index(4, 3) as u32);
        assert!((lake.level - filled.get(4, 3)).abs() < 1e-6);
        assert!((lake.depth - (filled.get(4, 3) - 5.0)).abs() < 1e-6);
        assert!((lake.area(10.0) - 100.0).abs() < 1e-6);
        assert_eq!(lakes.lake_of[cone.index(4, 3)], 0);
        assert_eq!(lakes.lake_of.iter().filter(|&&l| l != u32::MAX).count(), 1);
        // No lake without a flood.
        assert!(trace_lakes(&cone, &cone, &flow, 0.5).lakes.is_empty());
    }
}
