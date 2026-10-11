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

pub mod blocks;
pub mod example;
pub mod geom;
pub mod graph;
pub mod map;
pub mod streets;
pub mod tensor;
pub mod transit;

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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

impl DistrictSpec {
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
    /// A street shared by people, bikes and the service vehicles that may come in.
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

    /// The width between the building lines, metres.
    pub fn width(self) -> f64 {
        match self {
            Self::TramAvenue => 28.0,
            Self::Avenue => 22.0,
            Self::Shared => 12.0,
            Self::Pedestrian => 8.0,
            Self::Path => 4.0,
        }
    }
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

/// A landmark to place: snapped to the nearest main-road node.
#[derive(Clone, Debug, PartialEq)]
pub struct LandmarkSpec {
    /// Its name.
    pub name: String,
    /// Where it should stand, roughly.
    pub anchor: DVec2,
}

/// A placed landmark.
#[derive(Clone, Debug, PartialEq)]
pub struct Landmark {
    /// Its name.
    pub name: String,
    /// Where it stands.
    pub pos: DVec2,
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
        best.1
    })
}

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
    let mut seeds = vec![params.centre];
    seeds.extend(params.districts.iter().map(|d| d.anchor));
    let main = streets::trace_level(
        &field,
        site,
        &[],
        &main_params,
        &main_spacing,
        &seeds,
        seed.derive_str("main roads").value(),
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
    );
    lap("streets", &mut timings);

    let mut lines = main;
    lines.extend(streets);
    let mut roads = graph::build(site, &lines, 1.5, 60.0);
    let faces = graph::faces(&roads);
    lap("road graph and faces", &mut timings);

    let mut blocks = blocks::from_faces(site, &faces, &districts_field, &params.districts);
    blocks::add_parks(
        &mut blocks,
        params.park_reach,
        params.park_min_area,
        params.park_share_cap,
    );
    let lots = blocks::subdivide(&blocks, &params.districts, seed.derive_str("lots").value());
    lap("blocks, parks and lots", &mut timings);

    let tram_lines = transit::route_trams(&roads, &params.tram_lines, params.stop_spacing);
    let crossings = transit::boat_crossings(site, &roads, params.boat_gap);
    transit::assign_profiles(&mut roads, &tram_lines, &districts_field, &params.districts);
    let landmarks = params
        .landmarks
        .iter()
        .map(|l| Landmark {
            name: l.name.clone(),
            pos: transit::nearest_main_node(&roads, l.anchor).unwrap_or(l.anchor),
        })
        .collect();
    lap("transit and landmarks", &mut timings);

    let mut plan = CityPlan {
        size: site.size,
        districts: params.districts.clone(),
        district_field: districts_field,
        roads,
        blocks,
        lots,
        tram_lines,
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

#[cfg(test)]
mod tests;
