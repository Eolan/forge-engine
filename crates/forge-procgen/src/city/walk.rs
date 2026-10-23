//! Walking through a house (the owner, 2026-10-17: "using the nav mesh to test navigations inside
//! the houses, and check if there are no anomaly or obstacles"). The first stage of Recast's
//! pipeline (`research/game-ai-ecosystems.md`, the navigation Forge will write) on each storey of a
//! house with its insides: the floor inside the walls rasterised at [`CELL`], its partitions (open
//! at their doorways), its furniture and its flights blocked, the free cells eroded by a walker's
//! half-width ([`RADIUS`]); then a flood from inside the front door, up each flight from its foot
//! to the landing at its head on the storey above. What the flood does not reach is an anomaly:
//! a room, a flight's foot, a door. The navmesh proper (regions, polygons, the any-angle query) will
//! grow from this grid.

use glam::DVec2;

use super::cores::GOING;
use super::furnish::{self, Piece, Placed};
use super::geom;
use super::interior::{self, InRoom, Interior};
use super::mass::Mass;
use super::rooms::RoomKind;
use super::structure::{self, Opening, OpeningKind, Structure, WallRole};

/// The grid's cell, metres.
pub const CELL: f64 = 0.05;
/// A walker's half-width, metres: shoulders of 0.6 m pass a doorway of 0.9 m with a hand to spare.
pub const RADIUS: f64 = 0.3;
/// How far from a flight's head a walker steps off it, metres.
const STEP: f64 = 0.7;
/// How far from its foot's middle a walker steps on, metres: from the corridor beside it too,
/// where the foot stands against the back wall.
const FOOT: f64 = 1.0;
/// How far into its wall a door's leaves hang, metres (the kit's door, `forge_geom::modules`).
const LEAF_SET: f64 = 0.35;

/// What the walk through a house found unreached.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Walk {
    /// The rooms no walker reaches: kind, storey and middle.
    pub rooms: Vec<(RoomKind, usize, DVec2)>,
    /// The flights whose foot no walker reaches: the storey they climb from and the foot's middle.
    pub stairs: Vec<(usize, DVec2)>,
    /// The ground floor's outside doors (the front's first) no walker reaches from inside the
    /// front door, or, the front's, whose inside is blocked: each its middle on the wall.
    pub doors: Vec<DVec2>,
    /// The doors whose leaf swings into furniture, a flight or a partition: each its middle.
    pub swings: Vec<DVec2>,
    /// The cells walked, all storeys.
    pub reached: usize,
}

/// One storey's grid.
struct Grid {
    origin: DVec2,
    w: usize,
    h: usize,
    /// Whether a walker's middle may stand in each cell.
    free: Vec<bool>,
    /// Each cell's distance to the nearest blocked one, metres (0 where it is blocked).
    dist: Vec<f64>,
    /// Whether the flood reached it.
    seen: Vec<bool>,
}

impl Grid {
    fn at(&self, i: usize) -> DVec2 {
        self.origin + DVec2::new((i % self.w) as f64 + 0.5, (i / self.w) as f64 + 0.5) * CELL
    }

    fn cell(&self, p: DVec2) -> Option<usize> {
        let q = (p - self.origin) / CELL;
        let (x, y) = (q.x.floor(), q.y.floor());
        (x >= 0.0 && y >= 0.0 && (x as usize) < self.w && (y as usize) < self.h)
            .then(|| y as usize * self.w + x as usize)
    }

    /// The free cells within `r` of `p`.
    fn near(&self, p: DVec2, r: f64) -> Vec<usize> {
        let n = (r / CELL).ceil() as i64 + 1;
        let Some(c) = self.cell(p) else {
            return Vec::new();
        };
        let (cx, cy) = ((c % self.w) as i64, (c / self.w) as i64);
        let mut out = Vec::new();
        for y in (cy - n).max(0)..=(cy + n).min(self.h as i64 - 1) {
            for x in (cx - n).max(0)..=(cx + n).min(self.w as i64 - 1) {
                let i = y as usize * self.w + x as usize;
                if self.free[i] && self.at(i).distance(p) <= r {
                    out.push(i);
                }
            }
        }
        out
    }

    /// Floods from `seeds` over the free cells, 8-connected.
    fn flood(&mut self, seeds: &[usize]) {
        let mut stack: Vec<usize> = seeds
            .iter()
            .copied()
            .filter(|&i| self.free[i] && !self.seen[i])
            .collect();
        for &i in &stack {
            self.seen[i] = true;
        }
        while let Some(i) = stack.pop() {
            let (x, y) = ((i % self.w) as i64, (i / self.w) as i64);
            for (dx, dy) in [
                (-1, 0),
                (1, 0),
                (0, -1),
                (0, 1),
                (-1, -1),
                (1, -1),
                (-1, 1),
                (1, 1),
            ] {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= self.w as i64 || ny >= self.h as i64 {
                    continue;
                }
                let j = ny as usize * self.w + nx as usize;
                if self.free[j] && !self.seen[j] {
                    self.seen[j] = true;
                    stack.push(j);
                }
            }
        }
    }
}

/// The flight's run on the plan: its box's foot edge moved along the climb by its goings.
fn run(inner: &Interior, risers: u32) -> Option<[DVec2; 4]> {
    let stair = inner.stair?;
    let up = inner.climb * (f64::from(risers.saturating_sub(1)) * GOING);
    Some([stair[0], stair[1], stair[1] + up, stair[0] + up])
}

/// Storey `k`'s grid: the floor `floor` free where nothing stands, eroded by [`RADIUS`].
fn grid(floor: &[DVec2], inner: &Interior, furniture: &[Placed], k: usize) -> Grid {
    let (lo, hi) = floor.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let origin = lo - DVec2::splat(CELL);
    let w = ((hi.x - origin.x) / CELL).ceil() as usize + 2;
    let h = ((hi.y - origin.y) / CELL).ceil() as usize + 2;
    let partitions: Vec<&interior::Partition> =
        inner.partitions.iter().filter(|p| p.storey == k).collect();
    let pieces: Vec<[DVec2; 4]> = furniture
        .iter()
        .filter(|f| f.storey == k)
        .map(Placed::outline)
        .collect();
    // The flights' runs on this storey: the one climbing from it and the one arriving in it.
    let runs: Vec<[DVec2; 4]> = [k, k.wrapping_sub(1)]
        .iter()
        .filter_map(|&j| inner.risers.get(j).and_then(|&r| run(inner, r)))
        .collect();
    let mut blocked = vec![false; w * h];
    for (i, b) in blocked.iter_mut().enumerate() {
        let p = origin + DVec2::new((i % w) as f64 + 0.5, (i / w) as f64 + 0.5) * CELL;
        *b = !geom::contains(floor, p)
            || partitions.iter().any(|q| {
                let t = (q.b - q.a).normalize_or_zero();
                let u = (p - q.a).dot(t);
                geom::distance_to_segment(p, q.a, q.b) < q.thickness * 0.5
                    && !q.doors.iter().any(|&(d0, d1)| u > d0 && u < d1)
            })
            || pieces.iter().any(|o| geom::contains(o, p))
            || runs.iter().any(|o| geom::contains(o, p));
    }
    // The distance to the nearest blocked cell, in cells (a chamfer's two passes, 1 and √2).
    let mut d: Vec<f64> = blocked
        .iter()
        .map(|&b| if b { 0.0 } else { f64::MAX })
        .collect();
    let s2 = std::f64::consts::SQRT_2;
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let mut v = d[i];
            if x > 0 {
                v = v.min(d[i - 1] + 1.0);
            }
            if y > 0 {
                v = v.min(d[i - w] + 1.0);
                if x > 0 {
                    v = v.min(d[i - w - 1] + s2);
                }
                if x + 1 < w {
                    v = v.min(d[i - w + 1] + s2);
                }
            }
            d[i] = v;
        }
    }
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            let mut v = d[i];
            if x + 1 < w {
                v = v.min(d[i + 1] + 1.0);
            }
            if y + 1 < h {
                v = v.min(d[i + w] + 1.0);
                if x + 1 < w {
                    v = v.min(d[i + w + 1] + s2);
                }
                if x > 0 {
                    v = v.min(d[i + w - 1] + s2);
                }
            }
            d[i] = v;
        }
    }
    Grid {
        origin,
        w,
        h,
        free: d.iter().map(|&v| v * CELL >= RADIUS).collect(),
        dist: d.iter().map(|&v| v * CELL).collect(),
        seen: vec![false; w * h],
    }
}

/// A door's middle on its wall's inner face and the way into the house there.
fn door_inside(s: &Structure, o: &Opening) -> (DVec2, DVec2) {
    let w = &s.walls[o.wall];
    let t = (w.b - w.a).normalize_or_zero();
    let inward = DVec2::new(-t.y, t.x);
    (w.a + t * o.along + inward * w.thickness, inward)
}

/// The walk through house `m` (its structure `s`) among its furniture ([`furnished`]): `None`
/// where it has no insides ([`interior::house`]) or no door on the ground floor.
pub fn house(m: &Mass, s: &Structure) -> Option<Walk> {
    let inner = interior::house(m, s)?;
    let furniture = furnished(m, s, &inner);
    walk(m, s, &inner, &furniture)
}

/// How roomy house `m` is once built and furnished (#285: the owner's wider corridors, doors and
/// rooms): for each room and each storey's hall, the widest circle free of walls, furniture and
/// flights, metres (a wheelchair turns in 1.50 m); and how deep a strip 1.10 m wide stays clear
/// straight in from the front door, metres (a hall 1.20 m wide between walls holds it). `None` where [`house`] has nothing to walk.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Comfort {
    /// Each room's kind, storey, floor area (m²) and widest free circle (m).
    pub rooms: Vec<(RoomKind, usize, f64, f64)>,
    /// Each storey's hall's widest free circle, metres.
    pub halls: Vec<f64>,
    /// The clear depth behind the front door, metres (the law asks 1.70 m, 2.20 m where the door
    /// opens towards the one coming in).
    pub front: f64,
}

/// House `m`'s [`Comfort`], among the furniture [`furnished`] keeps.
pub fn comfort(m: &Mass, s: &Structure) -> Option<Comfort> {
    let inner = interior::house(m, s)?;
    let furniture = furnished(m, s, &inner);
    let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
    let floor = structure::floor(m, &thickness)?;
    let front = s
        .cores
        .iter()
        .find(|c| c.house)
        .and_then(|c| s.openings.get(c.door))?;
    let mut out = Comfort::default();
    for k in 0..s.storeys.len() {
        let g = grid(&floor, &inner, &furniture, k);
        let widest = |outline: &[DVec2]| {
            (0..g.dist.len())
                .filter(|&i| geom::contains(outline, g.at(i)))
                .map(|i| g.dist[i] * 2.0)
                .fold(0.0, f64::max)
        };
        for r in inner.rooms.iter().filter(|r| r.storey == k) {
            let area = geom::signed_area(&r.outline).abs();
            out.rooms.push((r.kind, k, area, widest(&r.outline)));
        }
        out.halls.push(widest(&inner.hall));
        if k == 0 {
            // Step in along the strip until a cell across it is blocked.
            let (p, inward) = door_inside(s, front);
            let across = DVec2::new(inward.y, -inward.x);
            let mut depth = 0.0;
            'strip: while depth < 4.0 {
                let q = p + inward * (depth + 0.05 + CELL * 0.5);
                for j in -11..=11 {
                    let c = q + across * (f64::from(j) * 0.05);
                    if g.cell(c).is_none_or(|i| g.dist[i] <= 0.0) {
                        break 'strip;
                    }
                }
                depth += CELL;
            }
            out.front = depth;
        }
    }
    Some(out)
}

/// The most pieces [`furnished`] takes out of a house.
const PRUNE: usize = 12;

/// House `m`'s furniture ([`furnish::furnish`]) less the pieces that cut a room off: while a
/// walker misses a room, the piece nearest that room's doorways goes (a bed across the way in, a
/// counter closing the passage), [`PRUNE`] at most. The walk vets the furnishing, as a navmesh
/// would a level's props.
pub fn furnished(m: &Mass, s: &Structure, inner: &Interior) -> Vec<Placed> {
    let mut pieces = furnish::furnish(s, inner);
    for _ in 0..PRUNE {
        let Some(w) = walk(m, s, inner, &pieces) else {
            break;
        };
        // The lowest storey's first unreached room: those above an unreached flight follow it.
        let Some(&(_, k, middle)) = w.rooms.first() else {
            break;
        };
        let Some(room) = inner
            .rooms
            .iter()
            .find(|r| r.storey == k && geom::centroid(&r.outline).distance(middle) < 1e-6)
        else {
            break;
        };
        let edge = |p: DVec2| {
            (0..4)
                .map(|i| geom::distance_to_segment(p, room.outline[i], room.outline[(i + 1) % 4]))
                .fold(f64::MAX, f64::min)
        };
        // Its doorways: those of the storey's partitions on its outline.
        let ways: Vec<DVec2> = inner
            .partitions
            .iter()
            .filter(|p| p.storey == k)
            .flat_map(|p| {
                let t = (p.b - p.a).normalize_or_zero();
                p.doors
                    .iter()
                    .map(move |&(d0, d1)| p.a + t * ((d0 + d1) * 0.5))
            })
            .filter(|&d| edge(d) < 0.25)
            .collect();
        let near = |f: &Placed| {
            let o = f.outline();
            ways.iter()
                .map(|&d| {
                    o.iter()
                        .chain(std::iter::once(&f.centre))
                        .map(|&c| c.distance(d))
                        .fold(f64::MAX, f64::min)
                })
                .fold(f.centre.distance(middle), f64::min)
        };
        let Some(j) = (0..pieces.len())
            .filter(|&j| pieces[j].storey == k)
            .min_by(|&a, &b| near(&pieces[a]).total_cmp(&near(&pieces[b])))
        else {
            break;
        };
        pieces.remove(j);
    }
    keep_a_turning_circle(m, s, inner, &mut pieces);
    pieces
}

/// The free circle the law keeps in one bedroom of a home at least, metres: a wheelchair's
/// turn (#285, the arrêté of 24 December 2015).
pub const TURNING: f64 = 1.5;

/// One bedroom at least keeps a [`TURNING`] circle free (#285): where none does, the roomiest
/// gives up its desk and its chair, then its wardrobe, until it does; last, its bed slides
/// against a wall where the walk still reaches every room.
fn keep_a_turning_circle(m: &Mass, s: &Structure, inner: &Interior, pieces: &mut Vec<Placed>) {
    let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
    let Some(floor) = structure::floor(m, &thickness) else {
        return;
    };
    let widest = |pieces: &[Placed], r: &InRoom| {
        let g = grid(&floor, inner, pieces, r.storey);
        (0..g.dist.len())
            .filter(|&i| geom::contains(&r.outline, g.at(i)))
            .map(|i| g.dist[i] * 2.0)
            .fold(0.0, f64::max)
    };
    let bedrooms: Vec<&InRoom> = inner
        .rooms
        .iter()
        .filter(|r| r.kind == RoomKind::Bedroom)
        .collect();
    if bedrooms.iter().any(|r| widest(pieces, r) >= TURNING) {
        return;
    }
    // The roomiest, the first of equals.
    let Some(room) = bedrooms.iter().copied().reduce(|a, b| {
        let area = |r: &InRoom| geom::signed_area(&r.outline).abs();
        if area(b) > area(a) + 1e-9 { b } else { a }
    }) else {
        return;
    };
    for kinds in [&[Piece::Desk, Piece::Chair][..], &[Piece::Wardrobe]] {
        pieces.retain(|p| {
            !(p.storey == room.storey
                && kinds.contains(&p.piece)
                && geom::contains(&room.outline, p.centre))
        });
        if widest(pieces, room) >= TURNING {
            return;
        }
    }
    // Last, its bed slid sideways against a wall, the side that frees the wider circle (#285: a
    // 2.8 m room's single bed stood clear of both walls, a 1.41 m circle beside it).
    let Some(b) = pieces.iter().position(|p| {
        p.storey == room.storey
            && matches!(p.piece, Piece::Bed | Piece::SingleBed)
            && geom::contains(&room.outline, p.centre)
    }) else {
        return;
    };
    let bed = pieces[b];
    let side = DVec2::new(bed.facing.y, -bed.facing.x);
    // How far it slides along `d` to stand a wall's gap off the room's outline: each corner cast
    // along `d` to the outline, the nearest less the gap (its head, against the wall it backs
    // onto, does not hold it back).
    let reach = |d: DVec2| {
        bed.outline()
            .iter()
            .map(|&c| {
                (0..4)
                    .filter_map(|k| {
                        let (p, q) = (room.outline[k], room.outline[(k + 1) % 4]);
                        geom::segment_intersection(c, c + d * 10.0, p, q).map(|(t, _)| t * 10.0)
                    })
                    .filter(|&t| t > 1e-6)
                    .fold(f64::MAX, f64::min)
            })
            .fold(f64::MAX, f64::min)
            .min(2.0)
            - BED_GAP
    };
    let best = [side, -side]
        .into_iter()
        .map(|d| {
            let mut tried = pieces.clone();
            tried[b].centre = bed.centre + d * reach(d).max(0.0);
            (widest(&tried, room), tried)
        })
        .max_by(|x, y| x.0.total_cmp(&y.0));
    // Only where the walk still reaches every room: a bed slid across a doorway would not.
    if let Some((w, tried)) = best
        && w >= TURNING
        && walk(m, s, inner, &tried).is_some_and(|w| w.rooms.is_empty())
    {
        *pieces = tried;
    }
}

/// How far a bed slid to a wall stands off it, metres.
const BED_GAP: f64 = 0.06;

/// The walk through house `m` among `furniture`, its insides `inner`.
fn walk(m: &Mass, s: &Structure, inner: &Interior, furniture: &[Placed]) -> Option<Walk> {
    let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
    let floor = structure::floor(m, &thickness)?;
    // The ground floor's outside doors, the front's first: the core's, else the first on a street.
    let mut doors: Vec<&Opening> = s
        .openings
        .iter()
        .filter(|o| {
            o.kind == OpeningKind::Door
                && o.storey == 0
                && matches!(s.walls[o.wall].role, WallRole::Street | WallRole::Court)
        })
        .collect();
    let front = s
        .cores
        .iter()
        .find(|c| c.house)
        .and_then(|c| s.openings.get(c.door))
        .or_else(|| {
            doors
                .iter()
                .copied()
                .find(|o| s.walls[o.wall].role == WallRole::Street)
        })
        .or_else(|| doors.first().copied())?;
    doors.retain(|o| !std::ptr::eq(*o, front));
    doors.insert(0, front);
    let mut walk = Walk::default();
    let storeys = s.storeys.len();
    let mut seeds: Vec<DVec2> = Vec::new();
    for k in 0..storeys {
        let mut g = grid(&floor, inner, furniture, k);
        if k == 0 {
            let (p, inward) = door_inside(s, front);
            let start = g.near(p + inward * (RADIUS + 0.1), 0.25);
            if start.is_empty() {
                walk.doors.push(p);
                return Some(walk);
            }
            g.flood(&start);
            for o in &doors[1..] {
                let (p, inward) = door_inside(s, o);
                if !g
                    .near(p + inward * (RADIUS + 0.1), 0.25)
                    .iter()
                    .any(|&i| g.seen[i])
                {
                    walk.doors.push(p);
                }
            }
            // Each door's two leaves, half its width each, hung [`LEAF_SET`] into the wall, swing
            // in past its inner face over the door's width, as far as a leaf reaches past it.
            for o in &doors {
                let (p, inward) = door_inside(s, o);
                let reach = LEAF_SET + o.width * 0.5 - s.walls[o.wall].thickness;
                if reach <= 0.02 {
                    continue;
                }
                let t = DVec2::new(inward.y, -inward.x);
                let half = o.width * 0.5 - 0.02;
                let leaf = [
                    p - t * half + inward * 0.02,
                    p + t * half + inward * 0.02,
                    p + t * half + inward * reach,
                    p - t * half + inward * reach,
                ];
                let meets = |r: &[DVec2; 4]| {
                    r.iter().any(|&c| geom::contains(&leaf, c))
                        || leaf.iter().any(|&c| geom::contains(r, c))
                };
                let hit = furniture
                    .iter()
                    .filter(|f| f.storey == 0)
                    .any(|f| meets(&f.outline()))
                    || inner
                        .risers
                        .first()
                        .and_then(|&r| run(inner, r))
                        .is_some_and(|r| meets(&r));
                if hit {
                    walk.swings.push(p);
                }
            }
        } else {
            let start: Vec<usize> = seeds.iter().flat_map(|&p| g.near(p, STEP)).collect();
            g.flood(&start);
        }
        walk.reached += g.seen.iter().filter(|&&v| v).count();
        for r in inner.rooms.iter().filter(|r| r.storey == k) {
            let reached =
                (0..g.free.len()).any(|i| g.seen[i] && geom::contains(&r.outline, g.at(i)));
            if !reached {
                walk.rooms.push((r.kind, k, geom::centroid(&r.outline)));
            }
        }
        // The flight up from this storey: its foot reached, its head the next storey's start.
        seeds.clear();
        if k + 1 < storeys
            && let (Some(stair), Some(&r)) = (inner.stair, inner.risers.get(k))
        {
            let foot = (stair[0] + stair[1]) * 0.5;
            let head = foot + inner.climb * (f64::from(r.saturating_sub(1)) * GOING);
            if g.near(foot, FOOT).iter().any(|&i| g.seen[i]) {
                seeds.push(head);
            } else {
                walk.stairs.push((k, foot));
            }
        }
        if seeds.is_empty() {
            // Nothing above is reached: its rooms count as unreached.
            for r in inner.rooms.iter().filter(|r| r.storey > k) {
                walk.rooms
                    .push((r.kind, r.storey, geom::centroid(&r.outline)));
            }
            break;
        }
    }
    Some(walk)
}

/// Storey `k` of house `m`'s walk grid as text, a line every other row: `#` blocked, `f`
/// furniture, `.` free, `D` inside the front door; for looking into what [`house`] reports.
pub fn map(m: &Mass, s: &Structure, k: usize) -> Option<String> {
    let inner = interior::house(m, s)?;
    let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
    let floor = structure::floor(m, &thickness)?;
    let furniture = furnished(m, s, &inner);
    let g = grid(&floor, &inner, &furniture, k);
    let door = s
        .cores
        .iter()
        .find(|c| c.house)
        .and_then(|c| s.openings.get(c.door))
        .map(|o| {
            let (p, inward) = door_inside(s, o);
            p + inward * (RADIUS + 0.1)
        });
    let mut out = String::new();
    for y in (0..g.h).rev().step_by(2) {
        for x in 0..g.w {
            let i = y * g.w + x;
            let p = g.at(i);
            let c = if door.is_some_and(|d| d.distance(p) < CELL) {
                'D'
            } else if !g.free[i] {
                if furniture
                    .iter()
                    .any(|f| f.storey == k && geom::contains(&f.outline(), p))
                {
                    'f'
                } else {
                    '#'
                }
            } else {
                '.'
            };
            out.push(c);
        }
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::super::{example, structure, village};
    use super::*;

    /// How roomy the village's houses are as built and furnished (#285), by room kind: the widest
    /// free circle's least, median and share of 1.50 m or more, the floor areas, the halls and
    /// the clear depth behind the front doors: `cargo test --release -p forge-procgen
    /// comfort_survey -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn comfort_survey() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village::village(&site, &village::params(size), 1);
        let structures = structure::structures(&v.plan, &v.masses);
        let all: Vec<Comfort> = v
            .masses
            .iter()
            .zip(&structures)
            .filter_map(|(m, s)| comfort(m, s))
            .collect();
        let line = |name: &str, mut x: Vec<f64>, at_least: f64| {
            x.sort_by(f64::total_cmp);
            if x.is_empty() {
                return;
            }
            let share = x.iter().filter(|&&v| v >= at_least).count() as f64 / x.len() as f64;
            println!(
                "{name}: {} of them, least {:.2}, median {:.2}, most {:.2}; {:.0} % at least {at_least}",
                x.len(),
                x[0],
                x[x.len() / 2],
                x[x.len() - 1],
                share * 100.0
            );
        };
        println!("{} houses with insides", all.len());
        let mut kinds: Vec<RoomKind> = Vec::new();
        for r in all.iter().flat_map(|c| c.rooms.iter()) {
            if !kinds.contains(&r.0) {
                kinds.push(r.0);
            }
        }
        for kind in kinds {
            let rooms: Vec<&(RoomKind, usize, f64, f64)> = all
                .iter()
                .flat_map(|c| c.rooms.iter())
                .filter(|r| r.0 == kind)
                .collect();
            line(
                &format!("{kind:?}, free circle (m)"),
                rooms.iter().map(|r| r.3).collect(),
                1.5,
            );
            line(
                &format!("{kind:?}, floor (m²)"),
                rooms.iter().map(|r| r.2).collect(),
                10.5,
            );
        }
        // The law's ask is per home: one bedroom at least with a 1.50 m circle (#285).
        line(
            "each house's roomiest bedroom, free circle (m)",
            all.iter()
                .filter_map(|c| {
                    c.rooms
                        .iter()
                        .filter(|r| r.0 == RoomKind::Bedroom)
                        .map(|r| r.3)
                        .max_by(f64::total_cmp)
                })
                .collect(),
            1.5,
        );
        line(
            "halls, free circle (m)",
            all.iter().flat_map(|c| c.halls.iter().copied()).collect(),
            1.5,
        );
        line(
            "behind the front door, a 1.10 m strip clear (m)",
            all.iter().map(|c| c.front).collect(),
            2.2,
        );
    }

    /// Prints the walk grid of the village's house `FORGE_WALK_HOUSE` on storey
    /// `FORGE_WALK_STOREY` (0 by default): `cargo test -p forge-procgen walk_map -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn walk_map() {
        let i: usize = std::env::var("FORGE_WALK_HOUSE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let k: usize = std::env::var("FORGE_WALK_STOREY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = village::village(&site, &village::params(size), 1);
        let structures = structure::structures(&v.plan, &v.masses);
        println!("{:?}", house(&v.masses[i], &structures[i]));
        println!(
            "{}",
            map(&v.masses[i], &structures[i], k).unwrap_or_default()
        );
    }
}

/// Views for walking the village by eye, as the owner does (`--view=` arguments for `city-plan
/// --village 600`): `cargo test --release -p forge-procgen survey -- --ignored --nocapture`.
#[cfg(test)]
mod survey {
    use super::super::{example, geom, interior, structure, village};
    use glam::DVec2;

    /// The village's size, metres (`--village 600`); the demo's views are about its middle.
    const SIZE: f64 = 600.0;

    /// The yaw, degrees, that looks along `look` (the demo's: 0 along −z, turning towards −x).
    fn yaw(look: DVec2) -> f64 {
        (-look.x).atan2(-look.y).to_degrees()
    }

    /// A street-level view of every tenth house, from 9 m out in front of its first side.
    #[test]
    #[ignore]
    fn survey_views() {
        let site = example::site(SIZE, 10.0);
        let v = village::village(&site, &village::params(SIZE), 1);
        for (i, m) in v.masses.iter().enumerate().step_by(10) {
            let (a, b) = (m.footprint[0], m.footprint[1]);
            let t = (b - a).normalize_or_zero();
            let mid = (a + b) * 0.5;
            let eye = mid + DVec2::new(t.y, -t.x) * 9.0 - t * 4.0;
            let half = SIZE * 0.5;
            println!(
                "survey {i}: --view={:.1},1.7,{:.1},{:.1},5",
                eye.x - half,
                eye.y - half,
                yaw((mid - eye).normalize_or_zero())
            );
        }
    }

    /// A view from the middle of a ground and an upper room of every eighth house with its
    /// insides, looking at the room's first side (its street side).
    #[test]
    #[ignore]
    fn survey_rooms() {
        let site = example::site(SIZE, 10.0);
        let v = village::village(&site, &village::params(SIZE), 1);
        let structures = structure::structures(&v.plan, &v.masses);
        let houses = v
            .masses
            .iter()
            .zip(&structures)
            .enumerate()
            .filter_map(|(i, (m, s))| interior::house(m, s).map(|inner| (i, s, inner)));
        for (i, s, inner) in houses.step_by(8) {
            for k in [0, 1] {
                let Some(r) = inner.rooms.iter().find(|r| r.storey == k) else {
                    continue;
                };
                let c = geom::centroid(&r.outline);
                let front = (r.outline[0] + r.outline[1]) * 0.5;
                let half = SIZE * 0.5;
                println!(
                    "room {i}-{k} {:?}: --view={:.1},{:.1},{:.1},{:.1},-5",
                    r.kind,
                    c.x - half,
                    s.storeys[k].level.max(0.32) + 1.6,
                    c.y - half,
                    yaw((front - c).normalize_or_zero())
                );
            }
        }
    }
}
