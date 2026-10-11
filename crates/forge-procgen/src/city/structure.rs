//! Buildings carry their structure (D-060, step 1; `docs/research/building-structure.md`): each
//! mass's storeys with their slabs, its walls with their thickness and role, the stretches of
//! each wall that face open air storey by storey, the openings fitted there, a door from the
//! street, a passage into each block's inner court, and its gross and net floor area.
//!
//! The façade's outer face lies on the building line and the thickness goes inwards. A wall is
//! open where nothing stands against it: not another mass of the building, not another
//! building 2.5 m off or closer; a lower neighbour covers only the storeys below its roof. The
//! openings go on the open stretches only, so no window is cut by a neighbour (the owner,
//! 2026-10-11: "not realistic at all"), and their bays line up storey over storey.

use std::collections::HashMap;

use glam::DVec2;

use super::facades::{FaceKind, edge_kinds};
use super::mass::Mass;
use super::{CityPlan, DistrictKind, geom};

/// How a building is built: its walls', slabs' and storeys' era.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Era {
    /// An old town's rubble-stone houses and villages.
    OldStone,
    /// The 1800s: cut-stone façades, a noble floor, an attic.
    Nineteenth,
    /// Concrete or brick, insulated: today's perimeter and eco blocks.
    Modern,
}

/// An era's dimensions, metres (the research note's §5 table: part sourced, part estimated).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EraTable {
    /// A street façade's thickness.
    pub facade: f64,
    /// A party wall's whole thickness, shared half and half with the neighbour.
    pub party: f64,
    /// A courtyard or rear wall's thickness.
    pub rear: f64,
    /// A partition's thickness.
    pub partition: f64,
    /// A floor slab with its finish.
    pub slab: f64,
    /// The ground storey's floor to floor under shops, cafés and bars.
    pub ground_shop: f64,
    /// The ground storey's floor to floor under homes.
    pub ground_home: f64,
    /// The upper storeys' floor to floor from the first up; the last repeats.
    pub upper: &'static [f64],
    /// The top storey's floor to floor where it is an attic (three storeys or more).
    pub attic: Option<f64>,
    /// A window bay's width.
    pub bay: f64,
    /// A window's width.
    pub window: f64,
    /// An upper window's sill over its floor.
    pub sill: f64,
    /// The stairs and landings a storey gives up, m².
    pub stairs: f64,
    /// The share of the rest the partitions, shafts and reveals take.
    pub partitions: f64,
}

/// A lift's shaft and landing a storey, m² (EN 81-70 type 2, 1.10 × 1.40 m inside).
pub const LIFT: f64 = 4.5;
/// Lifts from this many storeys (R+3: France, permits since 2019).
pub const LIFT_FROM: usize = 4;
/// Keep an opening this far from either end of its open stretch, metres.
pub const MARGIN: f64 = 0.6;
/// A passage into an inner court: its width and its height, metres (a carriage entrance).
pub const PASSAGE: (f64, f64) = (3.0, 3.6);
/// A door from the street: its width (0.90 m clear at least, #263), metres.
pub const DOOR: f64 = 1.2;

const OLD_STONE: EraTable = EraTable {
    facade: 0.60,
    party: 0.50,
    rear: 0.45,
    partition: 0.10,
    slab: 0.30,
    ground_shop: 3.6,
    ground_home: 3.6,
    upper: &[3.2, 3.1, 3.0, 2.9],
    attic: None,
    bay: 3.3,
    window: 1.0,
    sill: 0.9,
    stairs: 12.0,
    partitions: 0.07,
};

const NINETEENTH: EraTable = EraTable {
    facade: 0.45,
    party: 0.40,
    rear: 0.30,
    partition: 0.10,
    slab: 0.30,
    ground_shop: 4.2,
    ground_home: 4.2,
    upper: &[3.4, 3.1, 2.9],
    attic: Some(2.8),
    bay: 3.0,
    window: 1.2,
    sill: 0.1,
    // The main stairs and the service stairs.
    stairs: 24.0,
    // Partitions, and the chimney flues in them.
    partitions: 0.08,
};

const MODERN: EraTable = EraTable {
    facade: 0.35,
    party: 0.20,
    rear: 0.35,
    partition: 0.07,
    slab: 0.27,
    ground_shop: 4.2,
    ground_home: 3.0,
    upper: &[2.8],
    attic: None,
    bay: 2.7,
    window: 1.4,
    sill: 0.5,
    stairs: 18.0,
    partitions: 0.05,
};

impl Era {
    /// A district's era: the old towns and villages of stone, the centres and seafronts of the
    /// 1800s, the rest modern.
    pub fn of(kind: DistrictKind) -> Self {
        match kind {
            DistrictKind::OldTown | DistrictKind::Village => Era::OldStone,
            DistrictKind::Centre | DistrictKind::Waterfront => Era::Nineteenth,
            _ => Era::Modern,
        }
    }

    /// Its dimensions.
    pub fn table(self) -> &'static EraTable {
        match self {
            Era::OldStone => &OLD_STONE,
            Era::Nineteenth => &NINETEENTH,
            Era::Modern => &MODERN,
        }
    }

    /// The floor to floor of each of `floors` storeys, ground first: a shop's or a home's ground
    /// storey, then the uppers, the last an attic where the era has one.
    pub fn storeys(self, floors: u8, shops: bool) -> Vec<f64> {
        let t = self.table();
        let n = usize::from(floors.max(1));
        (0..n)
            .map(|i| match i {
                0 if shops => t.ground_shop,
                0 => t.ground_home,
                _ if i == n - 1 && n >= 3 && t.attic.is_some() => t.attic.unwrap_or(0.0),
                _ => t.upper[(i - 1).min(t.upper.len() - 1)],
            })
            .collect()
    }
}

/// A storey.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Storey {
    /// Its floor's level over the ground, metres.
    pub level: f64,
    /// Its floor to floor, metres.
    pub height: f64,
    /// The slab over it (the roof's over the last), metres.
    pub slab: f64,
}

impl Storey {
    /// Its clear height, floor to ceiling, metres.
    pub fn clear(&self) -> f64 {
        self.height - self.slab
    }
}

/// What a wall is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WallRole {
    /// On the street.
    Street,
    /// Onto its own courtyard or garden.
    Court,
    /// Against a neighbouring building.
    Party,
    /// Inside the building, against another of its masses.
    Inner,
}

/// A wall: a footprint edge seen from outside, with its thickness inwards.
#[derive(Clone, Debug, PartialEq)]
pub struct Wall {
    /// The footprint edge it stands on (from vertex `edge` to `edge + 1`).
    pub edge: usize,
    /// Its ends on the building line.
    pub a: DVec2,
    /// Its other end.
    pub b: DVec2,
    /// Its thickness, metres.
    pub thickness: f64,
    /// What it is.
    pub role: WallRole,
    /// Per storey, the stretches along it (metres from `a`) that face open air.
    pub open: Vec<Vec<(f64, f64)>>,
}

impl Wall {
    /// Its length, metres.
    pub fn length(&self) -> f64 {
        self.a.distance(self.b)
    }
}

/// What an opening is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpeningKind {
    /// A window.
    Window,
    /// A shop's, a café's or a bar's front.
    Shopfront,
    /// The building's door from the street, step-free.
    Door,
    /// A passage through the building into its block's inner court.
    Passage,
}

/// An opening in a wall.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Opening {
    /// Its wall, an index into the structure's walls.
    pub wall: usize,
    /// Its storey.
    pub storey: usize,
    /// Its middle, metres along the wall from its `a`.
    pub along: f64,
    /// Its width, metres.
    pub width: f64,
    /// Its sill over the storey's floor, metres.
    pub sill: f64,
    /// Its head over the storey's floor, metres.
    pub head: f64,
    /// What it is.
    pub kind: OpeningKind,
}

impl Opening {
    /// Its stretch along the wall, metres from the wall's `a`.
    pub fn span(&self) -> (f64, f64) {
        (self.along - self.width * 0.5, self.along + self.width * 0.5)
    }
}

/// A mass's structure.
#[derive(Clone, Debug, PartialEq)]
pub struct Structure {
    /// Its mass, an index into the masses.
    pub mass: u32,
    /// Its era.
    pub era: Era,
    /// Its storeys, ground first.
    pub storeys: Vec<Storey>,
    /// Its walls, one a footprint edge.
    pub walls: Vec<Wall>,
    /// Its openings.
    pub openings: Vec<Opening>,
    /// Whether it holds its building's stairs (and lift, from [`LIFT_FROM`] storeys).
    pub core: bool,
    /// Its gross floor area, to the walls' outer faces, m².
    pub gross: f64,
    /// Its net floor area: inside the walls, less the stairs, lift, partitions and shafts, m².
    pub net: f64,
}

/// A spatial index of the masses: the cells of `CELL` metres each one's box covers.
struct MassIndex {
    cells: HashMap<(i64, i64), Vec<u32>>,
}

const CELL: f64 = 16.0;

impl MassIndex {
    fn new(masses: &[Mass]) -> Self {
        let mut cells: HashMap<(i64, i64), Vec<u32>> = HashMap::new();
        for (i, m) in masses.iter().enumerate() {
            let (lo, hi) = m.footprint.iter().fold(
                (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                |(lo, hi), &p| (lo.min(p), hi.max(p)),
            );
            let c = |v: f64| (v / CELL).floor() as i64;
            for y in c(lo.y)..=c(hi.y) {
                for x in c(lo.x)..=c(hi.x) {
                    cells.entry((x, y)).or_default().push(i as u32);
                }
            }
        }
        Self { cells }
    }

    /// The masses whose footprint holds `p`.
    fn at<'a>(&'a self, masses: &'a [Mass], p: DVec2) -> impl Iterator<Item = u32> + 'a {
        let key = ((p.x / CELL).floor() as i64, (p.y / CELL).floor() as i64);
        self.cells
            .get(&key)
            .into_iter()
            .flatten()
            .copied()
            .filter(move |&k| geom::contains(&masses[k as usize].footprint, p))
    }
}

/// How finely a wall is probed for what stands against it, metres.
const PROBE_STEP: f64 = 0.5;

/// Each mass's era and storeys, from its district (as [`super::mass::masses`] made it).
fn era_of(plan: &CityPlan, m: &Mass) -> Era {
    let block = plan.lots[m.lot as usize].block as usize;
    Era::of(plan.districts[plan.blocks[block].district].kind)
}

/// The storeys of a mass: their levels from its floors to floor, its slabs by its era.
pub fn storeys(era: Era, heights: &[f64]) -> Vec<Storey> {
    let slab = era.table().slab;
    let mut level = 0.0;
    heights
        .iter()
        .map(|&h| {
            let s = Storey {
                level,
                height: h,
                slab,
            };
            level += h;
            s
        })
        .collect()
}

/// Every mass's structure, in mass order.
pub fn structures(plan: &CityPlan, masses: &[Mass]) -> Vec<Structure> {
    let kinds = edge_kinds(plan, masses);
    let index = MassIndex::new(masses);
    // Each lot's core: in its largest mass.
    let mut core_of: HashMap<u32, (u32, f64)> = HashMap::new();
    for (i, m) in masses.iter().enumerate() {
        let area = geom::signed_area(&m.footprint).abs();
        let e = core_of.entry(m.lot).or_insert((i as u32, area));
        if area > e.1 {
            *e = (i as u32, area);
        }
    }
    let mut out: Vec<Structure> = masses
        .iter()
        .enumerate()
        .map(|(i, m)| {
            let era = era_of(plan, m);
            let core = core_of.get(&m.lot).is_some_and(|c| c.0 == i as u32);
            structure(plan, masses, &index, &kinds[i], i, era, core)
        })
        .collect();
    passages(plan, masses, &mut out);
    out
}

/// One mass's structure, its passage left to [`passages`].
fn structure(
    plan: &CityPlan,
    masses: &[Mass],
    index: &MassIndex,
    kinds: &[Option<FaceKind>],
    i: usize,
    era: Era,
    core: bool,
) -> Structure {
    let m = &masses[i];
    let t = era.table();
    let storeys = storeys(era, &m.storeys);
    let n = m.footprint.len();
    let mut walls = Vec::with_capacity(n);
    for e in 0..n {
        let (a, b) = (m.footprint[e], m.footprint[(e + 1) % n]);
        let role = match kinds.get(e).copied().flatten() {
            Some(FaceKind::Street) => WallRole::Street,
            Some(FaceKind::Court) => WallRole::Court,
            Some(FaceKind::Party) => WallRole::Party,
            None => WallRole::Inner,
        };
        let thickness = match role {
            WallRole::Street => t.facade,
            WallRole::Court => t.rear,
            WallRole::Party => t.party * 0.5,
            WallRole::Inner => 0.0,
        };
        let open = if matches!(role, WallRole::Street | WallRole::Court) {
            open_stretches(plan, masses, index, i, a, b, &storeys)
        } else {
            vec![Vec::new(); storeys.len()]
        };
        walls.push(Wall {
            edge: e,
            a,
            b,
            thickness,
            role,
            open,
        });
    }
    let openings = openings(m, era, &storeys, &walls);
    let (gross, net) = areas(m, era, &walls, core);
    Structure {
        mass: i as u32,
        era,
        storeys,
        walls,
        openings,
        core,
        gross,
        net,
    }
}

/// Per storey, the stretches of wall `a b` of mass `i` that face open air: probed every
/// [`PROBE_STEP`], 0.75 m out (another mass against it) and 2.5 m out (another building that
/// close leaves a slit, not a view), each covering the storeys under its roof.
fn open_stretches(
    plan: &CityPlan,
    masses: &[Mass],
    index: &MassIndex,
    i: usize,
    a: DVec2,
    b: DVec2,
    storeys: &[Storey],
) -> Vec<Vec<(f64, f64)>> {
    let length = a.distance(b);
    let t = (b - a) / length.max(1e-9);
    let out = DVec2::new(t.y, -t.x);
    let me = &masses[i];
    let lot = me.lot;
    let block = plan.lots[lot as usize].block;
    let steps = (length / PROBE_STEP).ceil().max(1.0) as usize;
    let step = length / steps as f64;
    // How high what stands against each step reaches.
    let cover: Vec<f64> = (0..steps)
        .map(|k| {
            let p = a + t * ((k as f64 + 0.5) * step);
            let near = index
                .at(masses, p + out * 0.75)
                .filter(|&j| j as usize != i)
                .map(|j| masses[j as usize].height);
            let slit = index
                .at(masses, p + out * 2.5)
                .filter(|&j| {
                    let o = &masses[j as usize];
                    o.lot != lot && plan.lots[o.lot as usize].block == block
                })
                .map(|j| masses[j as usize].height);
            near.chain(slit).fold(0.0, f64::max)
        })
        .collect();
    storeys
        .iter()
        .map(|s| {
            let mut runs = Vec::new();
            let mut start = None;
            for (k, &c) in cover.iter().enumerate() {
                let open = c <= s.level + 0.01;
                match (open, start) {
                    (true, None) => start = Some(k),
                    (false, Some(k0)) => {
                        runs.push((k0 as f64 * step, k as f64 * step));
                        start = None;
                    }
                    _ => {}
                }
            }
            if let Some(k0) = start {
                runs.push((k0 as f64 * step, length));
            }
            runs
        })
        .collect()
}

/// The windows and shopfronts on a mass's open stretches, its bays centred on each stretch and
/// kept [`MARGIN`] off its ends, so they line up storey over storey; and its door, in the
/// middle bay of its longest street wall's ground storey.
fn openings(m: &Mass, era: Era, storeys: &[Storey], walls: &[Wall]) -> Vec<Opening> {
    let t = era.table();
    let shops = m.lively();
    let mut out = Vec::new();
    for (w, wall) in walls.iter().enumerate() {
        if !matches!(wall.role, WallRole::Street | WallRole::Court) {
            continue;
        }
        for (k, s) in storeys.iter().enumerate() {
            for &(lo, hi) in &wall.open[k] {
                let room = hi - lo - 2.0 * MARGIN;
                let bays = (room / t.bay).floor();
                if bays < 1.0 && room < t.window {
                    continue;
                }
                let bays = bays.max(1.0) as usize;
                let bay = (room / bays as f64).min(t.bay * 1.5);
                let first = lo + (hi - lo - bay * bays as f64) * 0.5;
                for j in 0..bays {
                    let along = first + bay * (j as f64 + 0.5);
                    let (kind, width, sill, head) = if k == 0 {
                        if shops && wall.role == WallRole::Street {
                            // Widths by the 10 cm, so the kit shares their meshes.
                            let width = ((bay - 0.6) * 10.0).floor() / 10.0;
                            (OpeningKind::Shopfront, width, 0.3, s.clear() - 0.5)
                        } else {
                            (OpeningKind::Window, t.window, 1.0, s.clear() - 0.4)
                        }
                    } else {
                        (
                            OpeningKind::Window,
                            t.window.min(bay - 0.6),
                            t.sill,
                            s.clear() - 0.35,
                        )
                    };
                    out.push(Opening {
                        wall: w,
                        storey: k,
                        along,
                        width,
                        sill,
                        head,
                        kind,
                    });
                }
            }
        }
    }
    // The door: the ground opening nearest the middle of the longest open street stretch.
    let best = walls
        .iter()
        .enumerate()
        .filter(|(_, w)| w.role == WallRole::Street)
        .flat_map(|(i, w)| w.open[0].iter().map(move |&(lo, hi)| (i, lo, hi)))
        .max_by(|x, y| (x.2 - x.1).total_cmp(&(y.2 - y.1)));
    if let Some((w, lo, hi)) = best {
        let middle = (lo + hi) * 0.5;
        let door = out
            .iter()
            .enumerate()
            .filter(|(_, o)| o.wall == w && o.storey == 0)
            .min_by(|x, y| {
                (x.1.along - middle)
                    .abs()
                    .total_cmp(&(y.1.along - middle).abs())
            })
            .map(|(k, _)| k);
        if let Some(k) = door {
            let clear = storeys[0].clear();
            out[k] = Opening {
                kind: OpeningKind::Door,
                width: DOOR,
                sill: 0.0,
                head: (clear - 0.4).min(2.6),
                ..out[k]
            };
        }
    }
    out
}

/// A mass's gross and net floor area: gross to the outer faces on every storey; net inside the
/// walls (each edge moved in by its wall's thickness), less its building's stairs and lift on
/// every storey of the mass holding them, less the era's share for partitions, shafts and
/// reveals.
fn areas(m: &Mass, era: Era, walls: &[Wall], core: bool) -> (f64, f64) {
    let t = era.table();
    let floors = m.storeys.len() as f64;
    let outline = geom::signed_area(&m.footprint).abs();
    let thickness: Vec<f64> = walls.iter().map(|w| w.thickness).collect();
    let inside = geom::inset_by(&m.footprint, &thickness)
        .map(|p| geom::signed_area(&p).abs())
        .unwrap_or_else(|| {
            let wall_area: f64 = walls.iter().map(|w| w.length() * w.thickness).sum();
            (outline - wall_area).max(0.0)
        });
    let circulation = if core && m.storeys.len() >= 2 {
        t.stairs
            + if m.storeys.len() >= LIFT_FROM {
                LIFT
            } else {
                0.0
            }
    } else {
        0.0
    };
    let net = ((inside - circulation).max(0.0) * (1.0 - t.partitions)) * floors;
    (outline * floors, net)
}

/// A passage from the street into each block's inner court: through the mass whose street wall
/// lies across from its court wall with the longest open ground stretch, on both walls, the
/// ground openings it meets taken out. A block with an open gap between its buildings needs
/// none.
fn passages(plan: &CityPlan, masses: &[Mass], structures: &mut [Structure]) {
    let mut by_block: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, m) in masses.iter().enumerate() {
        by_block
            .entry(plan.lots[m.lot as usize].block)
            .or_default()
            .push(i);
    }
    let open = reached(plan, masses);
    let mut blocks: Vec<u32> = by_block.keys().copied().collect();
    blocks.sort_unstable();
    for b in blocks {
        if plan.blocks[b as usize].courtyard.is_none() || open.contains(&b) {
            continue;
        }
        // (mass, street wall, court wall, the passage's middle on each).
        let mut best: Option<(usize, usize, usize, f64, f64, f64)> = None;
        for &i in &by_block[&b] {
            let s = &structures[i];
            for (sw, street) in s.walls.iter().enumerate() {
                if street.role != WallRole::Street {
                    continue;
                }
                for &(lo, hi) in &street.open[0] {
                    if hi - lo < PASSAGE.0 + 2.0 * MARGIN {
                        continue;
                    }
                    let mid = (lo + hi) * 0.5;
                    let p = street.a + (street.b - street.a).normalize() * mid;
                    // The court wall across: the nearest point of a court wall, open there.
                    for (cw, court) in s.walls.iter().enumerate() {
                        if court.role != WallRole::Court {
                            continue;
                        }
                        let along = geom::closest_on_segment(p, court.a, court.b) * court.length();
                        let fits = court.open[0].iter().any(|&(l, h)| {
                            along - PASSAGE.0 * 0.5 >= l && along + PASSAGE.0 * 0.5 <= h
                        });
                        if fits && best.is_none_or(|x| hi - lo > x.5) {
                            best = Some((i, sw, cw, mid, along, hi - lo));
                        }
                    }
                }
            }
        }
        let Some((i, sw, cw, at_street, at_court, _)) = best else {
            continue;
        };
        let s = &mut structures[i];
        // Under its storey's ceiling with a lintel, as the other openings: no higher than 3.6 m.
        let head = PASSAGE.1.min(s.storeys[0].clear() - 0.3);
        for (w, along) in [(sw, at_street), (cw, at_court)] {
            let (lo, hi) = (along - PASSAGE.0 * 0.5 - 0.3, along + PASSAGE.0 * 0.5 + 0.3);
            s.openings
                .retain(|o| !(o.wall == w && o.storey == 0 && o.span().1 > lo && o.span().0 < hi));
            s.openings.push(Opening {
                wall: w,
                storey: 0,
                along,
                width: PASSAGE.0,
                sill: 0.0,
                head,
                kind: OpeningKind::Passage,
            });
        }
    }
}

/// The built blocks with an inner court, and how many of them are reached from a street: by a
/// passage through one of their buildings, or by a gap between them.
pub fn court_access(plan: &CityPlan, masses: &[Mass], structures: &[Structure]) -> (usize, usize) {
    let open = reached(plan, masses);
    let mut blocks: HashMap<u32, bool> = HashMap::new();
    for s in structures {
        let b = plan.lots[masses[s.mass as usize].lot as usize].block;
        let passage = s.openings.iter().any(|o| o.kind == OpeningKind::Passage);
        *blocks.entry(b).or_insert(false) |= passage;
    }
    let courts: Vec<u32> = blocks
        .keys()
        .copied()
        .filter(|&b| plan.blocks[b as usize].courtyard.is_some())
        .collect();
    let reached = courts
        .iter()
        .filter(|&&b| blocks[&b] || open.contains(&b))
        .count();
    (courts.len(), reached)
}

/// The blocks whose court is reached without a passage: a lot on their streets left open (a gap
/// between their buildings).
fn reached(plan: &CityPlan, masses: &[Mass]) -> std::collections::HashSet<u32> {
    let built: std::collections::HashSet<u32> = masses.iter().map(|m| m.lot).collect();
    plan.lots
        .iter()
        .enumerate()
        .filter(|(i, l)| l.frontage && !built.contains(&(*i as u32)))
        .map(|(_, l)| l.block)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::city::{example, mass, plan};

    fn example_city() -> (CityPlan, Vec<Mass>) {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = mass::masses(&city, &crate::city::bible::StyleBible::default(), 7);
        (city, masses)
    }

    #[test]
    fn storeys_add_up_to_the_height_by_era() {
        for era in [Era::OldStone, Era::Nineteenth, Era::Modern] {
            for floors in 1..=12 {
                for shops in [false, true] {
                    let heights = era.storeys(floors, shops);
                    assert_eq!(heights.len(), usize::from(floors));
                    let s = storeys(era, &heights);
                    let top = s.last().map_or(0.0, |s| s.level + s.height);
                    assert!((top - heights.iter().sum::<f64>()).abs() < 1e-9);
                    // Every storey at least the legal 2.20 m clear (France's décence decree).
                    assert!(s.iter().all(|s| s.clear() >= 2.2), "{era:?} {floors}");
                }
            }
        }
        // The 1800s: a noble floor over the ground, an attic on top.
        let h = Era::Nineteenth.storeys(6, true);
        assert_eq!(h[1], 3.4);
        assert_eq!(h[5], 2.8);
    }

    #[test]
    fn walls_openings_and_areas_add_up() {
        let (city, masses) = example_city();
        let all = structures(&city, &masses);
        assert_eq!(all.len(), masses.len());
        let index = MassIndex::new(&masses);
        let mut windows = 0;
        let (mut gross, mut net) = (HashMap::new(), HashMap::new());
        for s in &all {
            let m = &masses[s.mass as usize];
            // The mass's height is its storeys'.
            let top = s.storeys.last().map_or(0.0, |t| t.level + t.height);
            assert!(
                (top - m.height).abs() < 1e-6,
                "mass {} {top} {}",
                s.mass,
                m.height
            );
            *gross.entry(s.era).or_insert(0.0) += s.gross;
            *net.entry(s.era).or_insert(0.0) += s.net;
            for o in &s.openings {
                let wall = &s.walls[o.wall];
                let storey = &s.storeys[o.storey];
                // Inside its wall and its storey.
                assert!(o.span().0 >= -1e-6 && o.span().1 <= wall.length() + 1e-6);
                assert!(o.sill >= 0.0 && o.head > o.sill && o.head <= storey.clear() + 1e-6);
                // No opening where another building stands against the wall at its storey.
                let t = (wall.b - wall.a).normalize();
                let out = DVec2::new(t.y, -t.x);
                for f in [0.1, 0.5, 0.9] {
                    let p = wall.a + t * (o.span().0 + (o.span().1 - o.span().0) * f);
                    let blocked = index
                        .at(&masses, p + out * 0.75)
                        .filter(|&j| j != s.mass)
                        .any(|j| masses[j as usize].height > storey.level + 0.01);
                    assert!(!blocked, "an opening of mass {} faces another mass", s.mass);
                }
                if o.kind == OpeningKind::Window {
                    windows += 1;
                }
            }
        }
        assert!(windows > 100_000, "{windows} windows");
        // Net over gross by era (the research note: about 0.80 modern, 0.70–0.75 old stone).
        for (era, (lo, hi)) in [
            (Era::Modern, (0.74, 0.86)),
            (Era::Nineteenth, (0.70, 0.84)),
            (Era::OldStone, (0.62, 0.78)),
        ] {
            if let (Some(g), Some(n)) = (gross.get(&era), net.get(&era)) {
                let r = n / g;
                assert!((lo..=hi).contains(&r), "{era:?} net over gross {r:.3}");
            }
        }
    }

    #[test]
    fn every_inner_court_is_reached_from_a_street() {
        let (city, masses) = example_city();
        let all = structures(&city, &masses);
        let (courts, reached) = court_access(&city, &masses, &all);
        assert!(courts > 100, "{courts} courtyards");
        // Nearly all; the rest are courts no mass lies between street and court for.
        assert!(
            (courts - reached) * 100 <= courts,
            "{} of {courts} courts unreached",
            courts - reached
        );
    }
}
