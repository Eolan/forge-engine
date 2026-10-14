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

use super::cores::{self, Core};
use super::facades::{FaceKind, edge_kinds};
use super::mass::Mass;
use super::units::{self, Landing, Unit};
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
    /// The share of the floor inside the walls and outside the cores that the partitions,
    /// shafts and reveals take.
    pub partitions: f64,
}

/// Lifts from this many storeys (R+3: France, permits since 2019).
pub const LIFT_FROM: usize = 4;
/// Keep an opening this far from either end of its open stretch, metres.
pub const MARGIN: f64 = 0.6;
/// A window's surround, which its shutters fold back beyond, metres
/// (`forge_geom::modules`: each leaf half the window's width).
pub const SHUTTER_BESIDE: f64 = 0.15;
/// The wall left between a window's folded shutter and a corner or a stretch's end, metres:
/// there is always some (the owner, a shutter wrapped round a building's corner).
pub const PIER: f64 = 0.4;
/// The wall left between two neighbouring windows' folded shutters, metres.
const SHUTTER_GAP: f64 = 0.1;
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
    // Partitions, the chimney flues in them, and the service stairs.
    partitions: 0.10,
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
    /// Its stair cores ([`super::cores`]): none on one storey, or in a small wing reached from
    /// its building's other mass.
    pub cores: Vec<Core>,
    /// Its flats, shops and store rooms, storey by storey ([`super::units`]).
    pub units: Vec<Unit>,
    /// Its storeys' lobbies and corridors, where the flats' doors open.
    pub landings: Vec<Landing>,
    /// Its gross floor area, to the walls' outer faces, m².
    pub gross: f64,
    /// Its net floor area: inside the walls, less the cores, partitions and shafts, m².
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
    let mut out: Vec<Structure> = masses
        .iter()
        .enumerate()
        .map(|(i, m)| structure(plan, masses, &index, &kinds[i], i, era_of(plan, m), &[]))
        .collect();
    // The passages into the courts: a core standing in a passage's way moves aside (it took the
    // only place for one, and a court went unreached).
    for (i, mouths) in passages(plan, masses, &out) {
        let avoid: Vec<(usize, f64, f64)> = mouths
            .iter()
            .map(|&(w, along)| (w, along - PASSAGE_CLEAR, along + PASSAGE_CLEAR))
            .collect();
        let blocked = out[i].cores.iter().any(|c| {
            avoid
                .iter()
                .any(|&(w, lo, hi)| c.wall == w && c.span.0 < hi && c.span.1 > lo)
        });
        if blocked {
            let era = era_of(plan, &masses[i]);
            out[i] = structure(plan, masses, &index, &kinds[i], i, era, &avoid);
        }
        open_passage(&mut out[i], mouths);
    }
    // A court door that opens onto a spot no one can walk to (a light well between the
    // building's wings, a garden closed by its neighbours) goes back to a window.
    let (doors, _) = unreached(plan, masses, &out);
    for (m, k) in doors {
        let s = &mut out[m as usize];
        let o = s.openings[k];
        // A street door too, onto a pocket the block's buildings close, while its building keeps
        // another door.
        let doors_left = s
            .openings
            .iter()
            .filter(|q| q.kind == OpeningKind::Door)
            .count();
        if s.walls[o.wall].role == WallRole::Court || doors_left > 1 {
            let t = s.era.table();
            s.openings[k] = Opening {
                kind: OpeningKind::Window,
                width: t.window,
                sill: 1.0,
                head: s.storeys[0].clear() - 0.4,
                ..o
            };
            // A core behind it is shut off with it.
            s.cores.retain(|c| c.door != k);
        }
    }
    // The windows the doors and the cores gave back, at their era's width, fitted to the room
    // their shutters have like the others.
    for s in &mut out {
        fit_shutters(s);
    }
    // The flats round the cores, once the doors and the passages are settled, and the net area
    // again (a dropped core gave its floor back).
    for (i, s) in out.iter_mut().enumerate() {
        let m = &masses[i];
        let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
        let inside = floor(m, &thickness);
        (s.gross, s.net) = areas(m, s.era, &s.walls, inside.as_deref(), &s.cores);
        if let Some(f) = &inside {
            let seed = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            let (u, l) = units::units(m, s, f, s.era.table().partitions, seed);
            (s.units, s.landings) = (u, l);
        }
    }
    out
}

/// One mass's structure, its passage left to [`passages`]; its cores clear of the stretches
/// `avoid` (wall, from, to) a passage keeps.
fn structure(
    plan: &CityPlan,
    masses: &[Mass],
    index: &MassIndex,
    kinds: &[Option<FaceKind>],
    i: usize,
    era: Era,
    avoid: &[(usize, f64, f64)],
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
    let mut openings = openings(m, era, &storeys, &walls);
    // The floor inside the walls, each edge moved in by its wall's thickness.
    let thickness: Vec<f64> = walls.iter().map(|w| w.thickness).collect();
    let inside = floor(m, &thickness);
    let plate = inside.as_ref().map_or(0.0, |p| geom::signed_area(p).abs());
    let cores = match &inside {
        Some(floor) if cores::needs_cores(m, &walls, plate) => {
            cores::place(m, &storeys, &walls, &mut openings, floor, t.window, avoid)
        }
        _ => Vec::new(),
    };
    // The street door where no core took one (a shop, a house, a wing), and the court's.
    if cores.is_empty() {
        add_door(m, &storeys, &walls, &mut openings, WallRole::Street);
    }
    add_door(m, &storeys, &walls, &mut openings, WallRole::Court);
    let (gross, net) = areas(m, era, &walls, inside.as_deref(), &cores);
    Structure {
        mass: i as u32,
        era,
        storeys,
        walls,
        openings,
        cores,
        units: Vec::new(),
        landings: Vec::new(),
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
                // Off the ends by the window's folded shutter and a pier, at least.
                let end = MARGIN.max(t.window * 0.5 + PIER);
                let room = hi - lo - 2.0 * end;
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
                            (
                                OpeningKind::Shopfront,
                                width,
                                outside(m, wall.role),
                                s.clear() - 0.5,
                            )
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
                    // Room for a window's shutters folded back either side: a pier off the
                    // stretch's ends, a gap to its neighbours' (they reached round corners and
                    // over each other).
                    let width = if kind == OpeningKind::Window {
                        let ends = (along - lo).min(hi - along) - PIER - SHUTTER_BESIDE;
                        let between = if bays > 1 {
                            (bay - 2.0 * SHUTTER_BESIDE - SHUTTER_GAP) * 0.5
                        } else {
                            f64::INFINITY
                        };
                        let fit = width.min(ends).min(between);
                        // Widths by the 10 cm, so the kit shares their meshes.
                        let fit = (fit * 10.0).floor() / 10.0;
                        if fit < 0.6 {
                            continue;
                        }
                        fit
                    } else {
                        width
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
    out
}

/// Each window of `s` as wide as its shutters, folded back either side, leave a [`PIER`] to its
/// stretch's ends and [`SHUTTER_GAP`] to its neighbours (their shutters, or the other openings'
/// edges), left to right, 0.6 m at least; widths by the 10 cm, so the kit shares their meshes.
fn fit_shutters(s: &mut Structure) {
    let walls = &s.walls;
    let mut order: Vec<usize> = (0..s.openings.len()).collect();
    order.sort_by(|&a, &b| {
        let (p, q) = (&s.openings[a], &s.openings[b]);
        (p.wall, p.storey)
            .cmp(&(q.wall, q.storey))
            .then(p.along.total_cmp(&q.along))
    });
    // How far an opening reaches either side of its middle.
    let reach = |o: &Opening| {
        if o.kind == OpeningKind::Window {
            o.width + SHUTTER_BESIDE
        } else {
            o.width * 0.5
        }
    };
    for (n, &i) in order.iter().enumerate() {
        let o = s.openings[i];
        if o.kind != OpeningKind::Window {
            continue;
        }
        let same = |j: usize| {
            let q = &s.openings[j];
            q.wall == o.wall && q.storey == o.storey
        };
        let Some(&(lo, hi)) = walls[o.wall].open[o.storey]
            .iter()
            .find(|&&(lo, hi)| lo <= o.along && o.along <= hi)
        else {
            continue;
        };
        let mut left = lo + PIER;
        if n > 0 && same(order[n - 1]) {
            let p = &s.openings[order[n - 1]];
            left = left.max(p.along + reach(p) + SHUTTER_GAP);
        }
        let mut right = hi - PIER;
        if n + 1 < order.len() && same(order[n + 1]) {
            let q = &s.openings[order[n + 1]];
            // The next window shares the wall between them.
            let theirs = if q.kind == OpeningKind::Window {
                SHUTTER_BESIDE + (q.along - o.along - 2.0 * SHUTTER_BESIDE - SHUTTER_GAP) * 0.5
            } else {
                q.width * 0.5 + SHUTTER_GAP
            };
            right = right.min(q.along - theirs);
        }
        let fit = (o.along - left).min(right - o.along) - SHUTTER_BESIDE;
        let fit = ((fit.min(o.width) * 10.0).floor() / 10.0).max(0.6);
        s.openings[i].width = fit;
    }
}

/// A door on a wall of `role`: the ground window or shopfront nearest the middle of that side's
/// longest open stretch. The street's where no core took one; the court's to the stairs (the
/// owner: no door onto the court).
fn add_door(m: &Mass, storeys: &[Storey], walls: &[Wall], out: &mut [Opening], role: WallRole) {
    let best = walls
        .iter()
        .enumerate()
        .filter(|(_, w)| w.role == role)
        .flat_map(|(i, w)| w.open[0].iter().map(move |&(lo, hi)| (i, lo, hi)))
        .max_by(|x, y| (x.2 - x.1).total_cmp(&(y.2 - y.1)));
    let Some((w, lo, hi)) = best else {
        return;
    };
    let middle = (lo + hi) * 0.5;
    let door = out
        .iter()
        .enumerate()
        .filter(|(_, o)| o.wall == w && o.storey == 0 && o.kind != OpeningKind::Door)
        .min_by(|x, y| {
            (x.1.along - middle)
                .abs()
                .total_cmp(&(y.1.along - middle).abs())
        })
        .map(|(k, _)| k);
    if let Some(k) = door {
        out[k] = cores::door(m, storeys, walls, &out[k]);
    }
}

/// The ground's level before a wall of mass `m`, metres: the walking level, everywhere people
/// walk (D-061), pavements, streets and courts alike. A door's sill and a shopfront's stand on
/// it: at 0 they sank under the pavement (the owner: buildings need foundations at the
/// pavement's or the ground's height).
pub fn outside(_m: &Mass, _role: WallRole) -> f64 {
    super::ground::WALK_LEVEL
}

/// A mass's floor inside its walls: each edge moved in by its wall's `thickness`, dropping the
/// short edges the move turns over (a footprint's chamfers and kinks; without, a tenth of the
/// buildings had no floor and so no stairs).
/// The inside walls between a building's masses count 5 cm: at nothing the inset refused them.
pub fn floor(m: &Mass, thickness: &[f64]) -> Option<Vec<DVec2>> {
    let d: Vec<f64> = thickness.iter().map(|&t| t.max(0.05)).collect();
    geom::inset_by(&m.footprint, &d).or_else(|| geom::inset_by_collapsing(&m.footprint, &d))
}

/// A mass's gross and net floor area: gross to the outer faces on every storey; net inside the
/// walls (`inside`, the floor with each edge moved in by its wall's thickness), less its cores
/// on every storey, less the era's share for partitions, shafts and reveals.
fn areas(
    m: &Mass,
    era: Era,
    walls: &[Wall],
    inside: Option<&[DVec2]>,
    cores: &[Core],
) -> (f64, f64) {
    let t = era.table();
    let floors = m.storeys.len() as f64;
    let outline = geom::signed_area(&m.footprint).abs();
    let inside = inside.map_or_else(
        || {
            let wall_area: f64 = walls.iter().map(|w| w.length() * w.thickness).sum();
            (outline - wall_area).max(0.0)
        },
        |p| geom::signed_area(p).abs(),
    );
    let circulation: f64 = cores.iter().map(Core::area).sum();
    let net = ((inside - circulation).max(0.0) * (1.0 - t.partitions)) * floors;
    (outline * floors, net)
}

/// A passage into a court through a mass: its mass, and its mouths, (wall, middle) on its
/// street wall then on its court wall.
type Passage = (usize, [(usize, f64); 2]);

/// The passages from the street into each block's inner court: through the mass whose street
/// wall lies across from its court wall with the longest open ground stretch, on both walls. A
/// block with an open gap between its buildings needs none.
fn passages(plan: &CityPlan, masses: &[Mass], structures: &[Structure]) -> Vec<Passage> {
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
    let mut out = Vec::new();
    for b in blocks {
        if plan.blocks[b as usize].courtyard.is_none() || open.contains(&b) {
            continue;
        }
        // (mass, street wall, court wall, the passage's middle on each, the stretch's length).
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
                        // Straight across if it is open there, else the nearest place of an open
                        // stretch wide enough, the passage running slant through the mass: only
                        // straight across, a court with no stretch facing one went unreached on
                        // other seeds.
                        let across = geom::closest_on_segment(p, court.a, court.b) * court.length();
                        let half = PASSAGE.0 * 0.5;
                        let along = court.open[0]
                            .iter()
                            .filter(|&&(l, h)| h - l >= PASSAGE.0)
                            .map(|&(l, h)| across.clamp(l + half, h - half))
                            .min_by(|a, b| (a - across).abs().total_cmp(&(b - across).abs()));
                        let Some(along) = along else { continue };
                        let q = court.a + (court.b - court.a).normalize() * along;
                        let slant = (along - across).abs();
                        if slant > 0.0 && !geom::contains(&masses[i].footprint, (p + q) * 0.5) {
                            continue;
                        }
                        // Straight ones first, then the longest street stretch.
                        let score = hi - lo - 4.0 * slant;
                        if best.is_none_or(|x| score > x.5) {
                            best = Some((i, sw, cw, mid, along, score));
                        }
                    }
                }
            }
        }
        if let Some((i, sw, cw, at_street, at_court, _)) = best {
            out.push((i, [(sw, at_street), (cw, at_court)]));
        }
    }
    out
}

/// The stretch a passage's mouth keeps clear on its wall, either side of its middle, metres.
const PASSAGE_CLEAR: f64 = PASSAGE.0 * 0.5 + 0.3;

/// Opens a passage's mouths in its mass: the ground openings they meet taken out, its cores'
/// doors found again after.
fn open_passage(s: &mut Structure, mouths: [(usize, f64); 2]) {
    let doors: Vec<Opening> = s.cores.iter().map(|c| s.openings[c.door]).collect();
    // Under its storey's ceiling with a lintel, as the other openings: no higher than 3.6 m.
    let head = PASSAGE.1.min(s.storeys[0].clear() - 0.3);
    for (w, along) in mouths {
        let (lo, hi) = (along - PASSAGE_CLEAR, along + PASSAGE_CLEAR);
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
    for (c, door) in s.cores.iter_mut().zip(doors) {
        if let Some(k) = s.openings.iter().position(|o| *o == door) {
            c.door = k;
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

/// What someone on foot cannot reach from the streets (the owner: emulate people's routines and
/// see what blocks them): each door whose step outside is shut off, as (mass, opening), and each
/// block whose court no one can walk into. Block by block, the ground on a grid of
/// [`REACH_CELL`]: the buildings block it, their passages open through them, and the streets
/// round the block (outside its outline) are where everyone starts. The flats and floors come
/// with the buildings' stairs and units (D-060's step 3).
pub fn unreached(
    plan: &CityPlan,
    masses: &[Mass],
    structures: &[Structure],
) -> (Vec<(u32, usize)>, Vec<u32>) {
    let mut by_block: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, m) in masses.iter().enumerate() {
        by_block
            .entry(plan.lots[m.lot as usize].block)
            .or_default()
            .push(i);
    }
    let mut blocks: Vec<u32> = by_block.keys().copied().collect();
    blocks.sort_unstable();
    let (mut doors, mut courts) = (Vec::new(), Vec::new());
    let c = REACH_CELL;
    for b in blocks {
        let block = &plan.blocks[b as usize];
        let list = &by_block[&b];
        // The block and a street's width round it.
        let (lo, hi) = block.polygon.iter().fold(
            (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
            |(lo, hi), &p| (lo.min(p), hi.max(p)),
        );
        let (lo, hi) = (lo - DVec2::splat(6.0), hi + DVec2::splat(6.0));
        let (nx, ny) = (
            ((hi.x - lo.x) / c).ceil() as usize,
            ((hi.y - lo.y) / c).ceil() as usize,
        );
        let index = |p: DVec2| {
            let (x, y) = (((p.x - lo.x) / c).floor(), ((p.y - lo.y) / c).floor());
            (x >= 0.0 && y >= 0.0 && (x as usize) < nx && (y as usize) < ny)
                .then(|| y as usize * nx + x as usize)
        };
        // Each polygon's cells, those whose middles it holds: row by row, between its edges'
        // crossings (even-odd, as `geom::contains`).
        let cells_of = |polygon: &[DVec2], f: &mut dyn FnMut(usize)| {
            let (a, z) = polygon.iter().fold(
                (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                |(a, z), &p| (a.min(p), z.max(p)),
            );
            let y0 = ((a.y - lo.y) / c).floor().max(0.0) as usize;
            let y1 = (((z.y - lo.y) / c).ceil() as usize).min(ny);
            let n = polygon.len();
            let mut crossings = Vec::new();
            for y in y0..y1 {
                let py = lo.y + (y as f64 + 0.5) * c;
                crossings.clear();
                for i in 0..n {
                    let (p, q) = (polygon[i], polygon[(i + 1) % n]);
                    if (p.y <= py) != (q.y <= py) {
                        crossings.push(p.x + (py - p.y) * (q.x - p.x) / (q.y - p.y));
                    }
                }
                crossings.sort_by(f64::total_cmp);
                for span in crossings.as_chunks::<2>().0 {
                    let from = ((span[0] - lo.x) / c - 0.5).ceil().max(0.0) as usize;
                    let to = ((span[1] - lo.x) / c - 0.5).floor();
                    if to < 0.0 {
                        continue;
                    }
                    for x in from..=(to as usize).min(nx - 1) {
                        f(y * nx + x);
                    }
                }
            }
        };
        let mut blocked = vec![false; nx * ny];
        for &i in list {
            cells_of(&masses[i].footprint, &mut |k| blocked[k] = true);
        }
        // The passages open: from their street mouth to their court mouth.
        for &i in list {
            let s = &structures[i];
            let mouths: Vec<(DVec2, DVec2)> = s
                .openings
                .iter()
                .filter(|o| o.kind == OpeningKind::Passage)
                .map(|o| {
                    let w = &s.walls[o.wall];
                    let t = (w.b - w.a).normalize_or_zero();
                    (w.a + t * o.span().0, w.a + t * o.span().1)
                })
                .collect();
            for pair in mouths.as_chunks::<2>().0 {
                let [(p0, q0), (p1, q1)] = *pair;
                let (p1, q1) =
                    if p0.distance(p1) + q0.distance(q1) <= p0.distance(q1) + q0.distance(p1) {
                        (p1, q1)
                    } else {
                        (q1, p1)
                    };
                let mut tunnel = vec![p0, q0, q1, p1];
                if geom::signed_area(&tunnel) < 0.0 {
                    tunnel.reverse();
                }
                cells_of(&tunnel, &mut |k| blocked[k] = false);
            }
        }
        // From the streets: every open cell outside the block's outline.
        let mut inside = vec![false; nx * ny];
        cells_of(&block.polygon, &mut |k| inside[k] = true);
        let mut seen = vec![false; nx * ny];
        let mut queue = std::collections::VecDeque::new();
        for k in 0..nx * ny {
            if !blocked[k] && !inside[k] {
                seen[k] = true;
                queue.push_back(k);
            }
        }
        while let Some(k) = queue.pop_front() {
            let (x, y) = (k % nx, k / nx);
            let mut visit = |k: usize| {
                if !blocked[k] && !seen[k] {
                    seen[k] = true;
                    queue.push_back(k);
                }
            };
            if x > 0 {
                visit(k - 1);
            }
            if x + 1 < nx {
                visit(k + 1);
            }
            if y > 0 {
                visit(k - nx);
            }
            if y + 1 < ny {
                visit(k + nx);
            }
        }
        // Each door's step outside, 0.6 m out from its middle.
        for &i in list {
            let s = &structures[i];
            for (k, o) in s.openings.iter().enumerate() {
                if o.kind != OpeningKind::Door {
                    continue;
                }
                let w = &s.walls[o.wall];
                let t = (w.b - w.a).normalize_or_zero();
                let step = w.a + t * o.along + DVec2::new(t.y, -t.x) * 0.6;
                if !index(step).is_some_and(|k| seen[k]) {
                    doors.push((i as u32, k));
                }
            }
        }
        // The court: any of its open cells reached.
        if let Some(court) = &block.courtyard {
            let mut open = 0;
            let mut reached = 0;
            cells_of(court, &mut |k| {
                if !blocked[k] {
                    open += 1;
                    reached += usize::from(seen[k]);
                }
            });
            if open > 0 && reached == 0 {
                courts.push(b);
            }
        }
    }
    (doors, courts)
}

/// The ground's cell for [`unreached`], metres: narrower than a door or a gap one walks through.
pub const REACH_CELL: f64 = 0.5;

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
                println!("{era:?} net over gross {r:.3}");
                assert!((lo..=hi).contains(&r), "{era:?} net over gross {r:.3}");
            }
        }
    }

    #[test]
    fn every_building_has_stairs_and_lifts_that_fit_and_reach_its_floors() {
        use crate::city::mass::Roof;
        let (city, masses) = example_city();
        let all = structures(&city, &masses);
        // Each lot's halls, to see that every upper floor is near one.
        let mut halls: HashMap<u32, Vec<DVec2>> = HashMap::new();
        let (mut cores, mut lifts, mut roofs, mut missing, mut needing) = (0, 0, 0, 0, 0);
        for s in &all {
            let m = &masses[s.mass as usize];
            let thickness: Vec<f64> = s.walls.iter().map(|w| w.thickness).collect();
            let floor = floor(m, &thickness);
            let plate = floor.as_ref().map_or(0.0, |p| geom::signed_area(p).abs());
            if floor.is_some() && cores::needs_cores(m, &s.walls, plate) {
                needing += 1;
                missing += usize::from(s.cores.is_empty());
            }
            for (k, c) in s.cores.iter().enumerate() {
                cores += 1;
                lifts += usize::from(c.lift.is_some());
                roofs += usize::from(c.roof);
                // Its door: in the middle of its hall, on the ground storey of its wall.
                let o = &s.openings[c.door];
                assert_eq!(o.kind, OpeningKind::Door);
                assert_eq!((o.wall, o.storey), (c.wall, 0));
                assert!(o.span().0 >= c.span.0 && o.span().1 <= c.span.1);
                // Inside the walls.
                let floor = floor.as_ref().expect("a core needs a floor");
                let middle = (c.outline[0] + c.outline[1] + c.outline[2] + c.outline[3]) * 0.25;
                for p in c.outline {
                    assert!(geom::contains(floor, p + (middle - p).normalize() * 0.05));
                }
                // Clear of the others on its wall.
                for d in &s.cores[k + 1..] {
                    assert!(d.wall != c.wall || d.span.0 >= c.span.1 || d.span.1 <= c.span.0);
                }
                // A lift from R+3; up to the roof under a garden or a terrace.
                assert_eq!(c.lift.is_some(), m.storeys.len() >= LIFT_FROM);
                assert_eq!(c.roof, matches!(m.roof, Roof::Green | Roof::Terrace));
                assert_eq!(c.risers.len(), m.storeys.len() + usize::from(c.roof));
                halls
                    .entry(m.lot)
                    .or_default()
                    .push((c.hall[0] + c.hall[1] + c.hall[2] + c.hall[3]) * 0.25);
            }
        }
        // Every upper floor within 25 m of a landing in its building (as the crow flies).
        let mut far = 0;
        let mut upper = 0;
        for m in masses.iter().filter(|m| m.storeys.len() >= 2) {
            upper += 1;
            let near = halls.get(&m.lot).is_some_and(|h| {
                m.footprint
                    .iter()
                    .all(|&p| h.iter().any(|&q| p.distance(q) <= cores::REACH))
            });
            far += usize::from(!near);
        }
        println!(
            "{cores} cores, {lifts} with a lift, {roofs} up to the roof; {missing} of {needing} \
             masses without; {far} of {upper} upper masses far from a landing"
        );
        assert!(cores > 10_000, "{cores} cores");
        assert!(
            missing * 100 <= needing,
            "{missing} of {needing} without a core"
        );
        assert!(far * 50 <= upper, "{far} of {upper} far from a landing");
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

    #[test]
    fn people_reach_every_door_and_court_from_the_streets() {
        let (city, masses) = example_city();
        let all = structures(&city, &masses);
        let (doors, courts) = unreached(&city, &masses, &all);
        let held: usize = all
            .iter()
            .map(|s| {
                s.openings
                    .iter()
                    .filter(|o| o.kind == OpeningKind::Door)
                    .count()
            })
            .sum();
        // Street doors and court doors; the court doors no one could reach were made windows.
        assert!(held > masses.len() / 2, "{held} doors");
        assert!(doors.is_empty(), "{} doors unreached", doors.len());
        assert!(courts.is_empty(), "{} courts unreached", courts.len());
    }
}
