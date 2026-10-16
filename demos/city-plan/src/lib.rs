//! `city-plan`: a city's plan in 3D (D-059's step 1 seen from the street, before step 2's
//! modules). Any [`CityPlan`] from `forge_procgen::city` and its masses become meshes in tiles
//! of [`TILE`] metres, drawn by `city-blocks`' renderer through its scene interface (#216), with
//! its sky, sun, ray-traced shadows, probes and anti-aliasing:
//! - the ground from the site's mask: land, and water a little below it;
//! - gardens and courtyards, parks and squares from the blocks;
//! - the street ground from the plan's map (D-061): carriageways, crossings, pavements and kerbs,
//!   tram tracks along the lines;
//! - every mass as walls and a roof: green, solar or terrace.
//!
//! The binary draws the engine's invented example city; a private sandbox hands its own plan to
//! [`PlanScenario::new`].

#![forbid(unsafe_code)]

use std::borrow::Cow;

use anyhow::Result;
use city_blocks::scenario::{Look, Running, Scenario};
use city_blocks::{Args, CityMaterials, Cooked, scene_origin};
use forge_app::{FlyCamera, Input, Setup};
use forge_core::material::{RenderLayer, TextureId, UvMapping};
use forge_geom::MeshletMesh;
use forge_geom::city::PropSpec;
use forge_geom::meshlet::CookOptions;
use forge_geom::procedural::TriMesh;
use forge_geom::{modules, vehicles};
use forge_procgen::city::facades::{self, FaceKind};
use forge_procgen::city::gardens;
use forge_procgen::city::lamps::Lamp;
use forge_procgen::city::mass::{Mass, Roof};
use forge_procgen::city::roofs::{RUN_MAX, ridge};
use forge_procgen::city::shore::Shore;
use forge_procgen::city::structure::{self, OpeningKind, Structure, WallRole};
use forge_procgen::city::trees::Tree;
use forge_procgen::city::units::{self, Occupancy};
use forge_procgen::city::{
    BlockUse, CityPlan, Site, cores, furnish, geom, ground, interior, roofs,
};
use forge_procgen::grammar::Grammar;
use forge_render::MeshletSceneBuilder;
use forge_render::textures;
use forge_render::{MeshletScene, MoverTransform};
use forge_sim::transit::{Service, Timetable};
use glam::{DVec2, Mat4, Vec3};
use winit::keyboard::KeyCode;

mod crowd;
mod residents;
mod signs;
mod taxis;
mod walk;

/// A tile's side, metres.
pub const TILE: f64 = 250.0;

/// Ground sections, in their material rows' order.
const GROUND: u8 = 0;
const GARDEN: u8 = 1;
const PARK: u8 = 2;
const PAVING: u8 = 3;
const STREET: u8 = 4;
const TRACK: u8 = 5;
const WATER: u8 = 6;
/// The quay walls' section on the ground.
const QUAY: u8 = 7;
/// The crossings' paint.
const MARKING: u8 = 8;
/// A square's water mirror: a film of water over dark stone (the owner: "water mirrors as in
/// Nantes").
const MIRROR: u8 = 9;
/// Mass sections: four wall tints, the shop band, three roofs.
const WALL: u8 = 0;
const TINTS: u8 = 4;
const SHOP: u8 = 4;
const ROOF_GREEN: u8 = 5;
const ROOF_SOLAR: u8 = 6;
const ROOF_TERRACE: u8 = 7;
/// The kit's merged sections on the masses: four render tints, stone, party walls.
const KIT_PLASTER: u8 = 8;
/// The insides' walls and ceilings: whitewash whatever the facade's tint, the white render without
/// its grain (the row after the tiled roof). In the facade's sand, ochre or rose, the daylight
/// bouncing round a room took the colour at every bounce, and the halls read orange; with the
/// facades' grain the walls a step away showed squiggles.
const KIT_INSIDE: u8 = 15;
/// The insides' floors: the stone's colour, smooth (its pits read as squiggles a step away).
const KIT_INSIDE_FLOOR: u8 = 16;
const KIT_STONE: u8 = 12;
const KIT_PARTY: u8 = 13;
/// The tiled roofs' section on the masses.
const ROOF_TILE: u8 = 14;
/// The roof tiles' repeat, metres: the roofs' texture coordinates run along the eaves.
const TILE_REPEAT: f64 = 2.5;
/// The kit's instance groups: four shutter colours, each with dark and lit glass.
const KIT_GROUPS: [&str; 8] = [
    "plan-kit-0",
    "plan-kit-1",
    "plan-kit-2",
    "plan-kit-3",
    "plan-kit-4",
    "plan-kit-5",
    "plan-kit-6",
    "plan-kit-7",
];

/// One tile's two meshes, positions relative to its centre.
struct Tile {
    centre: Vec3,
    ground: TriMesh,
    masses: TriMesh,
    /// The kit's railings' bars, one card each, drawn as a grating (`take_cards`).
    railings: TriMesh,
    /// The kit buildings' insides (`insides`), drawn with the masses' rows but cooked apart:
    /// merged into the masses, their own section changed how the masses simplify, and the
    /// coarser cut the sun's shadow rays meet put a ceiling through a roof (#273).
    insides: TriMesh,
}

impl Tile {
    fn new(centre: Vec3) -> Self {
        Self {
            centre,
            ground: TriMesh::default(),
            masses: TriMesh::default(),
            railings: TriMesh::default(),
            insides: TriMesh::default(),
        }
    }
}

/// Adds a triangle to a mesh, turned so its face looks along `normal`.
fn triangle(mesh: &mut TriMesh, mut p: [Vec3; 3], normal: Vec3, section: u8) {
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(normal) < 0.0 {
        p.swap(1, 2);
    }
    let base = mesh.positions.len() as u32;
    for q in p {
        mesh.positions.push(q.to_array());
        mesh.normals.push(normal.to_array());
    }
    mesh.indices.extend([base, base + 1, base + 2]);
    mesh.sections.push(section);
}

/// The plan's meshes, tiled.
struct Tiles {
    size: f64,
    side: usize,
    tiles: Vec<Tile>,
}

impl Tiles {
    fn new(size: f64) -> Self {
        let side = (size / TILE).ceil() as usize;
        let mut tiles = Vec::with_capacity(side * side);
        for y in 0..side {
            for x in 0..side {
                let c = DVec2::new((x as f64 + 0.5) * TILE, (y as f64 + 0.5) * TILE);
                tiles.push(Tile::new(world(size, c, 0.0)));
            }
        }
        Self { size, side, tiles }
    }

    /// The tile holding a point of the plan.
    fn at(&mut self, p: DVec2) -> &mut Tile {
        let x = ((p.x / TILE).floor().max(0.0) as usize).min(self.side - 1);
        let y = ((p.y / TILE).floor().max(0.0) as usize).min(self.side - 1);
        &mut self.tiles[y * self.side + x]
    }

    /// A flat polygon of the plan at `height`, facing up, in the tile of its first point.
    fn flat(&mut self, polygon: &[DVec2], height: f64, section: u8, mass: bool) {
        if polygon.len() < 3 {
            return;
        }
        // Whole: ear clipping alone gave up on some of the booleans' outlines, gaps in the ground.
        self.flat_triangles(
            polygon[0],
            &ground::triangles(polygon),
            height,
            section,
            mass,
        );
    }

    /// A flat polygon's `triangles` ([`ground::triangles`]) at `height`, facing up, in the tile
    /// of its point `first`.
    fn flat_triangles(
        &mut self,
        first: DVec2,
        triangles: &[[DVec2; 3]],
        height: f64,
        section: u8,
        mass: bool,
    ) {
        let size = self.size;
        let tile = self.at(first);
        let mesh = if mass {
            &mut tile.masses
        } else {
            &mut tile.ground
        };
        let centre = tile.centre;
        for t in triangles {
            let p = t.map(|q| world(size, q, height) - centre);
            triangle(mesh, p, Vec3::Y, section);
        }
    }
}

/// A point of the plan (`x` east, `y` south) at `height` in the world, the frame centred on the
/// origin (+Z south, as −Z is north).
fn world(size: f64, p: DVec2, height: f64) -> Vec3 {
    Vec3::new(
        (p.x - size * 0.5) as f32,
        height as f32,
        (p.y - size * 0.5) as f32,
    )
}

/// Lines ending at one hub stood their trams on one spot for their turns: each further line
/// ending within 30 m of another's terminus now ends 45 m before it, along its own track
/// (while its last leg stays 60 m or more).
fn stagger_termini(trams: &mut [TramRun]) {
    let mut ends: Vec<DVec2> = Vec::new();
    for run in trams.iter_mut() {
        let mut stops = run.timetable.stops().to_vec();
        let n = stops.len();
        for (end, next) in [(0, 1), (n - 1, n - 2)] {
            let p = run.track(stops[end]).0;
            let taken = ends.iter().filter(|e| e.distance(p) < 30.0).count() as f64;
            let moved = stops[end] + (stops[next] - stops[end]).signum() * 45.0 * taken;
            if (stops[next] - moved).abs() >= 60.0
                && (moved - stops[end]).abs() < (stops[next] - stops[end]).abs()
            {
                stops[end] = moved;
            }
            ends.push(p);
        }
        run.timetable = run.timetable.with_stops(&stops);
    }
}

/// How often two trams of different lines stand within 30 m of each other on one track (each
/// pair at each 5 s step of the service's second hour): trams running through each other.
fn tram_overlaps(trams: &[TramRun]) -> usize {
    let mut count = 0;
    let mut t = 3600.0;
    while t < 7200.0 {
        // Each tram's place on its track and its heading.
        let mut at: Vec<(usize, DVec2, DVec2)> = Vec::new();
        for (line, run) in trams.iter().enumerate() {
            for k in 0..run.timetable.vehicles() {
                let state = run.timetable.at(k, t);
                let (p, along) = run.track(state.s);
                let side = if state.outbound { 1.0 } else { -1.0 };
                let n = DVec2::new(-along.y, along.x);
                at.push((line, p + n * side * TRACK_OFFSET, along * side));
            }
        }
        for (i, a) in at.iter().enumerate() {
            for b in &at[i + 1..] {
                if a.0 != b.0 && a.1.distance(b.1) < 30.0 && a.2.dot(b.2) > 0.7 {
                    count += 1;
                }
            }
        }
        t += 5.0;
    }
    count
}

/// The point of the site's rivers nearest `p`, the river's direction there and its width.
fn river_crossing(site: &Site, p: DVec2) -> Option<(DVec2, DVec2, f64)> {
    site.rivers
        .iter()
        .filter_map(|r| {
            let line = geom::resample(&r.centreline, 5.0);
            let n = line.len();
            let i = (0..n).min_by(|&a, &b| line[a].distance(p).total_cmp(&line[b].distance(p)))?;
            let along = (line[(i + 1).min(n - 1)] - line[i.saturating_sub(1)]).normalize_or_zero();
            (along != DVec2::ZERO).then_some((line[i], along, r.width))
        })
        .min_by(|a, b| a.0.distance(p).total_cmp(&b.0.distance(p)))
}

/// A flat colour row.
fn colour(rgb: [f32; 3], power: f32, specular: f32) -> RenderLayer {
    RenderLayer {
        color_a: rgb,
        color_b: rgb,
        roughness: RenderLayer::roughness_for_power(power),
        specular,
        ..RenderLayer::default()
    }
}

/// A plan in 3D: its meshes, built on the CPU, cooked and drawn when the app builds the scene.
pub struct PlanScenario {
    tiles: Vec<Tile>,
    /// Each tree's place in the world, its scale (its crown over the model's 4.2 m radius, its
    /// height over the model's 10 m up), its turn and which of the two leaf colours it uses.
    trees: Vec<(Vec3, Vec3, f32, usize)>,
    /// The forest's plants by species (D-063): each one's place, its height and crown radius,
    /// its turn, its species (an index into `forge_flora::species::mediterranean()`) and a hash
    /// byte that picks which of that species' grown individuals it is and stretches its crown.
    flora: Vec<(Vec3, f32, f32, f32, u16, u8)>,
    /// Each lamp's place, scale (its height over the model's 8 m) and turn.
    lamps: Vec<(Vec3, f32, f32, bool)>,
    /// The style kit's instanced modules and their instances (module, transform, group).
    kit_meshes: Vec<TriMesh>,
    kit_instances: Vec<(u32, Mat4, u8)>,
    /// The tram lines' runs, and the service's clock at the start, seconds.
    trams: Vec<TramRun>,
    /// The river shuttles, one a crossing.
    boats: Vec<BoatRun>,
    clock: f64,
    /// The buildings' walls as convex pieces (the world's metres), for the walker.
    walls: Vec<Vec<Vec3>>,
    /// How many passages into courts, and where to look into one, for the log.
    passage_note: String,
    /// What each mass holds (D-060 step 3): its homes' households and residents, its jobs, its
    /// net floor area.
    holds: Vec<Occupancy>,
    walk_on_start: bool,
    /// The names the fingerposts' blades carry, a row of their atlas each.
    sign_names: Vec<String>,
    /// Seconds into the run at which to board the nearest tram (scripted rides).
    ride_after: Option<f64>,
    crowd: Option<crowd::Crowd>,
    /// The automated taxis (D-061).
    taxis: Option<taxis::Taxis>,
    size: f64,
    /// What the kit built, logged once the app logs.
    kit_report: Option<String>,
    /// The share of the kit's homes' windows their residents lit: the far city's too.
    far_lit: Option<f64>,
    /// The terraces' seats, for the crowd.
    seats: Vec<(Vec3, f32)>,
    /// The chess tables: each one's middle and its two seats among `seats`.
    games: Vec<(Vec3, [usize; 2])>,
    /// The market stalls, for the crowd's stallholders and shoppers.
    stalls: Vec<Mat4>,
    camera: FlyCamera,
    title: String,
    triangles: usize,
    /// The scene's name in the mesh cache: its size and short title (#274).
    scene: String,
    /// Its meshes, cooked or read from the cache on the loading thread ([`Scenario::prepare`]),
    /// in the order the build adds them, each plant's with its height and crown.
    cooked: Option<Vec<(MeshletMesh, f32, f32)>>,
}

/// Buildings drawn from a style set's grammar and kit (D-039's modules) within `radius` of
/// `centre`; the masses beyond stay walls with facade textures.
pub struct Detail<'a> {
    /// The style set's rules.
    pub grammar: &'a Grammar,
    /// What the style puts over the roof line, metres.
    pub top: f64,
    /// Where, on the plan.
    pub centre: DVec2,
    /// How far round it, metres.
    pub radius: f64,
    /// The style's seed.
    pub seed: u64,
    /// The railings' bars on one card each, a grating that cannot flicker, in place of
    /// geometry (`--railing-cards`): it costs the overview 0.45 ms while the cut-out raster
    /// runs every cluster twice (`research/trim-sheets.md` §2.9).
    pub railing_cards: bool,
    /// The hour the windows are lit for (the demos' `--time-of-day`), or `None`: two windows in
    /// five. The homes' windows light by their residents, at home and awake or not
    /// (`residents`), the shops' by `lit_share`.
    pub lit_hour: Option<f64>,
    /// The residents' roles (`with_people_mix`'s): `Mix::adults()` for a world with no children.
    pub mix: forge_sim::population::Mix,
}

/// The hour (0 to 24) of a `--time-of-day` (0 sunrise, 0.5 noon, 1 sunset, 1.5 midnight), with
/// the sun up from 6 to 18.
pub fn hour_of_day(time_of_day: f32) -> f64 {
    (6.0 + f64::from(time_of_day) * 12.0).rem_euclid(24.0)
}

/// Whether the trams pause at `hour` (0 to 24): from 1:30 to 4:30 the lines stop and their trams
/// stand in the depots, as French networks run from about half past four to half past one.
pub fn tram_pause(hour: f64) -> bool {
    (1.5..4.5).contains(&hour)
}

/// The share of a building's windows lit at `hour` (0 to 24), a shop's when `shop` (estimates,
/// until the residents light their own windows by what they do, D-060's step 3): homes lit
/// through the evening, half of them by nine, a tenth at midnight and one in twenty in the small
/// hours, some again before dawn; shops lit while open, 8 to 19, and a few windows' displays
/// all night. Daylight drowns them anyway.
pub fn lit_share(hour: f64, shop: bool) -> f64 {
    const HOMES: [(f64, f64); 12] = [
        (0.0, 0.10),
        (1.0, 0.05),
        (5.0, 0.05),
        (6.0, 0.15),
        (7.0, 0.25),
        (9.0, 0.15),
        (17.0, 0.25),
        (19.0, 0.45),
        (21.0, 0.50),
        (22.0, 0.40),
        (23.0, 0.25),
        (24.0, 0.10),
    ];
    const SHOPS: [(f64, f64); 7] = [
        (0.0, 0.10),
        (7.0, 0.10),
        (8.0, 0.60),
        (19.0, 0.80),
        (20.0, 0.30),
        (21.0, 0.15),
        (24.0, 0.10),
    ];
    let curve: &[(f64, f64)] = if shop { &SHOPS } else { &HOMES };
    let hour = hour.clamp(0.0, 24.0);
    curve
        .windows(2)
        .find(|w| hour <= w[1].0)
        .map_or(curve[curve.len() - 1].1, |w| {
            let t = (hour - w[0].0) / (w[1].0 - w[0].0);
            w[0].1 + (w[1].1 - w[0].1) * t
        })
}

impl PlanScenario {
    /// The scene of `plan` on `site` with its `masses` and `trees`, the camera starting at
    /// `camera`.
    pub fn new(
        site: &Site,
        plan: &CityPlan,
        masses: &[Mass],
        trees: &[Tree],
        lamps: &[Lamp],
        camera: FlyCamera,
        title: &str,
    ) -> Self {
        Self::detailed(site, plan, masses, trees, lamps, camera, title, None)
    }

    /// As [`PlanScenario::new`], with the buildings near `detail`'s centre made of its kit.
    #[allow(clippy::too_many_arguments)]
    pub fn detailed(
        site: &Site,
        plan: &CityPlan,
        masses: &[Mass],
        trees: &[Tree],
        lamps: &[Lamp],
        camera: FlyCamera,
        title: &str,
        detail: Option<&Detail>,
    ) -> Self {
        let mut laps = Laps::new();
        let size = plan.size;
        let mut tiles = Tiles::new(size);
        let shore = ground_shore(site);
        // The reserves' land as scrub and grass, the rest as the open ground's gravel.
        ground_and_water(&mut tiles, &shore);
        laps.mark("ground and water");
        // The buildings' structure (D-060): the kit dresses it, the walker walks against it, the
        // gardens and the paths round the courts start from its doors.
        let structures = structure::structures(plan, masses);
        // Who each building holds, counted by home and workplace (D-060 step 3).
        let holds: Vec<Occupancy> = structures.iter().map(units::occupancy).collect();
        let gardens = gardens::gardens(plan, masses, &structures);
        laps.mark("structures and gardens");
        // Each block's paved ground off its forecourt: the paths across its court, the flats'
        // terraces, its kitchen garden's. Paved with the forecourt, one surface: drawn over the
        // paving, a path began down the street's middle, lay across the carriageway, and showed
        // as a patch on the pavement (the owner).
        let mut paths = vec![Vec::new(); plan.blocks.len()];
        for g in gardens.iter().filter(|g| g.kind.paved()) {
            paths[g.block as usize].push(g.outline.clone());
        }
        // The large squares' middles: lawns and water mirrors.
        let middles = square_middles(plan);
        // The blocks off the streets, at the walking level as the pavements round them (D-061):
        // one level wherever people walk, no step at a gap between the buildings. Each block's
        // pieces cut and triangulated on the machine's threads, then laid in the tiles in the
        // blocks' order: one thread took 13 s over the city's.
        let pieces = par_map(plan.blocks.len(), cores(), None, |b| {
            let block = &plan.blocks[b];
            let mut pieces: Vec<(DVec2, Vec<[DVec2; 3]>, f64, u8)> = Vec::new();
            let mut lay = |shapes: &ground::Shapes, level: f64, section: u8| {
                for polygon in ground::simple_polygons(shapes) {
                    if polygon.len() >= 3 {
                        pieces.push((polygon[0], ground::triangles(&polygon), level, section));
                    }
                }
            };
            let section = match block.use_ {
                BlockUse::Built => GARDEN,
                BlockUse::Park => PARK,
                BlockUse::Square => PAVING,
                // A quay's face holds its river: the land under it is the ground's paving.
                BlockUse::Quay => return pieces,
            };
            if block.use_ == BlockUse::Built {
                // A paved forecourt along its streets, as the pavement it meets, its gardens
                // behind: grass met the pavement in the gaps between the buildings (the owner).
                // The paths across its green paved with it, one surface.
                // A tram depot's yard paved with the forecourt, then cut out of it in asphalt.
                let yard = plan
                    .tram_depots
                    .iter()
                    .find(|d| d.block == b as u32)
                    .map(|d| d.yard());
                let mut ways = paths[b].clone();
                ways.extend(yard.clone());
                let (front, rest) = plan.ground.forecourt(&block.polygon, FORECOURT, &ways);
                let front = match &yard {
                    Some(yard) => {
                        let (inside, outside) = ground::split(&front, yard);
                        lay(&inside, WALK_LEVEL, STREET);
                        outside
                    }
                    None => front,
                };
                lay(&front, WALK_LEVEL, PAVING);
                lay(&rest, WALK_LEVEL, section);
            } else if let Some((middle, mirror)) = &middles[b] {
                // A large square's lawn or water mirror, cut out of its paving at its level: one
                // laid over the other would fight it, and the water is level with the stone.
                let (inside, outside) = plan.ground.off_streets_split(&block.polygon, middle);
                let within = if *mirror { MIRROR } else { PARK };
                lay(&inside, SQUARE_LEVEL, within);
                lay(&outside, SQUARE_LEVEL, section);
            } else {
                lay(
                    &plan.ground.off_streets(&block.polygon),
                    WALK_LEVEL,
                    section,
                );
            }
            pieces
        });
        for (first, triangles, level, section) in pieces.iter().flatten() {
            tiles.flat_triangles(*first, triangles, *level, *section, false);
        }
        laps.mark("blocks");
        // The quays' land as lawns, cell by cell (their faces hold water), under the trees.
        for block in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Quay) {
            lawn(&mut tiles, &shore, &plan.ground, &block.polygon);
        }
        laps.mark("quays");
        // The rivers as smooth strips at the water's level, the sea's: the cells' staircase
        // would show along the quays. Their promenades are walking ground in the street map
        // (D-061), the banks' walls and balustrades along its edge over the water ([`bridges`]).
        for t in &plan.ground.tiles {
            for polygon in ground::simple_polygons(&plan.ground.water_in(t.min, t.max)) {
                tiles.flat(&polygon, WATER_LEVEL, WATER, false);
            }
        }
        laps.mark("rivers");
        street_ground(&mut tiles, &plan.ground, site);
        laps.mark("street ground");
        bridges(&mut tiles, plan, site, &shore);
        laps.mark("bridges");
        // Two tracks a line, one each way, on the right of the way they run; where lines share a
        // track (`transit::share_tracks`) only the first draws it: two lines' rails one over the
        // other would fight.
        for (i, line) in plan.tram_lines.iter().enumerate() {
            let shared = |p: DVec2| {
                plan.tram_lines[..i].iter().any(|o| {
                    o.points
                        .windows(2)
                        .any(|w| geom::distance_to_segment(p, w[0], w[1]) < 0.05)
                })
            };
            let points = geom::resample(&line.points, 4.0);
            let mut runs: Vec<Vec<DVec2>> = vec![Vec::new()];
            for (k, &p) in points.iter().enumerate() {
                let next = points.get(k + 1).copied().unwrap_or(p);
                if shared(p) && shared(next) {
                    if !runs.last().is_some_and(Vec::is_empty) {
                        runs.last_mut().expect("one").push(p);
                        runs.push(Vec::new());
                    }
                } else {
                    runs.last_mut().expect("one").push(p);
                }
            }
            for run in runs.iter().filter(|r| r.len() >= 2) {
                for track in [-TRACK_OFFSET, TRACK_OFFSET] {
                    for rail in [-0.72, 0.72] {
                        let rail = offset_line(run, track + rail);
                        ribbon(&mut tiles, &rail, 0.25, 0.22, TRACK);
                    }
                }
            }
        }
        laps.mark("tracks");
        let depot_walls = tram_depots(&mut tiles, plan);
        let garden_walls = garden_walls(&mut tiles, plan);
        laps.mark("depots and walls");
        // Line by line, each taking the stops the lines before it placed on its track.
        let mut trams: Vec<TramRun> = Vec::new();
        let mut taken: Vec<DVec2> = Vec::new();
        for (i, l) in plan.tram_lines.iter().enumerate() {
            if let Some(mut run) = TramRun::new(l, plan, site, &taken) {
                run.line = i;
                taken.extend(run.timetable.stops().iter().map(|&s| run.track(s).0));
                trams.push(run);
            }
        }
        stagger_termini(&mut trams);
        // The trams the depots keep: two places in three taken, in their lines' liveries; every
        // place in the night's pause, when the lines' trams are all in.
        let night = detail.and_then(|d| d.lit_hour).is_some_and(tram_pause);
        for d in &plan.tram_depots {
            let Some(run) = trams.iter_mut().find(|r| r.line == d.line) else {
                continue;
            };
            for (k, (at, along)) in d.places().into_iter().enumerate() {
                if night || !mix(k as u64 ^ (u64::from(d.block) << 16)).is_multiple_of(3) {
                    run.parked.push(parked_sections(at, along, size));
                }
            }
        }
        for run in &mut trams {
            run.paused = night;
        }
        // Each terminus's crossover, the rails from the track the trams come in on to the one
        // they leave by, 3 mm over the straight rails they cross.
        for run in &trams {
            let [(a0, a1), (b0, b1)] = run.crossovers();
            for ((from, to), outbound) in [((a0, a1), false), ((b0, b1), true)] {
                let steps = (to - from).ceil().max(1.0) as usize;
                for rail in [-0.72, 0.72] {
                    let points: Vec<DVec2> = (0..=steps)
                        .map(|k| {
                            let s = from + (to - from) * k as f64 / steps as f64;
                            let (p, t) = run.track(s);
                            p + DVec2::new(-t.y, t.x) * (run.side(s, outbound) + rail)
                        })
                        .collect();
                    ribbon(&mut tiles, &points, 0.25, 0.223, TRACK);
                }
            }
            // And its crossover pairs along the line.
            for start in run.crossover_pairs() {
                for from in [start, start + CROSSOVER + 10.0] {
                    let steps = CROSSOVER.ceil() as usize;
                    for rail in [-0.72, 0.72] {
                        let points: Vec<DVec2> = (0..=steps)
                            .map(|k| {
                                let s = from + CROSSOVER * k as f64 / steps as f64;
                                let (p, t) = run.track(s);
                                p + DVec2::new(-t.y, t.x) * (TramRun::pair_side(start, s) + rail)
                            })
                            .collect();
                        ribbon(&mut tiles, &points, 0.25, 0.223, TRACK);
                    }
                }
            }
        }
        laps.mark("trams");
        // What each wall looks onto: the kit's faces and where the roofs end in gables, which
        // want a neighbour as tall across the party wall (else the roof hips).
        let edges = facades::edge_kinds(plan, masses);
        let tall = roofs::tall(plan, masses);
        laps.mark("faces");
        let mut near = Vec::new();
        for (i, m) in masses.iter().enumerate() {
            match detail {
                Some(d) if geom::centroid(&m.footprint).distance(d.centre) < d.radius => {
                    near.push(i);
                }
                _ => mass(&mut tiles, m, &edges[i], &tall[i]),
            }
        }
        head_houses(&mut tiles, masses, &structures);
        // The planters, the hedges and the kitchen gardens' beds (the owner's note, 2026-10-15).
        for g in gardens.iter().filter(|g| {
            g.kind.standing() && !matches!(g.kind, gardens::Kind::Bench | gardens::Kind::BikeRack)
        }) {
            let side = match g.kind {
                gardens::Kind::Hedge => ROOF_GREEN,
                _ => KIT_STONE,
            };
            prism_topped(
                &mut tiles,
                &g.outline,
                WALK_LEVEL,
                WALK_LEVEL + g.height,
                side,
                ROOF_GREEN,
            );
        }
        laps.mark("masses");
        let kit = detail.map(|d| {
            // The residents at home and awake at the hour light their own windows.
            let residents = d.lit_hour.map(|h| {
                residents::Residents::new(plan, masses, &structures, &holds, &d.mix, h, &near)
            });
            kit(
                &mut tiles,
                masses,
                &structures,
                &edges,
                &tall,
                &near,
                d,
                residents.as_ref(),
                &plan.ground.bollards,
            )
        });
        laps.mark("kit");
        // Their insides, for a camera that goes through a wall.
        let inside_triangles = if detail.is_some() {
            insides(&mut tiles, masses, &structures, &near)
        } else {
            0
        };
        laps.mark("insides");
        // The passages into the courts (D-060), and where to look into one: 10 m out on the
        // street before the kit's first, for the log once the app logs.
        let passages_in = |list: &mut dyn Iterator<Item = usize>| {
            list.map(|i| {
                structures[i]
                    .openings
                    .iter()
                    .filter(|o| o.kind == OpeningKind::Passage)
                    .count()
                    / 2
            })
            .sum::<usize>()
        };
        let mut passage_note = format!(
            "passages into courts (D-060): {} in the city, {} in the kit",
            passages_in(&mut (0..masses.len())),
            passages_in(&mut near.iter().copied())
        );
        // And on the nearest bridge's pavement, a quarter along, looking along it.
        if let Some(d) = detail
            && let Some(e) = plan
                .roads
                .edges
                .iter()
                .filter(|e| e.bridge && e.profile.pavement() > 0.0 && e.points.len() >= 2)
                .min_by(|a, b| {
                    let mid = |e: &forge_procgen::city::RoadEdge| {
                        geom::point_at(&e.points, e.length() * 0.5)
                    };
                    mid(a)
                        .distance(d.centre)
                        .total_cmp(&mid(b).distance(d.centre))
                })
        {
            let s = e.length() * 0.25;
            let (p, t) = (geom::point_at(&e.points, s), geom::tangent_at(&e.points, s));
            let side =
                DVec2::new(-t.y, t.x) * (e.profile.width() * 0.5 - e.profile.pavement() * 0.5);
            let eye = world(size, p + side, 1.9);
            let yaw = (-t.x).atan2(-t.y).to_degrees();
            passage_note += &format!(
                "; a bridge's pavement at --view={:.1},1.9,{:.1},{yaw:.1},-10",
                eye.x, eye.z
            );
        }
        if let Some((wall, o)) = near.iter().find_map(|&i| {
            let s = &structures[i];
            s.openings
                .iter()
                .find(|o| o.kind == OpeningKind::Passage)
                .map(|o| (&s.walls[o.wall], o))
        }) {
            let t = (wall.b - wall.a).normalize_or_zero();
            let mouth = wall.a + t * o.along;
            let out = DVec2::new(t.y, -t.x);
            let eye = world(size, mouth + out * 10.0, 1.7);
            let to = (world(size, mouth, 1.7) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            passage_note += &format!("; one at --view={:.1},1.7,{:.1},{yaw:.1},0", eye.x, eye.z);
        }
        // And the court of the kitchen garden nearest the kit's centre, from 25 m south of it
        // and 20 m up, looking north over it.
        if let Some(d) = detail
            && let Some(plot) = gardens
                .iter()
                .filter(|g| g.kind == gardens::Kind::Plot)
                .map(|g| geom::centroid(&g.outline))
                .min_by(|a, b| a.distance(d.centre).total_cmp(&b.distance(d.centre)))
        {
            let eye = world(size, plot + DVec2::new(0.0, 25.0), 20.0);
            let flats = gardens
                .iter()
                .filter(|g| g.kind == gardens::Kind::Lawn)
                .count()
                / 2;
            passage_note += &format!(
                "; {flats} garden flats, a court's gardens at --view={:.1},20.0,{:.1},0.0,-38.7",
                eye.x, eye.z
            );
        }
        // And the terminus nearest the kit's centre, from 12 m up over the line 25 m before its
        // crossover, looking along it at the stop.
        if let Some(d) = detail
            && let Some((run, s)) = trams
                .iter()
                .flat_map(|r| {
                    let stops = r.timetable.stops();
                    [(r, stops[0]), (r, stops[stops.len() - 1])]
                })
                .min_by(|a, b| {
                    let far = |(r, s): &(&TramRun, f64)| r.track(*s).0.distance(d.centre);
                    far(a).total_cmp(&far(b))
                })
        {
            let first = s == run.timetable.stops()[0];
            let way = if first { -1.0 } else { 1.0 };
            let (p, _) = run.track(s - way * (tram_length() * 0.5 + CROSSOVER + 25.0));
            let eye = world(size, p, 12.0);
            let to = (world(size, run.track(s).0, 0.0) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                "; a tram terminus at --view={:.1},12,{:.1},{yaw:.1},{pitch:.1}",
                eye.x, eye.z
            );
        }
        // And the tram depots, the nearest the kit's centre from 15 m up over the street 30 m
        // before its yard, looking along it.
        let parked: usize = trams.iter().map(|r| r.parked.len()).sum();
        passage_note += &format!(
            "; {} tram depots keeping {parked} trams",
            plan.tram_depots.len()
        );
        if let Some(d) = detail
            && let Some(depot) = plan.tram_depots.iter().min_by(|a, b| {
                let far = |t: &forge_procgen::city::tram_depot::TramDepot| {
                    geom::centroid(&t.yard()).distance(d.centre)
                };
                far(a).total_cmp(&far(b))
            })
        {
            // From above, from the footway where its link crosses, and before its hall.
            let view = |eye: Vec3, at: Vec3| {
                let to = (at - eye).normalize_or_zero();
                format!(
                    "--view={:.1},{:.1},{:.1},{:.1},{:.1}",
                    eye.x,
                    eye.y,
                    eye.z,
                    (-to.x).atan2(-to.z).to_degrees(),
                    to.y.asin().to_degrees()
                )
            };
            let deep = 12.0;
            passage_note += &format!(
                ", one at {}, its link at {}, its hall at {}",
                view(
                    world(size, depot.at(-30.0, -depot.half), 15.0),
                    world(size, depot.at(depot.length * 0.6, deep), 0.0)
                ),
                view(
                    world(size, depot.at(-40.0, -2.0), 1.7),
                    world(size, depot.at(-15.0, -6.0), 0.3)
                ),
                view(
                    world(size, depot.at(depot.length - 110.0, -depot.half), 14.0),
                    world(size, depot.at(depot.length - 25.0, deep + 6.0), 4.0)
                )
            );
        }
        // And the crossover pairs, the nearest the kit's centre from 12 m up over the line 25 m
        // before it, looking along it.
        let pairs: Vec<(&TramRun, f64)> = trams
            .iter()
            .flat_map(|r| r.crossover_pairs().into_iter().map(move |s| (r, s)))
            .collect();
        passage_note += &format!("; {} tram crossover pairs", pairs.len());
        if let Some(d) = detail
            && let Some(&(run, s)) = pairs.iter().min_by(|a, b| {
                let far = |(r, s): &(&TramRun, f64)| r.track(*s).0.distance(d.centre);
                far(a).total_cmp(&far(b))
            })
        {
            let eye = world(size, run.track(s - 25.0).0, 12.0);
            let to = (world(size, run.track(s + CROSSOVER + 5.0).0, 0.0) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                ", one at --view={:.1},12,{:.1},{yaw:.1},{pitch:.1}",
                eye.x, eye.z
            );
        }
        // And the water mirror nearest the kit's centre, from 6 m off its long side's middle at
        // eye height, looking across it.
        let mirrors: Vec<&Vec<DVec2>> = middles
            .iter()
            .filter_map(|m| m.as_ref().filter(|(_, mirror)| *mirror).map(|(r, _)| r))
            .collect();
        passage_note += &format!("; {} water mirrors", mirrors.len());
        if let Some(d) = detail
            && let Some(r) = mirrors.iter().min_by(|a, b| {
                let far = |r: &Vec<DVec2>| geom::centroid(r).distance(d.centre);
                far(a).total_cmp(&far(b))
            })
        {
            let (side, middle) = ((r[0] + r[1]) * 0.5, geom::centroid(r));
            let eye = world(size, side + (side - middle).normalize_or_zero() * 6.0, 1.7);
            let to = (world(size, middle, 0.0) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                ", one at --view={:.1},1.7,{:.1},{yaw:.1},{pitch:.1}",
                eye.x, eye.z
            );
        }
        // And the gap between buildings nearest the kit's centre, from the street's middle
        // looking in along its path.
        if let Some(d) = detail
            && let Some(g) = gardens
                .iter()
                .filter(|g| g.kind == gardens::Kind::Path && g.outline.len() >= 8)
                .filter(|g| {
                    let n = g.outline.len();
                    // A gap's path starts on the block's edge, the street's middle.
                    !plan.blocks[g.block as usize]
                        .courtyard
                        .as_ref()
                        .is_some_and(|c| geom::contains(c, (g.outline[0] + g.outline[n - 1]) * 0.5))
                        && plan
                            .ground
                            .on_carriageway((g.outline[0] + g.outline[n - 1]) * 0.5)
                })
                .min_by(|a, b| {
                    let far = |g: &gardens::Piece| g.outline[0].distance(d.centre);
                    far(a).total_cmp(&far(b))
                })
        {
            let n = g.outline.len();
            let start = (g.outline[0] + g.outline[n - 1]) * 0.5;
            let next = (g.outline[1] + g.outline[n - 2]) * 0.5;
            let dir = (next - start).normalize_or_zero();
            let eye = world(size, start - dir * 3.0, 1.7);
            let to = world(size, start + dir * 20.0, 0.8) - eye;
            let to = to.normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                "; a gap between buildings at --view={:.1},1.7,{:.1},{yaw:.1},{pitch:.1}",
                eye.x, eye.z
            );
        }
        // And the court benches nearest the kit's centre, from the middle's far side.
        if let Some(d) = detail
            && let Some(g) = gardens
                .iter()
                .filter(|g| g.kind == gardens::Kind::Bench)
                .min_by(|a, b| {
                    let far = |g: &gardens::Piece| geom::centroid(&g.outline).distance(d.centre);
                    far(a).total_cmp(&far(b))
                })
        {
            let front = g.outline[1] - g.outline[0];
            let face = DVec2::new(front.y, -front.x).normalize_or_zero();
            let eye = world(size, geom::centroid(&g.outline) + face * 7.0, 1.7);
            let to = (world(size, geom::centroid(&g.outline), 0.5) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                "; a court's benches at --view={:.1},1.7,{:.1},{yaw:.1},{pitch:.1}",
                eye.x, eye.z
            );
        }
        // And the planters 6 m long or more nearest the kit's centre, from 4 m out on the street
        // and 6 m before them, looking along them.
        if let Some(d) = detail
            && let Some(g) = gardens
                .iter()
                .filter(|g| {
                    g.kind == gardens::Kind::Planter
                        && g.outline[0]
                            .distance(g.outline[1])
                            .max(g.outline[1].distance(g.outline[2]))
                            >= 6.0
                })
                .min_by(|a, b| {
                    let far = |g: &gardens::Piece| geom::centroid(&g.outline).distance(d.centre);
                    far(a).total_cmp(&far(b))
                })
        {
            let c = geom::centroid(&g.outline);
            let (e0, e1) = (g.outline[1] - g.outline[0], g.outline[2] - g.outline[1]);
            let u = if e0.length() > e1.length() { e0 } else { e1 }.normalize_or_zero();
            // Away from its building.
            let built = |p: DVec2| {
                masses.iter().any(|m| {
                    plan.lots[m.lot as usize].block == g.block && geom::contains(&m.footprint, p)
                })
            };
            let v = if built(c + u.perp() * 3.0) {
                -u.perp()
            } else {
                u.perp()
            };
            let eye = world(size, c + v * 4.0 - u * 6.0, 1.7);
            let to = (world(size, c + u * 2.0, 0.6) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                "; a home's planters at --view={:.1},1.7,{:.1},{yaw:.1},{pitch:.1}",
                eye.x, eye.z
            );
        }
        // The kit's office buildings (D-060 step 3), and one seen from its street: 15 m out from
        // the middle of its longest street wall (15 m: from 25 a square's pavilion stood in the
        // way), looking up at its third storey.
        if let Some(d) = detail {
            let offices = near
                .iter()
                .filter(|&&i| masses[i].office_building())
                .count();
            let one = near
                .iter()
                .filter(|&&i| masses[i].office_building())
                .filter_map(|&i| {
                    structures[i]
                        .walls
                        .iter()
                        .filter(|w| w.role == WallRole::Street)
                        .max_by(|a, b| a.length().total_cmp(&b.length()))
                })
                .min_by(|a, b| {
                    let far = |w: &structure::Wall| ((w.a + w.b) * 0.5).distance(d.centre);
                    far(a).total_cmp(&far(b))
                });
            passage_note += &format!("; {offices} office buildings in the kit");
            if let Some(w) = one {
                let t = (w.b - w.a).normalize_or_zero();
                let mid = (w.a + w.b) * 0.5;
                let eye = world(size, mid + DVec2::new(t.y, -t.x) * 15.0, 1.7);
                let to = (world(size, mid, 8.0) - eye).normalize_or_zero();
                let yaw = (-to.x).atan2(-to.z).to_degrees();
                let pitch = to.y.asin().to_degrees();
                passage_note += &format!(
                    ", one at --view={:.1},1.7,{:.1},{yaw:.1},{pitch:.1}",
                    eye.x, eye.z
                );
            }
        }
        let triangles = tiles
            .tiles
            .iter()
            .map(|t| {
                t.ground.triangle_count()
                    + t.masses.triangle_count()
                    + t.railings.triangle_count()
                    + t.insides.triangle_count()
            })
            .sum();
        let st = &plan.stats;
        // No tree over a tram stop or within 6 m of its platform (the owner, 2026-10-11): the
        // platforms in their own frames (x along, z away from the track).
        let stops: Vec<Mat4> = trams
            .iter()
            .flat_map(|r| r.platforms(size))
            .map(|(at, _, _)| at.inverse())
            .collect();
        let mut stop_grid = forge_procgen::city::geom::PointGrid::new(size, 50.0);
        for (k, s) in stops.iter().enumerate() {
            let c = s
                .inverse()
                .transform_point3(Vec3::new(PLATFORM * 0.5, 0.0, 1.15));
            stop_grid.insert(
                DVec2::new(f64::from(c.x) + size * 0.5, f64::from(c.z) + size * 0.5),
                k as u32,
            );
        }
        // Nor on the courts' paths, the terraces, the hedges and the beds: on the lawns only.
        let mut kept = forge_procgen::city::geom::PointGrid::new(size, 10.0);
        for (k, g) in gardens.iter().enumerate() {
            if g.kind != gardens::Kind::Lawn {
                let mut ring = g.outline.clone();
                ring.push(g.outline[0]);
                for p in geom::resample(&ring, 4.0) {
                    kept.insert(p, k as u32);
                }
            }
        }
        // A bench keeps a trunk 1.6 m from its middle, clear of its ends and back.
        let on_garden = |p: DVec2| {
            let mut on = false;
            kept.for_each_within(p, 10.0, |_, k| {
                let g = &gardens[k as usize];
                on = on
                    || geom::contains(&g.outline, p)
                    || (g.kind == gardens::Kind::Bench
                        && geom::centroid(&g.outline).distance(p) < 1.6);
            });
            on
        };
        // Each turned and given its model by its place: by its index in a list, dropping one
        // (here, or in the plan's) turned every tree after it.
        let trees: Vec<(usize, &Tree)> = trees
            .iter()
            .map(|t| {
                let key = t.pos.x.to_bits() ^ t.pos.y.to_bits().rotate_left(29);
                (mix(key) as usize, t)
            })
            .filter(|(_, t)| {
                let w = world(size, t.pos, 0.0);
                !stop_grid.any_within(t.pos, 30.0, |k| {
                    let l = stops[k as usize].transform_point3(w);
                    !((-6.0..PLATFORM + 6.0).contains(&l.x) && (-6.0..2.3 + 6.0).contains(&l.z))
                }) && !on_garden(t.pos)
            })
            .collect();
        let tree_count = trees.len();
        // The trunks, which the benches keep clear of.
        let mut trunks = forge_procgen::city::geom::PointGrid::new(size, 10.0);
        for (_, t) in &trees {
            trunks.insert(t.pos, 0);
        }
        laps.mark("views, trams and trees");
        // The forest's plants by species, each one of its species' grown individuals: its hash
        // picks which of those at its height's age.
        let mut flora: Vec<(Vec3, f32, f32, f32, u16, u8)> = trees
            .iter()
            .filter_map(|&(i, t)| {
                let species = t.species?;
                let turn = (i >> 8 & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
                Some((
                    world(size, t.pos, 0.0),
                    t.height as f32,
                    t.crown as f32,
                    turn,
                    species,
                    (i >> 24 & 0xFF) as u8,
                ))
            })
            .collect();
        let mut trees: Vec<(Vec3, Vec3, f32, usize)> = trees
            .iter()
            .filter(|(_, t)| t.species.is_none())
            .map(|&(i, t)| {
                let turn = (i >> 8 & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
                (
                    world(size, t.pos, 0.0),
                    Vec3::new(
                        (t.crown / 4.2) as f32,
                        (t.height / 10.0) as f32,
                        (t.crown / 4.2) as f32,
                    ),
                    turn,
                    i % 2,
                )
            })
            .collect();
        // Shrubs in the planters and vegetables in the kitchen gardens' beds within
        // [`PLANTS_REACH`] of the kit's centre, as small trees sunk in their boxes to the middle
        // of their crowns (7 of a tree's height's tenths up), the trunks hidden: the boxes' flat
        // green tops read as carpet, and standing on them the plants showed their trunks like
        // bonsais. Further off, a pixel or two each, they speckled the facades' feet from the
        // air. The planters grow rosemary where the forest's species are drawn (D-063): the one
        // model's smooth balls stood out beside the leaf cards.
        let rosemary = (!flora.is_empty())
            .then(|| {
                forge_flora::species::mediterranean()
                    .iter()
                    .position(|s| s.id == "rosemary")
                    .map(|i| i as u16)
            })
            .flatten();
        let mut plants = 0;
        for g in &gardens {
            let (spacing, crown, high, rows): (f64, f32, f32, &[f64]) = match g.kind {
                gardens::Kind::Planter => (1.0, 0.12, 0.08, &[0.0]),
                gardens::Kind::Bed => (0.8, 0.07, 0.04, &[-0.25, 0.25]),
                _ => continue,
            };
            let c = geom::centroid(&g.outline);
            // Each box's plants by its place: by its index in the list, a new garden piece
            // before it (the benches, the racks) changed every plant after it.
            let k = c.x.to_bits() ^ c.y.to_bits().rotate_left(29);
            if detail.is_none_or(|d| c.distance(d.centre) > PLANTS_REACH) {
                continue;
            }
            let (e0, e1) = (g.outline[1] - g.outline[0], g.outline[2] - g.outline[1]);
            let (long, short) = if e0.length() >= e1.length() {
                (e0, e1)
            } else {
                (e1, e0)
            };
            // Rosemary closer, so the box reads full (a metre apart they stood as two clumps).
            let spacing = if g.kind == gardens::Kind::Planter && rosemary.is_some() {
                0.7
            } else {
                spacing
            };
            let n = (long.length() / spacing).floor().max(1.0) as usize;
            for (r, &row) in rows.iter().enumerate() {
                for j in 0..n {
                    let h = mix(k ^ (r as u64) << 16 ^ (j as u64) << 40);
                    let vary = 0.85 + 0.3 * (h & 0xFF) as f32 / 255.0;
                    let p = c + long * ((j as f64 + 0.5) / n as f64 - 0.5) + short * row;
                    let turn = ((h >> 8) & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
                    if let (gardens::Kind::Planter, Some(sp)) = (g.kind, rosemary) {
                        // A rosemary 0.9 m over the box's soil, 0.8 m across, its foot 10 cm in.
                        flora.push((
                            world(size, p, WALK_LEVEL + g.height - 0.1),
                            1.0 * vary,
                            0.4 * vary,
                            turn,
                            sp,
                            ((h >> 24) & 0xFF) as u8,
                        ));
                        plants += 1;
                        continue;
                    }
                    let base = WALK_LEVEL + g.height - f64::from(6.6 * high * vary);
                    trees.push((
                        world(size, p, base),
                        Vec3::new(crown * vary, high * vary, crown * vary),
                        turn,
                        ((h >> 24) & 1) as usize,
                    ));
                    plants += 1;
                }
            }
        }
        passage_note += &format!("; {plants} plants in the planters and beds");
        // The lamps within the detailed reach and a little beyond: further off, a lamp head of
        // 3 000 cd/m² is far smaller than a pixel, and thousands of them flickered into a band
        // of light along the horizon at night (the owner, 2026-10-11). The far streets' light
        // comes with step 5's clustered lights (#254).
        let lamps = lamps
            .iter()
            .filter(|l| detail.is_none_or(|d| l.pos.distance(d.centre) < d.radius + 500.0))
            .map(|l| {
                let turn = (-l.facing.x).atan2(-l.facing.y) as f32;
                (
                    world(size, l.pos, 0.0),
                    (l.height / 8.0) as f32,
                    turn,
                    l.double,
                )
            })
            .collect();
        let mut seats = Vec::new();
        let far_lit = kit.as_ref().and_then(|k| k.homes_share);
        let (mut kit_meshes, mut kit_instances, kit_report) =
            kit.map_or((Vec::new(), Vec::new(), None), |k| {
                let instanced: usize = k
                    .instances
                    .iter()
                    .map(|i| k.meshes[i.0 as usize].triangle_count())
                    .sum();
                let report = format!(
                    "the kit's buildings: {} buildings, {} faces, {} terminals, {} instances of \
                     {} unique modules ({} k instanced triangles), longest run of one opening \
                     {}, derived and assembled in {:.0} ms; their insides {} k triangles",
                    near.len(),
                    k.faces,
                    k.terminals,
                    k.instances.len(),
                    k.meshes.len(),
                    instanced / 1000,
                    k.longest_run,
                    k.ms,
                    inside_triangles / 1000
                );
                let report = match k.lights {
                    Some(lights) => format!("{report}; {lights}"),
                    None => report,
                };
                seats = k.seats;
                (k.meshes, k.instances, Some(report))
            });
        // The landmarks with a model on their squares, inside the squares' tree rows, along their
        // longest side: the terraced hall and the crane facing the nearest water with their low
        // end or their jib.
        let mut hall_squares = Vec::new();
        for l in &plan.landmarks {
            use forge_procgen::city::LandmarkKind;
            // A weir bridge across the river nearest its anchor, square to the flow, landing on
            // both banks.
            if l.kind == LandmarkKind::WeirBridge {
                if let Some((at, along, width)) = river_crossing(site, l.pos) {
                    kit_meshes.push(forge_geom::landmarks::weir_bridge((width + 18.0) as f32));
                    let id = (kit_meshes.len() - 1) as u32;
                    let across = DVec2::new(-along.y, along.x);
                    let turn = glam::Quat::from_rotation_y((-across.y).atan2(across.x) as f32);
                    let at = Mat4::from_rotation_translation(turn, world(size, at, WATER_LEVEL));
                    kit_instances.push((id, at, HALL_GROUP));
                }
                continue;
            }
            let Some(i) = l.square else {
                continue;
            };
            let b = &plan.blocks[i];
            let (axis, extent, centre) = geom::oriented_extent(&b.buildable);
            // On a quay (it holds water): its land point, a fixed size, the water any way.
            let on_quay = b.use_ == BlockUse::Quay;
            let (centre, extent) = if on_quay {
                (b.centre, DVec2::new(74.0, 44.0))
            } else {
                (centre, extent)
            };
            let to_water = |dir: DVec2| {
                (1..40)
                    .map(|k| f64::from(k) * 10.0)
                    .find(|&s| site.water_at(centre + dir * s) != forge_procgen::city::LAND)
                    .unwrap_or(f64::MAX)
            };
            let normal = DVec2::new(-axis.y, axis.x);
            let ways: &[DVec2] = if on_quay {
                &[axis, -axis, normal, -normal]
            } else {
                &[axis, -axis]
            };
            let seaward = ways
                .iter()
                .copied()
                .min_by(|a, b| to_water(*a).total_cmp(&to_water(*b)))
                .unwrap_or(axis);
            let (mesh, group, dir) = match l.kind {
                LandmarkKind::MarketHall => {
                    let length = (extent.x - 30.0).clamp(18.0, 48.0) as f32;
                    let width = (extent.y - 30.0).clamp(12.0, 26.0) as f32;
                    (
                        forge_geom::landmarks::market_hall(length, width),
                        HALL_GROUP,
                        axis,
                    )
                }
                LandmarkKind::TerracedHall => {
                    let length = (extent.x - 26.0).clamp(30.0, 80.0) as f32;
                    let width = (extent.y - 26.0).clamp(14.0, 30.0) as f32;
                    (
                        forge_geom::landmarks::terraced_hall(length, width),
                        HALL_GROUP,
                        seaward,
                    )
                }
                LandmarkKind::GantryCrane => (
                    forge_geom::landmarks::gantry_crane(28.0, 30.0),
                    CRANE_GROUP,
                    seaward,
                ),
                LandmarkKind::Pavilion => (
                    forge_geom::landmarks::pavilion((b.area.sqrt() / 14.0).clamp(5.0, 9.0) as f32),
                    HALL_GROUP,
                    axis,
                ),
                LandmarkKind::WeirBridge | LandmarkKind::Other => continue,
            };
            kit_meshes.push(mesh);
            let id = (kit_meshes.len() - 1) as u32;
            let turn = glam::Quat::from_rotation_y((-dir.y).atan2(dir.x) as f32);
            // On a square, on its level; a quay's crane on the ground.
            let base = if b.use_ == BlockUse::Square {
                SQUARE_LEVEL + 0.08
            } else {
                0.16
            };
            let at = Mat4::from_rotation_translation(turn, world(size, centre, base));
            kit_instances.push((id, at, group));
            hall_squares.push(i);
        }
        // Benches round each square of 0.15 ha or more, every 15 m inside its outer tree row,
        // facing the middle; on a landmark's square they face it. None where a trunk stands
        // (the owner saw a tree through a bench).
        let benches: Vec<Mat4> = plan
            .blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Square && b.area >= 1500.0)
            .filter_map(|b| geom::inset(&b.buildable, 6.0).map(|ring| (b, ring)))
            .flat_map(|(b, ring)| {
                let middle = geom::centroid(&b.buildable);
                let mut closed = ring.clone();
                closed.push(ring[0]);
                let trunks = &trunks;
                geom::resample(&closed, 15.0)
                    .into_iter()
                    .filter(move |&p| !trunks.any_within(p, 2.2, |_| false))
                    .map(move |p| {
                        let face = (middle - p).normalize_or_zero();
                        // The bench faces +z: turned so +z points at the middle.
                        let turn = (face.x as f32).atan2(face.y as f32);
                        Mat4::from_rotation_translation(
                            glam::Quat::from_rotation_y(turn),
                            world(size, p, SQUARE_LEVEL + 0.08),
                        )
                    })
            })
            // And round the green courts' middles (`gardens::Kind::Bench`), each facing the
            // paving its front edge looks onto.
            .chain(
                gardens
                    .iter()
                    .filter(|g| g.kind == gardens::Kind::Bench)
                    .map(|g| {
                        let front = g.outline[1] - g.outline[0];
                        let face = DVec2::new(front.y, -front.x).normalize_or_zero();
                        let turn = (face.x as f32).atan2(face.y as f32);
                        Mat4::from_rotation_translation(
                            glam::Quat::from_rotation_y(turn),
                            world(size, geom::centroid(&g.outline), WALK_LEVEL),
                        )
                    }),
            )
            .collect();
        let bench_spots: Vec<Vec3> = benches.iter().map(|m| m.w_axis.truncate()).collect();
        if !benches.is_empty() {
            kit_meshes.push(forge_geom::furniture::bench());
            let id = (kit_meshes.len() - 1) as u32;
            kit_instances.extend(benches.into_iter().map(|at| (id, at, 0)));
        }
        // The gaps' bike racks (`gardens::Kind::BikeRack`), along their outlines' first edges.
        let racks: Vec<Mat4> = gardens
            .iter()
            .filter(|g| g.kind == gardens::Kind::BikeRack)
            .map(|g| {
                let along = (g.outline[1] - g.outline[0]).normalize_or_zero();
                // The row's x along the edge: turned about y so +x points along it.
                let turn = (-along.y as f32).atan2(along.x as f32);
                Mat4::from_rotation_translation(
                    glam::Quat::from_rotation_y(turn),
                    world(size, geom::centroid(&g.outline), WALK_LEVEL),
                )
            })
            .collect();
        if !racks.is_empty() {
            kit_meshes.push(forge_geom::furniture::bike_rack());
            let id = (kit_meshes.len() - 1) as u32;
            kit_instances.extend(racks.into_iter().map(|at| (id, at, 0)));
        }
        // The furniture of the houses whose insides are drawn (`city::furnish`), near the view:
        // a mesh a piece, each placed on its storey's floor turned to face its way.
        let mut homes: std::collections::BTreeMap<String, Vec<Mat4>> = Default::default();
        for &i in &near {
            let (m, s) = (&masses[i], &structures[i]);
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            for p in forge_procgen::city::walk::furnished(m, s, &inner) {
                let turn = (p.facing.x as f32).atan2(p.facing.y as f32);
                homes
                    .entry(home_name(p.piece).to_string())
                    .or_default()
                    .push(Mat4::from_rotation_translation(
                        glam::Quat::from_rotation_y(turn),
                        world(size, p.centre, floor_level(s, p.storey)),
                    ));
            }
        }
        for (name, at) in homes {
            let piece = furnish::Piece::ALL
                .into_iter()
                .find(|&p| home_name(p) == name)
                .unwrap_or(furnish::Piece::Shelves);
            let (w, d) = piece.size();
            if let Some(mesh) = forge_geom::furniture::home(&name, w as f32, d as f32) {
                kit_meshes.push(mesh);
                let id = (kit_meshes.len() - 1) as u32;
                kit_instances.extend(at.into_iter().map(|at| (id, at, 0)));
            }
        }
        // Chess tables round the fountain of each square of 0.15 ha or more without a landmark,
        // four 9 m from its middle on the diagonals, where they stand clear of the trunks and the
        // benches and inside the building lines; their players sit as the terraces' guests do,
        // in the share of the people out round them (the owner: give people something to do in
        // the squares rather than stand there).
        let held: Vec<usize> = plan.landmarks.iter().filter_map(|l| l.square).collect();
        // Nothing stands in a water mirror, nor within a metre and a half of it.
        let in_mirror = |i: usize, p: DVec2| {
            middles[i].as_ref().is_some_and(|(m, mirror)| {
                let n = m.len();
                *mirror
                    && (geom::contains(m, p)
                        || (0..n).any(|k| geom::distance_to_segment(p, m[k], m[(k + 1) % n]) < 1.5))
            })
        };
        // A market on one square in two of 0.25 ha or more without a landmark or a water mirror
        // (`research/ground-floors.md` §4.9: open-air markets on the boulevards and squares):
        // two rows of stalls facing each other across a 3 m aisle along the square's long axis,
        // 3.2 m apart, inside its benches, clear of the trunks, the fountain kept in a ring of
        // 7.5 m. Its canvases in the kit's four awning colours. They take the chess tables' place.
        let market_squares: Vec<usize> = plan
            .blocks
            .iter()
            .enumerate()
            .filter(|&(i, b)| {
                b.use_ == BlockUse::Square
                    && b.area >= 2500.0
                    && !held.contains(&i)
                    && !middles[i].as_ref().is_some_and(|(_, m)| *m)
                    && mix(i as u64 ^ 0x3A2C_5E1D).is_multiple_of(2)
            })
            .map(|(i, _)| i)
            .collect();
        // A morning market: packed away outside 6:30 to 14:00 when the run holds an hour, the
        // chess tables back on its square.
        let market_open = detail
            .and_then(|d| d.lit_hour)
            .is_none_or(|h| (6.5..14.0).contains(&h));
        let market_squares: Vec<usize> =
            market_squares.into_iter().filter(|_| market_open).collect();
        let mut stalls: Vec<(Mat4, u8)> = Vec::new();
        let mut stall_walls: Vec<Vec<Vec3>> = Vec::new();
        for &i in &market_squares {
            let b = &plan.blocks[i];
            let Some(inner) = geom::inset(&b.buildable, 7.5) else {
                continue;
            };
            let middle = geom::centroid(&b.buildable);
            let (u, _, _) = geom::oriented_extent(&b.buildable);
            let v = DVec2::new(-u.y, u.x);
            // Aisles down the middle and 11 m either side where the square is wide enough.
            for (aisle, side) in [0.0, -11.0, 11.0]
                .into_iter()
                .flat_map(|a| [(a, -1.0), (a, 1.0)])
            {
                for k in -15..=15 {
                    let at = middle + u * (f64::from(k) * 3.2) + v * (aisle + side * 2.5);
                    let corners = [(-1.5, -1.0), (1.5, -1.0), (1.5, 1.0), (-1.5, 1.0)]
                        .map(|(x, y)| at + u * x + v * (y * side));
                    let w = world(size, at, 0.0);
                    // A trunk keeps out of the stall: its half diagonal (1.8 m) and the trunk.
                    let fits = at.distance(b.centre) > 7.5
                        && corners.iter().all(|&c| geom::contains(&inner, c))
                        && !trunks.any_within(at, 2.2, |_| false)
                        && bench_spots.iter().all(|s| s.with_y(0.0).distance(w) > 3.0);
                    if !fits {
                        continue;
                    }
                    // Facing the aisle: +z turned onto it.
                    let face = -v * side;
                    let turn = glam::Quat::from_rotation_y((face.x as f32).atan2(face.y as f32));
                    let colour =
                        (mix(i as u64 ^ (k as u64) << 8 ^ side.to_bits() ^ aisle.to_bits()) % 4)
                            as u8;
                    stalls.push((
                        Mat4::from_rotation_translation(turn, world(size, at, SQUARE_LEVEL)),
                        colour * 2,
                    ));
                    stall_walls.push(
                        corners
                            .iter()
                            .flat_map(|&c| {
                                [
                                    world(size, c, SQUARE_LEVEL),
                                    world(size, c, SQUARE_LEVEL + 2.5),
                                ]
                            })
                            .collect(),
                    );
                }
            }
        }
        // The market nearest the kit's centre, from 8 m down its aisle.
        if let Some(d) = detail {
            let centre = world(size, d.centre, 0.0);
            if let Some((at, _)) = stalls.iter().min_by(|a, b| {
                let far = |m: &Mat4| m.w_axis.truncate().distance(centre);
                far(&a.0).total_cmp(&far(&b.0))
            }) {
                let (pos, face, side) = (
                    at.w_axis.truncate(),
                    at.z_axis.truncate(),
                    at.x_axis.truncate(),
                );
                let eye = pos + face * 1.5 + side * 8.0;
                let to = (pos + face * 1.0 + Vec3::Y * 1.0 - (eye + Vec3::Y * 1.7)).normalize();
                let yaw = (-to.x).atan2(-to.z).to_degrees();
                let pitch = to.y.asin().to_degrees();
                passage_note += &format!(
                    "; {} market stalls on {} squares, one at --view={:.1},{:.1},{:.1},{yaw:.1},{pitch:.1}",
                    stalls.len(),
                    market_squares.len(),
                    eye.x,
                    eye.y + 1.7,
                    eye.z
                );
            }
        }
        if !stalls.is_empty() {
            kit_meshes.push(forge_geom::furniture::market_stall());
            let id = (kit_meshes.len() - 1) as u32;
            kit_instances.extend(stalls.iter().map(|&(at, colour)| (id, at, colour)));
        }
        let mut chess = Vec::new();
        let mut games = Vec::new();
        for (i, b) in plan.blocks.iter().enumerate().filter(|(i, b)| {
            b.use_ == BlockUse::Square
                && b.area >= 1500.0
                && !held.contains(i)
                && !market_squares.contains(i)
        }) {
            let middle = geom::centroid(&b.buildable);
            let (axis, _, _) = geom::oriented_extent(&b.buildable);
            let spots = (0..4)
                .map(|k| {
                    let a = std::f64::consts::FRAC_PI_4 + k as f64 * std::f64::consts::FRAC_PI_2;
                    let (s, c) = a.sin_cos();
                    middle + (axis * c + DVec2::new(-axis.y, axis.x) * s) * 9.0
                })
                .filter(|&p| {
                    let w = world(size, p, 0.0);
                    geom::contains(&b.buildable, p)
                        && !trunks.any_within(p, 2.0, |_| false)
                        && bench_spots.iter().all(|s| s.with_y(0.0).distance(w) > 3.0)
                        && !in_mirror(i, p)
                });
            for p in spots {
                let face = (middle - p).normalize_or_zero();
                let turn = glam::Quat::from_rotation_y((face.x as f32).atan2(face.y as f32));
                chess.push(Mat4::from_rotation_translation(
                    turn,
                    world(size, p, SQUARE_LEVEL),
                ));
            }
        }
        if !chess.is_empty() {
            kit_meshes.push(forge_geom::furniture::chess_table());
            let id = (kit_meshes.len() - 1) as u32;
            for at in chess {
                kit_instances.push((id, at, 0));
                games.push((at.w_axis.truncate(), [seats.len(), seats.len() + 1]));
                for (p, d) in forge_geom::furniture::chess_seats() {
                    let (p, d) = (at.transform_point3(p), at.transform_vector3(d));
                    seats.push((p, (-d.z).atan2(d.x)));
                }
            }
        }
        // A fountain in the middle of each other square of 0.15 ha or more, but for those with a
        // water mirror there.
        let fountains: Vec<Mat4> = plan
            .blocks
            .iter()
            .enumerate()
            .filter(|&(i, _)| {
                !hall_squares.contains(&i) && !middles[i].as_ref().is_some_and(|(_, m)| *m)
            })
            .map(|(_, b)| b)
            .filter(|b| b.use_ == BlockUse::Square && b.area >= 1500.0)
            .filter(|b| geom::contains(&b.polygon, b.centre))
            // Grander on grander squares: 1.0 at 0.15 ha to 1.8 at 1 ha.
            .map(|b| {
                let scale = (b.area.sqrt() / 55.0).clamp(1.0, 1.8) as f32;
                Mat4::from_scale_rotation_translation(
                    Vec3::splat(scale),
                    glam::Quat::IDENTITY,
                    world(size, b.centre, SQUARE_LEVEL + 0.08),
                )
            })
            .collect();
        if !fountains.is_empty() {
            kit_meshes.push(forge_geom::furniture::fountain());
            let id = (kit_meshes.len() - 1) as u32;
            kit_instances.extend(fountains.into_iter().map(|at| (id, at, 0)));
        }
        // The taxi stands' totems behind their lay-bys, their panels lit and facing along the road.
        if !plan.taxi_stands.is_empty() {
            kit_meshes.push(forge_geom::furniture::taxi_totem());
            let id = (kit_meshes.len() - 1) as u32;
            for s in &plan.taxi_stands {
                let p = s.totem();
                let turn = glam::Quat::from_rotation_y((-s.along.y).atan2(s.along.x) as f32);
                let at =
                    Mat4::from_rotation_translation(turn, world(size, p, plan.ground.level_at(p)));
                kit_instances.push((id, at, 1));
            }
        }
        // The taxi depots' decks, their fronts on the building lines facing their roads (D-061).
        if !plan.depots.is_empty() {
            kit_meshes.push(forge_geom::depot::taxi_depot());
            let id = (kit_meshes.len() - 1) as u32;
            for d in &plan.depots {
                // The mesh's +z, out of its front, towards the road.
                let x = d.across();
                let turn = glam::Quat::from_rotation_y((-x.y).atan2(x.x) as f32);
                let at =
                    Mat4::from_rotation_translation(turn, world(size, d.entrance(), WALK_LEVEL));
                kit_instances.push((id, at, 0));
            }
            // A view of the first from over its road, 10 m up.
            let d = &plan.depots[0];
            let eye = world(size, d.kerb - d.out * 3.0 + d.along * 40.0, 10.0);
            let to = (world(size, d.entrance() + d.out * 16.0, 4.0) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            let pitch = to.y.asin().to_degrees();
            passage_note += &format!(
                "; {} taxi depots, one at --view={:.1},10,{:.1},{yaw:.1},{pitch:.1}",
                plan.depots.len(),
                eye.x,
                eye.z
            );
            // And its front from the pavement across the road: the ways in and out, the staff's door.
            let eye = world(size, d.kerb - d.out * 12.0, 1.7);
            let to = (world(size, d.entrance(), 1.0) - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            passage_note += &format!(
                ", its front at --view={:.1},1.7,{:.1},{yaw:.1},4",
                eye.x, eye.z
            );
        }
        // The bollards across the pedestrian ways' mouths onto the roads (D-061).
        if !plan.ground.bollards.is_empty() {
            kit_meshes.push(forge_geom::furniture::bollard());
            let id = (kit_meshes.len() - 1) as u32;
            for &p in &plan.ground.bollards {
                let at = Mat4::from_translation(world(size, p, plan.ground.level_at(p)));
                kit_instances.push((id, at, 0));
            }
        }
        // A street-workout park in each park of a hectare or more, along the park's longest
        // edge; the trees keep clear of it.
        let workouts = forge_procgen::city::trees::workout_sites(plan);
        if !workouts.is_empty() {
            kit_meshes.push(forge_geom::furniture::street_workout());
            let id = (kit_meshes.len() - 1) as u32;
            for (p, axis) in workouts {
                let turn = glam::Quat::from_rotation_y((-axis.y).atan2(axis.x) as f32);
                let at = Mat4::from_rotation_translation(turn, world(size, p, WALK_LEVEL + 0.01));
                kit_instances.push((id, at, 0));
            }
        }
        // Fingerposts at the avenues' junctions where the kit stands, on the pavement's level.
        let (signs, sign_names) = match detail {
            Some(d) => signs::place(plan, d.centre, d.radius, |p| {
                !trunks.any_within(p, 1.5, |_| false)
            }),
            None => (Vec::new(), Vec::new()),
        };
        let kit_report = kit_report.map(|r| match signs.first() {
            Some(s) => {
                let w = world(size, s.pos, 0.0);
                format!(
                    "{r}; {} fingerposts, one at --view={:.1},1.8,{:.1}",
                    signs.len(),
                    w.x,
                    w.z
                )
            }
            None => r,
        });
        for sign in &signs {
            kit_meshes.push(signs::mesh(sign));
            let id = (kit_meshes.len() - 1) as u32;
            kit_instances.push((
                id,
                Mat4::from_translation(world(size, sign.pos, 0.30)),
                SIGN_GROUP,
            ));
        }
        // The tram stops, a platform by each track.
        if !trams.is_empty() {
            kit_meshes.push(forge_geom::furniture::tram_stop(
                PLATFORM,
                boarding_deep() as f32,
                true,
            ));
            let stop = (kit_meshes.len() - 1) as u32;
            // Each platform once: lines sharing a stop share its platforms (drawn per line, they
            // stood one over another).
            let mut drawn: Vec<Mat4> = Vec::new();
            for run in &trams {
                for (at, _, _) in run.platforms(size) {
                    let same = drawn.iter().any(|o| {
                        o.w_axis.truncate().distance(at.w_axis.truncate()) < 3.0
                            && o.x_axis.truncate().dot(at.x_axis.truncate()) > 0.95
                    });
                    if same {
                        continue;
                    }
                    drawn.push(at);
                    // The kit's first colour, its glass lit: the shelter and the totem glow.
                    kit_instances.push((stop, at, 1));
                }
            }
        }
        laps.mark("the rest");
        eprintln!(
            "the plan laid out in its tiles, its steps (ms): {}",
            laps.report()
        );
        Self {
            tiles: tiles.tiles,
            trees,
            flora,
            lamps,
            kit_meshes,
            kit_instances,
            kit_report,
            far_lit,
            seats,
            trams,
            boats: plan
                .crossings
                .iter()
                .map(|c| BoatRun::new(c, site))
                .collect(),
            clock: 3600.0,
            walls: wall_pieces(size, masses, &structures)
                .into_iter()
                .chain(interior_pieces(size, masses, &structures, &near))
                .chain(parapet_pieces(size, plan, site, &shore))
                .chain(garden_pieces(size, &gardens))
                .chain(depot_walls)
                .chain(garden_walls)
                .chain(stall_walls)
                .collect(),
            passage_note,
            holds: holds.clone(),
            walk_on_start: false,
            ride_after: None,
            games,
            stalls: stalls.iter().map(|&(at, _)| at).collect(),
            sign_names,
            crowd: None,
            taxis: taxis::Taxis::new(plan, masses, &holds, 8.0, 3600.0),
            size,
            camera,
            title: format!(
                "{title}: {} masses ({} from the kit), {} trees, {:.0} km of streets, {} tram stops",
                masses.len(),
                near.len(),
                tree_count,
                st.main_km + st.street_km,
                st.stops
            ),
            triangles,
            scene: format!("plan{}-{title}", size as u32)
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .collect(),
            cooked: None,
        }
    }

    /// The plan's people on their clock from `hour` (game hours), those walking round the
    /// camera drawn as figures (`crowd`).
    pub fn with_people(self, plan: &CityPlan, masses: &[Mass], hour: f64) -> Self {
        self.with_people_mix(plan, masses, hour, &forge_sim::population::Mix::default())
    }

    /// [`Self::with_people`] with the population's `mix` of roles: `Mix::adults()` for a
    /// world with no children in it.
    pub fn with_people_mix(
        mut self,
        plan: &CityPlan,
        masses: &[Mass],
        hour: f64,
        mix: &forge_sim::population::Mix,
    ) -> Self {
        let seats = std::mem::take(&mut self.seats);
        // Each platform with its line, stop and way, and the lines' timetables: who waits there
        // boards the trams that stand at it.
        let platforms: Vec<crowd::Platform> = self
            .trams
            .iter()
            .enumerate()
            .flat_map(|(line, r)| {
                r.platforms(self.size)
                    .into_iter()
                    .map(move |(at, stop, outbound)| crowd::Platform {
                        at,
                        line,
                        stop,
                        outbound,
                    })
            })
            .collect();
        let timetables = self.trams.iter().map(|r| r.timetable.clone()).collect();
        let games = std::mem::take(&mut self.games);
        self.crowd = Some(
            crowd::Crowd::new(
                plan,
                masses,
                &self.holds,
                hour,
                seats,
                platforms,
                timetables,
                mix,
            )
            .with_games(games)
            .with_stalls(std::mem::take(&mut self.stalls))
            .with_trams_paused(self.trams.iter().any(|r| r.paused)),
        );
        // The taxis on the people's hour.
        self.taxis = taxis::Taxis::new(plan, masses, &self.holds, hour, self.clock);
        self
    }

    /// Walking from the start, in first person where the camera stands (Enter frees it).
    pub fn walking(mut self) -> Self {
        self.walk_on_start = true;
        self
    }

    /// Boarding the tram nearest the camera `after` seconds into the run, as C does.
    pub fn riding(mut self, after: f64) -> Self {
        self.ride_after = Some(after);
        self
    }

    /// The trams' service clock at the start, seconds (an hour in by default, every vehicle
    /// out).
    pub fn with_clock(mut self, seconds: f64) -> Self {
        self.clock = seconds;
        self
    }

    /// A clock, within the service's first two hours, at which a tram's front comes within
    /// 40 m of `p` on the plan, `lead` seconds before it is nearest: for a capture that shows one.
    pub fn tram_near(&self, p: DVec2, lead: f64) -> Option<f64> {
        let mut best: Option<(f64, f64)> = None;
        for run in &self.trams {
            for k in 0..run.timetable.vehicles() {
                let mut t = 3600.0;
                while t < 3600.0 + 7200.0 {
                    let s = run.timetable.at(k, t).s;
                    let d = run.track(s).0.distance(p);
                    if d < 40.0 && best.is_none_or(|(bd, _)| d < bd) {
                        best = Some((d, t));
                    }
                    t += 2.0;
                }
            }
        }
        best.map(|(_, t)| t - lead)
    }
}

/// The crowd's palettes, a material group each.
const PEOPLE_GROUPS: [&str; 8] = [
    "plan-people-0",
    "plan-people-1",
    "plan-people-2",
    "plan-people-3",
    "plan-people-4",
    "plan-people-5",
    "plan-people-6",
    "plan-people-7",
];
/// The landmarks' material groups, after the kit's eight: the halls', the crane's.
const HALL_GROUP: u8 = 8;
const CRANE_GROUP: u8 = 9;
/// The fingerposts' material group.
const SIGN_GROUP: u8 = 10;
/// The kit's key for a terrace's mesh (no grammar symbol is this high).
const TERRACE: u16 = u16::MAX;
/// How far each of a line's two tracks lies from its centreline, metres.
const TRACK_OFFSET: f64 = forge_procgen::city::transit::TRACK_OFFSET;
/// The street ground's two levels (D-061): the carriageways' and the one people walk on.
pub(crate) use forge_procgen::city::ground::{ROAD_LEVEL, WALK_LEVEL};
/// The squares' level: the walking level, as everywhere people walk.
const SQUARE_LEVEL: f64 = WALK_LEVEL;
/// A built block's paved forecourt along its streets, metres deep (the gaps' hedges stand
/// behind it).
const FORECOURT: f64 = gardens::FORECOURT;
/// A passage's paving through a building: a centimetre over the block's ground under it.
const PASSAGE_FLOOR: f64 = WALK_LEVEL + 0.01;

/// A tram stop's platform, metres: a tram's length and a little.
pub(crate) const PLATFORM: f32 = 36.0;

/// A tram stop's boarding area's depth from the track's side, metres: across the carriageway to
/// a tram avenue's kerb and half a metre into its pavement, at the platform's height, ramped
/// 1 in 12 at both ends and along its back down to the pavement (1.7 m over it), short of the
/// walkers' line 3 m from the buildings. Someone on the pavement rolls up onto it anywhere; the
/// bikes' strip by the kerb runs over it behind the shelter (the owner: a stop was not
/// accessible, and bikes rode through it).
pub(crate) fn boarding_deep() -> f64 {
    let p = forge_procgen::city::Profile::TramAvenue;
    p.width() * 0.5 - p.pavement() - (TRACK_OFFSET + 1.35) + 0.5
}

/// A tram stop's platform's height over the carriageway's base (0.16), metres.
pub(crate) const PLATFORM_HIGH: f64 = 0.28;
/// A stop's screen on its platform: on the totem's side towards the track, at eye height
/// (`furniture::tram_stop`'s totem stands 5 m short of the platform's middle, 1.6 m in), over the
/// top of its lit panel.
const SCREEN_ON_PLATFORM: Vec3 = Vec3::new(PLATFORM * 0.5 - 5.0, 0.28 + 1.98, 1.575);
/// Screens of each plate drawn at most, and how far from the camera, metres.
const SCREENS: usize = 12;
const SCREEN_REACH: f32 = 150.0;

/// A stop's screen: where it hangs (facing −z), and the stop it shows the next tram of.
#[derive(Clone, Copy, Debug)]
struct StopScreen {
    at: Mat4,
    line: usize,
    stop: usize,
    outbound: bool,
}
/// The trams' sections: a cab, three passenger sections, a cab.
const TRAM_SECTIONS: usize = 5;
/// How far from the kit's centre the planters and the beds carry plants, metres.
const PLANTS_REACH: f64 = 400.0;
/// A crossover's length between a line's two tracks, metres: their 3.2 m over 30 m, about a
/// tram turnout's 1 in 9.
const CROSSOVER: f64 = 30.0;
/// The room between a terminus crossover and the tram standing at its stop, metres.
const CROSSOVER_CLEAR: f64 = 4.0;
/// A line's crossover pairs' spacing, metres (`research/transit-and-vehicles.md` §10).
const CROSSOVER_EVERY: f64 = 3000.0;

/// A tram's length, metres: its sections and the gaps between them.
fn tram_length() -> f64 {
    use forge_geom::vehicles::{SECTION_GAP, SECTION_LENGTH};
    f64::from(SECTION_LENGTH) * TRAM_SECTIONS as f64
        + f64::from(SECTION_GAP) * (TRAM_SECTIONS - 1) as f64
}
/// The trams' liveries: one group a line, the stripe in its colour.
const TRAM_GROUPS: [&str; 8] = [
    "plan-tram-0",
    "plan-tram-1",
    "plan-tram-2",
    "plan-tram-3",
    "plan-tram-4",
    "plan-tram-5",
    "plan-tram-6",
    "plan-tram-7",
];

/// A tram line's runs: its track and its timetable (`forge_sim::transit`).
/// A river shuttle's runs between its two landings (Nantes' Navibus): one boat out and back on
/// a two-stop timetable, its place a function of the time as the trams' is.
#[derive(Clone)]
struct BoatRun {
    a: DVec2,
    b: DVec2,
    timetable: Timetable,
}

impl BoatRun {
    /// The shuttle of a crossing: 3.5 m/s at most, gentle at the landings, 45 s at each. It
    /// lands at the water's edges along the crossing, its ends at the quays.
    fn new(c: &forge_procgen::city::BoatCrossing, site: &Site) -> Self {
        let length = c.a.distance(c.b).max(1.0);
        let dir = (c.b - c.a) / length;
        let wet: Vec<f64> = (0..length as usize)
            .map(|k| k as f64)
            .filter(|&s| site.water_at(c.a + dir * s) != forge_procgen::city::LAND)
            .collect();
        let (first, last) = match (wet.first(), wet.last()) {
            (Some(&f), Some(&l)) if l - f > 20.0 => (f + 4.0, l - 4.0),
            _ => (0.0, length),
        };
        let service = Service {
            cruise: 3.5,
            accel: 0.25,
            dwell: 45.0,
            turnaround: 45.0,
            headway: 1e6,
        };
        Self {
            a: c.a + dir * first,
            b: c.a + dir * last,
            timetable: Timetable::new(&[0.0, last - first], service),
        }
    }

    /// Where the boat is at `time` and its turn, on the water's level, in a plan of side
    /// `size`.
    fn at(&self, time: f64, size: f64) -> (Vec3, f32) {
        let run = self.timetable.at(0, time);
        let dir = (self.b - self.a).normalize_or_zero();
        let p = self.a + dir * run.s;
        (world(size, p, WATER_LEVEL), (-dir.y).atan2(dir.x) as f32)
    }
}

#[derive(Clone)]
struct TramRun {
    points: Vec<DVec2>,
    /// Distance along `points` at each point, metres.
    along: Vec<f64>,
    timetable: Timetable,
    colour: [u8; 3],
    /// The junctions it runs through: each one's distance along the route and how far it
    /// reaches either way, metres.
    junctions: Vec<(f64, f64)>,
    /// The stretches of the route over water (its bridges), from and to, metres along it.
    wet: Vec<(f64, f64)>,
    /// Its line, an index into the plan's tram lines.
    line: usize,
    /// The trams its depot keeps, standing: each one's sections as [`TramRun::sections`] gives
    /// them.
    parked: Vec<[(Vec3, f32); TRAM_SECTIONS]>,
    /// In the night's pause ([`tram_pause`]): its trams all in their depots, none running.
    paused: bool,
}

/// The sections of a tram standing with its middle at `at`, the way `along`, in a depot's yard
/// (its rails at the walking level), in a plan of side `size`.
fn parked_sections(at: DVec2, along: DVec2, size: f64) -> [(Vec3, f32); TRAM_SECTIONS] {
    use forge_geom::vehicles::{SECTION_GAP, SECTION_LENGTH};
    let pitch = f64::from(SECTION_LENGTH + SECTION_GAP);
    let nose = at + along * tram_length() * 0.5;
    let mut yaw = (-along.y).atan2(along.x) as f32;
    std::array::from_fn(|i| {
        let front = nose - along * pitch * i as f64;
        let rear = front - along * f64::from(SECTION_LENGTH);
        if i + 1 == TRAM_SECTIONS {
            yaw += std::f32::consts::PI;
        }
        (world(size, (front + rear) * 0.5, WALK_LEVEL + 0.06), yaw)
    })
}

impl TramRun {
    /// A line's runs on `plan`, its stops at their distances along it; `None` with fewer than
    /// two.
    fn new(
        line: &forge_procgen::city::TramLine,
        plan: &CityPlan,
        site: &Site,
        taken: &[DVec2],
    ) -> Option<Self> {
        let points = line.points.clone();
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along[along.len() - 1] + w[0].distance(w[1]));
        }
        // The distance along the route of the point nearest `q`, and how far `q` lies off it.
        let project = |q: DVec2| {
            let mut best = (f64::MAX, 0.0);
            for (i, w) in points.windows(2).enumerate() {
                let t = geom::closest_on_segment(q, w[0], w[1]);
                let p = w[0] + (w[1] - w[0]) * t;
                let d = p.distance(q);
                if d < best.0 {
                    best = (d, along[i] + w[0].distance(p));
                }
            }
            best
        };
        // The junctions it runs through, each with how far it reaches along the route: its
        // widest street's half width and two metres.
        let mut widest = std::collections::HashMap::new();
        for e in &plan.roads.edges {
            for n in [e.a, e.b] {
                let w: &mut f64 = widest.entry(n).or_insert(0.0);
                *w = w.max(e.profile.width() * 0.5);
            }
        }
        let mut junctions: Vec<(f64, f64)> = line
            .edges
            .iter()
            .flat_map(|&e| {
                let e = &plan.roads.edges[e as usize];
                [e.a, e.b]
            })
            .filter_map(|n| {
                let (off, s) = project(plan.roads.nodes[n as usize].pos);
                // Past its corners and its crossings: a platform's ramp met a zebra.
                (off < 15.0).then(|| {
                    let reach = ground::KERB_RADIUS + ground::CROSSING + 2.0;
                    (s, widest.get(&n).copied().unwrap_or(0.0) + reach)
                })
            })
            .collect();
        // A tram depot's link leaving this route keeps the stops off it as a junction does.
        for d in &plan.tram_depots {
            let link = d.link();
            let (off, from) = project(link[0]);
            if off < 5.0 {
                let to = project(link[link.len() / 2]).1;
                junctions.push(((from + to) * 0.5, (to - from).abs() + 10.0));
            }
        }
        junctions.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Each stop at the nearest place along the route where its two platforms and their ramps
        // (37.4 m either way) stand clear of every junction's reach and on straight track, the
        // whole tram level with its platform: they stood across the junctions and on the
        // curves (the owner: bikes rode through them, a stop jutted into the crossroad). A stop
        // with no such place within 400 m is dropped. 60 m apart at least.
        let total = along[along.len() - 1];
        let span = f64::from(PLATFORM) + 1.4;
        let straight = |s: f64| {
            let (t0, t1) = (
                geom::tangent_at(&points, (s - span).max(0.0)),
                geom::tangent_at(&points, (s + span).min(total)),
            );
            let tm = geom::tangent_at(&points, s);
            t0.dot(t1) > 2f64.to_radians().cos() && t0.dot(tm) > 2f64.to_radians().cos()
        };
        // Never on a bridge: the whole platform on land, a metre past it each way (the owner).
        let on_land = |s: f64| {
            let steps = (2.0 * (span + 1.0) / 2.0).ceil() as usize;
            (0..=steps).all(|k| {
                let at = s - span - 1.0 + 2.0 * (span + 1.0) * k as f64 / steps as f64;
                site.water_at(geom::point_at(&points, at.clamp(0.0, total)))
                    == forge_procgen::city::LAND
            })
        };
        // Clear of the junctions, on land, and not within 120 m of another line's stop on this
        // track, which it shares instead (two stops stood one after the other, the owner).
        let clear = |s: f64| {
            s - span >= 0.0
                && s + span <= total
                && junctions.iter().all(|&(j, r)| (s - j).abs() >= r + span)
                && on_land(s)
                && taken.iter().all(|&q| {
                    let (off, at) = project(q);
                    off >= 2.5 || (at - s).abs() >= 120.0
                })
        };
        let fits = |s: f64| clear(s) && straight(s);
        let placed = |fallback: bool| -> Vec<f64> {
            line.stops
                .iter()
                .filter_map(|&stop| {
                    let s = project(stop).1;
                    // A stop another line already has on this track within 150 m: the same one,
                    // shared (several lines' stops stood one over another, the owner).
                    let shared = taken
                        .iter()
                        .map(|&q| project(q))
                        .filter(|&(off, at)| off < 2.5 && (at - s).abs() < 150.0 && fits(at))
                        .min_by(|a, b| (a.1 - s).abs().total_cmp(&(b.1 - s).abs()));
                    if let Some((_, at)) = shared {
                        return Some(at);
                    }
                    // A clean place within 400 m; else one clear of the junctions on a curve
                    // within 800 m; the stop's own place, a junction as often as not, only as
                    // a last resort (the owner: a stop over the intersection, the cross street's
                    // taxis driving over its platforms).
                    let around = |steps: u32| {
                        (0..=steps).flat_map(move |k| {
                            let d = f64::from(k) * 2.5;
                            [s - d, s + d]
                        })
                    };
                    around(160)
                        .find(|&c| fits(c))
                        .or_else(|| around(320).find(|&c| clear(c)))
                        .or(fallback.then_some(s))
                })
                .collect()
        };
        // A stop with no clean place is dropped; only a line left with fewer than two keeps them
        // where they were (a stop stood in a junction, the owner).
        let mut stops = placed(false);
        if stops.len() < 2 {
            stops = placed(true);
        }
        stops.sort_by(f64::total_cmp);
        stops.dedup_by(|a, b| (*a - *b).abs() < 60.0);
        // Its bridges, every 2 m along it.
        let mut wet: Vec<(f64, f64)> = Vec::new();
        let mut s = 0.0;
        while s <= total {
            if site.water_at(geom::point_at(&points, s)) != forge_procgen::city::LAND {
                match wet.last_mut() {
                    Some(w) if w.1 >= s - 2.5 => w.1 = s,
                    _ => wet.push((s, s)),
                }
            }
            s += 2.0;
        }
        (stops.len() >= 2).then(|| Self {
            points,
            along,
            timetable: Timetable::new(&stops, Service::default()),
            colour: line.colour,
            junctions,
            wet,
            line: 0,
            parked: Vec::new(),
            paused: false,
        })
    }

    /// Where its crossover pairs begin along the route, one about every [`CROSSOVER_EVERY`]
    /// metres between its ends' crossovers (a pair or a scissors every 2.5–4 km on French
    /// networks): two crossovers one each way, [`CROSSOVER`] long and 10 m apart, so a tram can
    /// pass a broken-down one on the other track and come back. Each on straight track on land,
    /// clear of the junctions and of the stops' platforms and ramps.
    fn crossover_pairs(&self) -> Vec<f64> {
        let pair = 2.0 * CROSSOVER + 10.0;
        let [(_, first), (last, _)] = self.crossovers();
        let total = self.along[self.along.len() - 1];
        let reach = f64::from(PLATFORM) * 0.5 + PLATFORM_HIGH * 12.0 + 5.0;
        let fits = |s: f64| {
            let (t0, t1) = (
                geom::tangent_at(&self.points, s),
                geom::tangent_at(&self.points, s + pair),
            );
            let tm = geom::tangent_at(&self.points, s + pair * 0.5);
            s > first + 20.0
                && s + pair < last.min(total) - 20.0
                && t0.dot(t1) > 1f64.to_radians().cos()
                && t0.dot(tm) > 1f64.to_radians().cos()
                && self
                    .junctions
                    .iter()
                    .all(|&(j, r)| j + r < s || j - r > s + pair)
                && self
                    .timetable
                    .stops()
                    .iter()
                    .all(|&p| p + reach < s || p - reach > s + pair)
                && self
                    .wet
                    .iter()
                    .all(|&(a, b)| b + 10.0 < s || a - 10.0 > s + pair)
        };
        let mut out: Vec<f64> = Vec::new();
        let mut aim = first + CROSSOVER_EVERY;
        while aim + pair < last {
            // The nearest place that fits within a kilometre either way, past the last pair.
            let found = (0..=400)
                .flat_map(|k| {
                    let d = f64::from(k) * 2.5;
                    [aim - d, aim + d]
                })
                .find(|&s| fits(s) && out.last().is_none_or(|&o| s > o + pair + 500.0));
            if let Some(s) = found {
                out.push(s);
                aim = s + CROSSOVER_EVERY;
            } else {
                aim += CROSSOVER_EVERY;
            }
        }
        out
    }

    /// How far left of the route a crossover pair beginning at `start` runs at `s`, its first
    /// crossover from the left track to the right, its second back.
    fn pair_side(start: f64, s: f64) -> f64 {
        let smooth = |u: f64| {
            let u = u.clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };
        let second = start + CROSSOVER + 10.0;
        if s < second {
            TRACK_OFFSET - 2.0 * TRACK_OFFSET * smooth((s - start) / CROSSOVER)
        } else {
            -TRACK_OFFSET + 2.0 * TRACK_OFFSET * smooth((s - second) / CROSSOVER)
        }
    }

    /// The point `s` metres along the route and its unit direction, carried straight on past
    /// either end.
    fn track(&self, s: f64) -> (DVec2, DVec2) {
        let length = self.along[self.along.len() - 1];
        let inside = s.clamp(0.0, length);
        let p = geom::point_at(&self.points, inside);
        let t = geom::tangent_at(&self.points, inside);
        (p + t * (s - inside), t)
    }

    /// Each stop's two platforms, one beyond each track (a tram's half width and a hand from
    /// it), each running the way its trams do, in a plan of side `size`: its transform, its stop
    /// and whether it serves the outbound trams.
    fn platforms(&self, size: f64) -> Vec<(Mat4, usize, bool)> {
        let mut out = Vec::new();
        let length = f64::from(PLATFORM);
        for (stop, &s) in self.timetable.stops().iter().enumerate() {
            let (p, t) = self.track(s);
            let n = DVec2::new(-t.y, t.x);
            // The outbound track lies to the left of the line (+n), the inbound to the right.
            for (side, outbound) in [(1.0, true), (-1.0, false)] {
                let (along, away) = (t * side, n * side);
                // A tram stands with its middle at the stop, both ways: the two platforms face
                // each other, and a tram turning at an end stays where it stood.
                let corner = p - along * (length * 0.5) + away * (TRACK_OFFSET + 1.35);
                let axis = |d: DVec2| Vec3::new(d.x as f32, 0.0, d.y as f32);
                let at = Mat4::from_cols(
                    axis(along).extend(0.0),
                    Vec3::Y.extend(0.0),
                    axis(away).extend(0.0),
                    world(size, corner, 0.16).extend(1.0),
                );
                out.push((at, stop, outbound));
            }
        }
        out
    }

    /// The terminus crossovers' spans along the route: past the first stop's standing tram on
    /// the way in from the line, and before the last stop's. A tram coming in crosses to the
    /// track it leaves by, and turns where it stands: it jumped a track's width and a tram's
    /// length across and along as it turned.
    fn crossovers(&self) -> [(f64, f64); 2] {
        let stops = self.timetable.stops();
        let reach = tram_length() * 0.5 + CROSSOVER_CLEAR;
        let (first, last) = (stops[0], stops[stops.len() - 1]);
        [
            (first + reach, first + reach + CROSSOVER),
            (last - reach - CROSSOVER, last - reach),
        ]
    }

    /// How far left of the route a tram running `outbound` rides at `s`: on its own track, out
    /// on the left one and back on the right, crossing over on an S-curve in the terminus
    /// crossover it runs through on its way in.
    fn side(&self, s: f64, outbound: bool) -> f64 {
        let [(a0, a1), (b0, b1)] = self.crossovers();
        let smooth = |u: f64| {
            let u = u.clamp(0.0, 1.0);
            u * u * (3.0 - 2.0 * u)
        };
        if outbound {
            TRACK_OFFSET - 2.0 * TRACK_OFFSET * smooth((s - b0) / (b1 - b0))
        } else {
            -TRACK_OFFSET + 2.0 * TRACK_OFFSET * smooth((a1 - s) / (a1 - a0))
        }
    }

    /// The transforms of vehicle `k`'s sections at `time`, in a plan of side `size`: the
    /// front cab, the passenger sections, the rear cab (turned round), each set by its two
    /// ends on the track of the way it runs, the tram's middle at its place in the timetable.
    fn sections(&self, k: usize, time: f64, size: f64) -> [(Vec3, f32); TRAM_SECTIONS] {
        use forge_geom::vehicles::{SECTION_GAP, SECTION_LENGTH};
        let run = self.timetable.at(k, time);
        let dir = if run.outbound { 1.0 } else { -1.0 };
        let on_track = |s: f64| {
            let (p, t) = self.track(s);
            p + DVec2::new(-t.y, t.x) * self.side(s, run.outbound)
        };
        let pitch = f64::from(SECTION_LENGTH + SECTION_GAP);
        let nose = run.s + dir * tram_length() * 0.5;
        std::array::from_fn(|i| {
            let front = nose - dir * pitch * i as f64;
            let rear = front - dir * f64::from(SECTION_LENGTH);
            let (a, b) = (on_track(front), on_track(rear));
            let heading = (a - b).normalize_or_zero();
            let mut yaw = (-heading.y).atan2(heading.x) as f32;
            if i + 1 == TRAM_SECTIONS {
                yaw += std::f32::consts::PI;
            }
            (world(size, (a + b) * 0.5, 0.22), yaw)
        })
    }
}

/// The kit's share of a scene.
struct Kit {
    /// The instanced modules.
    meshes: Vec<TriMesh>,
    /// Their instances: module, transform, material group.
    instances: Vec<(u32, Mat4, u8)>,
    faces: usize,
    terminals: usize,
    /// The longest run of identical openings along one floor of one street front.
    longest_run: usize,
    /// The terraces' seats: where each chair stands and the way its sitter faces.
    seats: Vec<(Vec3, f32)>,
    /// Derivation and assembly, milliseconds.
    ms: f64,
    /// The share of the homes' windows their residents lit, for the far city's.
    homes_share: Option<f64>,
    /// Who lit the homes' windows, for the log.
    lights: Option<String>,
}

/// Moves a module's railings' cards (its [`modules::GRILLE`] triangles) out into a mesh of their
/// own, merged into the tiles' railings: only their clusters then take the cut-out raster, not
/// the module's stone, frames and shutters with them. Empty without `--railing-cards`.
fn take_cards(mesh: &mut TriMesh) -> TriMesh {
    let mut cards = TriMesh::default();
    let mut indices = Vec::with_capacity(mesh.indices.len());
    let mut sections = Vec::with_capacity(mesh.sections.len());
    for t in 0..mesh.triangle_count() {
        let tri = &mesh.indices[t * 3..t * 3 + 3];
        if mesh.section(t) != modules::GRILLE {
            indices.extend_from_slice(tri);
            sections.push(mesh.section(t));
            continue;
        }
        let base = cards.positions.len() as u32;
        for &i in tri {
            cards.positions.push(mesh.positions[i as usize]);
            cards.normals.push(mesh.normals[i as usize]);
            cards.uvs.push(mesh.uvs[i as usize]);
        }
        cards.indices.extend([base, base + 1, base + 2]);
    }
    mesh.indices = indices;
    mesh.sections = sections;
    mesh.uvs.clear();
    cards
}

/// A 64-bit mix (SplitMix64's finaliser).
fn mix(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// A mass's render tint, one of [`TINTS`], the same near (the kit's walls) and far (the facade
/// texture's).
fn tint_of(lot: u32) -> u8 {
    (lot.wrapping_mul(2_654_435_761) >> 30) as u8 % TINTS
}

/// The masses `near` built from `d`'s grammar and kit over their structures (D-060): each
/// storey of each wall dressed round its openings, the walls, bands and cornices merged into
/// the tiles, the openings instanced, their reveals as deep as their walls allow; the passages
/// into the courts open through the buildings; and their roofs.
#[allow(clippy::too_many_arguments)]
fn kit(
    tiles: &mut Tiles,
    masses: &[Mass],
    structures: &[Structure],
    edges: &[Vec<Option<FaceKind>>],
    tall: &[Vec<bool>],
    near: &[usize],
    d: &Detail,
    residents: Option<&residents::Residents>,
    bollards: &[DVec2],
) -> Kit {
    let start = std::time::Instant::now();
    // The homes' windows: lit by their flats' households, and how many.
    let mut homes_lit = (0, 0, 0);
    // The offices' windows, and how many lit.
    let mut offices_lit = (0, 0);
    let size = tiles.size;
    let (dressed, _stats) = facades::dress(d.grammar, structures, masses, near, d.top, d.seed);
    // The houses whose insides are drawn (`city::interior`): their doors stand open, the way in.
    let entered: Vec<bool> = masses
        .iter()
        .zip(structures)
        .map(|(m, s)| m.rooms.is_some() && interior::house(m, s).is_some())
        .collect();
    let mut keys: std::collections::HashMap<(u16, i32, i32, i32), u32> =
        std::collections::HashMap::new();
    let mut meshes = Vec::new();
    // Each instanced module's railings' cards, by its mesh.
    let mut cards: std::collections::HashMap<u32, TriMesh> = std::collections::HashMap::new();
    let mut instances = Vec::new();
    let mut seats = Vec::new();
    let (mut run, mut longest_run, mut last) = (0, 0, None);
    // The cornices: each its mass, its wall, its foot and its height.
    let mut cornices: Vec<(usize, usize, f64, f64)> = Vec::new();
    for piece in &dressed {
        let t = piece.terminal;
        let m = &masses[piece.mass as usize];
        let wall = &structures[piece.mass as usize].walls[piece.wall as usize];
        let street = wall.role == WallRole::Street;
        let open = entered[piece.mass as usize] && d.grammar.name(t.module) == "door";
        let name = if open {
            "door_open"
        } else {
            d.grammar.name(t.module)
        };
        let (w, h) = (t.scope.size.x as f32, t.scope.size.y as f32);
        let o = t.scope.origin;
        let pos = Vec3::new(
            (o.x - size * 0.5) as f32,
            o.y as f32,
            (o.z - size * 0.5) as f32,
        );
        let (x, y, z) = (
            t.scope.x.as_vec3(),
            t.scope.y.as_vec3(),
            t.scope.normal().as_vec3(),
        );
        if modules::shared(name) {
            // The window set two thirds into its wall: old stone's deep reveals, modern walls'
            // shallow ones.
            let reveal = (wall.thickness * 0.65).clamp(0.12, 0.40) as f32;
            let key = (
                // An open door is its own mesh.
                t.module | if open { 0x8000 } else { 0 },
                (w * 100.0).round() as i32,
                (h * 100.0).round() as i32,
                (reveal * 100.0).round() as i32,
            );
            let id = match keys.get(&key) {
                Some(&id) => id,
                None => {
                    let Some(mut mesh) = modules::module_with(name, w, h, reveal, d.railing_cards)
                    else {
                        continue;
                    };
                    let railing = take_cards(&mut mesh);
                    meshes.push(mesh);
                    let id = (meshes.len() - 1) as u32;
                    keys.insert(key, id);
                    cards.insert(id, railing);
                    id
                }
            };
            let colour = (mix(u64::from(m.lot)) % 4) as u8;
            let spot = mix(u64::from(pos.x.to_bits()) << 32 | u64::from(pos.z.to_bits()))
                ^ mix(u64::from(pos.y.to_bits()));
            // Its unit, holding a point half a metre inside it. A home's window lit by its
            // household, at home and awake or not (an empty flat dark); an office's by the
            // working day (`office_lights`); a shop's in the share of the hour (`lit_share`).
            // Each window its own draw.
            let s = &structures[piece.mass as usize];
            let unit = piece.storey.and_then(|k| {
                let n = t.scope.normal();
                let inside = DVec2::new(o.x, o.z)
                    + DVec2::new(t.scope.x.x, t.scope.x.z) * (f64::from(w) * 0.5)
                    - DVec2::new(n.x, n.z) * (wall.thickness + 0.5);
                residents::unit_at(s, usize::from(k), inside)
            });
            let use_ = unit.map(|k| s.units[k].use_);
            let shop = name.starts_with("shop") || use_ == Some(units::Use::Shop);
            let office = use_ == Some(units::Use::Office);
            let by_residents = residents
                .filter(|_| !shop && !office)
                .and_then(|r| r.lit(s, unit, spot));
            if by_residents.is_some() {
                homes_lit.2 += usize::from(matches!(use_, Some(units::Use::Home(_))));
            }
            let lit = by_residents.unwrap_or_else(|| {
                let share = d.lit_hour.map_or(0.4, |h| {
                    if office {
                        residents::office_lights(h)
                    } else {
                        lit_share(h, shop)
                    }
                });
                (((spot >> 20) % 1000) as f64) < share * 1000.0
            });
            if let Some(l) = by_residents {
                homes_lit.0 += 1;
                homes_lit.1 += usize::from(l);
            }
            if office {
                offices_lit.0 += 1;
                offices_lit.1 += usize::from(lit);
            }
            let transform =
                Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), pos.extend(1.0));
            instances.push((id, transform, colour * 2 + u8::from(lit)));
            // Its railings' cards into its tile's railings.
            if let Some(c) = cards.get(&id).filter(|c| !c.indices.is_empty()) {
                let tile = tiles.at(m.footprint[0]);
                let at = pos - tile.centre;
                let dst = &mut tile.railings;
                let base = dst.positions.len() as u32;
                for ((p, n), uv) in c.positions.iter().zip(&c.normals).zip(&c.uvs) {
                    let (p, n) = (Vec3::from(*p), Vec3::from(*n));
                    dst.positions
                        .push((at + x * p.x + y * p.y + z * p.z).to_array());
                    dst.normals.push((x * n.x + y * n.y + z * n.z).to_array());
                    dst.uvs.push(*uv);
                }
                dst.indices.extend(c.indices.iter().map(|i| base + i));
                dst.sections
                    .extend(std::iter::repeat_n(0, c.triangle_count()));
            }
            // A café's terrace before a third of the shopfronts on the avenues' pavements and
            // in the lanes, and two thirds of those facing a square, a park or a quay
            // (restaurants and cafés by the water), on the pavement's level.
            let lively = m.lively();
            if name.starts_with("shop")
                && street
                && lively
                && (spot.is_multiple_of(3) || (m.open && spot % 3 == 1))
                // Clear of a pedestrian street's bollards: a terrace stood in their row (the owner:
                // the tables and chairs too close).
                && !bollards.iter().any(|b| {
                    world(size, *b, 0.0).with_y(0.0).distance(pos.with_y(0.0)) < 8.0
                })
            {
                // One of four layouts by the shopfront, so neighbouring terraces differ.
                let variant = ((spot >> 8) % 4) as u32;
                let key = (TERRACE, (w * 100.0).round() as i32, variant as i32, 0);
                let id = *keys.entry(key).or_insert_with(|| {
                    meshes.push(forge_geom::furniture::terrace(w, variant));
                    (meshes.len() - 1) as u32
                });
                let level = WALK_LEVEL as f32;
                let at = Mat4::from_cols(
                    x.extend(0.0),
                    y.extend(0.0),
                    z.extend(0.0),
                    Vec3::new(pos.x, level, pos.z).extend(1.0),
                );
                instances.push((id, at, colour * 2));
                for (p, d) in forge_geom::furniture::terrace_seats(w, variant) {
                    let (p, d) = (at.transform_point3(p), at.transform_vector3(d));
                    seats.push((p, (-d.z).atan2(d.x)));
                }
            }
            // Runs of one opening along a storey of a street front.
            if street {
                let row = (piece.mass, piece.wall, piece.storey);
                if last == Some((row, id)) {
                    run += 1;
                } else {
                    run = 1;
                }
                last = Some((row, id));
                longest_run = longest_run.max(run);
            }
        } else {
            let Some(mesh) = modules::module(name, w, h) else {
                continue;
            };
            if name == "cornice" {
                cornices.push((
                    piece.mass as usize,
                    piece.wall as usize,
                    o.y,
                    t.scope.size.y,
                ));
            }
            let tint = tint_of(m.lot);
            let tile = tiles.at(m.footprint[0]);
            let at = pos - tile.centre;
            let dst = &mut tile.masses;
            let base = dst.positions.len() as u32;
            for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
                let (p, n) = (Vec3::from(*p), Vec3::from(*n));
                dst.positions
                    .push((at + x * p.x + y * p.y + z * p.z).to_array());
                dst.normals.push((x * n.x + y * n.y + z * n.z).to_array());
                dst.uvs.push([0.0; 2]);
            }
            dst.indices.extend(mesh.indices.iter().map(|i| base + i));
            dst.sections
                .extend((0..mesh.triangle_count()).map(|t| match mesh.section(t) {
                    modules::PLASTER => KIT_PLASTER + tint,
                    modules::PARTY => KIT_PARTY,
                    _ => KIT_STONE,
                }));
        }
    }
    cornice_corners(tiles, masses, structures, &cornices);
    for &i in near {
        passages(tiles, &structures[i]);
        roof(tiles, &masses[i], &edges[i], &tall[i], KIT_PARTY);
    }
    Kit {
        meshes,
        instances,
        faces: dressed.len(),
        terminals: dressed.len(),
        longest_run,
        seats,
        ms: start.elapsed().as_secs_f64() * 1e3,
        homes_share: residents.map(|_| homes_lit.1 as f64 / homes_lit.0.max(1) as f64),
        lights: residents.map(|r| {
            let (windows, lit, in_flat) = homes_lit;
            let (offices, offices_on) = offices_lit;
            format!(
                "the residents' lights: {} of {} households at home and awake, {lit} of \
                 {windows} homes' windows lit ({in_flat} in their flats), {offices_on} of \
                 {offices} offices' windows, found in {:.0} ms",
                r.households.1,
                r.households.0,
                r.seconds * 1e3
            )
        }),
    }
}

/// The corners where two walls' cornices meet outside a building (the owner, 2026-10-16: "the
/// places between the roof borders at where they meet at the house sides are not filled. it could
/// be angular or rounded"): each step of the cornice ([`modules::CORNICE_STEPS`]) mitred round
/// the corner, a prism from the corner out to where the two steps' faces meet. `cornices` holds
/// each cornice's mass, wall, foot and height.
fn cornice_corners(
    tiles: &mut Tiles,
    masses: &[Mass],
    structures: &[Structure],
    cornices: &[(usize, usize, f64, f64)],
) {
    let size = tiles.size;
    for &(i, w, y, h) in cornices {
        let s = &structures[i];
        let wall = &s.walls[w];
        // The wall starting where this one ends, its cornice as high.
        let next = cornices.iter().find(|&&(j, v, y2, h2)| {
            j == i
                && v != w
                && s.walls[v].a.distance(wall.b) < 0.01
                && (y2 - y).abs() < 0.01
                && (h2 - h).abs() < 0.01
        });
        let Some(&(_, v, _, _)) = next else {
            continue;
        };
        let (d1, d2) = (
            (wall.b - wall.a).normalize_or_zero(),
            (s.walls[v].b - s.walls[v].a).normalize_or_zero(),
        );
        // Outside corners only: the outline turns left there (it winds anticlockwise). Inside
        // ones the two cornices cross.
        if d1.perp_dot(d2) <= 1e-6 {
            continue;
        }
        let (n1, n2) = (DVec2::new(d1.y, -d1.x), DVec2::new(d2.y, -d2.x));
        let cos = n1.dot(n2);
        // Sharper than about 70°, the mitre would reach far out: left open.
        if 1.0 + cos < 0.3 {
            continue;
        }
        let c = wall.b;
        let tile = tiles.at(masses[i].footprint[0]);
        let centre = tile.centre;
        let at = |q: DVec2, height: f64| world(size, q, height) - centre;
        for (f0, f1, out) in modules::CORNICE_STEPS {
            let out = f64::from(out);
            let tip = c + (n1 + n2) * (out / (1.0 + cos));
            let ring = [c, c + n1 * out, tip, c + n2 * out];
            let (y0, y1) = (y + h * f64::from(f0), y + h * f64::from(f1));
            solid(&mut tile.masses, &at, &ring, y0, y1, KIT_STONE);
        }
    }
}

/// How high a head house stands over its roof, metres: a storey's clear height and its slab.
const HEAD_HOUSE: f64 = 2.7;
/// How far a lift's overrun rises over the roof, or over the head house, metres.
const OVERRUN: f64 = 1.2;

/// The insides of the kit's buildings (D-060): each wall's inner face at the wall's thickness,
/// a floor and a ceiling a storey, so a camera inside one sees a dark room and not, through
/// its culled walls, the city beyond, the neighbours' party walls and the sun's blurred shadows
/// on them (the owner, 2026-10-15: "many useless things rendered inside the empty buildings").
/// Nothing of it shows from outside. The triangles it adds.
fn insides(tiles: &mut Tiles, masses: &[Mass], structures: &[Structure], near: &[usize]) -> usize {
    let size = tiles.size;
    let mut count = 0;
    for &i in near {
        let (m, s) = (&masses[i], &structures[i]);
        let tile = tiles.at(m.footprint[0]);
        let centre = tile.centre;
        let at = |q: DVec2, h: f64| world(size, q, h) - centre;
        let mesh = &mut tile.insides;
        let before = mesh.indices.len();
        let plaster = KIT_INSIDE;
        let top = m.height;
        let mut deepest: f64 = 0.2;
        for (w, wall) in s.walls.iter().enumerate() {
            if wall.role == WallRole::Inner || wall.length() < 0.05 {
                continue;
            }
            let deep = wall.thickness.max(0.2);
            deepest = deepest.max(deep);
            let t = (wall.b - wall.a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x);
            let normal = Vec3::new(-t.y as f32, 0.0, t.x as f32);
            // How far the face runs past an end: at a concave corner (the footprint turns right
            // there, it winds anticlockwise), the thickness of the wall it meets, so the two
            // faces reach each other; at a convex corner not at all, its end then on that wall's
            // outer face. Run past its ends at every corner by its own thickness, a side wall's
            // face stood out of the facade into the street, a pale upright strip beside the
            // doors (the owner, 2026-10-16).
            let n = s.walls.len();
            let along = |k: usize| (s.walls[k].b - s.walls[k].a).normalize_or_zero();
            let (prev, next) = ((w + n - 1) % n, (w + 1) % n);
            let run_a = if along(prev).perp_dot(t) < -1e-6 {
                s.walls[prev].thickness.max(0.2)
            } else {
                0.0
            };
            let run_b = if t.perp_dot(along(next)) < -1e-6 {
                s.walls[next].thickness.max(0.2)
            } else {
                0.0
            };
            // The face from `from` to `to` along the wall, `y0` to `y1`.
            let mut face = |from: f64, to: f64, y0: f64, y1: f64| {
                let start = if from <= 0.0 { run_a } else { 0.0 };
                let end = if to >= wall.length() { run_b } else { 0.0 };
                let p = wall.a + t * (from - start) + inward * deep;
                let q = wall.a + t * (to + end) + inward * deep;
                let (l, h) = ((p.distance(q) / 3.0) as f32, ((y1 - y0) / 3.0) as f32);
                triangle_uv(
                    mesh,
                    [at(p, y0), at(q, y0), at(q, y1)],
                    [[0.0, 0.0], [l, 0.0], [l, -h]],
                    normal,
                    plaster,
                );
                triangle_uv(
                    mesh,
                    [at(p, y0), at(q, y1), at(p, y1)],
                    [[0.0, 0.0], [l, -h], [0.0, -h]],
                    normal,
                    plaster,
                );
            };
            // The face is open at every opening of its wall, from the opening's sill to its head
            // on its storey: doors, windows, shopfronts and passages. Cut only at the passages
            // (whole, it closed them behind their mouths), it covered every door and window from
            // inside, and the rooms got no daylight through them (the owner, 2026-10-16: "inside
            // I don't even see outside light", a door open outside and a wall behind it).
            // The wall in bands between the openings' edges, each band's face filled from the
            // floor to the top round the openings that cross it.
            let length = wall.length();
            // Each hole with how deep the kit's module lines it from the facade (`kit`): a
            // window two thirds into its wall, a door 0.35 m, a shopfront 0.3; a passage's
            // linings are its own (`passages`).
            let window_reveal = (wall.thickness * 0.65).clamp(0.12, 0.40);
            let holes: Vec<(f64, f64, f64, f64, Option<f64>, f64)> = s
                .openings
                .iter()
                .filter(|o| o.wall == w)
                .map(|o| {
                    let level = s.storeys.get(o.storey).map_or(0.0, |st| st.level);
                    let (lo, hi) = o.span();
                    let lined = match o.kind {
                        OpeningKind::Window => Some(window_reveal),
                        OpeningKind::Door => Some(0.35),
                        OpeningKind::Shopfront => Some(0.3),
                        OpeningKind::Passage => None,
                    };
                    let head = (level + o.head).min(top);
                    let floor = floor_level(s, o.storey);
                    (
                        lo.max(0.0),
                        hi.min(length),
                        level + o.sill,
                        head,
                        lined,
                        floor,
                    )
                })
                .filter(|h| h.1 > h.0 + 0.01 && h.3 > h.2 + 0.01)
                .collect();
            let mut edges: Vec<f64> = vec![0.0, length];
            for h in &holes {
                edges.extend([h.0, h.1]);
            }
            edges.sort_by(f64::total_cmp);
            edges.dedup_by(|x, y| (*x - *y).abs() < 0.005);
            for pair in edges.windows(2) {
                let (x0, x1) = (pair[0], pair[1]);
                if x1 - x0 < 0.005 {
                    continue;
                }
                let mid = (x0 + x1) * 0.5;
                let mut over: Vec<(f64, f64)> = holes
                    .iter()
                    .filter(|h| h.0 <= mid && mid <= h.1)
                    .map(|h| (h.2.max(0.0), h.3.min(top)))
                    .collect();
                over.sort_by(|p, q| p.0.total_cmp(&q.0));
                let mut y = 0.0;
                for (c0, c1) in over {
                    if c0 > y + 0.005 {
                        face(x0, x1, y, c0);
                    }
                    y = y.max(c1);
                }
                if top > y + 0.005 {
                    face(x0, x1, y, top);
                }
            }
            // Each hole's reveals from where the kit's module stops to the inner face: the
            // jambs, the head and, over a window's sill, the sill. Without them a window seen
            // from inside showed the garden's lawn under its frame, through the hollow wall
            // (the owner, 2026-10-16: "windows issues", "door transitions are wrong no wall").
            for &(lo, hi, y0, y1, lined, floor) in &holes {
                let Some(from) = lined.filter(|r| deep > r + 0.005) else {
                    continue;
                };
                let p = |x: f64, d: f64| wall.a + t * x + inward * d;
                let (tx, h) = (
                    Vec3::new(t.x as f32, 0.0, t.y as f32),
                    ((y1 - y0) / 3.0) as f32,
                );
                let r = ((deep - from) / 3.0) as f32;
                for (x, n) in [(lo, tx), (hi, -tx)] {
                    let (a, b) = (p(x, from), p(x, deep));
                    triangle_uv(
                        mesh,
                        [at(a, y0), at(b, y0), at(b, y1)],
                        [[0.0, 0.0], [r, 0.0], [r, -h]],
                        n,
                        plaster,
                    );
                    triangle_uv(
                        mesh,
                        [at(a, y0), at(b, y1), at(a, y1)],
                        [[0.0, 0.0], [r, -h], [0.0, -h]],
                        n,
                        plaster,
                    );
                }
                let l = ((hi - lo) / 3.0) as f32;
                // The head; a window's sill; a door's floor through the wall at its storey's
                // floor (`floor_level`: the ground floor 2 cm over the sill the kit's threshold
                // stone stands on), in the floor's stone: from the threshold's inner end to the
                // inner face the street's paving showed through the doorway (the owner,
                // 2026-10-16: "entry pavement leaks into the house floor"). A door's sill stands
                // at the walking level, over the storey's level: compared with that, every door
                // took a window's sill cap, at the paving's height, and the two fought in every
                // doorway (the owner, 2026-10-17: "plenty of issue with z fighting on the floor").
                let mut caps = vec![(y1, Vec3::NEG_Y, plaster)];
                if y0 > floor + 0.05 {
                    caps.push((y0, Vec3::Y, plaster));
                } else {
                    caps.push((floor, Vec3::Y, KIT_INSIDE_FLOOR));
                }
                for (y, n, section) in caps {
                    let (a, b, c, d) = (p(lo, from), p(hi, from), p(hi, deep), p(lo, deep));
                    triangle_uv(
                        mesh,
                        [at(a, y), at(b, y), at(c, y)],
                        [[0.0, 0.0], [l, 0.0], [l, r]],
                        n,
                        section,
                    );
                    triangle_uv(
                        mesh,
                        [at(a, y), at(c, y), at(d, y)],
                        [[0.0, 0.0], [l, r], [0.0, r]],
                        n,
                        section,
                    );
                }
            }
        }
        // The floors and the ceilings: the footprint inside its walls, a slab a storey, the
        // ground floor 2 cm over the walking level its door's sill stands on (5 cm up, a garden's
        // lawn showed through it), the top ceiling under the roof.
        let Some(ring) = inner_ring(m, s) else {
            count += (mesh.indices.len() - before) / 3;
            continue;
        };
        let cap = |mesh: &mut TriMesh, ring: &[DVec2], y: f64, normal: Vec3, section: u8| {
            for t in geom::triangulate(ring) {
                let p = t.map(|k| at(ring[k], y));
                triangle_uv(mesh, p, [[0.0; 2]; 3], normal, section);
            }
        };
        // The ground floor keeps clear of the passages (their paving is the street's): the
        // ring cut on either side of each, from its street end to its court end.
        let ends: Vec<(DVec2, DVec2)> = s
            .openings
            .iter()
            .filter(|o| o.kind == OpeningKind::Passage)
            .map(|o| {
                let w = &s.walls[o.wall];
                let t = (w.b - w.a).normalize_or_zero();
                (w.a + t * o.span().0, w.a + t * o.span().1)
            })
            .collect();
        let mut ground = vec![ring.clone()];
        for pair in ends.as_chunks::<2>().0 {
            let ((p0, q0), (p1, q1)) = (pair[0], pair[1]);
            let (p1, q1) = if p0.distance(p1) + q0.distance(q1) <= p0.distance(q1) + q0.distance(p1)
            {
                (p1, q1)
            } else {
                (q1, p1)
            };
            let middle = (p0 + q0 + p1 + q1) * 0.25;
            let mut next = Vec::new();
            for poly in &ground {
                for (o, d) in [(p0, p1 - p0), (q0, q1 - q0)] {
                    // The side of the passage's wall away from its middle.
                    let dir = if d.perp_dot(middle - o) > 0.0 { -d } else { d };
                    let part = geom::clean(&geom::clip_half_plane(poly, o, dir), 1e-6);
                    if part.len() >= 3 && geom::signed_area(&part).abs() > 0.5 {
                        next.push(part);
                    }
                }
            }
            ground = next;
        }
        // A house's insides from its rooms (`city::interior`): each upper floor open over the
        // treads of the flight below, its landing at the top kept.
        let inner = interior::house(m, s);
        let hole = |k: usize| -> Option<[DVec2; 4]> {
            let inner = inner.as_ref()?;
            let stair = inner.stair?;
            let r = *inner.risers.get(k.checked_sub(1)?)?;
            let run = f64::from(r.saturating_sub(1)) * cores::GOING;
            let up = inner.climb * run;
            Some([stair[0], stair[1], stair[1] + up, stair[0] + up])
        };
        for (k, st) in s.storeys.iter().enumerate() {
            if k == 0 {
                for poly in &ground {
                    cap(
                        mesh,
                        poly,
                        st.level.max(WALK_LEVEL + 0.02),
                        Vec3::Y,
                        KIT_INSIDE_FLOOR,
                    );
                }
            } else {
                let pieces = hole(k).map_or_else(|| vec![ring.clone()], |h| without(&ring, &h));
                for poly in &pieces {
                    cap(mesh, poly, st.level, Vec3::Y, KIT_INSIDE_FLOOR);
                    cap(mesh, poly, st.level - st.slab, -Vec3::Y, plaster);
                }
            }
        }
        if let Some(last) = s.storeys.last() {
            let ceiling = (last.level + last.height - last.slab).min(top - 0.05);
            cap(mesh, &ring, ceiling, -Vec3::Y, plaster);
        }
        if let Some(inner) = &inner {
            // The flights' steps in the floors' smooth limestone: in the facades' pitted stone,
            // lit at a grazing angle through the door, their treads glittered (the owner,
            // 2026-10-17: "Stairs lighting is still strange").
            for (ring, y0, y1, stone) in interior_solids(s, inner, top) {
                let section = if stone { KIT_INSIDE_FLOOR } else { plaster };
                solid(mesh, &at, &ring, y0, y1, section);
            }
            soffits(mesh, &at, s, inner, plaster);
        }
        count += (mesh.indices.len() - before) / 3;
    }
    count
}

/// The kit's mesh for a piece of a home's furniture (`forge_geom::furniture::home`).
fn home_name(piece: furnish::Piece) -> &'static str {
    match piece {
        furnish::Piece::Bed => "bed",
        furnish::Piece::SingleBed => "single_bed",
        furnish::Piece::Wardrobe => "wardrobe",
        furnish::Piece::Sofa => "sofa",
        furnish::Piece::LowTable => "low_table",
        furnish::Piece::Counter => "counter",
        furnish::Piece::Table => "table",
        furnish::Piece::Chair => "chair",
        furnish::Piece::Bath => "bath",
        furnish::Piece::Basin => "basin",
        furnish::Piece::Toilet => "toilet",
        furnish::Piece::Desk => "desk",
        furnish::Piece::Shelves => "shelves",
    }
}

/// A mass's floor inside its walls: the footprint moved in by each wall's drawn thickness (its
/// inner face's, 0.2 m at least; 5 cm between a building's masses), or by the thickest where that
/// fails. A uniform inset by the thickest left a strip of the ground showing by the thinner party
/// walls.
fn inner_ring(m: &Mass, s: &Structure) -> Option<Vec<DVec2>> {
    let d: Vec<f64> = s
        .walls
        .iter()
        .map(|w| {
            if w.role == WallRole::Inner {
                0.05
            } else {
                w.thickness.max(0.2)
            }
        })
        .collect();
    let deepest = d.iter().copied().fold(0.2, f64::max);
    geom::inset_by(&m.footprint, &d)
        .or_else(|| geom::inset_by_collapsing(&m.footprint, &d))
        .or_else(|| geom::inset(&m.footprint, deepest))
        .filter(|r| r.len() >= 3)
}

/// A storey's floor inside a house, metres: the ground floor 2 cm over the walking level its
/// door's sill stands on.
fn floor_level(s: &Structure, k: usize) -> f64 {
    let st = &s.storeys[k];
    if k == 0 {
        st.level.max(WALK_LEVEL + 0.02)
    } else {
        st.level
    }
}

/// Each flight's soffit (`interior_solids` draws its steps): a slab sloping with the flight under
/// its steps, its underside through the steps' back lower corners, so the flight seen from below
/// is one smooth slope and not every step's flat bottom in a sawtooth; from where that underside
/// leaves the floor to the last step, plastered as the walls.
fn soffits(
    mesh: &mut TriMesh,
    at: &dyn Fn(DVec2, f64) -> Vec3,
    s: &Structure,
    inner: &interior::Interior,
    section: u8,
) {
    let Some(stair) = inner.stair else {
        return;
    };
    let across = stair[1] - stair[0];
    let width = across.length();
    let a = across / width.max(1e-6);
    let c = inner.climb.normalize_or_zero();
    let g = cores::GOING;
    for (k, &r) in inner.risers.iter().enumerate() {
        if k + 1 >= s.storeys.len() || r < 3 {
            break;
        }
        let base = floor_level(s, k);
        let rise = (s.storeys[k + 1].level - base) / f64::from(r);
        // The steps' nosings' line, the underside 2 risers and 15 cm under it (through each
        // step's back lower corner), the top a riser under it (inside the steps).
        let nosing = |v: f64| base + rise + v * rise / g;
        let under = |v: f64| nosing(v) - 2.0 * rise - 0.15;
        let over = |v: f64| nosing(v) - rise;
        let (v0, v1) = (g * (1.0 + 0.15 / rise), f64::from(r - 1) * g);
        if v1 <= v0 + 0.05 {
            continue;
        }
        let p = |v: f64, u: f64, y: f64| at(stair[0] + c * v + a * u, y);
        let slope = Vec3::new(c.x as f32, (rise / g) as f32, c.y as f32).normalize();
        let side = Vec3::new(a.x as f32, 0.0, a.y as f32);
        let mut down = slope.cross(side).normalize();
        if down.y > 0.0 {
            down = -down;
        }
        let quad = |mesh: &mut TriMesh, q: [Vec3; 4], n: Vec3| {
            triangle_uv(mesh, [q[0], q[1], q[2]], [[0.0; 2]; 3], n, section);
            triangle_uv(mesh, [q[0], q[2], q[3]], [[0.0; 2]; 3], n, section);
        };
        // The underside.
        quad(
            mesh,
            [
                p(v0, 0.0, under(v0)),
                p(v1, 0.0, under(v1)),
                p(v1, width, under(v1)),
                p(v0, width, under(v0)),
            ],
            down,
        );
        // Its two sides and its two ends.
        for (u, n) in [(0.0, -side), (width, side)] {
            quad(
                mesh,
                [
                    p(v0, u, under(v0)),
                    p(v1, u, under(v1)),
                    p(v1, u, over(v1)),
                    p(v0, u, over(v0)),
                ],
                n,
            );
        }
        let along = Vec3::new(c.x as f32, 0.0, c.y as f32);
        for (v, n) in [(v0, -along), (v1, along)] {
            quad(
                mesh,
                [
                    p(v, 0.0, under(v)),
                    p(v, width, under(v)),
                    p(v, width, over(v)),
                    p(v, 0.0, over(v)),
                ],
                n,
            );
        }
    }
}

/// A house's insides as solids (`city::interior`): its partitions floor to ceiling with a lintel
/// over each doorway, and its flights a stone step a riser with a hand of stone under it, from
/// the street wall into the house (the last riser climbs on to the floor above). Each a ring
/// (anticlockwise, the plan's metres), its bottom and its top, and whether it is stone.
fn interior_solids(
    s: &Structure,
    inner: &interior::Interior,
    top: f64,
) -> Vec<(Vec<DVec2>, f64, f64, bool)> {
    let mut out = Vec::new();
    for p in &inner.partitions {
        let st = &s.storeys[p.storey];
        let (y0, y1) = (
            floor_level(s, p.storey),
            (st.level + st.height - st.slab).min(top - 0.05),
        );
        let t = (p.b - p.a).normalize_or_zero();
        let side = DVec2::new(-t.y, t.x) * (p.thickness * 0.5);
        let piece = |from: f64, to: f64| {
            let (a, b) = (p.a + t * from, p.a + t * to);
            vec![a - side, b - side, b + side, a + side]
        };
        let mut from = 0.0;
        for &(d0, d1) in &p.doors {
            if d0 > from + 0.01 {
                out.push((piece(from, d0), y0, y1, false));
            }
            out.push((piece(d0, d1), y0 + interior::DOORWAY_HEAD, y1, false));
            from = d1;
        }
        if p.length() > from + 0.01 {
            out.push((piece(from, p.length()), y0, y1, false));
        }
    }
    if let Some(stair) = inner.stair {
        for (k, &r) in inner.risers.iter().enumerate() {
            if k + 1 >= s.storeys.len() || r == 0 {
                break;
            }
            let base = floor_level(s, k);
            let rise = (s.storeys[k + 1].level - base) / f64::from(r);
            for i in 0..r - 1 {
                let (v0, v1) = (f64::from(i) * cores::GOING, f64::from(i + 1) * cores::GOING);
                let (p0, p1) = (inner.climb * v0, inner.climb * v1);
                let tread = vec![stair[0] + p0, stair[1] + p0, stair[1] + p1, stair[0] + p1];
                let y1 = base + f64::from(i + 1) * rise;
                out.push((tread, (y1 - rise - 0.15).max(base), y1, true));
            }
        }
        // A guard round each upper floor's opening over the flight: a whitewashed half wall a
        // metre high along its long side away from the walls and across its head (over the
        // flight's foot), 10 cm thick and 5 cm back from the edge; the arrival end stays open.
        // Without it a walker stepped off the landing into the stairwell (seen 2026-10-16).
        let across = stair[1] - stair[0];
        let width = across.length();
        let a = across / width.max(1e-6);
        let c = inner.climb.normalize_or_zero();
        // The long side away from the walls: the one whose middle lies farther from their lines.
        let from_walls = |p: DVec2| {
            s.walls
                .iter()
                .map(|w| geom::distance_to_segment(p, w.a, w.b))
                .fold(f64::MAX, f64::min)
        };
        for (k, &r) in inner.risers.iter().enumerate() {
            if k + 1 >= s.storeys.len() || r < 2 {
                break;
            }
            let run = f64::from(r - 1) * cores::GOING;
            let open_far =
                from_walls(stair[1] + c * run * 0.5) > from_walls(stair[0] + c * run * 0.5);
            // A box in the opening's frame: across from `a0` to `a1`, along the climb from `c0`
            // to `c1`, from `stair[0]`.
            let piece = |a0: f64, a1: f64, c0: f64, c1: f64| {
                vec![
                    stair[0] + a * a0 + c * c0,
                    stair[0] + a * a1 + c * c0,
                    stair[0] + a * a1 + c * c1,
                    stair[0] + a * a0 + c * c1,
                ]
            };
            let y0 = floor_level(s, k + 1);
            let (side, head) = if open_far {
                (
                    piece(width + 0.05, width + 0.15, -0.05, run),
                    piece(0.0, width + 0.15, -0.15, -0.05),
                )
            } else {
                (
                    piece(-0.15, -0.05, -0.05, run),
                    piece(-0.15, width, -0.15, -0.05),
                )
            };
            for ring in [side, head] {
                // Wound anticlockwise, as the other solids.
                let ring = if geom::signed_area(&ring) < 0.0 {
                    ring.into_iter().rev().collect()
                } else {
                    ring
                };
                out.push((ring, y0, y0 + 1.0, false));
            }
        }
    }
    out
}

/// The upper floors of a house whose insides are drawn, as slabs round the treads of the flight
/// below (`without`), each a ring and its underside and top.
fn interior_floors(
    m: &Mass,
    s: &Structure,
    inner: &interior::Interior,
) -> Vec<(Vec<DVec2>, f64, f64)> {
    let Some(ring) = inner_ring(m, s) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (k, st) in s.storeys.iter().enumerate().skip(1) {
        let hole = inner.stair.zip(inner.risers.get(k - 1)).map(|(stair, &r)| {
            let up = inner.climb * (f64::from(r.saturating_sub(1)) * cores::GOING);
            [stair[0], stair[1], stair[1] + up, stair[0] + up]
        });
        let pieces = hole.map_or_else(|| vec![ring.clone()], |h| without(&ring, &h));
        for p in pieces {
            out.push((p, st.level - st.slab, st.level));
        }
    }
    out
}

/// The insides of the houses near the view as the walker's convex pieces, the world's metres:
/// their partitions, their flights' steps, their upper floors and their furniture.
fn interior_pieces(
    size: f64,
    masses: &[Mass],
    structures: &[Structure],
    near: &[usize],
) -> Vec<Vec<Vec3>> {
    let mut out = Vec::new();
    let solid = |ring: &[DVec2], y0: f64, y1: f64| -> Vec<Vec3> {
        ring.iter()
            .flat_map(|&p| [world(size, p, y0), world(size, p, y1)])
            .collect()
    };
    for &i in near {
        let (m, s) = (&masses[i], &structures[i]);
        let Some(inner) = interior::house(m, s) else {
            continue;
        };
        for (ring, y0, y1, _) in interior_solids(s, &inner, m.height) {
            out.push(solid(&ring, y0, y1));
        }
        for (ring, y0, y1) in interior_floors(m, s, &inner) {
            out.push(solid(&ring, y0, y1));
        }
        // The furniture but the chairs, which a walker brushes past.
        for p in forge_procgen::city::walk::furnished(m, s, &inner) {
            if p.piece != furnish::Piece::Chair {
                let y0 = floor_level(s, p.storey);
                out.push(solid(&p.outline(), y0, y0 + p.piece.height()));
            }
        }
    }
    out
}

/// A box over `ring` (anticlockwise, the plan's metres) from `y0` to `y1` into `mesh`, its points
/// placed by `at`: its sides, its top and its underside (a lintel's shows).
fn solid(
    mesh: &mut TriMesh,
    at: &dyn Fn(DVec2, f64) -> Vec3,
    ring: &[DVec2],
    y0: f64,
    y1: f64,
    section: u8,
) {
    if y1 <= y0 + 1e-3 || ring.len() < 3 {
        return;
    }
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let t = (b - a).normalize_or_zero();
        let out = Vec3::new(t.y as f32, 0.0, -t.x as f32);
        let (l, h) = ((a.distance(b) / 3.0) as f32, ((y1 - y0) / 3.0) as f32);
        triangle_uv(
            mesh,
            [at(a, y0), at(b, y0), at(b, y1)],
            [[0.0, 0.0], [l, 0.0], [l, -h]],
            out,
            section,
        );
        triangle_uv(
            mesh,
            [at(a, y0), at(b, y1), at(a, y1)],
            [[0.0, 0.0], [l, -h], [0.0, -h]],
            out,
            section,
        );
    }
    for t in geom::triangulate(ring) {
        triangle_uv(
            mesh,
            t.map(|i| at(ring[i], y1)),
            [[0.0; 2]; 3],
            Vec3::Y,
            section,
        );
        let [p, q, r] = t.map(|i| at(ring[i], y0));
        triangle_uv(mesh, [p, r, q], [[0.0; 2]; 3], -Vec3::Y, section);
    }
}

/// `ring` (convex, anticlockwise) without the rectangle `hole` (anticlockwise): the parts beyond
/// its back edge, then beside it, then before it, each clipped clear of the ones before.
fn without(ring: &[DVec2], hole: &[DVec2; 4]) -> Vec<Vec<DVec2>> {
    let mut out = Vec::new();
    let mut rest = ring.to_vec();
    // The hole's edges, back first: the part of `rest` outside each is kept, the inside goes on.
    for e in [2, 1, 3, 0] {
        let (a, b) = (hole[e], hole[(e + 1) % 4]);
        let outside = geom::clean(&geom::clip_half_plane(&rest, b, a - b), 1e-6);
        if outside.len() >= 3 && geom::signed_area(&outside).abs() > 0.01 {
            out.push(outside);
        }
        rest = geom::clean(&geom::clip_half_plane(&rest, a, b - a), 1e-6);
        if rest.len() < 3 {
            break;
        }
    }
    out
}

/// The cores' tops (D-060, step 3). Over each core that climbs to a garden or a terrace, a head
/// house the stair and the lift arrive in, its door out on to the roof from the hall; over each
/// lift a little higher, its overrun. On the other flat roofs the lift's overrun alone; under a
/// pitched roof nothing shows. The owner: a garden on the roof means a way up.
fn head_houses(tiles: &mut Tiles, masses: &[Mass], structures: &[Structure]) {
    for (m, s) in masses.iter().zip(structures) {
        if m.roof == Roof::Tiled {
            continue;
        }
        let section = KIT_PLASTER + tint_of(m.lot);
        for c in &s.cores {
            let top = m.height;
            if c.roof {
                prism(tiles, &c.outline, top, top + HEAD_HOUSE, section);
                // The door on the hall's open side: its back, or where the lift stands behind
                // the hall (a compact core), the side past the hall's end.
                let hall = (c.hall[0] + c.hall[1] + c.hall[2] + c.hall[3]) * 0.25;
                let compact = c.hall[0].distance(c.hall[3]) < c.deep - 0.1;
                let edges = if compact { [1, 3] } else { [2, 2] };
                let k = *edges
                    .iter()
                    .min_by(|&&a, &&b| {
                        let d = |k: usize| {
                            geom::distance_to_segment(hall, c.outline[k], c.outline[(k + 1) % 4])
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .unwrap_or(&2);
                let (a, b) = (c.outline[k], c.outline[(k + 1) % 4]);
                let along = geom::closest_on_segment(hall, a, b) * a.distance(b);
                roof_door(tiles, m, a, b, along, top);
            }
            if let Some(shaft) = c.lift {
                let base = if c.roof { top + HEAD_HOUSE } else { top };
                let middle = (shaft[0] + shaft[1] + shaft[2] + shaft[3]) * 0.25;
                let grown = shaft.map(|p| p + (p - middle).normalize_or_zero() * 0.28);
                prism(tiles, &grown, base, base + OVERRUN, section);
            }
        }
    }
}

/// A box over the outline `ring` (counter-clockwise) from `y0` to `y1`: its walls in `section`,
/// its flat top a roof terrace's.
fn prism(tiles: &mut Tiles, ring: &[DVec2], y0: f64, y1: f64, section: u8) {
    prism_topped(tiles, ring, y0, y1, section, ROOF_TERRACE);
}

/// A box over the outline `ring` (counter-clockwise) from `y0` to `y1`: its walls in `section`,
/// its flat top in `top`.
fn prism_topped(tiles: &mut Tiles, ring: &[DVec2], y0: f64, y1: f64, section: u8, top: u8) {
    let size = tiles.size;
    let tile = tiles.at(ring[0]);
    let centre = tile.centre;
    let at = |q: DVec2, h: f64| world(size, q, h) - centre;
    let n = ring.len();
    for i in 0..n {
        let (a, b) = (ring[i], ring[(i + 1) % n]);
        let t = (b - a).normalize_or_zero();
        let out = Vec3::new(t.y as f32, 0.0, -t.x as f32);
        let (l, h) = ((a.distance(b) / 3.0) as f32, ((y1 - y0) / 3.0) as f32);
        triangle_uv(
            &mut tile.masses,
            [at(a, y0), at(b, y0), at(b, y1)],
            [[0.0, 0.0], [l, 0.0], [l, -h]],
            out,
            section,
        );
        triangle_uv(
            &mut tile.masses,
            [at(a, y0), at(b, y1), at(a, y1)],
            [[0.0, 0.0], [l, -h], [0.0, -h]],
            out,
            section,
        );
    }
    for t in geom::triangulate(ring) {
        let p = t.map(|i| at(ring[i], y1));
        triangle_uv(&mut tile.masses, p, [[0.0; 2]; 3], Vec3::Y, top);
    }
}

/// A ribbon along `points` rising and falling with `heights`, a quad per segment.
fn ribbon_sloped(tiles: &mut Tiles, points: &[DVec2], heights: &[f64], width: f64, section: u8) {
    let half = width * 0.5;
    let size = tiles.size;
    for k in 0..points.len().min(heights.len()).saturating_sub(1) {
        let (a, b) = (points[k], points[k + 1]);
        let t = (b - a).normalize_or_zero();
        if t == DVec2::ZERO {
            continue;
        }
        let n = DVec2::new(-t.y, t.x) * half;
        let tile = tiles.at(a);
        let centre = tile.centre;
        let at = |p: DVec2, h: f64| world(size, p, h) - centre;
        let (ha, hb) = (heights[k], heights[k + 1]);
        let quad = [at(a + n, ha), at(b + n, hb), at(b - n, hb), at(a - n, ha)];
        triangle(
            &mut tile.ground,
            [quad[0], quad[1], quad[2]],
            Vec3::Y,
            section,
        );
        triangle(
            &mut tile.ground,
            [quad[0], quad[2], quad[3]],
            Vec3::Y,
            section,
        );
    }
}

/// The tram depots (`city::tram_depot`): each link's rails from the line's near track on the
/// carriageway up its verge's ramp to the walking level and across the footway, the yard's
/// tracks, the maintenance hall over the deepest tracks' far ends (open towards the yard, the
/// trams' way in, solar panels on its roof), and a low wall along the yard's front either side
/// of the link. The halls' and the walls' pieces for the walker.
fn tram_depots(tiles: &mut Tiles, plan: &CityPlan) -> Vec<Vec<Vec3>> {
    let size = tiles.size;
    // A rail's top over the ground under it.
    let top = 0.06;
    let mut pieces = Vec::new();
    // A slab along `a b`, `deep` metres towards `inside`, from `y0` to `y1`: drawn, and kept for
    // the walker.
    let mut slab = |tiles: &mut Tiles,
                    a: DVec2,
                    b: DVec2,
                    inside: DVec2,
                    deep: f64,
                    y0: f64,
                    y1: f64,
                    section: u8| {
        let t = (b - a).normalize_or_zero();
        let mut n = DVec2::new(-t.y, t.x);
        if n.dot(inside - a) < 0.0 {
            n = -n;
        }
        let mut ring = vec![a, b, b + n * deep, a + n * deep];
        if geom::signed_area(&ring) < 0.0 {
            ring.reverse();
        }
        prism(tiles, &ring, y0, y1, section);
        pieces.push(
            ring.iter()
                .flat_map(|&p| [world(size, p, y0), world(size, p, y1)])
                .collect::<Vec<Vec3>>(),
        );
    };
    for d in &plan.tram_depots {
        let link = d.link();
        for rail in [-0.72, 0.72] {
            let points = offset_line(&link, rail);
            let heights: Vec<f64> = points
                .iter()
                .map(|&p| ROAD_LEVEL + top + (WALK_LEVEL - ROAD_LEVEL) * d.level_at(p))
                .collect();
            ribbon_sloped(tiles, &points, &heights, 0.25, TRACK);
        }
        for track in d.tracks() {
            for rail in [-0.72, 0.72] {
                ribbon(
                    tiles,
                    &offset_line(&track, rail),
                    0.25,
                    WALK_LEVEL + top,
                    TRACK,
                );
            }
        }
        // The hall: its walls but the one facing the yard, a lintel over that one's opening,
        // its roof.
        let (_, high, _) = forge_procgen::city::tram_depot::HALL;
        let hall = d.hall();
        let middle = geom::centroid(&hall);
        let front = (0..4)
            .min_by(|&i, &j| {
                let x = |k: usize| ((hall[k] + hall[(k + 1) % 4]) * 0.5 - d.origin).dot(d.along);
                x(i).total_cmp(&x(j))
            })
            .unwrap_or(0);
        let (y0, y1) = (WALK_LEVEL, WALK_LEVEL + high);
        for k in 0..4 {
            let (a, b) = (hall[k], hall[(k + 1) % 4]);
            let from = if k == front { y1 - 3.5 } else { y0 };
            slab(tiles, a, b, middle, 0.3, from, y1, KIT_PLASTER);
        }
        prism_topped(tiles, &hall, y1, y1 + 0.3, KIT_PLASTER, ROOF_SOLAR);
        // The front's low wall either side of the link.
        let inside = d.at(d.length * 0.5, 10.0);
        for (a, b) in d.front() {
            slab(
                tiles,
                a,
                b,
                inside,
                0.3,
                WALK_LEVEL,
                WALK_LEVEL + 1.2,
                KIT_STONE,
            );
        }
    }
    pieces
}

/// The village's garden walls (`city::walls`, #266): stone from the walking level to each
/// wall's height on its ground plan, its top stone too; drawn, and kept for the walker.
fn garden_walls(tiles: &mut Tiles, plan: &CityPlan) -> Vec<Vec<Vec3>> {
    let size = tiles.size;
    let mut pieces = Vec::new();
    for w in &plan.walls {
        let ring = w.ring();
        if geom::signed_area(&ring) <= 0.0 {
            continue;
        }
        let (y0, y1) = (WALK_LEVEL, WALK_LEVEL + w.height);
        prism_topped(tiles, &ring, y0, y1, KIT_STONE, KIT_STONE);
        pieces.push(
            ring.iter()
                .flat_map(|&p| [world(size, p, y0), world(size, p, y1)])
                .collect(),
        );
    }
    pieces
}

/// The gardens' planters, hedges and beds as the walker's convex pieces, the world's metres.
fn garden_pieces(size: f64, gardens: &[gardens::Piece]) -> Vec<Vec<Vec3>> {
    gardens
        .iter()
        .filter(|g| g.kind.standing())
        .map(|g| {
            g.outline
                .iter()
                .flat_map(|&p| {
                    [
                        world(size, p, WALK_LEVEL),
                        world(size, p, WALK_LEVEL + g.height),
                    ]
                })
                .collect()
        })
        .collect()
}

/// The kit's door in the wall `a b` (counter-clockwise round its box) at `along`, on the roof at
/// `level`: out on to the garden or the terrace.
fn roof_door(tiles: &mut Tiles, m: &Mass, a: DVec2, b: DVec2, along: f64, level: f64) {
    let (w, h) = (1.0_f32, 2.2_f32);
    let Some(mesh) = modules::module("door", w, h) else {
        return;
    };
    let size = tiles.size;
    let t = (b - a).normalize_or_zero();
    let x = Vec3::new(t.x as f32, 0.0, t.y as f32);
    let z = Vec3::new(t.y as f32, 0.0, -t.x as f32);
    let y = Vec3::Y;
    let origin = a + t * (along - f64::from(w) * 0.5);
    let tint = tint_of(m.lot);
    let tile = tiles.at(origin);
    let at = world(size, origin, level) - tile.centre;
    let dst = &mut tile.masses;
    let base = dst.positions.len() as u32;
    for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
        let (p, n) = (Vec3::from(*p), Vec3::from(*n));
        dst.positions
            .push((at + x * p.x + y * p.y + z * p.z).to_array());
        dst.normals.push((x * n.x + y * n.y + z * n.z).to_array());
        dst.uvs.push([0.0; 2]);
    }
    dst.indices.extend(mesh.indices.iter().map(|i| base + i));
    dst.sections
        .extend((0..mesh.triangle_count()).map(|t| match mesh.section(t) {
            modules::PLASTER => KIT_PLASTER + tint,
            modules::PARTY => KIT_PARTY,
            _ => KIT_STONE,
        }));
}

/// A bridge's parapet: how high over its pavement and how thick, metres.
const PARAPET: (f64, f64) = (1.0, 0.35);

/// The parapets: wherever the street ground's own edge borders the water as drawn (a river's
/// strip or the smooth shore's open water), each segment of that edge with its inner face's line
/// [`PARAPET`]`.1` in, and the walking level they stand on (D-061). They follow a bridge's deck to
/// the banks exactly, and a quay street's edge over the water too. Run along the bridges' centre
/// lines' sides, they went on past a bridge's end into the junction at its head, a wedge on the
/// quay (the owner); cut by the site's cells, they first ran over the land and stopped short.
fn parapet_lines(plan: &CityPlan, site: &Site, shore: &Shore) -> Vec<Parapet> {
    let rivers: Vec<(DVec2, DVec2, f64)> = site
        .rivers
        .iter()
        .flat_map(|r| {
            r.centreline
                .windows(2)
                .map(move |w| (w[0], w[1], r.width * 0.5))
        })
        .collect();
    // The water as drawn: the smooth shore's or a river's strip, which no block covers now (a park
    // lay over a river's strip by the mouth, and a wall stood between it and an avenue, the owner).
    let wet = |p: DVec2| {
        shore.wetness(p) >= 0.5
            || rivers
                .iter()
                .any(|&(a, b, h)| geom::distance_to_segment(p, a, b) < h)
    };
    let mut segments = Vec::new();
    for t in &plan.ground.tiles {
        for &[a, b] in &t.outline {
            let d = b - a;
            let length = d.length();
            if length < 1e-3 {
                continue;
            }
            // The street lies on the segment's left; the water, if any, on its right.
            let inward = DVec2::new(-d.y, d.x) / length;
            if !wet((a + b) * 0.5 - inward * 0.6) {
                continue;
            }
            segments.push((a, b, inward));
        }
    }
    // Each segment's inner line mitred against its neighbour's at a shared end, so a corner
    // closes: each a box to its own end, they left a notch where the pavement wraps round the
    // water, open ends showing (the owner), and overlapped where the water wraps round it.
    let key = |p: DVec2| ((p.x * 100.0).round() as i64, (p.y * 100.0).round() as i64);
    let mut starts = std::collections::HashMap::new();
    let mut ends = std::collections::HashMap::new();
    for (i, &(a, b, _)) in segments.iter().enumerate() {
        starts.insert(key(a), i);
        ends.insert(key(b), i);
    }
    let inner = PARAPET.1;
    // Where the inner lines of `i` and `j` cross by their shared end `p`: `None` where they run on
    // (the plain ends meet) or meet too sharply.
    let mitre = |i: usize, j: usize, p: DVec2| -> Option<DVec2> {
        let (ai, bi, ni) = segments[i];
        let (aj, bj, nj) = segments[j];
        let (di, dj) = ((bi - ai).normalize(), (bj - aj).normalize());
        let turn = di.perp_dot(dj);
        if turn.abs() < 1e-3 {
            return None;
        }
        let (pi, pj) = (ai + ni * inner, aj + nj * inner);
        let m = pi + di * ((pj - pi).perp_dot(dj) / turn);
        (m.distance(p) < 4.0 * inner).then_some(m)
    };
    let mut out = Vec::new();
    for (i, &(a, b, n)) in segments.iter().enumerate() {
        let before = ends.get(&key(a)).copied().filter(|&j| j != i);
        let after = starts.get(&key(b)).copied().filter(|&j| j != i);
        let from = before.and_then(|j| mitre(j, i, a)).unwrap_or(a + n * inner);
        let to = after.and_then(|j| mitre(i, j, b)).unwrap_or(b + n * inner);
        out.push(Parapet {
            outer: [a, b],
            inner: [from, to],
            base: WALK_LEVEL,
            free: [before.is_none(), after.is_none()],
        });
    }
    out
}

/// A parapet's segment: its outer line over the water, its inner line over the pavement, the
/// walking level it stands on, and whether each end is free (no parapet goes on from it).
struct Parapet {
    outer: [DVec2; 2],
    inner: [DVec2; 2],
    base: f64,
    free: [bool; 2],
}

/// The bridges' stone parapets along both edges, a metre over the pavement, and the deck's side
/// below them down to the water: the pavements stopped at a bare edge over the river (the
/// owner: the bridges' pavements looked broken).
fn bridges(tiles: &mut Tiles, plan: &CityPlan, site: &Site, shore: &Shore) {
    let size = tiles.size;
    for Parapet {
        outer,
        inner,
        base,
        free,
    } in parapet_lines(plan, site, shore)
    {
        let top = base + PARAPET.0;
        strip(tiles, &outer, &inner, top, QUAY);
        // A face from `a` to `b` facing `away`, from `bottom` to the top.
        let mut face = |a: DVec2, b: DVec2, away: DVec2, bottom: f64| {
            let away = away.normalize_or_zero();
            let normal = Vec3::new(away.x as f32, 0.0, away.y as f32);
            let tile = tiles.at(a);
            let centre = tile.centre;
            let p = |q: DVec2, h: f64| world(size, q, h) - centre;
            triangle(
                &mut tile.ground,
                [p(a, bottom), p(b, bottom), p(b, top)],
                normal,
                QUAY,
            );
            triangle(
                &mut tile.ground,
                [p(a, bottom), p(b, top), p(a, top)],
                normal,
                QUAY,
            );
        };
        // Out over the water, and in over the pavement: facing away from the other line.
        face(outer[0], outer[1], outer[0] - inner[0], WATER_LEVEL);
        face(inner[0], inner[1], inner[0] - outer[0], base);
        // A free end closed: an open end showed the wall hollow.
        let along = outer[1] - outer[0];
        if free[0] {
            face(inner[0], outer[0], -along, WATER_LEVEL);
        }
        if free[1] {
            face(outer[1], inner[1], along, WATER_LEVEL);
        }
    }
}

/// The bridges' parapets as the walker's convex pieces, a segment each.
fn parapet_pieces(size: f64, plan: &CityPlan, site: &Site, shore: &Shore) -> Vec<Vec<Vec3>> {
    parapet_lines(plan, site, shore)
        .into_iter()
        .map(|w| {
            [w.outer[0], w.outer[1], w.inner[1], w.inner[0]]
                .into_iter()
                .flat_map(|q| [world(size, q, 0.0), world(size, q, w.base + PARAPET.0)])
                .collect()
        })
        .collect()
}

/// The buildings' walls as the walker's convex pieces, the world's metres (D-060): each street,
/// courtyard and party wall a slab of its thickness (0.2 m at least) inwards from its outer face,
/// from the ground to 0.9 m over its roof; where a passage runs through, the wall either side of
/// it and over its head, and the passage's two side walls.
fn wall_pieces(size: f64, masses: &[Mass], structures: &[Structure]) -> Vec<Vec<Vec3>> {
    let mut out = Vec::new();
    // A slab along `a b`, `deep` metres to its left (inwards), from `y0` to `y1`.
    let slab = |a: DVec2, b: DVec2, deep: f64, y0: f64, y1: f64| {
        let t = (b - a).normalize_or_zero();
        let inward = DVec2::new(-t.y, t.x) * deep;
        [a, b, b + inward, a + inward]
            .into_iter()
            .flat_map(|p| [world(size, p, y0), world(size, p, y1)])
            .collect::<Vec<Vec3>>()
    };
    for (s, m) in structures.iter().zip(masses) {
        let top = m.height + 0.9;
        // A house whose insides are drawn is open at its ground doors: the walker goes in.
        let entered = m.rooms.is_some() && interior::house(m, s).is_some();
        for (w, wall) in s.walls.iter().enumerate() {
            if wall.role == WallRole::Inner || wall.length() < 0.05 {
                continue;
            }
            let deep = wall.thickness.max(0.2);
            let t = (wall.b - wall.a).normalize_or_zero();
            let at = |along: f64| wall.a + t * along;
            let mut cuts: Vec<(f64, f64, f64)> = s
                .openings
                .iter()
                .filter(|o| {
                    o.wall == w
                        && (o.kind == OpeningKind::Passage
                            || (entered && o.kind == OpeningKind::Door && o.storey == 0))
                })
                .map(|o| (o.span().0, o.span().1, o.head))
                .collect();
            cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
            let mut from = 0.0;
            for &(lo, hi, head) in &cuts {
                if lo > from + 0.01 {
                    out.push(slab(at(from), at(lo), deep, 0.0, top));
                }
                out.push(slab(at(lo), at(hi), deep, head, top));
                from = hi;
            }
            if wall.length() > from + 0.01 {
                out.push(slab(at(from), at(wall.length()), deep, 0.0, top));
            }
        }
        // Each passage's sides, from its street end to its court end, 0.2 m thick outwards.
        let ends: Vec<(DVec2, DVec2, f64)> = s
            .openings
            .iter()
            .filter(|o| o.kind == OpeningKind::Passage)
            .map(|o| {
                let w = &s.walls[o.wall];
                let t = (w.b - w.a).normalize_or_zero();
                (w.a + t * o.span().0, w.a + t * o.span().1, o.head)
            })
            .collect();
        for pair in ends.as_chunks::<2>().0 {
            let ((p0, q0, head), (p1, q1, _)) = (pair[0], pair[1]);
            let (p1, q1) = if p0.distance(p1) + q0.distance(q1) <= p0.distance(q1) + q0.distance(p1)
            {
                (p1, q1)
            } else {
                (q1, p1)
            };
            // Each side's slab goes away from the passage's middle.
            let middle = (p0 + q0 + p1 + q1) * 0.25;
            for (a, b) in [(p0, p1), (q0, q1)] {
                let t = (b - a).normalize_or_zero();
                let left = DVec2::new(-t.y, t.x);
                let (a, b) = if left.dot(middle - a) > 0.0 {
                    (b, a)
                } else {
                    (a, b)
                };
                out.push(slab(a, b, 0.2, 0.0, head));
            }
        }
    }
    out
}

/// The passages into the courts through a mass (D-060): each a tunnel from its street opening
/// to its court opening, two side walls and a ceiling in stone, the paving under it.
fn passages(tiles: &mut Tiles, s: &Structure) {
    let size = tiles.size;
    let ends: Vec<(DVec2, DVec2, f64)> = s
        .openings
        .iter()
        .filter(|o| o.kind == OpeningKind::Passage)
        .map(|o| {
            let w = &s.walls[o.wall];
            let t = (w.b - w.a).normalize_or_zero();
            (w.a + t * o.span().0, w.a + t * o.span().1, o.head)
        })
        .collect();
    // Street and court ends in pairs, as `structure::passages` adds them.
    for pair in ends.as_chunks::<2>().0 {
        let ((p0, q0, head), (p1, q1, _)) = (pair[0], pair[1]);
        // Match the sides: the court end's points nearest the street end's.
        let (p1, q1) = if p0.distance(p1) + q0.distance(q1) <= p0.distance(q1) + q0.distance(p1) {
            (p1, q1)
        } else {
            (q1, p1)
        };
        let tile = tiles.at(p0);
        let centre = tile.centre;
        let at = |q: DVec2, h: f64| world(size, q, h) - centre;
        let middle = (p0 + q0 + p1 + q1) * 0.25;
        let inward = |a: DVec2, b: DVec2| {
            let v = (middle - (a + b) * 0.5).normalize_or_zero();
            Vec3::new(v.x as f32, 0.0, v.y as f32)
        };
        let faces = [
            // The side walls, facing into the passage.
            (
                [at(p0, 0.0), at(p1, 0.0), at(p1, head), at(p0, head)],
                inward(p0, p1),
            ),
            (
                [at(q0, 0.0), at(q1, 0.0), at(q1, head), at(q0, head)],
                inward(q0, q1),
            ),
            // The ceiling, facing down; the paving, facing up.
            (
                [at(p0, head), at(p1, head), at(q1, head), at(q0, head)],
                -Vec3::Y,
            ),
            (
                [
                    at(p0, PASSAGE_FLOOR),
                    at(p1, PASSAGE_FLOOR),
                    at(q1, PASSAGE_FLOOR),
                    at(q0, PASSAGE_FLOOR),
                ],
                Vec3::Y,
            ),
        ];
        for (quad, normal) in faces {
            for tri in [[0, 1, 2], [0, 2, 3]] {
                // With texture coordinates, as everything in the masses' mesh: without, the
                // vertices after them took the wrong ones.
                triangle_uv(
                    &mut tile.masses,
                    tri.map(|k| quad[k]),
                    [[0.0; 2]; 3],
                    normal,
                    KIT_STONE,
                );
            }
        }
    }
}

/// The water's level, metres: the sea's, the lagoons' and the rivers', one level where they
/// meet (the river lay over the ground and the harbour 0.6 m under it, a step between the two
/// waters: the owner's second walk).
const WATER_LEVEL: f64 = -0.6;

/// The site's shore for the ground: the rivers' beds widened by three quarters of a cell each
/// side, drawn as water under the rivers' smooth strips and their promenades, so no land juts
/// into a river.
fn ground_shore(site: &Site) -> Shore {
    Shore::new(site, 0.75 * site.water.spacing)
}

/// The site's land and water: the grid's whole squares in runs along each row (a run is cut at
/// tile edges), and the squares the shore crosses cut along it ([`Shore`]), not the cells'
/// staircase (the owner's plan, 2026-10-11). Along the shore a stone quay wall from the ground
/// down past the water: the drop showed nothing, the water's edge a dark band.
fn ground_and_water(tiles: &mut Tiles, shore: &Shore) {
    let s = shore.spacing();
    let n = shore.size() - 1;
    let per_tile = (TILE / s).round().max(1.0) as u32;
    let size = tiles.size;
    for y in 0..n {
        let mut x = 0;
        while x < n {
            let Some(wet) = shore.whole(x, y) else {
                let square = shore.square(x, y);
                for piece in &square.dry {
                    tiles.flat(piece, 0.0, GROUND, false);
                }
                for piece in &square.wet {
                    tiles.flat(piece, WATER_LEVEL, WATER, false);
                }
                for &([a, b], out) in &square.shore {
                    let along = DVec2::new(-out.y, out.x);
                    let (a, b) = if (b - a).dot(along) < 0.0 {
                        (b, a)
                    } else {
                        (a, b)
                    };
                    let tile = tiles.at((a + b) * 0.5);
                    let centre = tile.centre;
                    let p = |q: DVec2, h: f64| world(size, q, h) - centre;
                    let normal = Vec3::new(out.x as f32, 0.0, out.y as f32);
                    triangle(
                        &mut tile.ground,
                        [p(a, 0.0), p(b, 0.0), p(b, -1.2)],
                        normal,
                        QUAY,
                    );
                    triangle(
                        &mut tile.ground,
                        [p(a, 0.0), p(b, -1.2), p(a, -1.2)],
                        normal,
                        QUAY,
                    );
                }
                x += 1;
                continue;
            };
            let start = x;
            let tile_end = ((x / per_tile) + 1) * per_tile;
            while x < n && x < tile_end && shore.whole(x, y) == Some(wet) {
                x += 1;
            }
            let (x0, x1) = (f64::from(start) * s, f64::from(x) * s);
            let (y0, y1) = (f64::from(y) * s, f64::from(y + 1) * s);
            let quad = [
                DVec2::new(x0, y0),
                DVec2::new(x1, y0),
                DVec2::new(x1, y1),
                DVec2::new(x0, y1),
            ];
            if wet {
                tiles.flat(&quad, WATER_LEVEL, WATER, false);
            } else {
                tiles.flat(&quad, 0.0, GROUND, false);
            }
        }
    }
}

/// A polyline moved `d` metres to its left.
fn offset_line(points: &[DVec2], d: f64) -> Vec<DVec2> {
    let n = points.len();
    (0..n)
        .map(|i| {
            let prev = if i > 0 {
                points[i] - points[i - 1]
            } else {
                points[1] - points[0]
            };
            let next = if i + 1 < n {
                points[i + 1] - points[i]
            } else {
                prev
            };
            let t = (prev.normalize_or_zero() + next.normalize_or_zero()).normalize_or_zero();
            points[i] + DVec2::new(-t.y, t.x) * d
        })
        .collect()
}

/// A flat ribbon `width` metres wide along a polyline, a quad per segment.
fn ribbon(tiles: &mut Tiles, points: &[DVec2], width: f64, height: f64, section: u8) {
    let half = width * 0.5;
    for w in points.windows(2) {
        let t = (w[1] - w[0]).normalize_or_zero();
        if t == DVec2::ZERO {
            continue;
        }
        let n = DVec2::new(-t.y, t.x) * half;
        // A little past each end, so the joints close.
        let (a, b) = (w[0] - t * half.min(2.0), w[1] + t * half.min(2.0));
        tiles.flat(&[a + n, b + n, b - n, a - n], height, section, false);
    }
}

/// A bay's width on the facades, metres: one repeat of the facade texture across.
const BAY: f64 = 3.0;
/// The far facades' texture coordinates in their levels of detail, metres of error per unit
/// (`CookOptions::uv_weight`): a unit is the texture's [`textures::FACADE_CELLS`] bays, 12 m.
const FACADE_UV_WEIGHT: f32 = textures::FACADE_CELLS as f32 * BAY as f32;
/// A window bay's plain wall, left of its shutter, as a share of the bay across
/// (`textures::facade`: the shutters from 0.17).
const BAY_PLAIN: (f32, f32) = (0.02, 0.13);

/// Adds a triangle with texture coordinates, turned so its face looks along `normal`.
fn triangle_uv(
    mesh: &mut TriMesh,
    mut p: [Vec3; 3],
    mut uv: [[f32; 2]; 3],
    normal: Vec3,
    section: u8,
) {
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(normal) < 0.0 {
        p.swap(1, 2);
        uv.swap(1, 2);
    }
    let base = mesh.positions.len() as u32;
    for (q, t) in p.into_iter().zip(uv) {
        mesh.positions.push(q.to_array());
        mesh.normals.push(normal.to_array());
        mesh.uvs.push(t);
    }
    mesh.indices.extend([base, base + 1, base + 2]);
    mesh.sections.push(section);
}

/// A wall quad from `y0` to `y1`, its texture repeating every `repeat` bays across and every
/// `repeat` floors of `floor` metres up from `y0`. Whole bays only, centred on the wall; the
/// wall left at either end (all of a wall shorter than a bay) is plain, read from the bay's
/// plain strip `plain` (a share of a bay across): mapped on, the end's part of a bay put a
/// window's edge and its shutter at the building's corner (the owner).
#[allow(clippy::too_many_arguments)]
fn wall_band(
    mesh: &mut TriMesh,
    (a, b): (Vec3, Vec3),
    length: f64,
    (y0, y1): (f32, f32),
    floor: f32,
    repeat: f32,
    plain: (f32, f32),
    out: Vec3,
    section: u8,
) {
    let len = length as f32;
    let bays = (length / BAY).floor() as f32;
    let margin = (len - bays * BAY as f32) * 0.5;
    let flat = (plain.0 / repeat, plain.1 / repeat);
    let pieces = if bays < 1.0 {
        vec![(0.0, len, flat)]
    } else {
        vec![
            (0.0, margin, flat),
            (margin, len - margin, (0.0, bays / repeat)),
            (len - margin, len, flat),
        ]
    };
    let v = |y: f32| -(y - y0) / floor / repeat;
    for (s0, s1, (u0, u1)) in pieces {
        if s1 - s0 < 1e-3 {
            continue;
        }
        let (pa, pb) = (a + (b - a) * (s0 / len), a + (b - a) * (s1 / len));
        let (pa0, pb0) = (pa + Vec3::Y * y0, pb + Vec3::Y * y0);
        let (pa1, pb1) = (pa + Vec3::Y * y1, pb + Vec3::Y * y1);
        triangle_uv(
            mesh,
            [pa0, pb0, pb1],
            [[u0, v(y0)], [u1, v(y0)], [u1, v(y1)]],
            out,
            section,
        );
        triangle_uv(
            mesh,
            [pa0, pb1, pa1],
            [[u0, v(y0)], [u1, v(y1)], [u0, v(y1)]],
            out,
            section,
        );
    }
}

/// A mass: its walls (a shop band on the ground floor where the street is lively, window bays
/// above, one of four tints by its lot) and its roof, `edges` saying what each wall looks onto.
fn mass(tiles: &mut Tiles, m: &Mass, edges: &[Option<FaceKind>], tall: &[bool]) {
    let size = tiles.size;
    let tile = tiles.at(m.footprint[0]);
    let centre = tile.centre;
    let mesh = &mut tile.masses;
    let n = m.footprint.len();
    let tint = tint_of(m.lot);
    // Shops along avenues and in pedestrian lanes.
    let shops = m.lively();
    let ground = m.ground() as f32;
    let top = m.height as f32;
    for i in 0..n {
        let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
        let t = (b - a).normalize_or_zero();
        if t == DVec2::ZERO {
            continue;
        }
        // Outward: right of the edge for a positive outline.
        let out = Vec3::new(t.y as f32, 0.0, -t.x as f32);
        let ends = (world(size, a, 0.0) - centre, world(size, b, 0.0) - centre);
        let length = a.distance(b);
        // A wall onto the neighbour's lot is blank plaster where it shows over a lower
        // neighbour: windows and shutters showed on the building's side (the owner).
        if !matches!(edges.get(i), Some(Some(FaceKind::Street | FaceKind::Court))) {
            let plain = (0.0, 1.0);
            wall_band(
                mesh,
                ends,
                length,
                (0.0, top),
                top,
                1.0,
                plain,
                out,
                KIT_PLASTER + tint,
            );
            continue;
        }
        let cells = textures::FACADE_CELLS as f32;
        // The ground storey is one cell, stretched to its height: at 3.2 m a cell, its 4.2 m showed
        // a row of window stubs under the first floor, cut where the upper band starts again
        // (the owner's "bugged windows" on the first floor).
        // The plain strips: a shopfront's stone pier, a window bay's wall left of its shutter.
        let (lower, floor, repeat, plain) = if shops {
            (SHOP, ground, 1.0, (0.01, 0.06))
        } else {
            (WALL + tint, ground, cells, BAY_PLAIN)
        };
        let base = (0.0, ground.min(top));
        wall_band(mesh, ends, length, base, floor, repeat, plain, out, lower);
        if top > ground {
            let band = (ground, top);
            wall_band(
                mesh,
                ends,
                length,
                band,
                m.upper() as f32,
                cells,
                BAY_PLAIN,
                out,
                WALL + tint,
            );
        }
    }
    roof(tiles, m, edges, tall, KIT_PLASTER + tint);
}

/// A mass's roof: green, solar, a terrace, or pitched and tiled, its gables (where the
/// neighbour across a party wall is as `tall`) in `gable`.
fn roof(tiles: &mut Tiles, m: &Mass, edges: &[Option<FaceKind>], tall: &[bool], gable: u8) {
    let size = tiles.size;
    let tile = tiles.at(m.footprint[0]);
    let centre = tile.centre;
    let section = match m.roof {
        Roof::Green => ROOF_GREEN,
        Roof::Solar => ROOF_SOLAR,
        Roof::Terrace => ROOF_TERRACE,
        Roof::Tiled => {
            if pitched(&mut tile.masses, m, edges, tall, gable, size, centre) {
                return;
            }
            ROOF_TERRACE
        }
    };
    for t in geom::triangulate(&m.footprint) {
        let p = t.map(|i| world(size, m.footprint[i], m.height) - centre);
        triangle_uv(&mut tile.masses, p, [[0.0; 2]; 3], Vec3::Y, section);
    }
}

/// A pitched roof over a mass at about 29°: the sloping edges ([`pitches`]) moved in as far as
/// they go to the ridge (the straight skeleton's roof for a convex footprint, nearly) or
/// [`RUN_MAX`], the gable ends standing up from their walls in `gable`. False where the
/// footprint will not take one; it then gets a flat roof.
fn pitched(
    mesh: &mut TriMesh,
    m: &Mass,
    edges: &[Option<FaceKind>],
    tall: &[bool],
    gable: u8,
    size: f64,
    centre: Vec3,
) -> bool {
    // The footprint, or one corner less where a short side would turn over (`roofs::fit`).
    let Some((outline, w)) = roofs::fit(&m.footprint, edges, tall) else {
        return false;
    };
    let fp = &outline;
    let n = fp.len();
    let fits = |d: f64| ridge(fp, &w, d);
    if fits(0.5).is_none() {
        return false;
    }
    // The run to the ridge, or the longest, the top then a terrace.
    let (d, top) = if fits(RUN_MAX).is_some() {
        (RUN_MAX, ROOF_TERRACE)
    } else {
        let (mut lo, mut hi) = (0.5, RUN_MAX);
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if fits(mid).is_some() {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        (lo * 0.97, ROOF_TILE)
    };
    let Some(ridge) = fits(d) else {
        return false;
    };
    let rise = d * 0.55;
    // The tiles run along the eaves: u along the eave, v up the slope, in the texture's
    // repeats (projected from the world, their rows ran askew to the building, the owner).
    let slope = ((d * d + rise * rise).sqrt() / TILE_REPEAT) as f32;
    let at = |p: DVec2, h: f64| world(size, p, h) - centre;
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (at(fp[i], m.height), at(fp[j], m.height));
        let (c, e) = (at(ridge[j], m.height + rise), at(ridge[i], m.height + rise));
        let t = (fp[j] - fp[i]).normalize_or_zero();
        let (normal, section) = if w[i] == 0.0 {
            // A gable: the wall carried up, facing out.
            (Vec3::new(t.y as f32, 0.0, -t.x as f32), gable)
        } else {
            let normal = (b - a).cross(c - a).normalize_or_zero();
            (normal * normal.y.signum(), ROOF_TILE)
        };
        let u = |p: DVec2| ((p - fp[i]).dot(t) / TILE_REPEAT) as f32;
        let (ub, uc, ue) = (u(fp[j]), u(ridge[j]), u(ridge[i]));
        triangle_uv(
            mesh,
            [a, b, c],
            [[0.0, 0.0], [ub, 0.0], [uc, slope]],
            normal,
            section,
        );
        triangle_uv(
            mesh,
            [a, c, e],
            [[0.0, 0.0], [uc, slope], [ue, slope]],
            normal,
            section,
        );
    }
    for t in geom::triangulate(&ridge) {
        let p = t.map(|i| at(ridge[i], m.height + rise));
        let uv = t.map(|i| {
            let q = ridge[i] / TILE_REPEAT;
            [q.x as f32, q.y as f32]
        });
        triangle_uv(mesh, p, uv, Vec3::Y, top);
    }
    true
}

/// Where a load's time goes, step by step: the milliseconds since the last mark, logged once.
struct Laps {
    last: std::time::Instant,
    steps: Vec<(&'static str, u128)>,
}

impl Laps {
    fn new() -> Self {
        Self {
            last: std::time::Instant::now(),
            steps: Vec::new(),
        }
    }

    /// Ends the step `name`.
    fn mark(&mut self, name: &'static str) {
        let now = std::time::Instant::now();
        self.steps.push((name, (now - self.last).as_millis()));
        self.last = now;
    }

    /// The steps, `name ms` each.
    fn report(&self) -> String {
        self.steps
            .iter()
            .map(|(name, ms)| format!("{name} {ms}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// The machine's threads.
fn cores() -> usize {
    std::thread::available_parallelism().map_or(1, std::num::NonZero::get)
}

/// `f(i)` for every `i` below `n`, on `threads` threads, the results in order: each depends on
/// its `i` alone, never on the threads. The items are taken in `order` (a permutation of `0..n`,
/// the longest first so no thread is left with a long one at the end), or `0..n`.
fn par_map<T: Send>(
    n: usize,
    threads: usize,
    order: Option<&[usize]>,
    f: impl Fn(usize) -> T + Sync,
) -> Vec<T> {
    let threads = threads.min(n);
    if threads <= 1 {
        return (0..n).map(f).collect();
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done: Vec<Vec<(usize, T)>> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut made = Vec::new();
                    loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        if k >= n {
                            return made;
                        }
                        let i = order.map_or(k, |o| o[k]);
                        made.push((i, f(i)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().unwrap_or_else(|p| std::panic::resume_unwind(p)))
            .collect()
    });
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    for (i, t) in done.into_iter().flatten() {
        out[i] = Some(t);
    }
    out.into_iter()
        .map(|t| t.expect("every item made"))
        .collect()
}

/// A digest of everything a mesh holds: with its cook's options and the cooking code's digest,
/// the key its cooked copy is kept under in the derived cache (#274).
fn mesh_digest(mesh: &TriMesh) -> u64 {
    let mut hash = xxhash_rust::xxh3::Xxh3::new();
    for part in [
        bytemuck::cast_slice::<[f32; 3], u8>(&mesh.positions),
        bytemuck::cast_slice(&mesh.normals),
        bytemuck::cast_slice(&mesh.indices),
        &mesh.sections,
        bytemuck::cast_slice(&mesh.uvs),
    ] {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.digest()
}

/// A tile's meshes that hold triangles: its ground, masses and railings, each with its kind (0,
/// 1, 2: its material row in the build), the weight its texture coordinates take in the LOD
/// and its name. Cooked with the normals weighed in the LOD too: a pavement's kerb and back
/// faces folded into its top at the coarser levels, long tilted triangles where the paving
/// streaked (the owner saw two patterns blend on the pavements). The far facades' texture
/// coordinates too: across their jumps the windows smeared into streaks.
fn tile_meshes(tile: &Tile) -> impl Iterator<Item = (&TriMesh, usize, f32, &'static str)> {
    [
        (&tile.ground, 0, 0.0, "ground"),
        (&tile.masses, 1, FACADE_UV_WEIGHT, "masses"),
        (&tile.railings, 2, 0.0, "railings"),
    ]
    .into_iter()
    .filter(|(mesh, ..)| !mesh.indices.is_empty())
}

/// The forest's species in the order their first plants come (D-063): each grows
/// `FLORA_INDIVIDUALS` individuals, at a quarter, half and all of its maturity, three of each.
fn planted(
    flora: &[(Vec3, f32, f32, f32, u16, u8)],
    species: &[forge_flora::species::Species],
) -> Vec<u16> {
    let mut planted: Vec<u16> = Vec::new();
    for &(.., sp, _) in flora {
        if species.get(usize::from(sp)).is_some() && !planted.contains(&sp) {
            planted.push(sp);
        }
    }
    planted
}

impl PlanScenario {
    /// Every mesh of the scene in the order the build adds them (the tiles', the plants', the
    /// kit's modules, the tiles' insides), each plant's with its height and crown. Each is kept
    /// in the derived cache (D-053) under its name (its place in the scene, so a changed tile
    /// replaces its old copy) and a key: its content's digest, its cook's options and the
    /// cooking code's digest (`cook_code`).
    fn cook_meshes(
        &self,
        species: &[forge_flora::species::Species],
    ) -> Vec<(MeshletMesh, f32, f32)> {
        enum Cook<'a> {
            Mesh(&'a TriMesh, CookOptions, String),
            Plant(u16, u8),
        }
        let scene = &self.scene;
        let jobs: Vec<Cook> = self
            .tiles
            .iter()
            .enumerate()
            .flat_map(|(t, tile)| {
                tile_meshes(tile).map(move |(mesh, _, uv_weight, kind)| {
                    let options = CookOptions {
                        normal_weight: 2.0,
                        uv_weight,
                    };
                    Cook::Mesh(mesh, options, format!("{scene}-tile{t}-{kind}"))
                })
            })
            .chain(
                planted(&self.flora, species)
                    .into_iter()
                    .flat_map(|sp| (0..FLORA_INDIVIDUALS).map(move |k| Cook::Plant(sp, k))),
            )
            .chain(self.kit_meshes.iter().enumerate().map(|(m, mesh)| {
                Cook::Mesh(mesh, CookOptions::default(), format!("{scene}-kit{m}"))
            }))
            .chain(
                self.tiles
                    .iter()
                    .enumerate()
                    .filter(|(_, tile)| !tile.insides.indices.is_empty())
                    .map(|(t, tile)| {
                        let name = format!("{scene}-tile{t}-insides");
                        Cook::Mesh(&tile.insides, CookOptions::default(), name)
                    }),
            )
            .collect();
        // What each cooks, on all the threads: a plant grown and meshed first (a quarter of its
        // time, its cook the rest), with its height and crown.
        struct Made<'a> {
            mesh: Cow<'a, TriMesh>,
            options: CookOptions,
            name: String,
            key: String,
            height: f32,
            crown: f32,
        }
        let code = forge_geom::cache::cook_code();
        let made: Vec<Made> = par_map(jobs.len(), cores(), None, |i| {
            let (mesh, options, name, height, crown) = match &jobs[i] {
                Cook::Mesh(mesh, options, name) => {
                    (Cow::Borrowed(*mesh), *options, name.clone(), 0.0, 0.0)
                }
                &Cook::Plant(sp, k) => {
                    let s = &species[usize::from(sp)];
                    let shrub = s.habit == forge_flora::species::Habit::Shrub;
                    let broad = s.foliage == forge_flora::species::Foliage::Broad;
                    let conditions = flora_conditions(s, sp, k);
                    let graph = forge_flora::grow::grow(s, &conditions);
                    let mesh = grown_mesh(
                        &graph,
                        shrub,
                        broad,
                        conditions.pruning.lift as f32,
                        leaf_spray(&s.id),
                    );
                    let (height, crown) = (graph.height().max(0.1), graph.crown_width().max(0.1));
                    let name = format!("plant-{}-{k}", s.id);
                    let options = CookOptions::default();
                    (Cow::Owned(mesh), options, name, height, crown)
                }
            };
            let key = format!(
                "plan mesh {:016x} {options:?} code {code:016x}",
                mesh_digest(&mesh)
            );
            Made {
                mesh,
                options,
                name,
                key,
                height,
                crown,
            }
        });
        drop(jobs);
        let dir = forge_core::derived::cache_dir(forge_core::derived::CacheKind::Meshes);
        let take = |m: &Made| {
            let (done, stored) =
                forge_geom::cache::cook_cached_with(&dir, &m.name, &m.key, true, || {
                    MeshletMesh::build_with(&m.mesh, m.options)
                });
            if let Err(error) = stored {
                tracing::warn!(mesh = %m.name, %error, "cooked mesh not cached");
            }
            done.mesh
        };
        let (hits, misses): (Vec<usize>, Vec<usize>) = (0..made.len()).partition(|&i| {
            let key = forge_geom::cache::key(&made[i].key);
            forge_geom::cache::path(&dir, &made[i].name, key).exists()
        });
        // The cached ones read on all the threads; the others cooked a third of the threads at a
        // time, the largest first: a large mesh's DAG spreads its groups over threads of its own,
        // and the city's 783 meshes cooked fastest 4–6 at a time (2.8–3.2 s; 9.9 s one by one,
        // 7.6 s sixteen at a time).
        let loaded = par_map(hits.len(), cores(), None, |j| take(&made[hits[j]]));
        let order = largest_first(misses.iter().map(|&i| made[i].mesh.indices.len()));
        let cooked_now = par_map(misses.len(), (cores() / 3).max(2), Some(&order), |j| {
            take(&made[misses[j]])
        });
        tracing::info!(
            from_cache = hits.len(),
            cooked = misses.len(),
            "the plan's meshes"
        );
        let mut taken: Vec<Option<MeshletMesh>> = (0..made.len()).map(|_| None).collect();
        for (&i, mesh) in hits.iter().zip(loaded).chain(misses.iter().zip(cooked_now)) {
            taken[i] = Some(mesh);
        }
        taken
            .into_iter()
            .zip(&made)
            .map(|(mesh, m)| (mesh.expect("every mesh taken"), m.height, m.crown))
            .collect()
    }
}

/// The order of `sizes`' indices, the largest first.
fn largest_first(sizes: impl IntoIterator<Item = usize>) -> Vec<usize> {
    let sizes: Vec<usize> = sizes.into_iter().collect();
    let mut order: Vec<usize> = (0..sizes.len()).collect();
    order.sort_by_key(|&i| std::cmp::Reverse(sizes[i]));
    order
}

impl Scenario for PlanScenario {
    fn props(&self) -> Vec<PropSpec> {
        Vec::new()
    }

    fn look(&self) -> Look {
        Look {
            // An afternoon sun from the south-west.
            sun: Some(Vec3::new(-0.45, 0.55, 0.7)),
            // The village's maps (a plan with a forest) are walked into: a hall lit through its
            // door took its light from probes whose eight rotations (TAA's jitter cycle, #48)
            // each found the opening with a different few of their 128 rays, and the light on
            // the stairs swung at 7.5 Hz (the owner saw the stairs shimmer, 2026-10-16: 3–7 % of
            // a still view's pixels between frames 10 apart; #272). With 256 rays, as the models
            // lab's rooms (#171), 0.004–0.015 % once settled; 0.8 ms more along the road
            // (3.73 → 4.51 ms). And 2 m apart, in six cascades (the coarsest 64 m apart as the
            // city's): 4 m apart, coarser than the rooms (3 × 4 m) and their walls, the sky's
            // light leaked in blue, the shadows sat in odd places and swung with the jitter (the
            // owner, 2026-10-17: "the shadow ... where shimmering a lot and are very strangely
            // placed"; 15 000 pixels of a hall changed between frames 10 apart, 106 at 2 m;
            // about 0.3 ms more). At 2 m the 7.5 Hz shimmer of #272 is gone at 128 rays as at 256
            // (the loading session's measures, #274: the stairs 3–15 px against 4–12, the landing
            // 204–253 against 205–262, the room 5–27 against 4–28; here 75 and 52 against 91 and
            // 56), and the probes' rays cost 1.1–1.3 ms rather than 1.9–2.1 (the frame 4.50 →
            // 3.70 ms): 128.
            probes: (!self.flora.is_empty()).then_some((128, 2.0, 6)),
            // And metered: at the street's fixed exposure (EV 14) a hall lit through its door
            // and the ground under the holm oaks were black (the owner, 2026-10-16: "insides are
            // way too dark", "light below trees is too dark"); the light was there, the eye
            // adapts to it (the hall metered at EV 9.4, the wood at 11.0, a garden at 13.4).
            metered: !self.flora.is_empty(),
            ..Look::default()
        }
    }

    fn liquid(&self) -> Option<city_blocks::scenario::LiquidSetup> {
        None
    }

    fn camera(&self) -> FlyCamera {
        self.camera.clone()
    }

    fn prepare(&mut self) {
        let start = std::time::Instant::now();
        self.cooked = Some(self.cook_meshes(&forge_flora::species::mediterranean()));
        tracing::info!(
            ms = start.elapsed().as_millis(),
            "the plan's meshes, on the loading thread"
        );
    }

    fn build(
        mut self: Box<Self>,
        ctx: &Setup,
        args: &Args,
        _cooked: Cooked,
    ) -> Result<(MeshletScene, Box<dyn Running>)> {
        let start = std::time::Instant::now();
        let mut laps = Laps::new();
        let mut materials = CityMaterials::new(&ctx.device)?;
        laps.mark("materials");
        // The ground's textures, projected from above, their repeats dividing a tile's 250 m so
        // the tiles meet without a seam: pale gravel on the quays, grass, limestone paving,
        // asphalt.
        let mut ground = |[albedo, normal]: [textures::TextureData; 2]| {
            Ok::<_, anyhow::Error>((
                materials.textures.add(&albedo)?,
                materials.textures.add(&normal)?,
            ))
        };
        let grass = ground(textures::grass(32, 512))?;
        let paving = ground(textures::paving(33, 1024))?;
        let asphalt = ground(textures::asphalt(34, 1024))?;
        laps.mark("ground textures");
        let textured = |layer: RenderLayer,
                        (albedo, normal): (TextureId, TextureId),
                        scale: f32,
                        stochastic: bool| RenderLayer {
            albedo_texture: Some(albedo),
            normal_texture: Some(normal),
            texture_scale: scale,
            normal_strength: 1.0,
            hex_tiling: stochastic,
            // One pattern and one noise across the tiles' meshes: from each tile's origin, a
            // polygon kept in the next tile's mesh showed as a band of another brightness.
            world_projected: true,
            ..layer
        };
        materials.add_rows(
            "plan-ground",
            vec![
                (
                    "plan ground".into(),
                    // The open land: a drier, olive grass than the parks', the garrigue's (it was
                    // gravel, the owner took it for paving).
                    textured(colour([2.4, 2.5, 1.7], 6.0, 0.02), grass, 5.0, true),
                ),
                (
                    "plan garden".into(),
                    textured(colour([3.0, 3.1, 3.4], 6.0, 0.02), grass, 5.0, true),
                ),
                (
                    "plan park".into(),
                    textured(colour([2.6, 3.0, 3.2], 6.0, 0.02), grass, 5.0, true),
                ),
                (
                    "plan paving".into(),
                    textured(colour([1.0, 1.0, 1.0], 16.0, 0.04), paving, 2.5, false),
                ),
                (
                    "plan street".into(),
                    textured(colour([1.0, 1.0, 1.0], 14.0, 0.05), asphalt, 5.0, true),
                ),
                ("plan track".into(), colour([0.16, 0.16, 0.17], 14.0, 0.3)),
                ("plan water".into(), colour([0.05, 0.12, 0.15], 900.0, 0.9)),
                (
                    "plan quay wall".into(),
                    colour([0.62, 0.58, 0.51], 10.0, 0.03),
                ),
                (
                    "plan marking".into(),
                    colour([0.80, 0.80, 0.77], 30.0, 0.05),
                ),
                // Two centimetres of still water over dark granite: the sky and the
                // buildings in it, as in Bordeaux's and Nantes' mirrors.
                (
                    "plan water mirror".into(),
                    colour([0.03, 0.035, 0.04], 2000.0, 0.9),
                ),
            ],
        );
        // The facades: a window bay and a shopfront, read by the walls' coordinates.
        // Their windows lit in the share the kit's residents lit theirs, or the homes' share of
        // the hour (`lit_share`) without a kit: the far city goes to bed with the near one.
        // Dimming a fixed two in five instead left the far city's windows lit at 3:00, the
        // exposure raised to meet them.
        let far_share = self.far_lit.unwrap_or_else(|| {
            args.time_of_day
                .map_or(0.4, |t| lit_share(hour_of_day(t), false))
        });
        let [facade_albedo, facade_normal, facade_lights] =
            textures::facade(21, 1024, far_share as f32);
        let [shop_albedo, shop_normal] = textures::shopfront(22, 512);
        let shop_lights = textures::shopfront_lights(512);
        laps.mark("facade textures");
        let facade = (
            materials.textures.add(&facade_albedo)?,
            materials.textures.add(&facade_normal)?,
            materials.textures.add(&facade_lights)?,
        );
        let shop = (
            materials.textures.add(&shop_albedo)?,
            materials.textures.add(&shop_normal)?,
            materials.textures.add(&shop_lights)?,
        );
        // Lit windows of about 5 cd/m² on average (behind curtains, seen from the street: 12 blew
        // them out white, the owner found the lights too strong and all alike). The rows' emissive
        // is in cd/m² (`RenderLayer::emissive_absolute`, #254): the renderer converts it by the
        // frame's reference illuminance, so it holds through a running day (`--day`) as at one
        // time of day. It was scaled once by the reference of `--time-of-day`'s hour.
        let glow = 5.0;
        // The textured boxes' shopfronts glow at 15 % of it: at the horizon facade stacks on
        // facade with no dark roof or street between, and the far city read as a band of white
        // (the owner, 2026-10-11). A stopgap until a proxy per building (#38) draws the far city
        // as the kit would.
        let far_glow = glow * 0.15;
        // Their homes' windows near the kit's lit rooms: the lamp's share of a lit row's luminance
        // the room gets (ROOM_LAMP in meshlet.slang, 25 %) over walls, floor and furniture about
        // 0.6 as bright as white would be 15 %, but at the horizon the facades stack with no
        // street between and at 15 % the far city read as a band of white again. 6 % keeps it
        // dotted (2.5 % went with the rooms' 4 %).
        let far_rooms = glow * 0.06;
        let mapped = |rgb: [f32; 3],
                      (albedo, normal, lights): (TextureId, TextureId, TextureId),
                      glow: f32| {
            RenderLayer {
                albedo_texture: Some(albedo),
                normal_texture: Some(normal),
                normal_strength: 1.0,
                emissive: [glow; 3],
                emissive_absolute: true,
                uv: Some(UvMapping {
                    emissive_texture: Some(lights),
                    ..UvMapping::default()
                }),
                ..colour(rgb, 12.0, 0.04)
            }
        };
        // The kit's render and stone, projected: concrete's relief, faint, under the tint (its
        // colour map's pits read as stains on a render).
        let [_, grain_normal] = textures::concrete(23, 512);
        let grain = materials.textures.add(&grain_normal)?;
        let render = |rgb: [f32; 3]| RenderLayer {
            normal_texture: Some(grain),
            texture_scale: 2.0,
            normal_strength: 0.25,
            hex_tiling: true,
            ..colour(rgb, 10.0, 0.03)
        };
        let stone = RenderLayer {
            texture_scale: 1.5,
            ..render([0.82, 0.78, 0.70])
        };
        // Terracotta tiles: the brick courses, tinted, read as rows of tiles from the street.
        let [tile_albedo, tile_normal] = textures::brick(35, 512);
        laps.mark("grain and tiles");
        // Read by the roofs' texture coordinates, which run along the eaves.
        let tiles_row = RenderLayer {
            albedo_texture: Some(materials.textures.add(&tile_albedo)?),
            normal_texture: Some(materials.textures.add(&tile_normal)?),
            texture_scale: TILE_REPEAT as f32,
            normal_strength: 0.8,
            uv: Some(UvMapping::default()),
            ..colour([1.75, 2.0, 2.1], 10.0, 0.04)
        };
        // The kit's railings' bars, one card each between the rails (D-013's far form): as
        // geometry they flickered past 18 m.
        materials.add_rows(
            "plan-grille",
            vec![(
                "plan grille".into(),
                RenderLayer {
                    grating: Some(modules::BAR_SHARE),
                    ..colour([0.05, 0.05, 0.05], 30.0, 0.15)
                },
            )],
        );
        materials.add_rows(
            "plan-mass",
            vec![
                // The render tints of the palette: sand, ochre, pale rose, off-white.
                (
                    "plan wall (sand)".into(),
                    mapped([0.94, 0.88, 0.76], facade, far_rooms),
                ),
                (
                    "plan wall (ochre)".into(),
                    mapped([0.92, 0.80, 0.62], facade, far_rooms),
                ),
                (
                    "plan wall (rose)".into(),
                    mapped([0.90, 0.76, 0.68], facade, far_rooms),
                ),
                (
                    "plan wall (white)".into(),
                    mapped([0.95, 0.93, 0.88], facade, far_rooms),
                ),
                (
                    "plan shopfront".into(),
                    mapped([1.0, 1.0, 1.0], shop, far_glow),
                ),
                (
                    "plan green roof".into(),
                    colour([0.24, 0.40, 0.17], 6.0, 0.02),
                ),
                (
                    "plan solar roof".into(),
                    colour([0.05, 0.07, 0.11], 300.0, 0.5),
                ),
                (
                    "plan terrace".into(),
                    colour([0.66, 0.62, 0.56], 14.0, 0.04),
                ),
                ("plan render (sand)".into(), render([0.94, 0.88, 0.76])),
                ("plan render (ochre)".into(), render([0.92, 0.80, 0.62])),
                ("plan render (rose)".into(), render([0.90, 0.76, 0.68])),
                ("plan render (white)".into(), render([0.95, 0.93, 0.88])),
                ("plan stone".into(), stone),
                ("plan party wall".into(), render([0.78, 0.72, 0.64])),
                ("plan tiled roof".into(), tiles_row),
                // The insides' walls and ceilings (`KIT_INSIDE`): the white render, smooth; a
                // step away indoors the facades' grain read as squiggles, and at a third of its
                // strength as rims round pebbles. A lime whitewash, nearly grey: a room lit by
                // its door bounces the light many times, and a cream (0.95, 0.93, 0.88) turned
                // ochre once the metered exposure showed it.
                (
                    "plan inside plaster".into(),
                    colour([0.92, 0.915, 0.90], 10.0, 0.03),
                ),
                // The insides' floors (`KIT_INSIDE_FLOOR`): a pale grey limestone, smooth.
                (
                    "plan inside floor".into(),
                    colour([0.74, 0.73, 0.70], 10.0, 0.03),
                ),
            ],
        );
        // The kit's groups: shutters and fascias in sage, blue-grey, blue and warm white, with
        // their awnings and frames; each with dark glass and with lit glass.
        let palettes = [
            ([0.36, 0.46, 0.34], [0.60, 0.28, 0.18], [0.86, 0.85, 0.80]),
            ([0.40, 0.48, 0.55], [0.14, 0.30, 0.20], [0.86, 0.85, 0.80]),
            ([0.22, 0.38, 0.52], [0.14, 0.19, 0.32], [0.20, 0.20, 0.21]),
            ([0.76, 0.73, 0.66], [0.78, 0.70, 0.55], [0.20, 0.20, 0.21]),
        ];
        // Each lit building's light: its luminance (cd/m²) and colour. People choose dim, diffuse
        // light of different colours (the owner): a dim warm lamp, a brighter warm room, a
        // neutral one, a television's blue. Until the residents light their own windows by
        // what they do (D-060's step 3).
        const LIT: [(f32, [f32; 3]); 4] = [
            (3.0, [1.0, 0.68, 0.40]),
            (8.0, [1.0, 0.80, 0.55]),
            (6.0, [1.0, 0.92, 0.80]),
            (2.5, [0.70, 0.82, 1.0]),
        ];
        let glass = |light: Option<(f32, [f32; 3])>| RenderLayer {
            emissive: light.map_or([0.0; 3], |(l, c)| c.map(|v| v * l)),
            emissive_absolute: true,
            reflectance: 0.06,
            // Each window its own mix of dark glass and the net curtains behind it, from bare
            // to drawn (the owner, 2026-10-11: the windows read dark; in France many homes keep
            // sheers so the street cannot see in). A share per style comes with #259.
            color_b: [0.52, 0.50, 0.46],
            ..colour([0.03, 0.035, 0.04], 3000.0, 0.6)
        };
        for (k, &group) in KIT_GROUPS.iter().enumerate() {
            let (shutter, fabric, frame) = palettes[k / 2];
            materials.add_rows(
                group,
                vec![
                    (format!("{group} render"), render([0.94, 0.88, 0.76])),
                    (format!("{group} stone"), stone),
                    (format!("{group} frame"), colour(frame, 40.0, 0.1)),
                    // A room behind each window (D-060's step 3): the windows read dark.
                    (
                        format!("{group} glass"),
                        RenderLayer {
                            interior: true,
                            ..glass((k % 2 == 1).then_some(LIT[k / 2]))
                        },
                    ),
                    (format!("{group} shutter"), colour(shutter, 14.0, 0.05)),
                    (
                        format!("{group} iron"),
                        colour([0.05, 0.05, 0.05], 30.0, 0.15),
                    ),
                    (format!("{group} awning"), colour(fabric, 6.0, 0.02)),
                    (
                        format!("{group} door"),
                        colour([0.28, 0.17, 0.10], 30.0, 0.1),
                    ),
                    (format!("{group} party"), render([0.78, 0.72, 0.64])),
                    // The railings' cards are drawn by the tiles' own row (`take_cards`).
                    (
                        format!("{group} grille"),
                        colour([0.05, 0.05, 0.05], 30.0, 0.15),
                    ),
                    // A shop behind the shopfronts' glass: no curtains, nor a living room
                    // with a sofa as the homes had.
                    (
                        format!("{group} shop glass"),
                        RenderLayer {
                            interior: true,
                            shop: true,
                            ..glass((k % 2 == 1).then_some(LIT[k / 2]))
                        },
                    ),
                ],
            );
        }
        // The market halls' group: pale glazing that catches the sky, iron painted dark green
        // (as Baltard's halls were), striped canvas over the stalls.
        materials.add_rows(
            "plan-kit-hall",
            vec![
                ("plan hall render".into(), render([0.94, 0.88, 0.76])),
                ("plan hall stone".into(), stone),
                (
                    "plan hall frame".into(),
                    colour([0.10, 0.20, 0.16], 40.0, 0.15),
                ),
                (
                    "plan hall glass".into(),
                    RenderLayer {
                        reflectance: 0.08,
                        ..colour([0.42, 0.50, 0.50], 2000.0, 0.7)
                    },
                ),
                (
                    "plan hall paint".into(),
                    colour([0.10, 0.20, 0.16], 30.0, 0.1),
                ),
                (
                    "plan hall iron".into(),
                    colour([0.10, 0.20, 0.16], 40.0, 0.15),
                ),
                (
                    "plan hall canvas".into(),
                    colour([0.78, 0.30, 0.22], 6.0, 0.02),
                ),
                (
                    "plan hall stalls".into(),
                    colour([0.40, 0.27, 0.16], 20.0, 0.05),
                ),
                ("plan hall party".into(), render([0.78, 0.72, 0.64])),
            ],
        );
        let mut kit_rows: Vec<_> = KIT_GROUPS.iter().map(|g| materials.of(g)).collect();
        kit_rows.push(materials.of("plan-kit-hall"));
        // The crane's: steel painted shipyard red, dark bogies and cables.
        materials.add_rows(
            "plan-kit-crane",
            vec![
                ("plan crane render".into(), render([0.94, 0.88, 0.76])),
                ("plan crane stone".into(), stone),
                (
                    "plan crane frame".into(),
                    colour([0.12, 0.12, 0.13], 40.0, 0.15),
                ),
                ("plan crane glass".into(), glass(None)),
                (
                    "plan crane paint".into(),
                    colour([0.55, 0.11, 0.07], 40.0, 0.2),
                ),
                (
                    "plan crane steel".into(),
                    colour([0.10, 0.10, 0.11], 40.0, 0.25),
                ),
                (
                    "plan crane canvas".into(),
                    colour([0.78, 0.30, 0.22], 6.0, 0.02),
                ),
                (
                    "plan crane wood".into(),
                    colour([0.40, 0.27, 0.16], 20.0, 0.05),
                ),
                ("plan crane party".into(), render([0.78, 0.72, 0.64])),
            ],
        );
        kit_rows.push(materials.of("plan-kit-crane"));
        // The fingerposts: their names' atlas on the blades' faces, a dark green pole, blue
        // edges.
        let names = materials.textures.add(&signs::atlas(&self.sign_names))?;
        materials.add_rows(
            "plan-kit-sign",
            vec![
                (
                    "plan sign face".into(),
                    RenderLayer {
                        albedo_texture: Some(names),
                        uv: Some(UvMapping::default()),
                        ..colour([1.0, 1.0, 1.0], 20.0, 0.05)
                    },
                ),
                (
                    "plan sign pole".into(),
                    colour([0.05, 0.09, 0.07], 40.0, 0.2),
                ),
                (
                    "plan sign edge".into(),
                    colour([0.03, 0.12, 0.33], 20.0, 0.05),
                ),
            ],
        );
        kit_rows.push(materials.of("plan-kit-sign"));
        // The trams' liveries: white bodies, dark glass, the line's colour as the stripe.
        let linear = |c: u8| (f32::from(c) / 255.0).powf(2.2);
        for (k, &group) in TRAM_GROUPS.iter().enumerate() {
            let stripe = self
                .trams
                .get(k)
                .map_or([0.6, 0.1, 0.1], |r| r.colour.map(linear));
            materials.add_rows(
                group,
                vec![
                    (
                        format!("{group} body"),
                        colour([0.86, 0.86, 0.84], 60.0, 0.25),
                    ),
                    (format!("{group} glass"), glass(None)),
                    (format!("{group} stripe"), colour(stripe, 60.0, 0.25)),
                    (
                        format!("{group} running gear"),
                        colour([0.07, 0.07, 0.08], 30.0, 0.1),
                    ),
                ],
            );
        }
        let tram_rows: Vec<_> = TRAM_GROUPS.iter().map(|g| materials.of(g)).collect();
        // The taxis: ivory bodies, dark glass, an amber band, dark wheels.
        materials.add_rows(
            "plan-taxi",
            vec![
                (
                    "plan taxi body".into(),
                    colour([0.90, 0.88, 0.82], 60.0, 0.3),
                ),
                ("plan taxi glass".into(), glass(None)),
                (
                    "plan taxi band".into(),
                    colour([0.85, 0.55, 0.10], 60.0, 0.3),
                ),
                (
                    "plan taxi wheels".into(),
                    colour([0.06, 0.06, 0.07], 30.0, 0.1),
                ),
            ],
        );
        let materials_taxi = materials.of("plan-taxi");
        // The boats: a white hull, dark glass, a teal band, the solar roof.
        materials.add_rows(
            "plan-boat",
            vec![
                (
                    "plan boat hull".into(),
                    colour([0.88, 0.88, 0.86], 80.0, 0.3),
                ),
                ("plan boat glass".into(), glass(None)),
                (
                    "plan boat band".into(),
                    colour([0.05, 0.42, 0.45], 60.0, 0.25),
                ),
                (
                    "plan boat roof".into(),
                    colour([0.04, 0.06, 0.10], 300.0, 0.5),
                ),
            ],
        );
        let materials_boat = materials.of("plan-boat");
        // The stops' screens: their atlas lit, amber on black, about 240 cd/m².
        let screen_atlas = materials.textures.add(&signs::screen_atlas())?;
        materials.add_rows(
            "plan-screen",
            vec![(
                "plan screen".into(),
                RenderLayer {
                    albedo_texture: Some(screen_atlas),
                    emissive: [glow * 20.0; 3],
                    emissive_absolute: true,
                    uv: Some(UvMapping {
                        emissive_texture: Some(screen_atlas),
                        ..UvMapping::default()
                    }),
                    ..colour([1.0, 1.0, 1.0], 20.0, 0.1)
                },
            )],
        );
        let materials_screen = materials.of("plan-screen");
        // The figures' clothes: skin, top, bottom, hair and shoes, eight ways.
        let people: [([f32; 3], [f32; 3], [f32; 3]); 8] = [
            ([0.62, 0.44, 0.34], [0.80, 0.80, 0.78], [0.10, 0.14, 0.26]),
            ([0.45, 0.30, 0.22], [0.55, 0.12, 0.10], [0.20, 0.20, 0.22]),
            ([0.70, 0.52, 0.42], [0.20, 0.35, 0.50], [0.55, 0.48, 0.36]),
            ([0.30, 0.20, 0.15], [0.90, 0.85, 0.70], [0.12, 0.12, 0.14]),
            ([0.66, 0.48, 0.38], [0.25, 0.42, 0.28], [0.15, 0.20, 0.35]),
            ([0.52, 0.36, 0.27], [0.75, 0.55, 0.20], [0.30, 0.25, 0.20]),
            ([0.72, 0.56, 0.46], [0.40, 0.25, 0.45], [0.85, 0.82, 0.76]),
            ([0.38, 0.26, 0.19], [0.15, 0.15, 0.17], [0.40, 0.42, 0.45]),
        ];
        for (k, &group) in PEOPLE_GROUPS.iter().enumerate() {
            let (skin, top, bottom) = people[k];
            materials.add_rows(
                group,
                vec![
                    (format!("{group} skin"), colour(skin, 20.0, 0.04)),
                    (format!("{group} top"), colour(top, 8.0, 0.02)),
                    (format!("{group} bottom"), colour(bottom, 8.0, 0.02)),
                    (
                        format!("{group} hair and shoes"),
                        colour([0.06, 0.05, 0.04], 20.0, 0.05),
                    ),
                ],
            );
        }
        let people_rows: Vec<_> = PEOPLE_GROUPS.iter().map(|g| materials.of(g)).collect();
        // Each tree its own mix of two tones (the rows' colour A and B): bark from grey-brown to
        // the planes' pale patches, leaves from deep to yellowish green.
        let tones = |a: [f32; 3], b: [f32; 3], power: f32| RenderLayer {
            color_b: b,
            ..colour(a, power, 0.02)
        };
        let trunk = tones([0.20, 0.15, 0.11], [0.42, 0.38, 0.30], 8.0);
        materials.add_rows(
            "plan-tree-a",
            vec![
                ("plan trunk".into(), trunk),
                (
                    "plan leaves".into(),
                    tones([0.08, 0.19, 0.06], [0.16, 0.27, 0.07], 6.0),
                ),
            ],
        );
        materials.add_rows(
            "plan-tree-b",
            vec![
                ("plan trunk (b)".into(), trunk),
                (
                    "plan leaves (b)".into(),
                    tones([0.12, 0.23, 0.07], [0.22, 0.30, 0.09], 6.0),
                ),
            ],
        );
        let tree_rows = [materials.of("plan-tree-a"), materials.of("plan-tree-b")];
        // The forest's species (D-063): each its bark, its crowns' cores and its leaf cards
        // (D-013), in two tones; the cards cut out of the leaf atlas.
        let species = forge_flora::species::mediterranean();
        let leaf_atlas = if self.flora.is_empty() {
            None
        } else {
            Some(materials.textures.add(&textures::leaves(41, 1024))?)
        };
        let flora_rows: Vec<_> = species
            .iter()
            .map(|s| {
                let (group, bark, leaves) = flora_tones(&s.id);
                let foliage = tones(leaves.0, leaves.1, 6.0);
                // The crowns' cores at three tenths of the leaves' colour: the shade inside a
                // crown (at half, once the metered exposure lit the wood, their sunlit facets
                // read paler than the cards round them).
                let shade = |c: [f32; 3]| c.map(|k| k * 0.3);
                materials.add_rows(
                    group,
                    vec![
                        (format!("{} bark", s.id), tones(bark.0, bark.1, 8.0)),
                        (
                            format!("{} leaves", s.id),
                            tones(shade(leaves.0), shade(leaves.1), 6.0),
                        ),
                        (
                            format!("{} leaf cards", s.id),
                            RenderLayer {
                                albedo_texture: leaf_atlas,
                                uv: Some(UvMapping {
                                    alpha_cutoff: Some(0.5),
                                    ..UvMapping::default()
                                }),
                                foliage: true,
                                ..foliage
                            },
                        ),
                    ],
                );
                materials.of(group)
            })
            .collect();
        // Lamp heads of about 1 000 cd/m²: shielded LEDs at 2 700 K behind a frosted diffuser (the
        // style bible's lighting). At 3 000 their halos read too strong (the owner).
        let head = 1000.0;
        materials.add_rows(
            "plan-lamp",
            vec![
                (
                    "plan lamp pole".into(),
                    colour([0.06, 0.09, 0.08], 40.0, 0.2),
                ),
                (
                    "plan lamp head".into(),
                    RenderLayer {
                        emissive: [head, head * 0.78, head * 0.55],
                        emissive_absolute: true,
                        ..colour([0.8, 0.8, 0.78], 40.0, 0.2)
                    },
                ),
            ],
        );
        let lamp_row = materials.of("plan-lamp");
        let ground_row = materials.of("plan-ground");
        let mass_row = materials.of("plan-mass");
        let grille_row = materials.of("plan-grille");
        laps.mark("rows and leaves");
        let mut builder = MeshletSceneBuilder::new();
        materials.apply(&mut builder, &[], &[]);
        laps.mark("materials applied");
        builder.set_ray_traced(!args.no_shadows);
        builder.set_origin(scene_origin(args));
        let rows = [ground_row, mass_row, grille_row];
        let planted = planted(&self.flora, &species);
        // The meshes the loading thread cooked or read from the cache (`prepare`), or now.
        let cooked = match self.cooked.take() {
            Some(cooked) => cooked,
            None => self.cook_meshes(&species),
        };
        let mut cooked = cooked.into_iter();
        laps.mark("cooked");
        let mut meshes = 0;
        for tile in &self.tiles {
            let at = Mat4::from_translation(tile.centre);
            for (_, kind, ..) in tile_meshes(tile) {
                let (mesh, _, _) = cooked.next().expect("a tile's mesh cooked");
                let id = builder.add_mesh(&mesh);
                builder.add_instance_with_material(id, at, rows[kind]);
                meshes += 1;
            }
        }
        laps.mark("tiles");
        // The forest by species (D-063): a few individuals of each grown once (`forge_flora`),
        // their meshes instanced, each plant scaled to its height and crown.
        if !self.flora.is_empty() {
            // Each species' individuals, grown at a quarter, half and all of its maturity, three
            // of each; a plant takes the age nearest its height, so a young street tree is a
            // young tree, not a mature one shrunk (a 10 m plane was a ball on a stem), and of
            // that age the individual its hash picks, its crown stretched by up to 8 % along its
            // turn.
            let mut models: std::collections::HashMap<u16, Vec<(forge_render::MeshId, f32, f32)>> =
                Default::default();
            for &sp in &planted {
                let grown = (0..FLORA_INDIVIDUALS)
                    .map(|_| {
                        let (mesh, height, crown) = cooked.next().expect("a plant cooked");
                        (builder.add_mesh(&mesh), height, crown)
                    })
                    .collect();
                models.insert(sp, grown);
            }
            for &(at, height, crown, turn, sp, pick) in &self.flora {
                let Some(grown) = models.get(&sp) else {
                    continue;
                };
                let ages = usize::from(FLORA_AGES);
                let Some(age) = (0..ages.min(grown.len())).min_by(|&a, &b| {
                    (grown[a].1 / height)
                        .ln()
                        .abs()
                        .total_cmp(&(grown[b].1 / height).ln().abs())
                }) else {
                    continue;
                };
                let seeds = grown.len().div_ceil(ages);
                let k = age + ages * (usize::from(pick) % seeds);
                let Some(&(id, h, w)) = grown.get(k).or(grown.get(age)) else {
                    continue;
                };
                let across = crown * 2.0 / w;
                let stretch = 0.92 + f32::from(pick >> 2) / 63.0 * 0.16;
                let transform = Mat4::from_scale_rotation_translation(
                    Vec3::new(across * stretch, height / h, across / stretch),
                    glam::Quat::from_rotation_y(turn),
                    at,
                );
                builder.add_instance_with_material(id, transform, flora_rows[usize::from(sp)]);
            }
        }
        laps.mark("flora");
        if !self.trees.is_empty() {
            let model = builder.add_mesh(&MeshletMesh::build(&tree_model()));
            for &(at, scale, turn, kind) in &self.trees {
                let transform = Mat4::from_scale_rotation_translation(
                    scale,
                    glam::Quat::from_rotation_y(turn),
                    at,
                );
                builder.add_instance_with_material(model, transform, tree_rows[kind]);
            }
        }
        if !self.lamps.is_empty() {
            let models = [false, true]
                .map(|double| builder.add_mesh(&MeshletMesh::build(&lamp_model(double))));
            for &(at, scale, turn, double) in &self.lamps {
                let transform = Mat4::from_scale_rotation_translation(
                    Vec3::splat(scale),
                    glam::Quat::from_rotation_y(turn),
                    at,
                );
                builder.add_instance_with_material(
                    models[usize::from(double)],
                    transform,
                    lamp_row,
                );
            }
        }
        laps.mark("trees and lamps");
        let kit_ids: Vec<_> = self
            .kit_meshes
            .iter()
            .map(|_| builder.add_mesh(&cooked.next().expect("a kit module cooked").0))
            .collect();
        for &(module, transform, group) in &self.kit_instances {
            builder.add_instance_with_material(
                kit_ids[module as usize],
                transform,
                kit_rows[group as usize],
            );
        }
        if let Some(report) = &self.kit_report {
            tracing::info!("{report}");
        }
        laps.mark("kit");
        // The trams: a cab and a passenger section's meshes a line, in its livery; their
        // instances are movers, the table's last.
        let mut tram_movers = Vec::new();
        for (i, run) in self.trams.iter().enumerate() {
            let row = tram_rows[i % tram_rows.len()];
            let cab = builder.add_mesh(&MeshletMesh::build(&vehicles::tram_section(true)));
            let middle = builder.add_mesh(&MeshletMesh::build(&vehicles::tram_section(false)));
            builder.set_mesh_material(cab, row);
            builder.set_mesh_material(middle, row);
            let n = (run.timetable.vehicles() + run.parked.len()) as u32;
            tram_movers.extend([(cab, 2 * n), (middle, (TRAM_SECTIONS as u32 - 2) * n)]);
        }
        // The crowd's figures, each palette in its poses (`crowd::POSES`): the left stride,
        // standing (the parks' and the platforms' people), riding, seated (the cafés'), the
        // right stride, and the half strides between; a mesh each.
        if self.crowd.is_some() {
            for (g, &row) in people_rows.iter().enumerate().take(crowd::GROUPS) {
                let height = 1.62 + 0.04 * (g % 5) as f32;
                for figure in [
                    forge_geom::figures::walker(height, 0.3),
                    forge_geom::figures::walker(height, 0.0),
                    forge_geom::figures::cyclist(height),
                    forge_geom::figures::sitter(height),
                    forge_geom::figures::walker(height, -0.3),
                    forge_geom::figures::walker(height, 0.15),
                    forge_geom::figures::walker(height, -0.15),
                ] {
                    let mesh = builder.add_mesh(&MeshletMesh::build(&figure));
                    builder.set_mesh_material(mesh, row);
                    tram_movers.push((mesh, crowd::PER_MESH as u32));
                }
            }
        }
        // The boats, one a crossing, in their livery.
        if !self.boats.is_empty() {
            let boat = builder.add_mesh(&MeshletMesh::build(&vehicles::shuttle_boat()));
            builder.set_mesh_material(boat, materials_boat);
            tram_movers.push((boat, self.boats.len() as u32));
        }
        // The stops' screens: a plate a row of their atlas, `SCREENS` of each, moved onto the
        // nearest totems each frame by what they show.
        if !self.trams.is_empty() {
            for row in 0..signs::ROWS {
                let plate = builder.add_mesh(&MeshletMesh::build(&signs::screen_plate(row)));
                builder.set_mesh_material(plate, materials_screen);
                tram_movers.push((plate, SCREENS as u32));
            }
        }
        // The taxis, last.
        if let Some(t) = &self.taxis {
            tracing::info!(
                taxis = t.len(),
                unserved = format!("{:.0} %", t.unserved * 100.0),
                taxi_share = format!("{:.0} %", t.taxi_share * 100.0),
                "the taxi fleet: the trips by tram or taxi the trams do not serve, the share of them that book a taxi"
            );
            // The cabs, then the shuttles: the fleet's last taxis.
            let cab = builder.add_mesh(&MeshletMesh::build(&vehicles::taxi_cab()));
            builder.set_mesh_material(cab, materials_taxi);
            tram_movers.push((cab, (t.len() - t.shuttles()) as u32));
            let shuttle = builder.add_mesh(&MeshletMesh::build(&vehicles::taxi_shuttle()));
            builder.set_mesh_material(shuttle, materials_taxi);
            tram_movers.push((shuttle, t.shuttles() as u32));
        }
        laps.mark("movers");
        // The kit buildings' insides, a mesh a tile cooked apart from the masses (#273), last
        // among the static instances so every other keeps its id (each tree's tone is hashed
        // from it); the movers follow.
        for tile in &self.tiles {
            if tile.insides.indices.is_empty() {
                continue;
            }
            let id = builder.add_mesh(&cooked.next().expect("a tile's insides cooked").0);
            builder.add_instance_with_material(id, Mat4::from_translation(tile.centre), mass_row);
        }
        debug_assert!(cooked.next().is_none(), "every cooked mesh added");
        if !tram_movers.is_empty() {
            builder.reserve_movers(&tram_movers);
        }
        laps.mark("insides");
        let mut scene = builder.build(&ctx.device)?;
        laps.mark("scene upload");
        scene.build_tlas(&ctx.device, &ctx.shaders)?;
        laps.mark("tlas");
        tracing::info!("the plan's scene's steps (ms): {}", laps.report());
        tracing::info!(
            meshes,
            trees = self.trees.len(),
            lamps = self.lamps.len(),
            trams = self
                .trams
                .iter()
                .map(|r| r.timetable.vehicles())
                .sum::<usize>(),
            triangles = self.triangles,
            ms = start.elapsed().as_millis(),
            "the plan's scene"
        );
        if self.trams.len() > 1 {
            tracing::info!(
                pairs = tram_overlaps(&self.trams),
                "trams of two lines within 30 m on one track, over a game hour in 5 s steps"
            );
        }
        tracing::info!("{}", self.passage_note);
        // Where to look at a stop's screen: on its platform, 1.2 m in front of the first.
        if let Some((at, _, _)) = self
            .trams
            .first()
            .and_then(|r| r.platforms(self.size).first().copied())
        {
            let screen = at.transform_point3(SCREEN_ON_PLATFORM);
            let eye = screen - at.z_axis.truncate() * 1.2 + at.x_axis.truncate() * 0.4;
            let to = (screen - eye).normalize_or_zero();
            let yaw = (-to.x).atan2(-to.z).to_degrees();
            tracing::info!(
                "a stop's screen: --view={:.1},1.7,{:.1},{yaw:.1},5",
                eye.x,
                eye.z
            );
        }
        Ok((
            scene,
            Box::new(PlanRunning {
                title: self.title.clone(),
                trams: self.trams.clone(),
                boats: self.boats.clone(),
                clock: self.clock,
                walls: walk::Walls::new(self.walls.clone()),
                walker: None,
                walk_on_start: self.walk_on_start,
                ride_at: self.ride_after.map(|s| self.clock + s),
                riding: None,
                screens: self
                    .trams
                    .iter()
                    .enumerate()
                    .flat_map(|(line, run)| {
                        run.platforms(self.size)
                            .into_iter()
                            .map(move |(at, stop, outbound)| StopScreen {
                                at: at * Mat4::from_translation(SCREEN_ON_PLATFORM),
                                line,
                                stop,
                                outbound,
                            })
                    })
                    .collect(),
                crowd: self.crowd.take(),
                taxis: self.taxis.take(),
                walkers: Vec::new(),
                camera: glam::Vec2::ZERO,
                size: self.size,
                origin: scene_origin(args).to_f64().as_vec3(),
            }),
        ))
    }
}

/// The plan's scene while it runs: the trams run their timetables.
struct PlanRunning {
    title: String,
    trams: Vec<TramRun>,
    boats: Vec<BoatRun>,
    /// The service's clock, seconds.
    clock: f64,
    size: f64,
    /// The masses as walls, the walker among them, and whether it walks from the start.
    walls: walk::Walls,
    walker: Option<walk::Walker>,
    walk_on_start: bool,
    /// The population on foot round the camera, and where the camera last stood (x, z).
    crowd: Option<crowd::Crowd>,
    /// The automated taxis.
    taxis: Option<taxis::Taxis>,
    /// The people on foot drawn last frame, on the plan.
    walkers: Vec<DVec2>,
    camera: glam::Vec2,
    /// The scene's origin, which the movers' positions are relative to.
    origin: Vec3,
    /// The service clock at which to board the nearest tram (a scripted ride).
    ride_at: Option<f64>,
    /// The tram ridden (C): its line, its vehicle and section, and whether the rider walked
    /// before boarding.
    riding: Option<Ride>,
    /// Each stop's screen, on its totem (none without trams).
    screens: Vec<StopScreen>,
}

/// A ride aboard a tram ([`PlanRunning::toggle_ride`]).
#[derive(Clone, Copy, Debug)]
struct Ride {
    line: usize,
    vehicle: usize,
    section: usize,
    walked: bool,
    /// Whether the camera was turned the way the tram runs.
    facing: bool,
}

/// How near a tram's section the camera must stand to board it, metres.
const BOARD_REACH: f32 = 10.0;

impl PlanRunning {
    /// Walks from where the camera stands, or frees the camera.
    fn toggle_walk(&mut self, camera: &FlyCamera) {
        self.walker = match self.walker.take() {
            Some(_) => None,
            None => walk::Walker::new(&self.walls, camera.position)
                .inspect_err(|e| tracing::warn!("the walker: {e}"))
                .ok(),
        };
    }

    /// Boards the tram section nearest the camera within `reach` metres, standing in its
    /// aisle; or, aboard, gets off on the platform's side, walking again if the rider walked.
    fn toggle_ride(&mut self, camera: &FlyCamera, reach: f32) {
        if let Some(ride) = self.riding.take() {
            let (at, yaw) =
                self.trams[ride.line].sections(ride.vehicle, self.clock, self.size)[ride.section];
            // The platforms stand left of the way the tram runs.
            let left = glam::Quat::from_rotation_y(yaw) * Vec3::NEG_Z;
            let off = at + left * (vehicles::TRAM_WIDTH * 0.5 + 1.2) + Vec3::Y * walk::EYES;
            tracing::info!(line = ride.line, "off the tram");
            if ride.walked {
                self.walker = walk::Walker::new(&self.walls, off)
                    .inspect_err(|e| tracing::warn!("the walker: {e}"))
                    .ok();
            }
            return;
        }
        let mut best: Option<(f32, Ride)> = None;
        for (line, run) in self.trams.iter().enumerate() {
            for vehicle in 0..run.timetable.vehicles() {
                for (section, (at, _)) in run
                    .sections(vehicle, self.clock, self.size)
                    .iter()
                    .enumerate()
                {
                    let d = (*at - camera.position).with_y(0.0).length();
                    if d < reach && best.is_none_or(|(b, _)| d < b) {
                        let walked = self.walker.is_some();
                        best = Some((
                            d,
                            Ride {
                                line,
                                vehicle,
                                section,
                                walked,
                                facing: false,
                            },
                        ));
                    }
                }
            }
        }
        match best {
            Some((_, ride)) => {
                tracing::info!(
                    line = ride.line,
                    vehicle = ride.vehicle,
                    "aboard a tram (C to get off)"
                );
                self.walker = None;
                self.riding = Some(ride);
            }
            None => tracing::info!("no tram within {reach} m to board"),
        }
    }
}

impl Running for PlanRunning {
    fn key_pressed(&mut self, code: KeyCode, camera: &FlyCamera) -> bool {
        // Enter: walk the streets in first person, or fly again. C: board the nearest tram,
        // or get off it.
        match code {
            KeyCode::Enter if self.riding.is_none() => {
                self.toggle_walk(camera);
                true
            }
            KeyCode::KeyC => {
                self.toggle_ride(camera, BOARD_REACH);
                true
            }
            _ => false,
        }
    }

    fn update(
        &mut self,
        args: &Args,
        input: &Input,
        camera: &FlyCamera,
        _frame: u64,
        _dt: f32,
        step: f32,
    ) -> Option<f64> {
        self.clock += f64::from(step);
        self.camera = glam::Vec2::new(camera.position.x, camera.position.z);
        if let Some(crowd) = &mut self.crowd {
            crowd.advance(f64::from(step));
        }
        if let Some(taxis) = &mut self.taxis {
            let before = (self.clock - f64::from(step)) / 300.0;
            taxis.set_people(&self.walkers);
            taxis.advance(self.clock);
            // Every five minutes of the service, how the taxis keep up.
            if before.floor() != (self.clock / 300.0).floor() {
                let (waiting, wait) = taxis.service();
                tracing::info!(
                    taxis = taxis.len(),
                    parked = taxis.parked(),
                    waiting,
                    mean_wait_s = wait.round(),
                    "the taxis"
                );
            }
        }
        if self.walk_on_start {
            self.walk_on_start = false;
            self.toggle_walk(camera);
        }
        if self.ride_at.is_some_and(|at| self.clock >= at) {
            self.ride_at = None;
            // A scripted ride boards the nearest tram anywhere.
            self.toggle_ride(camera, f32::MAX);
        }
        if let Some(walker) = &mut self.walker {
            let walk = city_blocks::wished_walk(args, camera.yaw, input);
            walker.advance(&self.walls, walk, f64::from(step));
        }
        None
    }

    fn player(&mut self) -> Option<Vec3> {
        None
    }

    fn follow(&mut self, camera: &mut FlyCamera) {
        // Aboard: standing in the section's aisle, the view where the mouse turns it.
        if let Some(ride) = &mut self.riding {
            let (at, yaw) =
                self.trams[ride.line].sections(ride.vehicle, self.clock, self.size)[ride.section];
            camera.position = at + Vec3::Y * (vehicles::FLOOR + walk::EYES);
            // On boarding, facing the way it runs (the rear cab is turned round).
            if !ride.facing {
                ride.facing = true;
                let ahead = if ride.section + 1 == TRAM_SECTIONS {
                    yaw - std::f32::consts::PI
                } else {
                    yaw
                };
                camera.yaw = ahead - std::f32::consts::FRAC_PI_2;
                camera.pitch = 0.0;
            }
            return;
        }
        // In first person: the camera at the walker's eyes, the view where the mouse turns it.
        if let Some(walker) = &self.walker {
            camera.position = walker.feet() + Vec3::Y * walk::EYES;
        }
    }

    fn movers(&mut self, scene: &mut MeshletScene) {
        if self.trams.is_empty()
            && self.crowd.is_none()
            && self.boats.is_empty()
            && self.taxis.is_none()
        {
            return;
        }
        // In the order the movers were reserved: a line's cabs (front and rear of each
        // vehicle), then its passenger sections; then the crowd's figures, mesh by mesh.
        let mut transforms = Vec::new();
        let origin = self.origin;
        let place = move |(at, yaw): (Vec3, f32)| MoverTransform {
            position: at - origin,
            rotation: glam::Quat::from_rotation_y(yaw),
            scale: 1.0,
        };
        for run in &self.trams {
            // Its running trams, then those its depot keeps. In the night's pause the running
            // ones wait far under the ground, as unused figures do.
            let away = [(Vec3::new(0.0, -1000.0, 0.0) + origin, 0.0); TRAM_SECTIONS];
            let vehicles: Vec<_> = (0..run.timetable.vehicles())
                .map(|k| {
                    if run.paused {
                        away
                    } else {
                        run.sections(k, self.clock, self.size)
                    }
                })
                .chain(run.parked.iter().copied())
                .collect();
            for v in &vehicles {
                transforms.extend([place(v[0]), place(v[TRAM_SECTIONS - 1])]);
            }
            for v in &vehicles {
                transforms.extend(v[1..TRAM_SECTIONS - 1].iter().map(|&s| place(s)));
            }
        }
        // A crowd slot keeps its person while they stay in its mesh, and moves with them; a slot
        // that changed hands (a walker's pose is another mesh every 0.7 m; people come and go)
        // has no motion of its own that frame, or TAA would smear the figure along the swap.
        let mut still = vec![false; transforms.len()];
        if let Some(crowd) = &mut self.crowd {
            // Unused figures wait far under the ground.
            let parked = (Vec3::new(0.0, -1000.0, 0.0) + self.origin, 0.0);
            // The people on foot, on the plan: the taxis stop for those on a carriageway.
            self.walkers.clear();
            let half = (self.size * 0.5) as f32;
            for (mesh, figures) in crowd
                .figures(self.camera, self.clock)
                .into_iter()
                .enumerate()
            {
                let pose = mesh % crowd::POSES;
                for figure in figures {
                    still.push(!figure.is_some_and(|f| f.kept));
                    transforms.push(place(figure.map_or(parked, |f| (f.at, f.turn))));
                    // Not riding (2) nor seated (3).
                    if let Some(f) = figure.filter(|_| pose != 2 && pose != 3) {
                        self.walkers.push(DVec2::new(
                            f64::from(f.at.x + half),
                            f64::from(f.at.z + half),
                        ));
                    }
                }
            }
        }
        // Then the boats, one a crossing.
        for boat in &self.boats {
            transforms.push(place(boat.at(self.clock, self.size)));
        }
        // Then the stops' screens: the plate of each nearby totem's next tram, [`SCREENS`] of
        // each plate, the nearest first; still, as a plate changes hands when the minute does.
        if !self.screens.is_empty() {
            still.resize(transforms.len(), false);
            let mut near: Vec<(f32, usize)> = self
                .screens
                .iter()
                .enumerate()
                .filter_map(|(i, s)| {
                    let p = s.at.w_axis;
                    let d = (p.x - self.camera.x).powi(2) + (p.z - self.camera.y).powi(2);
                    (d < SCREEN_REACH * SCREEN_REACH).then_some((d, i))
                })
                .collect();
            near.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut rows: Vec<Vec<Mat4>> = vec![Vec::new(); signs::ROWS];
            for (_, i) in near {
                let s = &self.screens[i];
                let row = match self.trams[s.line]
                    .timetable
                    .next_arrival(s.stop, s.outbound, self.clock)
                {
                    Some(wait) if wait < 30.0 => 0,
                    Some(wait) => ((wait / 60.0).ceil() as usize).min(signs::NO_SERVICE - 1),
                    // No tram due this way (a terminus's far platform, the night's pause): the
                    // screen says so rather than nothing.
                    None => signs::NO_SERVICE,
                };
                if rows[row].len() < SCREENS {
                    rows[row].push(s.at);
                }
            }
            for row in rows {
                for k in 0..SCREENS {
                    still.push(true);
                    transforms.push(match row.get(k) {
                        Some(at) => {
                            let (_, rotation, position) = at.to_scale_rotation_translation();
                            MoverTransform {
                                position: position - origin,
                                rotation,
                                scale: 1.0,
                            }
                        }
                        None => place((Vec3::new(0.0, -1000.0, 0.0) + origin, 0.0)),
                    });
                }
            }
        }
        // Then the taxis, moving.
        if let Some(taxis) = &self.taxis {
            still.resize(transforms.len(), false);
            for pose in taxis.poses() {
                still.push(false);
                let mut at = taxis::world(self.size, &pose);
                // Not drawn: out of sight under the ground.
                if !pose.shown {
                    at.y -= 50.0;
                }
                transforms.push(place((at, pose.yaw)));
            }
        }
        scene.set_movers_still(&transforms, |k| still.get(k).copied().unwrap_or(false));
    }

    fn title(&mut self) -> String {
        self.title.clone()
    }
}

/// The middle of the squares, inside their trees and benches (the owner: "water mirrors as in
/// Nantes"; Bordeaux's is 130 × 42 m, Nantes' smaller):
/// - each large square (1.5 ha or more): a lawn 14 m inside its building lines, and on every
///   other one without a landmark a water mirror instead, a rectangle along the square of up to
///   80 × 30 m;
/// - every third square of 0.15 ha or more without a landmark: a water mirror of up to
///   30 × 9 m in place of its fountain, its chess tables beside it; 14 m inside its building
///   lines from 0.5 ha (its alley's inner row of trees stands 14 m in), 8 m below (its benches
///   stand 6 m in).
///
/// Each with whether it is a mirror; `None` for the other blocks.
fn square_middles(plan: &CityPlan) -> Vec<Option<(Vec<DVec2>, bool)>> {
    let held: Vec<usize> = plan.landmarks.iter().filter_map(|l| l.square).collect();
    let (mut large, mut medium) = (0, 0);
    plan.blocks
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if b.use_ != BlockUse::Square || b.area < 1_500.0 {
                return None;
            }
            let lawn = b.area >= 15_000.0;
            let middle = || geom::inset(&b.buildable, 14.0).filter(|_| lawn);
            let margin = if b.area >= 5_000.0 { 14.0 } else { 8.0 };
            if held.contains(&i) {
                return middle().map(|m| (m, false));
            }
            let (cap, wanted) = if lawn {
                large += 1;
                ((40.0, 15.0), large % 2 == 1)
            } else {
                medium += 1;
                ((15.0, 4.5), medium % 3 == 1)
            };
            if wanted && let Some(rect) = mirror_in(&b.buildable, margin, cap) {
                return Some((rect, true));
            }
            middle().map(|m| (m, false))
        })
        .collect()
}

/// The largest rectangle along `polygon`'s longest side, at most `cap` metres either side of
/// its middle (along, across), 8 × 4 m at least, whose corners and sides' middles lie `margin`
/// metres or more inside it.
fn mirror_in(polygon: &[DVec2], margin: f64, cap: (f64, f64)) -> Option<Vec<DVec2>> {
    let (axis, extent, centre) = geom::oriented_extent(polygon);
    let across = DVec2::new(-axis.y, axis.x);
    let n = polygon.len();
    let inside = |p: DVec2| {
        geom::contains(polygon, p)
            && (0..n)
                .all(|k| geom::distance_to_segment(p, polygon[k], polygon[(k + 1) % n]) >= margin)
    };
    let (mut l, mut w) = (
        (extent.x * 0.5 - margin).min(cap.0),
        (extent.y * 0.5 - margin).min(cap.1),
    );
    while l >= 4.0 && w >= 2.0 {
        let rect = vec![
            centre - axis * l - across * w,
            centre + axis * l - across * w,
            centre + axis * l + across * w,
            centre - axis * l + across * w,
        ];
        let sides = (0..4).map(|k| (rect[k] + rect[(k + 1) % 4]) * 0.5);
        if rect.iter().copied().chain(sides).all(inside) {
            return Some(rect);
        }
        (l, w) = (l * 0.8, w * 0.8);
    }
    None
}

/// The land of a polygon as lawn, 3 cm over the ground: runs of the grid's dry squares whose
/// middles lie inside it along each row, and the dry pieces of the squares the shore crosses.
fn lawn(tiles: &mut Tiles, shore: &Shore, map: &ground::GroundMap, polygon: &[DVec2]) {
    // At the walking level, cut round the streets and promenades where they meet: lawns at
    // 3 cm met the quays' walks with a step (the owner: think wheelchair).
    // Each piece cut to the quay's own face: whole cells ran past it under the next block's
    // ground, a lighter rectangle where the two lay one on the other (the owner).
    let grass = |tiles: &mut Tiles, piece: &[DVec2]| {
        for inside in ground::simple_polygons(&ground::clip(piece, polygon)) {
            if map.touches(&inside) {
                for p in ground::simple_polygons(&map.off_streets(&inside)) {
                    tiles.flat(&p, WALK_LEVEL, PARK, false);
                }
            } else {
                tiles.flat(&inside, WALK_LEVEL, PARK, false);
            }
        }
    };
    let s = shore.spacing();
    let n = shore.size() - 1;
    let (lo, hi) = polygon.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let cell = |v: f64| ((v / s).floor().max(0.0) as u32).min(n - 1);
    let (x0, x1) = (cell(lo.x), cell(hi.x));
    for y in cell(lo.y)..=cell(hi.y) {
        let mut x = x0;
        while x <= x1 {
            let middle = |x: u32| DVec2::new((f64::from(x) + 0.5) * s, (f64::from(y) + 0.5) * s);
            if shore.whole(x, y).is_none() {
                for piece in shore.square(x, y).dry {
                    if geom::contains(polygon, geom::centroid(&piece)) {
                        grass(tiles, &piece);
                    }
                }
                x += 1;
                continue;
            }
            let inside =
                |x: u32| shore.whole(x, y) == Some(false) && geom::contains(polygon, middle(x));
            if !inside(x) {
                x += 1;
                continue;
            }
            let start = x;
            while x <= x1 && inside(x) {
                x += 1;
            }
            let (a, b) = (f64::from(start) * s, f64::from(x) * s);
            let (c, d) = (f64::from(y) * s, f64::from(y + 1) * s);
            let quad = [
                DVec2::new(a, c),
                DVec2::new(b, c),
                DVec2::new(b, d),
                DVec2::new(a, d),
            ];
            grass(tiles, &quad);
        }
    }
}

/// A flat strip between two polylines of the same length, a quad between each pair of points,
/// in the tile of each quad's first corner.
fn strip(tiles: &mut Tiles, left: &[DVec2], right: &[DVec2], height: f64, section: u8) {
    for i in 0..left.len().min(right.len()).saturating_sub(1) {
        let quad = [left[i], left[i + 1], right[i + 1], right[i]];
        tiles.flat(&quad, height, section, false);
    }
}

/// A broadleaf tree 10 m tall, its crown 4.2 m in radius; each instance scales it to its tree's
/// height and crown. A tapering trunk forks into three limbs (section 0) under a crown of lumps
/// (section 1): a large one in the middle, five round it at different heights and one on top,
/// each an icosahedron subdivided once, a little irregular and smooth-shaded.
fn tree_model() -> TriMesh {
    use std::f32::consts::TAU;
    let mut mesh = TriMesh::default();
    limb(&mut mesh, Vec3::ZERO, Vec3::new(0.0, 4.4, 0.0), 0.3, 0.2);
    for k in 0..3 {
        let a = k as f32 * TAU / 3.0 + 0.5;
        let end = Vec3::new(a.cos() * 1.7, 6.8, a.sin() * 1.7);
        limb(&mut mesh, Vec3::new(0.0, 4.0, 0.0), end, 0.17, 0.07);
    }
    lump(&mut mesh, Vec3::new(0.0, 7.0, 0.0), 2.7, 0);
    for k in 0..5_u32 {
        let a = k as f32 * TAU / 5.0 + 0.2;
        let y = 6.3 + 0.5 * ((k * 2) % 3) as f32;
        lump(
            &mut mesh,
            Vec3::new(a.cos() * 2.2, y, a.sin() * 2.2),
            2.0,
            k + 1,
        );
    }
    lump(&mut mesh, Vec3::new(0.3, 8.4, -0.2), 1.6, 7);
    mesh
}

/// The ages each species is grown at for the forest (D-063): a quarter, half and all of its
/// maturity.
const FLORA_AGES: u8 = 3;
/// The individuals grown at each age, each from its own seed, in its own breeze and on its own
/// site: one an age made every holm oak of the wood's edge the same tree (the owner, 2026-10-16:
/// "trees are too much identical, they need variations").
const FLORA_SEEDS: u8 = 3;
/// How many individuals of each species are grown: individual `k` is age `k % FLORA_AGES`, seed
/// `k / FLORA_AGES`.
const FLORA_INDIVIDUALS: u8 = FLORA_AGES * FLORA_SEEDS;

/// The growing conditions of species `s`'s (its index `sp`) individual `k`: a quarter, half and
/// all of its maturity, the first seed on a good site in still air, the others leaning in a breeze
/// each its own way on a poorer site; the planted trees' crowns lifted as a street's and a
/// square's are, clear of the passers-by (a young plane was a bush on a stick): planes and limes
/// on 38 % of their height, umbrella pines on half their long bole, olives 15 %.
fn flora_conditions(
    s: &forge_flora::species::Species,
    sp: u16,
    k: u8,
) -> forge_flora::grow::Conditions {
    let variant = usize::from(k / FLORA_AGES) % usize::from(FLORA_SEEDS);
    let k = usize::from(k);
    let age = s.maturity * [0.25, 0.5, 1.0][k % usize::from(FLORA_AGES)];
    // A share of the height it grows to by then, so a tree scaled to a street's height keeps
    // its clear trunk in proportion (lifted in metres, a 9 m plane's crown hung at the eyes).
    let tall =
        (s.height[0] + s.height[1]) * 0.5 * forge_flora::species::growth_share(age, s.maturity);
    let lift = tall
        * match s.id.as_str() {
            "london-plane" | "common-lime" => 0.38,
            "umbrella-pine" => 0.5,
            "olive" => 0.15,
            _ => 0.0,
        };
    forge_flora::grow::Conditions {
        age,
        seed: 1000 + u64::from(sp) * 10 + k as u64,
        site: [1.0, 0.8, 0.9][variant],
        pruning: forge_flora::grow::Pruning {
            lift,
            ..Default::default()
        },
        wind: [DVec2::ZERO, DVec2::new(0.3, 0.0), DVec2::new(-0.1, -0.25)][variant],
    }
}

/// A species' material group and its bark's and foliage's two tones (`forge_flora`'s ids):
/// the Aleppo pine's reddish bark and light needles, the holm oak's dark leaves, the olive's
/// silver, the cypress's near black, the garrigue's greys.
#[allow(clippy::type_complexity)]
fn flora_tones(id: &str) -> (&'static str, ([f32; 3], [f32; 3]), ([f32; 3], [f32; 3])) {
    let bark = ([0.20, 0.15, 0.11], [0.36, 0.30, 0.24]);
    match id {
        "aleppo-pine" => (
            "flora-aleppo-pine",
            ([0.26, 0.17, 0.12], [0.40, 0.31, 0.24]),
            ([0.10, 0.18, 0.07], [0.17, 0.25, 0.09]),
        ),
        "holm-oak" => (
            "flora-holm-oak",
            ([0.16, 0.14, 0.12], [0.28, 0.25, 0.22]),
            ([0.05, 0.11, 0.04], [0.10, 0.16, 0.06]),
        ),
        "olive" => (
            "flora-olive",
            ([0.26, 0.24, 0.20], [0.40, 0.37, 0.31]),
            ([0.22, 0.26, 0.18], [0.32, 0.35, 0.26]),
        ),
        "italian-cypress" => (
            "flora-italian-cypress",
            bark,
            ([0.04, 0.09, 0.04], [0.08, 0.13, 0.06]),
        ),
        "rosemary" => (
            "flora-rosemary",
            bark,
            ([0.16, 0.22, 0.13], [0.24, 0.29, 0.19]),
        ),
        "cistus" => (
            "flora-cistus",
            bark,
            ([0.24, 0.28, 0.20], [0.33, 0.36, 0.27]),
        ),
        "kermes-oak" => (
            "flora-kermes-oak",
            bark,
            ([0.07, 0.14, 0.05], [0.13, 0.20, 0.08]),
        ),
        "london-plane" => (
            "flora-london-plane",
            ([0.38, 0.36, 0.29], [0.55, 0.53, 0.44]),
            ([0.10, 0.20, 0.06], [0.18, 0.28, 0.09]),
        ),
        "common-lime" => (
            "flora-common-lime",
            ([0.22, 0.19, 0.16], [0.34, 0.30, 0.25]),
            ([0.12, 0.24, 0.06], [0.20, 0.32, 0.09]),
        ),
        "umbrella-pine" => (
            "flora-umbrella-pine",
            ([0.28, 0.18, 0.12], [0.44, 0.33, 0.25]),
            ([0.09, 0.17, 0.06], [0.15, 0.24, 0.08]),
        ),
        _ => (
            "flora-other",
            bark,
            ([0.08, 0.19, 0.06], [0.16, 0.27, 0.07]),
        ),
    }
}

/// The leaf atlas's spray a species' cards are cut from (`textures::leaves`): broad leaves for
/// planes and limes, needles for the pines, the cypress and rosemary, narrow leaves for olives
/// and cistus, small oval ones for the oaks.
fn leaf_spray(id: &str) -> textures::LeafSpray {
    use textures::LeafSpray;
    match id {
        "london-plane" | "common-lime" => LeafSpray::Broad,
        "aleppo-pine" | "umbrella-pine" | "italian-cypress" | "rosemary" => LeafSpray::Needles,
        "olive" | "cistus" => LeafSpray::Narrow,
        _ => LeafSpray::Small,
    }
}

/// A grown plant's mesh (`forge_flora`, D-063), a tree's few thousand triangles and a shrub's a
/// few hundred, the full graph's being 2 000 to 250 000: a tree's wood 5 cm thick or more as
/// tapered limbs (section 0; a shrub's twigs hide in its leaves), its leaf clusters gathered
/// on a grid of a sixth of its crown's width (an eighth of a broad-leaved tree's, a third of a
/// shrub's) or a tenth of its height, whichever is larger (a cypress's narrow column took 50 000
/// triangles), each cell's into one lump at their middle: a core of 20 triangles (section 1) in
/// leaf cards cut from `spray` (section 2, [`leaf_cards`]). The cells are keyed in order, so a
/// plant's mesh is the same from run to run.
fn grown_mesh(
    g: &forge_flora::graph::TreeGraph,
    shrub: bool,
    broad: bool,
    lift: f32,
    spray: textures::LeafSpray,
) -> TriMesh {
    let mut mesh = TriMesh::default();
    let width = g.crown_width();
    let cell = (width
        / if shrub {
            3.0
        } else if broad {
            8.0
        } else {
            6.0
        })
    .max(g.height() / 10.0)
    .max(0.3);
    if !shrub {
        for s in g.segments() {
            let node = &g.nodes[s.node as usize];
            let parent = g.nodes[node.parent as usize].radius.max(s.radius);
            if s.radius >= 0.05 && s.a.distance(s.b) > 1e-3 {
                limb(&mut mesh, s.a, s.b, parent, s.radius);
            }
        }
    }
    let mut cells: std::collections::BTreeMap<(i32, i32, i32), (Vec3, u32, f32)> =
        Default::default();
    for l in &g.leaves {
        let k = (l.pos / cell).floor();
        let e = cells
            .entry((k.x as i32, k.y as i32, k.z as i32))
            .or_insert((Vec3::ZERO, 0, 0.0));
        e.0 += l.pos;
        e.1 += 1;
        e.2 = e.2.max(l.size);
    }
    let faces = icosahedron();
    let mean = g.leaves.len() as f32 / cells.len().max(1) as f32;
    let lumps: Vec<(Vec3, f32)> = cells
        .into_values()
        .map(|(sum, n, size)| {
            let radius = cell * (0.45 + 0.35 * (n as f32 / mean).sqrt().min(1.6)) + size * 0.3;
            // Kept over a lifted crown's clear trunk: a lump round a low cluster hung below it.
            let mut centre = sum / n as f32;
            centre.y = centre.y.max(lift + radius * 0.7);
            (centre, radius)
        })
        .collect();
    // The crown's middle, which the cards' normals bend away from with their lump's.
    let middle = lumps.iter().fold(Vec3::ZERO, |s, l| s + l.0) / lumps.len().max(1) as f32;
    for (i, &(centre, radius)) in lumps.iter().enumerate() {
        // A core of two fifths of the lump, the shade the cards hide and the far crown keeps
        // (it simplifies as a closed mesh; the cards cannot), then the cards round it.
        lump_of(&mut mesh, centre, radius * CROWN_CORE, i as u32 % 8, &faces);
        leaf_cards(&mut mesh, centre, radius, middle, spray, shrub, i as u32);
    }
    let missing = mesh.positions.len() - mesh.uvs.len();
    mesh.uvs.extend(std::iter::repeat_n([0.0, 0.0], missing));
    mesh
}

/// A crown lump's core, as a share of the lump's radius ([`grown_mesh`]): three tenths, drawn
/// darker, the shade between the cards (at 0.6 in the leaves' colour, then at half with 12 cards
/// at most a lump, the near holm oaks showed it as balls; at two fifths, once the metered
/// exposure lit the wood, as faceted green stones among the leaves).
const CROWN_CORE: f32 = 0.3;

/// Leaf cards round a crown's lump at `centre`, `radius` across (D-013), section 2, cut from
/// `spray`'s quarter of the leaf atlas (`textures::leaves`): 6 to 16 squares (a shrub's 4),
/// their middles 0.4 to 0.7 radii out along directions spread over the sphere and turned by
/// `seed`, facing out with a tilt, their sprays growing upwards. Their normals bend out from the
/// crown's `middle` and the lump's, so the crown is lit as a rounded mass of leaves.
fn leaf_cards(
    mesh: &mut TriMesh,
    centre: Vec3,
    radius: f32,
    middle: Vec3,
    spray: textures::LeafSpray,
    shrub: bool,
    seed: u32,
) {
    let ([u0, v0], [u1, v1]) = spray.uv_box();
    // Leaves near their own size: a card's half side at most 0.6 m for broad leaves (a plane's
    // 20–25 cm), 0.4 m for small and narrow ones, 0.5 m for needles; a card of 0.75 lump radii
    // drew a plane's leaves 40–60 cm wide by the eye. More cards where they are smaller, so the
    // lump keeps its cover.
    let cap = match spray {
        textures::LeafSpray::Broad => 0.6,
        textures::LeafSpray::Needles => 0.5,
        textures::LeafSpray::Small | textures::LeafSpray::Narrow => 0.4,
    };
    let half = (radius * 0.75).min(cap);
    let count = if shrub {
        4
    } else {
        (6.0 * (radius * 0.75 / half).powi(2))
            .round()
            .clamp(6.0, 16.0) as u32
    };
    let random = |k: u32, salt: u32| {
        let h = (seed.wrapping_mul(0x9E37_79B9) ^ k.wrapping_mul(0x85EB_CA6B) ^ salt)
            .wrapping_mul(0xC2B2_AE35);
        ((h ^ (h >> 15)) & 0xFFFF) as f32 / 65535.0
    };
    let golden = std::f32::consts::PI * (3.0 - 5.0_f32.sqrt());
    let spin = random(0, 1) * std::f32::consts::TAU;
    for k in 0..count {
        // A Fibonacci spiral over the sphere.
        let y = 1.0 - 2.0 * (k as f32 + 0.5) / count as f32;
        let ring = (1.0 - y * y).max(0.0).sqrt();
        let a = spin + golden * k as f32;
        let out = Vec3::new(ring * a.cos(), y, ring * a.sin());
        let tilt = out.any_orthonormal_vector();
        let tilt = (tilt * (random(k, 2) - 0.5) + out.cross(tilt) * (random(k, 3) - 0.5)) * 1.2;
        let normal = (out + tilt).normalize();
        // The spray grows upwards, turned a little about the card's normal.
        let level = Vec3::Y - normal * normal.dot(Vec3::Y);
        let up = if level.length_squared() > 1e-4 {
            level.normalize()
        } else {
            normal.any_orthonormal_vector()
        };
        let turn = (random(k, 4) - 0.5) * 1.4;
        let up = glam::Quat::from_axis_angle(normal, turn) * up;
        let side = up.cross(normal);
        let at =
            centre + Vec3::new(out.x, out.y * 0.85, out.z) * radius * (0.4 + 0.3 * random(k, 5));
        let corners = [
            (at - up * half - side * half, [u0, v1]),
            (at - up * half + side * half, [u1, v1]),
            (at + up * half + side * half, [u1, v0]),
            (at + up * half - side * half, [u0, v0]),
        ];
        let missing = mesh.positions.len() - mesh.uvs.len();
        mesh.uvs.extend(std::iter::repeat_n([0.0, 0.0], missing));
        let base = mesh.positions.len() as u32;
        for (p, uv) in corners {
            let bent = ((p - middle).normalize_or_zero() + (p - centre).normalize_or_zero())
                .normalize_or(normal);
            mesh.positions.push(p.to_array());
            mesh.normals.push(bent.to_array());
            mesh.uvs.push(uv);
        }
        // Counter-clockwise seen from the normal's side.
        mesh.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        mesh.sections.extend([2, 2]);
    }
}

/// A triangle with a normal per corner, turned so it faces along their sum.
fn smooth_triangle(mesh: &mut TriMesh, mut p: [Vec3; 3], mut n: [Vec3; 3], section: u8) {
    if (p[1] - p[0]).cross(p[2] - p[0]).dot(n[0] + n[1] + n[2]) < 0.0 {
        p.swap(1, 2);
        n.swap(1, 2);
    }
    let base = mesh.positions.len() as u32;
    for (q, m) in p.into_iter().zip(n) {
        mesh.positions.push(q.to_array());
        mesh.normals.push(m.to_array());
    }
    mesh.indices.extend([base, base + 1, base + 2]);
    mesh.sections.push(section);
}

/// A tapering seven-sided limb from `a` (radius `ra`) to `b` (radius `rb`), section 0.
fn limb(mesh: &mut TriMesh, a: Vec3, b: Vec3, ra: f32, rb: f32) {
    let axis = (b - a).normalize();
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    let sides = 7;
    let ring = |k: u32| {
        let t = k as f32 / sides as f32 * std::f32::consts::TAU;
        u * t.cos() + v * t.sin()
    };
    for k in 0..sides {
        let (d0, d1) = (ring(k), ring(k + 1));
        let (p0, p1) = (a + d0 * ra, a + d1 * ra);
        let (q0, q1) = (b + d0 * rb, b + d1 * rb);
        smooth_triangle(mesh, [p0, p1, q1], [d0, d1, d1], 0);
        smooth_triangle(mesh, [p0, q1, q0], [d0, d1, d0], 0);
    }
}

/// A crown's lump round `centre`, about `radius` across, a little flattened, its surface
/// rippled by `seed`; section 1.
fn lump(mesh: &mut TriMesh, centre: Vec3, radius: f32, seed: u32) {
    lump_of(mesh, centre, radius, seed, &icosphere());
}

/// [`lump`] on the faces `faces` (unit directions): an [`icosahedron`]'s 20 for a shrub's.
fn lump_of(mesh: &mut TriMesh, centre: Vec3, radius: f32, seed: u32, faces: &[[Vec3; 3]]) {
    let s = seed as f32 * 1.7;
    let place = |d: Vec3| {
        let d = d.normalize();
        let bump = 1.0
            + 0.09 * (d.x * 7.3 + d.y * 3.1 + d.z * 5.7 + s).sin()
            + 0.05 * (d.x * 13.1 - d.z * 11.3 + s * 2.3).sin();
        (
            centre + Vec3::new(d.x, d.y * 0.85, d.z) * radius * bump,
            Vec3::new(d.x, d.y / 0.85, d.z).normalize(),
        )
    };
    for &tri in faces {
        let p = tri.map(place);
        smooth_triangle(mesh, p.map(|q| q.0), p.map(|q| q.1), 1);
    }
}

/// An icosahedron's 20 faces, their corners as directions (not of unit length).
fn icosahedron() -> Vec<[Vec3; 3]> {
    let t = (1.0 + 5.0_f32.sqrt()) * 0.5;
    let v: [Vec3; 12] = [
        Vec3::new(-1.0, t, 0.0),
        Vec3::new(1.0, t, 0.0),
        Vec3::new(-1.0, -t, 0.0),
        Vec3::new(1.0, -t, 0.0),
        Vec3::new(0.0, -1.0, t),
        Vec3::new(0.0, 1.0, t),
        Vec3::new(0.0, -1.0, -t),
        Vec3::new(0.0, 1.0, -t),
        Vec3::new(t, 0.0, -1.0),
        Vec3::new(t, 0.0, 1.0),
        Vec3::new(-t, 0.0, -1.0),
        Vec3::new(-t, 0.0, 1.0),
    ];
    let faces: [[usize; 3]; 20] = [
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    faces.iter().map(|f| [v[f[0]], v[f[1]], v[f[2]]]).collect()
}

/// An icosahedron's faces, each cut in four: 80 triangles of unit directions.
fn icosphere() -> Vec<[Vec3; 3]> {
    let mut out = Vec::with_capacity(80);
    for [a, b, c] in icosahedron() {
        let (ab, bc, ca) = ((a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5);
        out.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
    }
    out
}

/// A pedestrian's eye on the pavement of the avenue nearest `near`, 1.7 m up, looking along it.
pub fn street_camera(plan: &CityPlan, near: DVec2) -> Option<FlyCamera> {
    use forge_procgen::city::Profile;
    let edge = plan
        .roads
        .edges
        .iter()
        .filter(|e| matches!(e.profile, Profile::TramAvenue | Profile::Avenue) && !e.bridge)
        .min_by(|a, b| {
            let da = geom::point_at(&a.points, a.length() * 0.5).distance_squared(near);
            let db = geom::point_at(&b.points, b.length() * 0.5).distance_squared(near);
            da.partial_cmp(&db).expect("finite")
        })?;
    let s = edge.length() * 0.3;
    let t = geom::tangent_at(&edge.points, s);
    let side = DVec2::new(-t.y, t.x) * (edge.profile.width() * 0.5 - 4.5);
    let p = geom::point_at(&edge.points, s) + side;
    let half = plan.size * 0.5;
    Some(FlyCamera {
        position: Vec3::new((p.x - half) as f32, 1.7, (p.y - half) as f32),
        // Yaw 0 looks north (−z), a quarter turn west.
        yaw: (-t.x).atan2(-t.y) as f32,
        pitch: 0.05,
        speed: 3.0,
        ..FlyCamera::default()
    })
}

/// The street ground from the plan's map (D-061), each surface once at its level:
/// - carriageways and crossings at [`ROAD_LEVEL`], the crossings' stripes painted;
/// - the walking ground at [`WALK_LEVEL`];
/// - the dropped kerbs sloping between the two;
/// - a kerb's face along each carriageway, down to a centimetre at a dropped kerb;
/// - the ramps' sides;
/// - a face down from the street ground's edge to the open ground beside it (none over the
///   water, where the bridges' sides stand).
fn street_ground(tiles: &mut Tiles, map: &ground::GroundMap, site: &Site) {
    let size = tiles.size;
    // A vertical face along `a b` from `low` to `high` at each end, facing `out`.
    let face =
        |tiles: &mut Tiles, a: DVec2, b: DVec2, low: [f64; 2], high: [f64; 2], out: DVec2| {
            if (high[0] - low[0]).max(high[1] - low[1]) < 0.002 {
                return;
            }
            let tile = tiles.at(a);
            let centre = tile.centre;
            let p = |q: DVec2, h: f64| world(size, q, h) - centre;
            let normal = Vec3::new(out.x as f32, 0.0, out.y as f32);
            triangle(
                &mut tile.ground,
                [p(a, low[0]), p(b, low[1]), p(b, high[1])],
                normal,
                PAVING,
            );
            triangle(
                &mut tile.ground,
                [p(a, low[0]), p(b, high[1]), p(a, high[0])],
                normal,
                PAVING,
            );
        };
    for t in &map.tiles {
        for (shapes, level, section) in [
            (&t.carriageway, ROAD_LEVEL, STREET),
            (&t.crossings, ROAD_LEVEL, STREET),
            (&t.stripes, ROAD_LEVEL, MARKING),
            (&t.raised, WALK_LEVEL, STREET),
            (&t.raised_stripes, WALK_LEVEL, MARKING),
            (&t.walk, WALK_LEVEL, PAVING),
        ] {
            for polygon in ground::simple_polygons(shapes) {
                tiles.flat(&polygon, level, section, false);
            }
        }
        for (r, shapes) in &t.ramps {
            let ramp = map.ramps[*r as usize];
            // Cut along its creases into its planar pieces (the middle, each flare), each
            // triangle's corners at their levels. The flares slope up to the pavement, so no side
            // faces: there is no edge to step or roll over (the owner).
            let [(left, _), (right, _)] = ramp.creases();
            let cut = DVec2::new(ramp.along.y, -ramp.along.x);
            for polygon in ground::simple_polygons(shapes) {
                let middle =
                    geom::clip_half_plane(&geom::clip_half_plane(&polygon, left, cut), right, -cut);
                let pieces = [
                    geom::clip_half_plane(&polygon, left, -cut),
                    middle,
                    geom::clip_half_plane(&polygon, right, cut),
                ];
                for piece in pieces.iter().filter(|p| p.len() >= 3) {
                    let tile = tiles.at(piece[0]);
                    let centre = tile.centre;
                    for tri in ground::triangles(piece) {
                        let p = tri.map(|q| world(size, q, ramp.level(q)) - centre);
                        let mut up = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                        if up.y < 0.0 {
                            up = -up;
                        }
                        // A table's ramp is the road's, a dropped kerb the pavement's.
                        let section = if ramp.lip == 0.0 { STREET } else { PAVING };
                        triangle(&mut tile.ground, p, up, section);
                    }
                }
            }
        }
        // The ramps near this tile, for the kerbs' height along them.
        let ramps: Vec<ground::Ramp> = t
            .ramps
            .iter()
            .map(|(r, _)| map.ramps[*r as usize])
            .collect();
        let top = |q: DVec2| {
            ramps
                .iter()
                .filter(|r| r.lip > 0.0 && r.contains(q))
                .map(|r| r.level(q))
                .fold(WALK_LEVEL, f64::min)
        };
        // The road side's level: a raised table's and its ramps' (no kerb along a table), else
        // the carriageway's.
        let low = |q: DVec2| {
            if let Some(r) = ramps.iter().find(|r| r.lip == 0.0 && r.contains(q)) {
                r.level(q)
            } else if ground::inside(&t.raised, q) || ground::inside(&t.raised_stripes, q) {
                WALK_LEVEL
            } else {
                ROAD_LEVEL
            }
        };
        for &[a, b] in &t.kerbs {
            let d = b - a;
            let length = d.length();
            if length < 1e-4 {
                continue;
            }
            // The walking side is on the kerb's right; its face looks the other way, at the road.
            let walk = DVec2::new(d.y, -d.x) / length;
            let near_ramp = ramps
                .iter()
                .any(|r| geom::distance_to_segment(r.kerb, a, b) < r.half + ground::FLARE + 0.5);
            let steps = if near_ramp {
                (length / 0.5).ceil() as usize
            } else {
                1
            };
            for k in 0..steps {
                let (p, q) = (
                    a + d * (k as f64 / steps as f64),
                    a + d * ((k + 1) as f64 / steps as f64),
                );
                let (hp, hq) = (top(p + walk * 0.05), top(q + walk * 0.05));
                let (lp, lq) = (low(p - walk * 0.05), low(q - walk * 0.05));
                face(tiles, p, q, [lp, lq], [hp, hq], -walk);
            }
        }
        for &[a, b] in &t.outline {
            let d = b - a;
            let length = d.length();
            if length < 1e-4 {
                continue;
            }
            let out = DVec2::new(d.y, -d.x) / length;
            let mid = (a + b) * 0.5;
            if site.water_at(mid + out * 0.5) != forge_procgen::city::LAND {
                continue;
            }
            let level = map.level_at(mid - out * 0.05);
            face(tiles, a, b, [0.0; 2], [level; 2], out);
        }
    }
}

/// A street lamp 8 m tall: a hexagonal pole and an arm reaching 1.4 m along −z over the road, and
/// when `double` a second reaching 1 m along +z over the pavement (section 0); under each arm's end
/// a dark housing (section 0) lit only underneath (section 1, emissive): shielded, no light into
/// the sky (the owner, 2026-10-11).
fn lamp_model(double: bool) -> TriMesh {
    let mut mesh = TriMesh::default();
    let sides = 6;
    for k in 0..sides {
        let a = k as f32 / sides as f32 * std::f32::consts::TAU;
        let b = (k + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let (pa, pb) = (
            Vec3::new(a.cos() * 0.07, 0.0, a.sin() * 0.07),
            Vec3::new(b.cos() * 0.07, 0.0, b.sin() * 0.07),
        );
        let up = Vec3::Y * 8.0;
        let out = ((pa + pb) * 0.5).normalize();
        triangle(&mut mesh, [pa, pb, pb + up], out, 0);
        triangle(&mut mesh, [pa, pb + up, pa + up], out, 0);
    }
    // A box from `lo` to `hi` in a section.
    let boxed = |mesh: &mut TriMesh, lo: Vec3, hi: Vec3, section: u8| {
        let c = |x: bool, y: bool, z: bool| {
            Vec3::new(
                if x { hi.x } else { lo.x },
                if y { hi.y } else { lo.y },
                if z { hi.z } else { lo.z },
            )
        };
        let faces = [
            (
                Vec3::X,
                [
                    c(true, false, false),
                    c(true, true, false),
                    c(true, true, true),
                    c(true, false, true),
                ],
            ),
            (
                -Vec3::X,
                [
                    c(false, false, false),
                    c(false, false, true),
                    c(false, true, true),
                    c(false, true, false),
                ],
            ),
            (
                Vec3::Y,
                [
                    c(false, true, false),
                    c(false, true, true),
                    c(true, true, true),
                    c(true, true, false),
                ],
            ),
            (
                -Vec3::Y,
                [
                    c(false, false, false),
                    c(true, false, false),
                    c(true, false, true),
                    c(false, false, true),
                ],
            ),
            (
                Vec3::Z,
                [
                    c(false, false, true),
                    c(true, false, true),
                    c(true, true, true),
                    c(false, true, true),
                ],
            ),
            (
                -Vec3::Z,
                [
                    c(false, false, false),
                    c(false, true, false),
                    c(true, true, false),
                    c(true, false, false),
                ],
            ),
        ];
        for (n, q) in faces {
            triangle(mesh, [q[0], q[1], q[2]], n, section);
            triangle(mesh, [q[0], q[2], q[3]], n, section);
        }
    };
    // The arms: over the road along −z, and over the pavement along +z.
    let arms: &[f32] = if double { &[-1.0, 1.0] } else { &[-1.0] };
    for &way in arms {
        let reach = if way < 0.0 { 1.45 } else { 1.05 };
        let (z0, z1) = if way < 0.0 {
            (-reach, 0.0)
        } else {
            (0.0, reach)
        };
        boxed(
            &mut mesh,
            Vec3::new(-0.04, 7.85, z0),
            Vec3::new(0.04, 7.95, z1),
            0,
        );
        // The housing, dark, and under it the lit lens, facing down only.
        let (h0, h1) = if way < 0.0 {
            (-reach - 0.25, -reach + 0.35)
        } else {
            (reach - 0.35, reach + 0.25)
        };
        boxed(
            &mut mesh,
            Vec3::new(-0.14, 7.70, h0),
            Vec3::new(0.14, 7.85, h1),
            0,
        );
        let lens = [
            Vec3::new(-0.12, 7.69, h0 + 0.02),
            Vec3::new(0.12, 7.69, h0 + 0.02),
            Vec3::new(0.12, 7.69, h1 - 0.02),
            Vec3::new(-0.12, 7.69, h1 - 0.02),
        ];
        triangle(&mut mesh, [lens[0], lens[1], lens[2]], -Vec3::Y, 1);
        triangle(&mut mesh, [lens[0], lens[2], lens[3]], -Vec3::Y, 1);
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_procgen::city::{example, plan};

    /// The forest's grown plants keep to their budgets: a tree 40 000 triangles at most, a
    /// shrub 1 500 (the full graphs' run to 250 000).
    #[test]
    fn the_grown_plants_keep_their_budgets() {
        for (sp, s) in forge_flora::species::mediterranean().iter().enumerate() {
            let shrub = s.habit == forge_flora::species::Habit::Shrub;
            let broad = s.foliage == forge_flora::species::Foliage::Broad;
            for k in 0..FLORA_INDIVIDUALS {
                let conditions = flora_conditions(s, sp as u16, k);
                let g = forge_flora::grow::grow(s, &conditions);
                let m = grown_mesh(
                    &g,
                    shrub,
                    broad,
                    conditions.pruning.lift as f32,
                    leaf_spray(&s.id),
                );
                let budget = if shrub { 1_500 } else { 40_000 };
                println!("{} {k}: {} triangles", s.id, m.triangle_count());
                assert!(
                    m.triangle_count() > 0 && m.triangle_count() <= budget,
                    "{} {k}: {} triangles",
                    s.id,
                    m.triangle_count()
                );
            }
        }
    }

    #[test]
    fn a_depot_s_trams_stand_on_its_tracks_in_their_places() {
        use forge_procgen::city::tram_depot::PLACE;
        // A tram and a few metres in each place: the cabs never touch the next tram.
        assert!(
            tram_length() + 4.0 <= PLACE,
            "a {:.1} m tram",
            tram_length()
        );
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let depot = city.tram_depots.first().expect("a depot in the example");
        for (at, along) in depot.places() {
            let sections = parked_sections(at, along, size);
            // Its middle at the place's, along its track at the yard's level.
            let middle = (sections[0].0 + sections[TRAM_SECTIONS - 1].0) * 0.5;
            assert!(middle.distance(world(size, at, WALK_LEVEL + 0.06)) < 1e-3);
            for (p, _) in sections {
                let q = DVec2::new(f64::from(p.x) + size * 0.5, f64::from(p.z) + size * 0.5);
                assert!(depot.on_yard(q, 0.0), "a section off the yard");
            }
        }
    }

    #[test]
    fn trams_turn_where_they_stand_having_crossed_over_on_the_way_in() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let mut runs: Vec<TramRun> = Vec::new();
        let mut taken: Vec<DVec2> = Vec::new();
        for l in &city.tram_lines {
            if let Some(run) = TramRun::new(l, &city, &site, &taken) {
                taken.extend(run.timetable.stops().iter().map(|&s| run.track(s).0));
                runs.push(run);
            }
        }
        stagger_termini(&mut runs);
        assert!(!runs.is_empty());
        // Every quarter second of a whole cycle, each section's middle within a quarter second's
        // run at the cruise of where a section was the moment before (the cabs swap ends as a
        // tram turns, so the nearest), the outer track on the tightest curve running longer
        // than the line: a tram turning at an end jumped 3.2 m across and a tram's length along.
        let dt = 0.25;
        let outer = 1.0 + (TRACK_OFFSET + 0.5) / forge_procgen::city::transit::TRAM_RADIUS;
        let reach = (Service::default().cruise * dt * outer) as f32 + 0.1;
        for (i, run) in runs.iter().enumerate() {
            let (mut worst, mut prev): (f32, Option<Vec<Vec3>>) = (0.0, None);
            let mut t = 0.0;
            while t < run.timetable.cycle() {
                let now: Vec<Vec3> = run.sections(0, t, size).iter().map(|s| s.0).collect();
                if let Some(before) = &prev {
                    for p in &now {
                        let moved = before
                            .iter()
                            .map(|q| q.distance(*p))
                            .fold(f32::MAX, f32::min);
                        worst = worst.max(moved);
                    }
                }
                prev = Some(now);
                t += dt;
            }
            println!("line {i}: a section moved {worst:.2} m in {dt} s at most");
            assert!(
                worst <= reach,
                "line {i}: a section moved {worst:.2} m in {dt} s"
            );
        }
    }

    #[test]
    fn crossover_pairs_stand_every_few_kilometres_clear_of_stops_and_junctions() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let (mut pairs, mut km) = (0, 0.0);
        for seed in [None, Some(0x5EED_0001_u64), Some(0xC17E_0003)] {
            let mut params = example::params(size);
            if let Some(seed) = seed {
                params.seed = forge_core::Seed::new(seed);
            }
            let city = plan(&site, &params);
            let mut taken: Vec<DVec2> = Vec::new();
            for l in &city.tram_lines {
                let Some(run) = TramRun::new(l, &city, &site, &taken) else {
                    continue;
                };
                taken.extend(run.timetable.stops().iter().map(|&s| run.track(s).0));
                let stops = run.timetable.stops();
                km += (stops[stops.len() - 1] - stops[0]) / 1000.0;
                let starts = run.crossover_pairs();
                pairs += starts.len();
                let span = 2.0 * CROSSOVER + 10.0;
                for &s in &starts {
                    for &p in stops {
                        let reach = f64::from(PLATFORM) * 0.5;
                        assert!(
                            p + reach < s || p - reach > s + span,
                            "a crossover pair on a stop's platform"
                        );
                    }
                    for &(j, r) in &run.junctions {
                        assert!(
                            j + r < s || j - r > s + span,
                            "a crossover pair in a junction"
                        );
                    }
                    // On land, off the bridges.
                    for k in 0..=(span / 2.0) as usize {
                        let p = run.track(s + 2.0 * k as f64).0;
                        assert_eq!(
                            site.water_at(p),
                            forge_procgen::city::LAND,
                            "a crossover pair on a bridge"
                        );
                    }
                    // Both tracks join again past it.
                    assert!((TramRun::pair_side(s, s + span) - TRACK_OFFSET).abs() < 1e-9);
                    assert!((TramRun::pair_side(s, s) - TRACK_OFFSET).abs() < 1e-9);
                }
                for w in starts.windows(2) {
                    assert!(w[1] - w[0] > 500.0, "two pairs {:.0} m apart", w[1] - w[0]);
                }
            }
        }
        println!("{pairs} crossover pairs on {km:.1} km of line");
        assert!(km > 0.0);
        // About one every 3 km where the streets leave room.
        assert!(
            pairs as f64 >= km / 3.0 * 0.5,
            "{pairs} pairs on {km:.1} km"
        );
    }

    #[test]
    fn windows_light_by_the_hour() {
        // `--time-of-day`: sunrise at 6, noon at 12, sunset at 18, midnight at 1.5.
        assert_eq!(hour_of_day(0.0), 6.0);
        assert_eq!(hour_of_day(1.0), 18.0);
        assert_eq!(hour_of_day(1.5), 0.0);
        // Homes: the evening's peak, the small hours dark; shops lit while open.
        let home = |h: f64| lit_share(h, false);
        let shop = |h: f64| lit_share(h, true);
        assert!(home(21.0) >= 0.45 && home(3.0) <= 0.06);
        assert!(home(21.0) > home(23.5) && home(23.5) > home(2.0));
        assert!(shop(12.0) > 0.6 && shop(2.0) <= 0.1);
        // The trams pause from 1:30 to 4:30.
        assert!(tram_pause(hour_of_day(1.75)) && !tram_pause(hour_of_day(1.5)));
        assert!(!tram_pause(5.0) && !tram_pause(23.0));
        for h in 0..=48 {
            let h = f64::from(h) * 0.5;
            assert!((0.0..=1.0).contains(&home(h)) && (0.0..=1.0).contains(&shop(h)));
        }
    }

    #[test]
    fn water_mirrors_lie_level_in_their_squares_clear_of_the_trees_and_benches() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let (mut mirrors, mut squares) = (0, 0);
        // The example's seed and the other seeds the plan's rules are checked on.
        for seed in [
            None,
            Some(0x5EED_0001_u64),
            Some(0x5EED_0042),
            Some(0xC17E_0003),
        ] {
            let mut params = example::params(size);
            if let Some(seed) = seed {
                params.seed = forge_core::Seed::new(seed);
            }
            let city = plan(&site, &params);
            squares += city
                .blocks
                .iter()
                .filter(|b| b.use_ == BlockUse::Square && b.area >= 1_500.0)
                .count();
            mirrors += mirrors_checked(&city);
        }
        println!("{squares} squares of 0.15 ha or more, {mirrors} water mirrors");
        // Most of the example's squares hold its landmarks.
        assert!(mirrors >= 1, "no water mirror in four cities");
    }

    /// `city`'s water mirrors, each checked: on a square without a landmark, clear of its trees
    /// and benches.
    fn mirrors_checked(city: &CityPlan) -> usize {
        let middles = square_middles(city);
        let mut mirrors = 0;
        for (i, m) in middles.iter().enumerate() {
            let Some((outline, mirror)) = m else {
                continue;
            };
            let b = &city.blocks[i];
            assert_eq!(b.use_, BlockUse::Square);
            if !*mirror {
                assert!(b.area >= 15_000.0, "a lawn on a square of {:.0} m²", b.area);
                continue;
            }
            mirrors += 1;
            assert!(
                city.landmarks.iter().all(|l| l.square != Some(i)),
                "a mirror under a landmark"
            );
            // Inside the alley's inner row of trees (14 m in from 0.5 ha) and the benches (6 m
            // in), with room to walk round it.
            let margin = if b.area >= 5_000.0 { 14.0 } else { 8.0 };
            let n = b.buildable.len();
            for &p in outline {
                let edge = (0..n)
                    .map(|k| geom::distance_to_segment(p, b.buildable[k], b.buildable[(k + 1) % n]))
                    .fold(f64::MAX, f64::min);
                assert!(
                    geom::contains(&b.buildable, p) && edge >= margin - 1e-6,
                    "a mirror's corner {edge:.1} m from its square's edge"
                );
            }
            let area = geom::signed_area(outline);
            assert!(area >= 32.0, "a mirror of {area:.0} m²");
        }
        mirrors
    }

    #[test]
    fn the_court_paths_are_paved_with_the_forecourts_off_the_carriageways() {
        let size = 4000.0;
        let site = example::site(size, 10.0);
        // The example's seed and the other seeds the plan's rules are checked on.
        for seed in [
            None,
            Some(0x5EED_0001_u64),
            Some(0x5EED_0042),
            Some(0xC17E_0003),
        ] {
            let mut params = example::params(size);
            if let Some(seed) = seed {
                params.seed = forge_core::Seed::new(seed);
            }
            let city = plan(&site, &params);
            let masses = forge_procgen::city::mass::masses(
                &city,
                &forge_procgen::city::bible::StyleBible::default(),
                7,
            );
            let structures = structure::structures(&city, &masses);
            let mut paths = vec![Vec::new(); city.blocks.len()];
            for g in gardens::gardens(&city, &masses, &structures) {
                if g.kind.paved() {
                    paths[g.block as usize].push(g.outline);
                }
            }
            // Every metre inside each paved piece: on the block's ground it is paved (in the
            // forecourt, not the garden), and none of the paving is on a carriageway (a path's
            // first metres lay over the road, then showed as a patch on the pavement: the owner).
            let (mut samples, mut on_road, mut grass) = (0, 0, 0);
            for (b, block) in city.blocks.iter().enumerate() {
                if paths[b].is_empty() {
                    continue;
                }
                let (front, rest) = city.ground.forecourt(&block.polygon, FORECOURT, &paths[b]);
                for polygon in &paths[b] {
                    let (lo, hi) = polygon.iter().fold(
                        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                        |(lo, hi), p| (lo.min(*p), hi.max(*p)),
                    );
                    let mut y = lo.y + 0.5;
                    while y < hi.y {
                        let mut x = lo.x + 0.5;
                        while x < hi.x {
                            let p = DVec2::new(x, y);
                            if geom::contains(polygon, p) {
                                samples += 1;
                                let paved = ground::inside(&front, p);
                                on_road += usize::from(paved && city.ground.on_carriageway(p));
                                // A sample on the outline itself (within a centimetre) may fall
                                // either side of the overlay's edge.
                                let n = polygon.len();
                                let on_edge = || {
                                    (0..n).any(|k| {
                                        geom::distance_to_segment(
                                            p,
                                            polygon[k],
                                            polygon[(k + 1) % n],
                                        ) < 0.01
                                    })
                                };
                                grass += usize::from(ground::inside(&rest, p) && !on_edge());
                            }
                            x += 1.0;
                        }
                        y += 1.0;
                    }
                }
            }
            println!(
                "seed {seed:?}: {samples} path samples, {on_road} paved on a carriageway, {grass} on grass"
            );
            assert!(samples > 1000, "seed {seed:?}: only {samples} path samples");
            assert_eq!(
                on_road, 0,
                "seed {seed:?}: {on_road} of {samples} on a carriageway"
            );
            assert_eq!(
                grass, 0,
                "seed {seed:?}: {grass} of {samples} left as grass"
            );
        }
    }
}
