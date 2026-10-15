//! A city's plan before any building: districts, streets, blocks, lots, parks, tram lines,
//! boat crossings and landmarks, judged as a map image (`docs/research/city-generation.md`,
//! "Recommendation for Forge", stages 1–5, and its extension §7–§19; D-039, D-059).
//!
//! Everything is a pure function of a [`Site`] (the frame's water and ground) and a
//! [`CityParams`] record, deterministic on every machine (D-016):
//! 1. **Districts** ([`district_field`]): each land cell goes to the district whose anchor is
//!    nearest relative to its radius, the boundaries roughened by noise.
//! 2. **The street pattern** ([`tensor`]): a tensor field summing a grid per district, rings
//!    round radial squares and the water's tangents (Chen et al. 2008).
//! 3. **Streets** ([`streets`]): main roads traced first at the main spacing, bridging the
//!    rivers; then streets at each district's spacing.
//! 4. **The road graph and blocks** ([`graph`]): the streets cut where they meet, dead ends
//!    pruned, the blocks as the graph's faces.
//! 5. **Blocks, parks and lots** ([`blocks`]): parks where a district is a park, then enough
//!    blocks turned into parks that every block has one of a hectare within reach (the
//!    3-30-300 rule); lots by oriented splits, those without a street front left as
//!    courtyards.
//! 6. **Transit** ([`transit`]): tram lines routed over the main roads through their
//!    waypoints, stops at a spacing, boat crossings where bridges are far apart.
//! 7. **Landmarks**, the street profiles that follow from the transit, and the
//!    [`PlanStats`] the map is judged with ([`map`] draws it).

pub mod bible;
pub mod blocks;
pub mod cores;
pub mod example;
pub mod facades;
pub mod gardens;
pub mod geom;
pub mod graph;
pub mod ground;
pub mod lamps;
pub mod map;
pub mod mass;
pub mod shore;
pub mod streets;
pub mod structure;
pub mod taxi;
pub mod tensor;
pub mod tram_depot;
pub mod transit;
pub mod trees;
pub mod units;

use std::time::Instant;

use forge_core::Seed;
use glam::DVec2;

use crate::field::Field2;
use crate::noise;

pub use blocks::{Block, BlockUse, Lot};
pub use graph::{RoadEdge, RoadGraph, RoadNode};
pub use tensor::BoundaryInfluence;
pub use transit::{BoatCrossing, TramLine};

/// A water mask's land.
pub const LAND: u8 = 0;
/// A water mask's sea.
pub const SEA: u8 = 1;
/// A water mask's river.
pub const RIVER: u8 = 2;
/// A water mask's lagoon or lake.
pub const LAGOON: u8 = 3;

/// A river through the frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Waterway {
    /// Its name.
    pub name: String,
    /// Its centreline, metres on the plan, downstream last.
    pub centreline: Vec<DVec2>,
    /// Its width between the banks, metres.
    pub width: f64,
}

/// What the city is built on: a square frame of `size` metres (`x` east, `y` south, the
/// top-left corner at the origin), its water and its ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Site {
    /// The frame's side, metres.
    pub size: f64,
    /// [`LAND`], [`SEA`], [`RIVER`] or [`LAGOON`] per cell.
    pub water: Field2<u8>,
    /// The ground's height above the sea, metres (for slopes; flat sites hold zeros).
    pub ground: Field2<f32>,
    /// The rivers, painted into `water` by [`Site::paint_rivers`].
    pub rivers: Vec<Waterway>,
}

impl Site {
    /// A dry, flat frame: `size` metres in cells of `cell` metres.
    pub fn flat(size: f64, cell: f64) -> Self {
        let n = (size / cell).ceil() as u32 + 1;
        Self {
            size,
            water: Field2::new(n, cell),
            ground: Field2::new(n, cell),
            rivers: Vec::new(),
        }
    }

    /// The water code at `p` (the nearest cell; outside the frame, the nearest edge cell).
    pub fn water_at(&self, p: DVec2) -> u8 {
        let n = self.water.size;
        let s = self.water.spacing;
        let x = (p.x / s).round().clamp(0.0, (n - 1) as f64) as u32;
        let y = (p.y / s).round().clamp(0.0, (n - 1) as f64) as u32;
        self.water.get(x, y)
    }

    /// Whether the straight line from `a` to `b` crosses any water.
    pub fn segment_crosses_water(&self, a: DVec2, b: DVec2) -> bool {
        let steps = (a.distance(b) / (self.water.spacing * 0.5)).ceil().max(1.0) as usize;
        (0..=steps).any(|i| self.water_at(a + (b - a) * (i as f64 / steps as f64)) != LAND)
    }

    /// Turns land cells with six or more of their eight neighbours water into water (the most
    /// common water among them), a few passes: the spits and slivers a cell or two wide that
    /// painting the rivers' beds leaves by the sea. One stood at a river's mouth as a wedge of
    /// stone walls in the water (the owner).
    pub fn clear_slivers(&mut self, passes: usize) {
        let n = self.water.size;
        for _ in 0..passes {
            let mut changes = Vec::new();
            for y in 1..n.saturating_sub(1) {
                for x in 1..n.saturating_sub(1) {
                    if self.water.get(x, y) != LAND {
                        continue;
                    }
                    let mut counts = [0_u32; 4];
                    for (dx, dy) in [
                        (-1, -1),
                        (0, -1),
                        (1, -1),
                        (-1, 0),
                        (1, 0),
                        (-1, 1),
                        (0, 1),
                        (1, 1),
                    ] {
                        let w = self
                            .water
                            .get((x as i32 + dx) as u32, (y as i32 + dy) as u32);
                        counts[usize::from(w.min(3))] += 1;
                    }
                    if counts[1] + counts[2] + counts[3] >= 6 {
                        let kind = (1..4)
                            .max_by_key(|&k| (counts[k], std::cmp::Reverse(k)))
                            .unwrap_or(1);
                        changes.push((x, y, kind as u8));
                    }
                }
            }
            if changes.is_empty() {
                break;
            }
            for (x, y, kind) in changes {
                self.water.set(x, y, kind);
            }
        }
    }

    /// Opens the land by `r` cells (eroded, then grown back): land thinner than about `2 r + 1`
    /// cells goes to the water beside it. A spit between a river's bed and the sea stood at the
    /// mouth as a wedge of walls in the water (the owner); [`Self::clear_slivers`] only takes the
    /// thinnest.
    pub fn open_land(&mut self, r: i32) {
        let n = self.water.size as i32;
        let at = |f: &Field2<u8>, x: i32, y: i32| {
            f.get(x.clamp(0, n - 1) as u32, y.clamp(0, n - 1) as u32)
        };
        let land = |x: i32, y: i32| at(&self.water, x, y) == LAND;
        let mut core = vec![false; (n * n) as usize];
        for y in 0..n {
            for x in 0..n {
                core[(y * n + x) as usize] =
                    (-r..=r).all(|dy| (-r..=r).all(|dx| land(x + dx, y + dy)));
            }
        }
        let cored = |x: i32, y: i32| {
            (0..n).contains(&x) && (0..n).contains(&y) && core[(y * n + x) as usize]
        };
        let mut changes = Vec::new();
        for y in 0..n {
            for x in 0..n {
                if !land(x, y) || (-r..=r).any(|dy| (-r..=r).any(|dx| cored(x + dx, y + dy))) {
                    continue;
                }
                // The nearest water's kind, ring by ring.
                let kind = (1..=r + 2).find_map(|k| {
                    (-k..=k)
                        .flat_map(|dy| (-k..=k).map(move |dx| (dx, dy)))
                        .find_map(|(dx, dy)| {
                            let w = at(&self.water, x + dx, y + dy);
                            (w != LAND).then_some(w)
                        })
                });
                if let Some(kind) = kind {
                    changes.push((x as u32, y as u32, kind));
                }
            }
        }
        for (x, y, kind) in changes {
            self.water.set(x, y, kind);
        }
    }

    /// Paints each river's bed, its centreline widened to its width, into the water mask
    /// (never over the sea).
    pub fn paint_rivers(&mut self) {
        let s = self.water.spacing;
        let n = self.water.size;
        for river in &self.rivers {
            let half = river.width * 0.5;
            for w in river.centreline.windows(2) {
                let (lo, hi) = (w[0].min(w[1]) - half, w[0].max(w[1]) + half);
                let x0 = (lo.x / s).floor().max(0.0) as u32;
                let y0 = (lo.y / s).floor().max(0.0) as u32;
                let x1 = ((hi.x / s).ceil() as u32).min(n - 1);
                let y1 = ((hi.y / s).ceil() as u32).min(n - 1);
                for y in y0..=y1 {
                    for x in x0..=x1 {
                        let p = DVec2::new(x as f64 * s, y as f64 * s);
                        if geom::distance_to_segment(p, w[0], w[1]) <= half
                            && self.water.get(x, y) != SEA
                        {
                            self.water.set(x, y, RIVER);
                        }
                    }
                }
            }
        }
    }

    /// The ground's height at `p` (the nearest cell) and its slope there, rise over run.
    pub fn ground_at(&self, p: DVec2) -> (f32, f32) {
        let n = self.ground.size;
        let s = self.ground.spacing;
        let x = (p.x / s).round().clamp(0.0, (n - 1) as f64) as u32;
        let y = (p.y / s).round().clamp(0.0, (n - 1) as f64) as u32;
        let (gx, gy) = self.ground.gradient(x, y);
        (self.ground.get(x, y), (gx * gx + gy * gy).sqrt())
    }

    /// The land's area, square metres.
    pub fn land_area(&self) -> f64 {
        let s = self.water.spacing;
        self.water.data.iter().filter(|&&w| w == LAND).count() as f64 * s * s
    }
}

/// How a district's streets are laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pattern {
    /// A grid at the district's angle.
    Grid,
    /// Rings and spokes round the anchor.
    Radial,
    /// A grid turned by a slow noise of this many radians: lanes that wander.
    Organic(f64),
}

/// What a district is for; it decides its streets' profile, its parks and its colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DistrictKind {
    /// An old town: lanes, small squares, pedestrian streets.
    OldTown,
    /// A dense centre round the main squares.
    Centre,
    /// Perimeter blocks round courtyards (a 19th–20th century ring).
    Perimeter,
    /// Courtyard blocks.
    Courtyard,
    /// A harbour and seafront.
    Waterfront,
    /// Kept industrial halls and their infill.
    Industrial,
    /// An eco-quarter of superblocks.
    Eco,
    /// A village at the city's edge.
    Village,
    /// A park: no streets but paths.
    Park,
    /// A nature reserve: nothing built.
    Reserve,
}

impl DistrictKind {
    /// Whether its blocks are parks rather than built.
    pub fn is_green(self) -> bool {
        matches!(self, Self::Park | Self::Reserve)
    }

    /// How deep its buildings are from the building line, metres: what a block keeps round its
    /// courtyard. `None` where blocks have no courtyard (halls, parks).
    pub fn building_depth(self) -> Option<f64> {
        match self {
            Self::OldTown => Some(10.0),
            Self::Centre | Self::Perimeter => Some(14.0),
            Self::Courtyard | Self::Eco => Some(12.0),
            Self::Waterfront => Some(16.0),
            Self::Village => Some(10.0),
            Self::Industrial | Self::Park | Self::Reserve => None,
        }
    }
}

/// A district of the plan.
#[derive(Clone, Debug, PartialEq)]
pub struct DistrictSpec {
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: DistrictKind,
    /// Its centre, metres on the plan.
    pub anchor: DVec2,
    /// How far it reaches, metres (relative to its neighbours').
    pub radius: f64,
    /// Its street pattern.
    pub pattern: Pattern,
    /// Its grid's angle, radians from east.
    pub grid_angle: f64,
    /// The spacing of its streets, metres (the block's size).
    pub street_spacing: f64,
    /// A lot's area there, square metres.
    pub lot_area: f64,
    /// Its buildings' floors, lowest and highest.
    pub floors: (u8, u8),
    /// Its buildings' depth behind their building line, metres; its kind's when `None`.
    pub depth: Option<f64>,
    /// The share of its roofs pitched and tiled; its kind's when `None`.
    pub tiled_roofs: Option<f64>,
    /// Open blocks: the share of a block's lots between its corners left as gardens, gaps
    /// between its buildings that let light, views and people into the courtyard. None by
    /// default.
    pub gaps: Option<f64>,
    /// Open blocks: the longest front a building runs before a gap, metres (120 when `None`).
    /// Under 60 m the corners open too: separate buildings round a shared green rather than a
    /// ring (the owner's photos of open blocks, 2026-10-11).
    pub run: Option<f64>,
}

impl DistrictSpec {
    /// The share of its roofs pitched and tiled: its own, or its kind's (most of an old town's
    /// and a village's, none elsewhere, where roofs are green, solar or terraces).
    pub fn tiled_share(&self) -> f64 {
        self.tiled_roofs.unwrap_or(match self.kind {
            DistrictKind::OldTown => 0.85,
            DistrictKind::Village => 0.9,
            _ => 0.0,
        })
    }

    /// The longest front its buildings run before a gap, metres ([`DistrictSpec::run`]).
    pub fn run_max(&self) -> f64 {
        self.run.unwrap_or(120.0).max(10.0)
    }

    /// The share of its blocks' middle lots left as gardens ([`DistrictSpec::gaps`]): 0 unless
    /// given.
    pub fn gap_share(&self) -> f64 {
        self.gaps.unwrap_or(0.0).clamp(0.0, 0.5)
    }

    /// Its buildings' depth behind their building line, metres: its own, or its kind's; `None`
    /// where it has no perimeter blocks.
    pub fn building_depth(&self) -> Option<f64> {
        self.kind.building_depth().map(|d| self.depth.unwrap_or(d))
    }

    /// A plain district for tests.
    pub fn test(anchor: DVec2, radius: f64, pattern: Pattern, grid_angle: f64) -> Self {
        Self {
            name: "test".into(),
            kind: DistrictKind::Perimeter,
            anchor,
            radius,
            pattern,
            grid_angle,
            street_spacing: 100.0,
            lot_area: 400.0,
            floors: (4, 6),
            depth: None,
            tiled_roofs: None,
            gaps: None,
            run: None,
        }
    }
}

/// A road's place in the hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RoadClass {
    /// Boulevards and avenues, traced first, bridging the rivers.
    Main,
    /// Streets, lanes and paths.
    Street,
}

/// Who uses a street: the street profiles of a transit-first city
/// (`city-generation.md` §12).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Profile {
    /// A main road with a tram: track, taxis on the paved track, wide pavements, trees.
    TramAvenue,
    /// A main road without a tram yet: shared lanes, bikes, trees.
    Avenue,
    /// A pedestrian street, 10 m: people and bikes, deliveries at set hours behind its bollards,
    /// no taxis (D-061: the owner, "taxis don't go where pedestrians are").
    Shared,
    /// A pedestrian street or lane.
    Pedestrian,
    /// A path through a park.
    Path,
}

impl Profile {
    /// The profile a class gets before transit and districts refine it.
    pub fn default_for(class: RoadClass) -> Self {
        match class {
            RoadClass::Main => Self::Avenue,
            RoadClass::Street => Self::Shared,
        }
    }

    /// Its cross-section from one building line to the other (D-061). A city without private
    /// cars needs little carriageway (the owner, 2026-10-11: "a lot of wasted space for the
    /// road"): a tram avenue's two tracks, with the taxis on them, and its bike strips take
    /// 12 m, an avenue's two lanes 6 m.
    pub fn section(self) -> &'static [Strip] {
        use Lane::*;
        const fn s(lane: Lane, width: f64) -> Strip {
            Strip { lane, width }
        }
        const TRAM_AVENUE: [Strip; 7] = [
            s(Footway, 3.5),
            s(Verge, 2.5),
            s(Cycle, 1.75),
            s(Tracks, 8.5),
            s(Cycle, 1.75),
            s(Verge, 2.5),
            s(Footway, 3.5),
        ];
        const AVENUE: [Strip; 6] = [
            s(Footway, 3.5),
            s(Verge, 2.5),
            s(Drive, 3.0),
            s(Drive, 3.0),
            s(Verge, 2.5),
            s(Footway, 3.5),
        ];
        const SHARED: [Strip; 1] = [s(Shared, 10.0)];
        const PEDESTRIAN: [Strip; 1] = [s(Footway, 8.0)];
        const PATH: [Strip; 1] = [s(Footway, 4.0)];
        match self {
            Self::TramAvenue => &TRAM_AVENUE,
            Self::Avenue => &AVENUE,
            Self::Shared => &SHARED,
            Self::Pedestrian => &PEDESTRIAN,
            Self::Path => &PATH,
        }
    }

    /// The width between the building lines, metres: its section's.
    pub fn width(self) -> f64 {
        self.section().iter().map(|s| s.width).sum()
    }

    /// Its carriageway's width, metres: the strips below the kerb; 0 where the whole street is
    /// one walking surface.
    pub fn carriageway(self) -> f64 {
        self.section()
            .iter()
            .filter(|s| !s.lane.raised())
            .map(|s| s.width)
            .sum()
    }

    /// Whether it has a carriageway below its kerbs.
    pub fn driven(self) -> bool {
        self.carriageway() > 0.0
    }

    /// Each raised pavement's width, metres, room for a row of trees by the kerb and café
    /// terraces by the shops; 0 where the whole street is one shared surface.
    pub fn pavement(self) -> f64 {
        if self.driven() {
            (self.width() - self.carriageway()) * 0.5
        } else {
            0.0
        }
    }
}

/// What a strip of a street's cross-section carries (D-061).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lane {
    /// People walking, at the walking level.
    Footway,
    /// Trees, lamps, benches and terraces by the kerb, at the walking level.
    Verge,
    /// Bikes, on the carriageway by the kerb.
    Cycle,
    /// Taxis and service vehicles.
    Drive,
    /// The trams' two tracks, the taxis on them.
    Tracks,
    /// People and bikes on one level, deliveries at set hours.
    Shared,
}

impl Lane {
    /// Whether it lies at the walking level, over the kerb.
    pub fn raised(self) -> bool {
        matches!(self, Self::Footway | Self::Verge | Self::Shared)
    }
}

/// A strip of a street's cross-section.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Strip {
    /// What it carries.
    pub lane: Lane,
    /// Its width, metres.
    pub width: f64,
}

/// A tram line to route: its waypoints in order (its ends and what it must pass through).
#[derive(Clone, Debug, PartialEq)]
pub struct TramLineSpec {
    /// Its name.
    pub name: String,
    /// Its colour on the map.
    pub colour: [u8; 3],
    /// The places it runs through, first and last its termini.
    pub waypoints: Vec<DVec2>,
}

/// What a landmark is, for the model that draws it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LandmarkKind {
    /// A covered market hall of iron and glass, standing on its square.
    MarketHall,
    /// A long, low civic hall whose roof steps down in public terraces towards the water.
    TerracedHall,
    /// A shipyard gantry crane kept as a landmark, its jib over the water.
    GantryCrane,
    /// A belvedere's domed pavilion, open on every side to the view.
    Pavilion,
    /// A weir across a river with a footbridge over it, its sluice gates' portals standing
    /// over the piers. It spans the river at its anchor and needs no block.
    WeirBridge,
    /// Anything else: placed, not drawn yet.
    #[default]
    Other,
}

/// A landmark to place: snapped to the nearest main-road node.
#[derive(Clone, Debug, PartialEq)]
pub struct LandmarkSpec {
    /// Its name.
    pub name: String,
    /// Where it should stand, roughly.
    pub anchor: DVec2,
    /// What it is.
    pub kind: LandmarkKind,
}

/// A placed landmark.
#[derive(Clone, Debug, PartialEq)]
pub struct Landmark {
    /// Its name.
    pub name: String,
    /// Where it stands: the main-road node nearest its anchor, or the anchor itself for a weir
    /// bridge.
    pub pos: DVec2,
    /// What it is.
    pub kind: LandmarkKind,
    /// The block it stands on (an index into the plan's blocks): the nearest square of 0.15 ha
    /// or more within 160 m of its anchor; for a terraced hall without one, the nearest quay
    /// within 250 m; for a crane, the nearest quay within 600 m first. None for a weir bridge.
    pub square: Option<usize>,
}

/// Everything the plan is made from, besides the site.
#[derive(Clone, Debug, PartialEq)]
pub struct CityParams {
    /// The seed every stage derives its own from.
    pub seed: Seed,
    /// The districts.
    pub districts: Vec<DistrictSpec>,
    /// The main roads' spacing, metres.
    pub main_spacing: f64,
    /// How the water bends the streets.
    pub boundary: BoundaryInfluence,
    /// The longest bridge, metres.
    pub max_bridge: f64,
    /// Where tracing starts: the main square or the tram hub.
    pub centre: DVec2,
    /// The tram lines.
    pub tram_lines: Vec<TramLineSpec>,
    /// The spacing of tram stops, metres.
    pub stop_spacing: f64,
    /// Lines are added ([`transit::cover`]) until this share of the built blocks has a stop
    /// within 400 m...
    pub stop_target: f64,
    /// ... or this many lines were added (0: none).
    pub max_extra_lines: usize,
    /// A boat crossing every this many metres of river without a bridge.
    pub boat_gap: f64,
    /// The landmarks.
    pub landmarks: Vec<LandmarkSpec>,
    /// Every built block should have a park of `park_min_area` within this distance, metres.
    pub park_reach: f64,
    /// The smallest park that counts, square metres.
    pub park_min_area: f64,
    /// The largest share of the land the parks may take.
    pub park_share_cap: f64,
    /// Ground higher than this, metres, is left as forest (to the first reserve district).
    pub forest_above: f32,
    /// Ground steeper than this (rise over run) is left as forest.
    pub forest_slope: f32,
    /// How far a district reaches, as a multiple of its radius: land farther from every district
    /// stays countryside (no streets, no blocks; main roads still cross it). `None`: every land
    /// cell takes its nearest district (islands of city grew wherever the hills left room, the
    /// owner).
    pub reach: Option<f64>,
    /// Superblocks cut by streets: a built block longer than this many times its district's
    /// street spacing gets a shared street across its middle, again until none is. `None` keeps
    /// the traced streets alone.
    pub max_block: Option<f64>,
}

/// The plan's numbers, the table judged beside the map.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlanStats {
    /// The land's area, km².
    pub land_km2: f64,
    /// Main roads, km.
    pub main_km: f64,
    /// Streets, km.
    pub street_km: f64,
    /// Bridges.
    pub bridges: usize,
    /// Road nodes.
    pub nodes: usize,
    /// Built blocks.
    pub built_blocks: usize,
    /// Park blocks.
    pub park_blocks: usize,
    /// A built block's mean area, m².
    pub mean_block_m2: f64,
    /// Lots.
    pub lots: usize,
    /// Built blocks with a courtyard.
    pub courtyards: usize,
    /// The parks' share of the land.
    pub park_share: f64,
    /// The share of built blocks with a park of a hectare within reach.
    pub park_coverage: f64,
    /// The share of built blocks within 400 m of a tram stop.
    pub stop_coverage: f64,
    /// The streets' share of the land.
    pub street_share: f64,
    /// Tram lines' length, km.
    pub tram_km: f64,
    /// Tram stops.
    pub stops: usize,
    /// Boat crossings.
    pub crossings: usize,
    /// Squares (slivers and piazzas).
    pub squares: usize,
    /// The plan's digest (D-016): equal on every machine and thread count.
    pub digest: u64,
}

/// The plan.
#[derive(Clone, Debug, PartialEq)]
pub struct CityPlan {
    /// The frame's side, metres.
    pub size: f64,
    /// The districts it was made with.
    pub districts: Vec<DistrictSpec>,
    /// The district of each cell (255: water).
    pub district_field: Field2<u8>,
    /// The roads.
    pub roads: RoadGraph,
    /// The blocks.
    pub blocks: Vec<Block>,
    /// The lots.
    pub lots: Vec<Lot>,
    /// The tram lines.
    pub tram_lines: Vec<TramLine>,
    /// The street ground: every surface of the streets once, with its level (D-061).
    pub ground: ground::GroundMap,
    /// The taxi stands (D-061).
    pub taxi_stands: Vec<taxi::Stand>,
    /// The taxi depots (D-061): parking decks where the cabs no stand holds wait and charge.
    pub depots: Vec<taxi::Depot>,
    /// The tram depots: yards where the trams are stabled and kept, near the lines' ends.
    pub tram_depots: Vec<tram_depot::TramDepot>,
    /// The boat crossings.
    pub crossings: Vec<BoatCrossing>,
    /// The landmarks.
    pub landmarks: Vec<Landmark>,
    /// Its numbers.
    pub stats: PlanStats,
    /// Each stage's time, milliseconds.
    pub timings: Vec<(&'static str, f64)>,
}

impl CityPlan {
    /// The district at `p`, if it is on land.
    pub fn district_at(&self, p: DVec2) -> Option<&DistrictSpec> {
        district_index(&self.district_field, p).map(|i| &self.districts[i])
    }
}

/// The index of the district at `p` in a district field.
pub fn district_index(field: &Field2<u8>, p: DVec2) -> Option<usize> {
    let n = field.size;
    let s = field.spacing;
    let x = (p.x / s).round().clamp(0.0, (n - 1) as f64) as u32;
    let y = (p.y / s).round().clamp(0.0, (n - 1) as f64) as u32;
    let d = field.get(x, y);
    (d != u8::MAX).then_some(d as usize)
}

/// Stage 1: each land cell's district, by the nearest anchor relative to its radius, the
/// boundaries roughened by a noise of ±12 %. Ground higher than `forest.0` metres or steeper
/// than `forest.1` goes to the first [`DistrictKind::Reserve`] district, if there is one: the
/// hills stay wooded.
pub fn district_field(
    site: &Site,
    districts: &[DistrictSpec],
    forest: (f32, f32),
    seed: u64,
    cell: f64,
    reach: Option<f64>,
) -> Field2<u8> {
    let n = (site.size / cell).ceil() as u32 + 1;
    let reserve = districts
        .iter()
        .position(|d| d.kind == DistrictKind::Reserve)
        .map(|i| i as u8);
    Field2::from_fn(n, cell, |x, y| {
        let p = DVec2::new(x as f64 * cell, y as f64 * cell);
        if site.water_at(p) != LAND {
            return u8::MAX;
        }
        if let Some(reserve) = reserve {
            let (height, slope) = site.ground_at(p);
            if height > forest.0 || slope > forest.1 {
                return reserve;
            }
        }
        let wobble = 1.0 + 0.12 * noise::fbm(seed, p.x / 700.0, p.y / 700.0, 3, 2.0, 0.5);
        let mut best = (f64::MAX, u8::MAX);
        for (i, d) in districts.iter().enumerate() {
            let score = p.distance(d.anchor) / d.radius * wobble;
            if score < best.0 {
                best = (score, i as u8);
            }
        }
        // Beyond every district's reach: countryside.
        if reach.is_some_and(|r| best.0 > r) {
            return u8::MAX;
        }
        best.1
    })
}

/// The block a landmark stands on ([`Landmark::square`]).
fn landmark_site(blocks: &[Block], l: &LandmarkSpec) -> Option<usize> {
    let nearest = |use_: BlockUse, min_area: f64, reach: f64| {
        blocks
            .iter()
            .enumerate()
            .filter(|(_, b)| b.use_ == use_ && b.area >= min_area)
            .map(|(i, b)| (b.centre.distance(l.anchor), i))
            .filter(|&(d, _)| d <= reach)
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, i)| i)
    };
    // A weir bridge spans its river; a crane stands on a quay, the nearest within 600 m.
    if l.kind == LandmarkKind::WeirBridge {
        return None;
    }
    if l.kind == LandmarkKind::GantryCrane {
        return nearest(BlockUse::Quay, 2000.0, 600.0)
            .or_else(|| nearest(BlockUse::Square, 1500.0, 160.0));
    }
    let waterside = l.kind == LandmarkKind::TerracedHall;
    nearest(BlockUse::Square, 1500.0, 160.0).or_else(|| {
        waterside
            .then(|| nearest(BlockUse::Quay, 2000.0, 250.0))
            .flatten()
    })
}

/// How far apart along a river its bridges stand, metres: about a city centre's (Paris's run 300
/// to 400 m apart in its centre; the boats cross between them).
pub const BRIDGE_SPACING: f64 = 450.0;

/// How far an industrial quay's walk keeps from the sea's edge, metres: a pedestrian street's half
/// width and 2 m, the stone edge and its balustrade.
pub const QUAY_SETBACK: f64 = 6.0;

/// How far the seafront's promenade keeps from the sea, metres: a beach's width and the
/// promenade's half (the owner: sand beaches, hotels by the sea).
pub const BEACH: f64 = 34.0;

/// Makes the plan.
pub fn plan(site: &Site, params: &CityParams) -> CityPlan {
    let mut timings = Vec::new();
    let mut clock = Instant::now();
    let mut lap = |name: &'static str, timings: &mut Vec<(&'static str, f64)>| {
        timings.push((name, clock.elapsed().as_secs_f64() * 1e3));
        clock = Instant::now();
    };
    let seed = params.seed;
    let districts_field = district_field(
        site,
        &params.districts,
        (params.forest_above, params.forest_slope),
        seed.derive_str("districts").value(),
        20.0,
        params.reach,
    );
    lap("districts", &mut timings);

    let field = tensor::TensorField::bake(
        site,
        &params.districts,
        params.boundary,
        seed.derive_str("tensor").value(),
        20.0,
    );
    lap("tensor field", &mut timings);

    let kind_at = |p: DVec2| district_index(&districts_field, p).map(|i| &params.districts[i]);
    let main_params = streets::TraceParams {
        class: RoadClass::Main,
        test_ratio: 0.5,
        step: 10.0,
        max_length: 14_000.0,
        min_length: 300.0,
        join_ratio: 0.9,
        max_bridge: params.max_bridge,
        random_tries: 80,
    };
    let main_spacing = |p: DVec2| match kind_at(p) {
        Some(d) if d.kind == DistrictKind::Reserve => None,
        Some(_) => Some(params.main_spacing),
        None => Some(params.main_spacing),
    };
    // The bridges first (D-061): one every BRIDGE_SPACING along each river between built
    // districts, square to it at its narrowest; main roads cross only there, and each site's
    // ends seed them.
    let built = |p: DVec2| {
        kind_at(p).is_some_and(|d| d.kind != DistrictKind::Reserve && !d.kind.is_green())
    };
    let sites = if params.max_bridge > 0.0 {
        streets::bridge_sites(site, BRIDGE_SPACING, params.max_bridge, &built)
    } else {
        Vec::new()
    };
    let mut seeds = vec![params.centre];
    seeds.extend(sites.iter().flat_map(|s| [s.a, s.b]));
    seeds.extend(params.districts.iter().map(|d| d.anchor));
    let main = streets::trace_level(
        &field,
        site,
        &[],
        &main_params,
        &main_spacing,
        &seeds,
        seed.derive_str("main roads").value(),
        &sites,
    );
    lap("main roads", &mut timings);

    let street_params = streets::TraceParams {
        class: RoadClass::Street,
        test_ratio: 0.5,
        step: 5.0,
        max_length: 3_000.0,
        min_length: 40.0,
        join_ratio: 1.0,
        max_bridge: 0.0,
        random_tries: 400,
    };
    let street_spacing = |p: DVec2| match kind_at(p) {
        Some(d) if d.kind.is_green() => None,
        Some(d) => Some(d.street_spacing),
        None => None,
    };
    let streets = streets::trace_level(
        &field,
        site,
        &main,
        &street_params,
        &street_spacing,
        &seeds,
        seed.derive_str("streets").value(),
        &[],
    );
    lap("streets", &mut timings);

    let mut lines = main;
    lines.extend(streets);
    // The bridge sites no traced road took get one, straight on inland to the first street on
    // either side.
    let connected = streets::connect_sites(site, &sites, &lines);
    lines.extend(connected);
    // The quays along the rivers' banks, after the bridges took their roads inland.
    // Where no traced street already runs along the bank within 25 m: two walks side by side.
    let mut traced = geom::PointGrid::new(site.size, 40.0);
    for l in &lines {
        for p in geom::resample(&l.points, 5.0) {
            traced.insert(p, 0);
        }
    }
    let mut quays: Vec<streets::Streamline> = Vec::new();
    for quay in streets::quay_lines(site, ground::PROMENADE, &built) {
        let mut run: Vec<DVec2> = Vec::new();
        let flush = |run: &mut Vec<DVec2>, quays: &mut Vec<streets::Streamline>| {
            if geom::polyline_length(run) >= 100.0 {
                quays.push(streets::Streamline {
                    points: std::mem::take(run),
                    ..quay.clone()
                });
            }
            run.clear();
        };
        for p in geom::resample(&quay.points, 5.0) {
            if traced.any_within(p, 25.0, |_| false) {
                flush(&mut run, &mut quays);
            } else {
                run.push(p);
            }
        }
        flush(&mut run, &mut quays);
    }
    // And along the sea: a beach before the promenade, or a working quay's edge in industry.
    let coast_setback = |p: DVec2| match kind_at(p).map(|d| d.kind) {
        Some(DistrictKind::Industrial) => QUAY_SETBACK,
        _ => BEACH,
    };
    // The seafront stops where it would run beside a river's quay: at the mouth both ran there,
    // two walks 10 m apart.
    let river_quays: Vec<DVec2> = quays
        .iter()
        .flat_map(|q| geom::resample(&q.points, 5.0))
        .collect();
    for coast in streets::coast_lines(site, &coast_setback, &built) {
        let mut run: Vec<DVec2> = Vec::new();
        let flush = |run: &mut Vec<DVec2>, quays: &mut Vec<streets::Streamline>| {
            if geom::polyline_length(run) >= 100.0 {
                quays.push(streets::Streamline {
                    points: std::mem::take(run),
                    ..coast.clone()
                });
            }
            run.clear();
        };
        for p in geom::resample(&coast.points, 5.0) {
            if river_quays.iter().any(|q| q.distance(p) < 30.0) {
                flush(&mut run, &mut quays);
            } else {
                run.push(p);
            }
        }
        flush(&mut run, &mut quays);
    }
    lines.extend(quays.iter().cloned());
    let mut roads = graph::build(site, &lines, 1.5, 60.0);
    // Twice: the first pass moves junctions, and a pair it left may now be close.
    graph::tidy(&mut roads, site);
    graph::tidy(&mut roads, site);
    graph::smooth_jogs(&mut roads, site);
    let mut faces = graph::faces(&roads);
    // Superblocks cut by streets, a cut a round (a 600 m block takes three).
    if let Some(max) = params.max_block {
        for _ in 0..4 {
            let spacing = |p| {
                kind_at(p)
                    .filter(|d| d.building_depth().is_some())
                    .map(|d| d.street_spacing)
            };
            let cuts = superblock_cuts(site, &faces, max, spacing);
            if cuts.is_empty() {
                break;
            }
            lines.extend(cuts);
            roads = graph::build(site, &lines, 1.5, 60.0);
            // Twice: the first pass moves junctions, and a pair it left may now be close.
            graph::tidy(&mut roads, site);
            graph::tidy(&mut roads, site);
            graph::smooth_jogs(&mut roads, site);
            faces = graph::faces(&roads);
        }
    }
    lap("road graph and faces", &mut timings);

    let mut blocks = blocks::from_faces(site, &faces, &districts_field, &params.districts);
    blocks::add_parks(
        &mut blocks,
        params.park_reach,
        params.park_min_area,
        params.park_share_cap,
    );
    // Squares at the built districts' centres and the landmarks, and the old towns' small ones.
    // A district's square at most 1 ha; a landmark's 0.15–3.5 ha (a hall needs room). A crane
    // (on a quay) and a weir bridge (across its river) get none.
    let anchors: Vec<(DVec2, (f64, f64))> = params
        .districts
        .iter()
        .filter(|d| d.kind.building_depth().is_some())
        .map(|d| (d.anchor, (0.0, 10_000.0)))
        .chain(
            params
                .landmarks
                .iter()
                .filter(|l| !matches!(l.kind, LandmarkKind::GantryCrane | LandmarkKind::WeirBridge))
                .map(|l| (l.anchor, (1_500.0, 35_000.0))),
        )
        .collect();
    blocks::add_squares(&mut blocks, &anchors, &params.districts, 180.0);
    lap("blocks and parks", &mut timings);

    // The lines end in the city: a waypoint out in the countryside (the lines were drawn to the
    // frame's edges) moves in along its line to where the built districts begin, and one with no
    // city between it and the next goes. Lines ran on into empty land (the owner).
    let in_city = |p: DVec2| kind_at(p).is_some_and(|d| d.kind != DistrictKind::Reserve);
    // The first point in the city from `from` towards `to`, every 25 m.
    let edge_of_city = |from: DVec2, to: DVec2| {
        let steps = (from.distance(to) / 25.0).ceil().max(1.0) as usize;
        (1..=steps)
            .map(|k| from + (to - from) * (k as f64 / steps as f64))
            .find(|&q| in_city(q))
    };
    // From one end: drop the waypoints until one lies in the city, or moves in towards the next.
    let trim_front = |w: &mut Vec<DVec2>| {
        while w.len() >= 2 && !in_city(w[0]) {
            match edge_of_city(w[0], w[1]) {
                Some(q) => w[0] = q,
                None => {
                    w.remove(0);
                }
            }
        }
    };
    let specs: Vec<TramLineSpec> = params
        .tram_lines
        .iter()
        .map(|spec| {
            let mut w = spec.waypoints.clone();
            trim_front(&mut w);
            w.reverse();
            trim_front(&mut w);
            w.reverse();
            TramLineSpec {
                waypoints: w,
                ..spec.clone()
            }
        })
        .filter(|s| s.waypoints.len() >= 2)
        .collect();
    let mut tram_lines = transit::route_trams(&roads, &specs, params.stop_spacing);
    if params.max_extra_lines > 0 {
        let centres: Vec<DVec2> = blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Built)
            .map(|b| b.centre)
            .collect();
        let extra = transit::cover(
            &roads,
            &centres,
            &tram_lines,
            400.0,
            params.stop_spacing,
            params.stop_target,
            params.max_extra_lines,
        );
        tram_lines.extend(extra);
    }
    // One track where lines share a street.
    transit::share_tracks(&mut tram_lines);
    let crossings = transit::boat_crossings(site, &roads, params.boat_gap);
    transit::assign_profiles(&mut roads, &tram_lines, &districts_field, &params.districts);
    // The quays are walks: pedestrian streets whatever their district.
    for e in &mut roads.edges {
        let mid = geom::point_at(&e.points, e.length() * 0.5);
        let on_quay = quays.iter().any(|q| {
            q.points
                .windows(2)
                .any(|w| geom::distance_to_segment(mid, w[0], w[1]) < 2.0)
        });
        if on_quay && e.class == RoadClass::Street {
            e.profile = Profile::Pedestrian;
        }
    }
    lap("transit", &mut timings);

    // The taxi stands, before the street ground: their lay-bys are carriageway.
    let junctions: Vec<DVec2> = {
        let mut degree = vec![0_usize; roads.nodes.len()];
        for e in &roads.edges {
            degree[e.a as usize] += 1;
            degree[e.b as usize] += 1;
        }
        (0..roads.nodes.len())
            .filter(|&i| degree[i] >= 3)
            .map(|i| roads.nodes[i].pos)
            .collect()
    };
    let taxi_stands = taxi::stands(&roads, site, &tram_lines, &blocks, &junctions);
    let laybys: Vec<Vec<DVec2>> = taxi_stands.iter().map(|s| s.outline()).collect();
    // The depots behind the building lines of the taxi roads, industrial districts first; their
    // driveways' ramps up the verges.
    let industrial: Vec<bool> = blocks
        .iter()
        .map(|b| {
            params
                .districts
                .get(b.district)
                .is_some_and(|d| d.kind == DistrictKind::Industrial)
        })
        .collect();
    let anchors: Vec<DVec2> = params.landmarks.iter().map(|l| l.anchor).collect();
    let depots = taxi::depots(
        &roads,
        site,
        &blocks,
        &industrial,
        &taxi_stands,
        &junctions,
        &anchors,
    );
    let mut driveways: Vec<ground::Ramp> = depots
        .iter()
        .flat_map(|d| {
            let verge = roads.edges[d.edge as usize]
                .profile
                .section()
                .iter()
                .find(|s| s.lane == Lane::Verge)
                .map_or(2.5, |s| s.width);
            d.ramps(verge)
        })
        .collect();
    // The tram depots near the lines' ends, clear of the stops, the junctions, the taxi stands
    // and the taxi depots; their links' ramps up the verges.
    let tram_depots = {
        let stops: Vec<DVec2> = tram_lines.iter().flat_map(|l| l.stops.clone()).collect();
        let stands: Vec<DVec2> = taxi_stands
            .iter()
            .map(|s| s.kerb)
            .chain(depots.iter().flat_map(|d| [d.kerb, d.exit_kerb()]))
            .collect();
        let taken: Vec<u32> = depots.iter().map(|d| d.block).collect();
        tram_depot::depots(
            &roads,
            site,
            &blocks,
            &industrial,
            &tram_lines,
            &tram_depot::Avoid {
                stops: &stops,
                junctions: &junctions,
                stands: &stands,
                blocks: &taken,
            },
        )
    };
    driveways.extend(tram_depots.iter().map(tram_depot::TramDepot::ramp));
    let tracks: Vec<Vec<DVec2>> = tram_lines.iter().map(|l| l.points.clone()).collect();
    let walks = streets::promenades(site, ground::PROMENADE);
    let rivers: Vec<(Vec<DVec2>, f64)> = site
        .rivers
        .iter()
        .map(|r| (r.centreline.clone(), r.width))
        .collect();
    let ground = ground::GroundMap::build(
        &roads,
        site.size,
        ground::TILE,
        &tracks,
        &walks,
        &rivers,
        &laybys,
        &driveways,
    );
    lap("street ground", &mut timings);
    // Courtyards and lots once the streets' widths are known.
    blocks::courtyards(&mut blocks, &roads, &ground, &params.districts, site.size);
    let lots = blocks::subdivide(&blocks, &params.districts, seed.derive_str("lots").value());
    lap("courtyards and lots", &mut timings);

    let landmarks = params
        .landmarks
        .iter()
        .map(|l| Landmark {
            name: l.name.clone(),
            // A weir bridge spans its river at its anchor, off the roads.
            pos: match l.kind {
                LandmarkKind::WeirBridge => l.anchor,
                _ => transit::nearest_main_node(&roads, l.anchor).unwrap_or(l.anchor),
            },
            kind: l.kind,
            square: landmark_site(&blocks, l),
        })
        .collect();
    lap("landmarks", &mut timings);

    let mut plan = CityPlan {
        size: site.size,
        districts: params.districts.clone(),
        district_field: districts_field,
        roads,
        blocks,
        lots,
        tram_lines,
        ground,
        taxi_stands,
        depots,
        tram_depots,
        crossings,
        landmarks,
        stats: PlanStats::default(),
        timings: Vec::new(),
    };
    plan.stats = map::stats(site, &plan, params);
    lap("stats", &mut timings);
    plan.timings = timings;
    plan
}

/// The streets that cut the superblocks among `faces` ([`CityParams::max_block`]): across the
/// middle of each built block on land longer than `max` times its district's street spacing
/// (`spacing` at a point, `None` where nothing is built), square to its length and 15 m past
/// its edges, so the graph joins it to the streets round it. Where the traced streets ran far
/// apart, a block's courtyard was a park of a hectare or more and its fronts ran for hundreds
/// of metres (the owner's second walk, 2026-10-11).
fn superblock_cuts(
    site: &Site,
    faces: &[Vec<DVec2>],
    max: f64,
    spacing: impl Fn(DVec2) -> Option<f64>,
) -> Vec<streets::Streamline> {
    let mut cuts = Vec::new();
    for face in faces {
        let area = geom::signed_area(face).abs();
        if !(1_000.0..2.0e6).contains(&area) {
            continue;
        }
        let (axis, extent, centre) = geom::oriented_extent(face);
        let Some(spacing) = spacing(centre) else {
            continue;
        };
        let (long, short) = (extent.x.max(extent.y), extent.x.min(extent.y));
        if long < max * spacing || site.water_at(centre) != LAND {
            continue;
        }
        let along = if extent.x >= extent.y {
            axis
        } else {
            DVec2::new(-axis.y, axis.x)
        };
        let across = DVec2::new(-along.y, along.x);
        let reach = short * 0.5 + 15.0;
        let points = geom::resample(&[centre - across * reach, centre + across * reach], 10.0);
        if points.iter().any(|&p| site.water_at(p) != LAND) {
            continue;
        }
        cuts.push(streets::Streamline {
            points,
            minor: true,
            class: RoadClass::Street,
            bridges: Vec::new(),
        });
    }
    cuts
}

#[cfg(test)]
mod tests;
