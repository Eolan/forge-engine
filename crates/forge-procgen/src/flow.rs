//! Water over a heightfield: priority flood (Barnes, Lehman & Mullen 2014) fills every
//! depression to its spill level with an ε slope towards the outlet, so each land cell has a
//! lower neighbour; D8 receivers take the steepest of the eight (ties to the lowest index);
//! the stack orders the cells downstream first (Braun & Willett 2013), so an implicit erosion
//! step visits a cell after its receiver; and the drainage area is an integer count of the
//! cells upstream. Every choice is pinned, so the same field gives the same bytes on every
//! machine (D-016).

use std::cmp::Reverse;
use std::collections::BinaryHeap;

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

/// Where the water goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Flow {
    /// Per cell, the cell it drains into (itself for an outlet: the sea and the border).
    pub receiver: Vec<u32>,
    /// Every cell, each after its receiver: outlets first, then upstream.
    pub stack: Vec<u32>,
    /// Per cell, how many cells drain through it, itself included.
    pub area: Vec<u32>,
    /// Per cell, the distance to its receiver, metres (0 for an outlet).
    pub distance: Vec<f32>,
}

/// A heap key ordered by height then index, lowest first through `Reverse`.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Key(u32, u32);

/// The bits of a non-negative float in height order (negative heights are ordered too, by
/// flipping them below the positives).
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

/// D8 routing over a flooded field: receivers, the downstream-first stack, drainage areas.
pub fn route(filled: &Field2<f32>, sea_level: f32) -> Flow {
    let n = filled.size;
    let count = filled.len();
    let mut receiver = vec![0_u32; count];
    let mut distance = vec![0.0_f32; count];
    for i in 0..count {
        let (x, y) = filled.coords(i);
        let h = filled.data[i];
        receiver[i] = i as u32;
        if h <= sea_level || filled.on_border(x, y) {
            continue;
        }
        let mut best_slope = 0.0_f32;
        for (dx, dy, d) in NEIGHBOURS {
            let (nx, ny) = (x as i32 + dx, y as i32 + dy);
            if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                continue;
            }
            let j = filled.index(nx as u32, ny as u32);
            let slope = (h - filled.data[j]) / d;
            // The steepest; at equal slope the lowest index (the neighbour order is fixed).
            if slope > best_slope
                || (slope == best_slope && slope > 0.0 && (j as u32) < receiver[i])
            {
                best_slope = slope;
                receiver[i] = j as u32;
                distance[i] = d * filled.spacing as f32;
            }
        }
    }
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
    // The stack: from every outlet, its donors, theirs, and so on (depth first, index order).
    let mut stack = Vec::with_capacity(count);
    let mut pending = Vec::new();
    for (i, &r) in receiver.iter().enumerate() {
        if r as usize == i {
            pending.push(i as u32);
            while let Some(c) = pending.pop() {
                stack.push(c);
                let (from, to) = (
                    donor_count[c as usize] as usize,
                    donor_count[c as usize + 1] as usize,
                );
                for &d in donors[from..to].iter().rev() {
                    pending.push(d);
                }
            }
        }
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
        area,
        distance,
    }
}

impl Flow {
    /// Whether cell `i` is an outlet (drains nowhere: the sea or the border).
    pub fn is_outlet(&self, i: usize) -> bool {
        self.receiver[i] as usize == i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 7 × 7 cone rising to the centre, with a pit dug next to the peak.
    fn cone_with_pit() -> Field2<f32> {
        let mut f = Field2::from_fn(7, 10.0, |x, y| {
            let (dx, dy) = (x as f32 - 3.0, y as f32 - 3.0);
            30.0 - 5.0 * (dx * dx + dy * dy).sqrt()
        });
        f.set(4, 3, 5.0); // a pit: lower than every neighbour
        f
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
        // The peak drains to the border, every cell reaches an outlet, and the stack visits a
        // receiver before its donors.
        let mut c = field.index(3, 3);
        let mut hops = 0;
        while !flow.is_outlet(c) {
            c = flow.receiver[c] as usize;
            hops += 1;
            assert!(hops < 49);
        }
        let mut position = vec![usize::MAX; 49];
        for (k, &cell) in flow.stack.iter().enumerate() {
            position[cell as usize] = k;
        }
        assert_eq!(flow.stack.len(), 49);
        for i in 0..49 {
            assert!(position[flow.receiver[i] as usize] <= position[i]);
        }
        // The areas add up: every cell drains to an outlet, and the outlets' areas sum to 49.
        let outlets: u32 = (0..49)
            .filter(|&i| flow.is_outlet(i))
            .map(|i| flow.area[i])
            .sum();
        assert_eq!(outlets, 49);
        assert!(flow.area[field.index(3, 3)] == 1);
        // The same field gives the same flow.
        assert_eq!(route(&filled, 0.0), flow);
    }
}
