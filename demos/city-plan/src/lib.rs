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

use anyhow::Result;
use city_blocks::scenario::{Look, Running, Scenario};
use city_blocks::{Args, CityMaterials, Cooked, scene_origin};
use forge_app::{FlyCamera, Input, Setup};
use forge_core::material::{RenderLayer, TextureId, UvMapping};
use forge_geom::MeshletMesh;
use forge_geom::city::PropSpec;
use forge_geom::procedural::TriMesh;
use forge_geom::{modules, vehicles};
use forge_procgen::city::facades::{self, FaceKind};
use forge_procgen::city::lamps::Lamp;
use forge_procgen::city::mass::{Mass, Roof};
use forge_procgen::city::shore::Shore;
use forge_procgen::city::structure::{self, OpeningKind, Structure, WallRole};
use forge_procgen::city::trees::Tree;
use forge_procgen::city::{BlockUse, CityPlan, Site, geom, ground};
use forge_procgen::grammar::Grammar;
use forge_render::MeshletSceneBuilder;
use forge_render::textures;
use forge_render::{MeshletScene, MoverTransform};
use forge_sim::transit::{Service, Timetable};
use glam::{DVec2, Mat4, Vec3};
use winit::keyboard::KeyCode;

mod crowd;
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
/// Mass sections: four wall tints, the shop band, three roofs.
const WALL: u8 = 0;
const TINTS: u8 = 4;
const SHOP: u8 = 4;
const ROOF_GREEN: u8 = 5;
const ROOF_SOLAR: u8 = 6;
const ROOF_TERRACE: u8 = 7;
/// The kit's merged sections on the masses: four render tints, stone, party walls.
const KIT_PLASTER: u8 = 8;
const KIT_STONE: u8 = 12;
const KIT_PARTY: u8 = 13;
/// The tiled roofs' section on the masses.
const ROOF_TILE: u8 = 14;
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
}

impl Tile {
    fn new(centre: Vec3) -> Self {
        Self {
            centre,
            ground: TriMesh::default(),
            masses: TriMesh::default(),
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
        let size = self.size;
        let tile = self.at(polygon[0]);
        let mesh = if mass {
            &mut tile.masses
        } else {
            &mut tile.ground
        };
        let centre = tile.centre;
        // Whole: ear clipping alone gave up on some of the booleans' outlines, gaps in the ground.
        for t in ground::triangles(polygon) {
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
    /// Each mass's net floor area (D-060), for the people's homes and jobs.
    nets: Vec<f64>,
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
    /// The terraces' seats, for the crowd.
    seats: Vec<(Vec3, f32)>,
    /// The chess tables: each one's middle and its two seats among `seats`.
    games: Vec<(Vec3, [usize; 2])>,
    camera: FlyCamera,
    title: String,
    triangles: usize,
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
        let size = plan.size;
        let mut tiles = Tiles::new(size);
        let shore = ground_shore(site);
        // The reserves' land as scrub and grass, the rest as the open ground's gravel.
        ground_and_water(&mut tiles, &shore);
        // The blocks off the streets, at the walking level as the pavements round them (D-061):
        // one level wherever people walk, no step at a gap between the buildings.
        for block in &plan.blocks {
            let section = match block.use_ {
                BlockUse::Built => GARDEN,
                BlockUse::Park => PARK,
                BlockUse::Square => PAVING,
                // A quay's face holds its river: the land under it is the ground's paving.
                BlockUse::Quay => continue,
            };
            if block.use_ == BlockUse::Built {
                // A paved forecourt along its streets, as the pavement it meets, its gardens
                // behind: grass met the pavement in the gaps between the buildings (the owner).
                let (front, rest) = plan.ground.forecourt(&block.polygon, FORECOURT);
                for polygon in ground::simple_polygons(&front) {
                    tiles.flat(&polygon, WALK_LEVEL, PAVING, false);
                }
                for polygon in ground::simple_polygons(&rest) {
                    tiles.flat(&polygon, WALK_LEVEL, section, false);
                }
            } else {
                for polygon in ground::simple_polygons(&plan.ground.off_streets(&block.polygon)) {
                    tiles.flat(&polygon, WALK_LEVEL, section, false);
                }
            }
            // A large square (1.5 ha or more): a lawn in its middle, 14 m inside its building
            // lines, its paved border under the trees and the benches.
            if block.use_ == BlockUse::Square
                && block.area >= 15_000.0
                && let Some(middle) = geom::inset(&block.buildable, 14.0)
            {
                tiles.flat(&middle, SQUARE_LEVEL + 0.02, PARK, false);
            }
        }
        for polygon in court_paths(plan, masses) {
            tiles.flat(&polygon, WALK_LEVEL + 0.01, PAVING, false);
        }
        // The quays' land as lawns, cell by cell (their faces hold water), under the trees.
        for block in plan.blocks.iter().filter(|b| b.use_ == BlockUse::Quay) {
            lawn(&mut tiles, &shore, &plan.ground, &block.polygon);
        }
        // The rivers as smooth strips at the water's level, the sea's: the cells' staircase
        // would show along the quays. Their promenades are walking ground in the street map
        // (D-061), the banks' walls and balustrades along its edge over the water ([`bridges`]).
        for t in &plan.ground.tiles {
            for polygon in ground::simple_polygons(&plan.ground.water_in(t.min, t.max)) {
                tiles.flat(&polygon, WATER_LEVEL, WATER, false);
            }
        }
        street_ground(&mut tiles, &plan.ground, site);
        bridges(&mut tiles, plan, site, &shore);
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
        // Line by line, each taking the stops the lines before it placed on its track.
        let mut trams: Vec<TramRun> = Vec::new();
        let mut taken: Vec<DVec2> = Vec::new();
        for l in &plan.tram_lines {
            if let Some(run) = TramRun::new(l, plan, site, &taken) {
                taken.extend(run.timetable.stops().iter().map(|&s| run.track(s).0));
                trams.push(run);
            }
        }
        stagger_termini(&mut trams);
        // What each wall looks onto: the kit's faces and where the roofs end in gables.
        let edges = facades::edge_kinds(plan, masses);
        let mut near = Vec::new();
        for (i, m) in masses.iter().enumerate() {
            match detail {
                Some(d) if geom::centroid(&m.footprint).distance(d.centre) < d.radius => {
                    near.push(i);
                }
                _ => mass(&mut tiles, m, &edges[i]),
            }
        }
        // The buildings' structure (D-060): the kit dresses it, the walker walks against it.
        let structures = structure::structures(plan, masses);
        head_houses(&mut tiles, masses, &structures);
        let kit = detail.map(|d| {
            kit(
                &mut tiles,
                masses,
                &structures,
                &edges,
                &near,
                d,
                &plan.ground.bollards,
            )
        });
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
        let triangles = tiles
            .tiles
            .iter()
            .map(|t| t.ground.triangle_count() + t.masses.triangle_count())
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
        let trees: Vec<&Tree> = trees
            .iter()
            .filter(|t| {
                let w = world(size, t.pos, 0.0);
                !stop_grid.any_within(t.pos, 30.0, |k| {
                    let l = stops[k as usize].transform_point3(w);
                    !((-6.0..PLATFORM + 6.0).contains(&l.x) && (-6.0..2.3 + 6.0).contains(&l.z))
                })
            })
            .collect();
        let tree_count = trees.len();
        // The trunks, which the benches keep clear of.
        let mut trunks = forge_procgen::city::geom::PointGrid::new(size, 10.0);
        for t in &trees {
            trunks.insert(t.pos, 0);
        }
        let trees = trees
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let turn = (i as f32 * 2.399_963).rem_euclid(std::f32::consts::TAU);
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
                     {}, derived and assembled in {:.0} ms",
                    near.len(),
                    k.faces,
                    k.terminals,
                    k.instances.len(),
                    k.meshes.len(),
                    instanced / 1000,
                    k.longest_run,
                    k.ms
                );
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
            .collect();
        let bench_spots: Vec<Vec3> = benches.iter().map(|m| m.w_axis.truncate()).collect();
        if !benches.is_empty() {
            kit_meshes.push(forge_geom::furniture::bench());
            let id = (kit_meshes.len() - 1) as u32;
            kit_instances.extend(benches.into_iter().map(|at| (id, at, 0)));
        }
        // Chess tables round the fountain of each square of 0.15 ha or more without a landmark,
        // four 9 m from its middle on the diagonals, where they stand clear of the trunks and the
        // benches and inside the building lines; their players sit as the terraces' guests do,
        // in the share of the people out round them (the owner: give people something to do in
        // the squares rather than stand there).
        let held: Vec<usize> = plan.landmarks.iter().filter_map(|l| l.square).collect();
        let mut chess = Vec::new();
        let mut games = Vec::new();
        for (_, b) in
            plan.blocks.iter().enumerate().filter(|(i, b)| {
                b.use_ == BlockUse::Square && b.area >= 1500.0 && !held.contains(i)
            })
        {
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
        // A fountain in the middle of each other square of 0.15 ha or more.
        let fountains: Vec<Mat4> = plan
            .blocks
            .iter()
            .enumerate()
            .filter(|(i, _)| !hall_squares.contains(i))
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
        let nets: Vec<f64> = structures.iter().map(|s| s.net).collect();
        Self {
            tiles: tiles.tiles,
            trees,
            lamps,
            kit_meshes,
            kit_instances,
            kit_report,
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
                .chain(parapet_pieces(size, plan, site, &shore))
                .collect(),
            passage_note,
            nets: nets.clone(),
            walk_on_start: false,
            ride_after: None,
            games,
            sign_names,
            crowd: None,
            taxis: taxis::Taxis::new(plan, masses, &nets, 8.0, 3600.0),
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
                plan, masses, &self.nets, hour, seats, platforms, timetables, mix,
            )
            .with_games(games),
        );
        // The taxis on the people's hour.
        self.taxis = taxis::Taxis::new(plan, masses, &self.nets, hour, self.clock);
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
/// A built block's paved forecourt along its streets, metres deep.
const FORECOURT: f64 = 2.0;
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
        let fits = |s: f64| {
            s - span >= 0.0
                && s + span <= total
                && junctions.iter().all(|&(j, r)| (s - j).abs() >= r + span)
                && straight(s)
                && on_land(s)
        };
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
                    (0..=160)
                        .flat_map(|k| {
                            let d = f64::from(k) * 2.5;
                            [s - d, s + d]
                        })
                        .find(|&c| fits(c))
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
        (stops.len() >= 2).then(|| Self {
            points,
            along,
            timetable: Timetable::new(&stops, Service::default()),
            colour: line.colour,
        })
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
                // A tram stands with its front at the stop and its body behind.
                let corner = p - along * (length - 2.0) + away * (TRACK_OFFSET + 1.35);
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

    /// The transforms of vehicle `k`'s sections at `time`, in a plan of side `size`: the
    /// front cab, the passenger sections, the rear cab (turned round), each set by its two
    /// ends on the track of the way it runs.
    fn sections(&self, k: usize, time: f64, size: f64) -> [(Vec3, f32); TRAM_SECTIONS] {
        use forge_geom::vehicles::{SECTION_GAP, SECTION_LENGTH};
        let run = self.timetable.at(k, time);
        let (dir, side) = if run.outbound {
            (1.0, TRACK_OFFSET)
        } else {
            (-1.0, -TRACK_OFFSET)
        };
        let on_track = |s: f64| {
            let (p, t) = self.track(s);
            p + DVec2::new(-t.y, t.x) * side
        };
        let pitch = f64::from(SECTION_LENGTH + SECTION_GAP);
        std::array::from_fn(|i| {
            let front = run.s - dir * (pitch * i as f64 + f64::from(SECTION_GAP) * 0.5);
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
fn kit(
    tiles: &mut Tiles,
    masses: &[Mass],
    structures: &[Structure],
    edges: &[Vec<Option<FaceKind>>],
    near: &[usize],
    d: &Detail,
    bollards: &[DVec2],
) -> Kit {
    let start = std::time::Instant::now();
    let size = tiles.size;
    let (dressed, _stats) = facades::dress(d.grammar, structures, masses, near, d.top, d.seed);
    let mut keys: std::collections::HashMap<(u16, i32, i32, i32), u32> =
        std::collections::HashMap::new();
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let mut seats = Vec::new();
    let (mut run, mut longest_run, mut last) = (0, 0, None);
    for piece in &dressed {
        let t = piece.terminal;
        let m = &masses[piece.mass as usize];
        let wall = &structures[piece.mass as usize].walls[piece.wall as usize];
        let street = wall.role == WallRole::Street;
        let name = d.grammar.name(t.module);
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
                t.module,
                (w * 100.0).round() as i32,
                (h * 100.0).round() as i32,
                (reveal * 100.0).round() as i32,
            );
            let id = match keys.get(&key) {
                Some(&id) => id,
                None => {
                    let Some(mesh) = modules::module_deep(name, w, h, reveal) else {
                        continue;
                    };
                    meshes.push(mesh);
                    let id = (meshes.len() - 1) as u32;
                    keys.insert(key, id);
                    id
                }
            };
            let colour = (mix(u64::from(m.lot)) % 4) as u8;
            let spot = mix(u64::from(pos.x.to_bits()) << 32 | u64::from(pos.z.to_bits()))
                ^ mix(u64::from(pos.y.to_bits()));
            let lit = spot % 10 < 4;
            let transform =
                Mat4::from_cols(x.extend(0.0), y.extend(0.0), z.extend(0.0), pos.extend(1.0));
            instances.push((id, transform, colour * 2 + u8::from(lit)));
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
    for &i in near {
        passages(tiles, &structures[i]);
        roof(tiles, &masses[i], &edges[i], KIT_PARTY);
    }
    Kit {
        meshes,
        instances,
        faces: dressed.len(),
        terminals: dressed.len(),
        longest_run,
        seats,
        ms: start.elapsed().as_secs_f64() * 1e3,
    }
}

/// How high a head house stands over its roof, metres: a storey's clear height and its slab.
const HEAD_HOUSE: f64 = 2.7;
/// How far a lift's overrun rises over the roof, or over the head house, metres.
const OVERRUN: f64 = 1.2;

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

/// A box over the outline `ring` (counter-clockwise) from `y0` to `y1`: its walls and its flat
/// top, in `section`.
fn prism(tiles: &mut Tiles, ring: &[DVec2], y0: f64, y1: f64, section: u8) {
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
        triangle_uv(&mut tile.masses, p, [[0.0; 2]; 3], Vec3::Y, ROOF_TERRACE);
    }
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
fn parapet_lines(
    plan: &CityPlan,
    site: &Site,
    shore: &Shore,
) -> Vec<(Vec<DVec2>, Vec<DVec2>, f64)> {
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
    let mut out = Vec::new();
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
            let inner = PARAPET.1;
            out.push((
                vec![a, b],
                vec![a + inward * inner, b + inward * inner],
                WALK_LEVEL,
            ));
        }
    }
    out
}

/// The bridges' stone parapets along both edges, a metre over the pavement, and the deck's side
/// below them down to the water: the pavements stopped at a bare edge over the river (the
/// owner: the bridges' pavements looked broken).
fn bridges(tiles: &mut Tiles, plan: &CityPlan, site: &Site, shore: &Shore) {
    let size = tiles.size;
    for (outer, inner, base) in parapet_lines(plan, site, shore) {
        let top = base + PARAPET.0;
        strip(tiles, &outer, &inner, top, QUAY);
        for (line, other, bottom) in [(&outer, &inner, WATER_LEVEL), (&inner, &outer, base)] {
            for (k, w) in line.windows(2).enumerate() {
                let (a, b) = (w[0], w[1]);
                // Facing away from the other line: out over the water, or in over the pavement.
                let away = (a - other[k]).normalize_or_zero();
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
            }
        }
    }
}

/// The bridges' parapets as the walker's convex pieces, a segment each.
fn parapet_pieces(size: f64, plan: &CityPlan, site: &Site, shore: &Shore) -> Vec<Vec<Vec3>> {
    let mut out = Vec::new();
    for (outer, inner, base) in parapet_lines(plan, site, shore) {
        for k in 0..outer.len().min(inner.len()).saturating_sub(1) {
            out.push(
                [outer[k], outer[k + 1], inner[k + 1], inner[k]]
                    .into_iter()
                    .flat_map(|q| [world(size, q, 0.0), world(size, q, base + PARAPET.0)])
                    .collect(),
            );
        }
    }
    out
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
                .filter(|o| o.wall == w && o.kind == OpeningKind::Passage)
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

/// A wall quad from `y0` to `y1`, its texture repeating every `repeat` bays across (the bays
/// centred on the wall) and every `repeat` floors of `floor` metres up from `y0`.
#[allow(clippy::too_many_arguments)]
fn wall_band(
    mesh: &mut TriMesh,
    (a, b): (Vec3, Vec3),
    length: f64,
    (y0, y1): (f32, f32),
    floor: f32,
    repeat: f32,
    out: Vec3,
    section: u8,
) {
    let bays = (length / BAY).floor().max(1.0);
    let offset = ((length - bays * BAY) * 0.5 / BAY) as f32;
    let (u0, u1) = (-offset / repeat, ((length / BAY) as f32 - offset) / repeat);
    let v = |y: f32| -(y - y0) / floor / repeat;
    let (pa0, pb0) = (a + Vec3::Y * y0, b + Vec3::Y * y0);
    let (pa1, pb1) = (a + Vec3::Y * y1, b + Vec3::Y * y1);
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

/// A mass: its walls (a shop band on the ground floor where the street is lively, window bays
/// above, one of four tints by its lot) and its roof, `edges` saying what each wall looks onto.
fn mass(tiles: &mut Tiles, m: &Mass, edges: &[Option<FaceKind>]) {
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
        let cells = textures::FACADE_CELLS as f32;
        // The ground storey is one cell, stretched to its height: at 3.2 m a cell, its 4.2 m showed
        // a row of window stubs under the first floor, cut where the upper band starts again
        // (the owner's "bugged windows" on the first floor).
        let (lower, floor, repeat) = if shops {
            (SHOP, ground, 1.0)
        } else {
            (WALL + tint, ground, cells)
        };
        let base = (0.0, ground.min(top));
        wall_band(mesh, ends, length, base, floor, repeat, out, lower);
        if top > ground {
            let band = (ground, top);
            wall_band(
                mesh,
                ends,
                length,
                band,
                m.upper() as f32,
                cells,
                out,
                WALL + tint,
            );
        }
    }
    roof(tiles, m, edges, KIT_PLASTER + tint);
}

/// A mass's roof: green, solar, a terrace, or pitched and tiled, its gables in `gable`.
fn roof(tiles: &mut Tiles, m: &Mass, edges: &[Option<FaceKind>], gable: u8) {
    let size = tiles.size;
    let tile = tiles.at(m.footprint[0]);
    let centre = tile.centre;
    let section = match m.roof {
        Roof::Green => ROOF_GREEN,
        Roof::Solar => ROOF_SOLAR,
        Roof::Terrace => ROOF_TERRACE,
        Roof::Tiled => {
            if pitched(&mut tile.masses, m, edges, gable, size, centre) {
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

/// The longest wall a gable stands on, metres: about a building's depth. Longer walls slope.
const GABLE_MAX: f64 = 18.0;
/// The longest run of a pitched roof from its eaves, metres (a rise of 4.4 m); a roof that
/// would rise further stops in a terrace.
const RUN_MAX: f64 = 8.0;

/// Which edges of a footprint slope (1) and which end in a gable (0): a wall onto a neighbour
/// or within the building, across the main front (its longest street wall, else its longest
/// court wall) and at most [`GABLE_MAX`] long, carries a gable, so the houses of a row share
/// one ridge along the street. The rest slope; with no gable the roof is hipped. `None` with
/// no front.
fn pitches(fp: &[DVec2], edges: &[Option<FaceKind>]) -> Option<Vec<f64>> {
    let n = fp.len();
    let edge = |i: usize| fp[(i + 1) % n] - fp[i];
    let open = |i: usize| matches!(edges[i], Some(FaceKind::Street | FaceKind::Court));
    let longest = |kind: FaceKind| {
        (0..n)
            .filter(|&i| edges[i] == Some(kind))
            .max_by(|&a, &b| edge(a).length().total_cmp(&edge(b).length()))
    };
    let front = longest(FaceKind::Street).or_else(|| longest(FaceKind::Court))?;
    let along = edge(front).normalize_or_zero();
    Some(
        (0..n)
            .map(|i| {
                let across = edge(i).normalize_or_zero().dot(along).abs() < 0.5;
                let length = edge(i).length();
                if !open(i) && (0.5..=GABLE_MAX).contains(&length) && across {
                    0.0
                } else {
                    1.0
                }
            })
            .collect(),
    )
}

/// The outline `fp` with each edge `i` moved in by `w[i] * d`: a pitched roof's top at the run
/// `d`. `None` once an edge turns over or two cross (past the ridge).
fn ridge(fp: &[DVec2], w: &[f64], d: f64) -> Option<Vec<DVec2>> {
    let n = fp.len();
    let line = |i: usize| {
        let t = (fp[(i + 1) % n] - fp[i]).normalize_or_zero();
        (fp[i] + DVec2::new(-t.y, t.x) * (w[i] * d), t)
    };
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let prev = (i + n - 1) % n;
        let ((p, t), (q, u)) = (line(prev), line(i));
        let denom = geom::cross(t, u);
        if denom.abs() < 1e-9 {
            // Edges in line meet only where they move alike.
            if w[i] != w[prev] {
                return None;
            }
            out.push(q);
            continue;
        }
        out.push(p + t * (geom::cross(q - p, u) / denom));
    }
    let turned = (0..n).any(|i| (out[(i + 1) % n] - out[i]).dot(line(i).1) <= 1e-6);
    let crossing = (0..n).any(|i| {
        ((i + 2)..n).any(|j| {
            (j + 1) % n != i
                && geom::segment_intersection(out[i], out[(i + 1) % n], out[j], out[(j + 1) % n])
                    .is_some()
        })
    });
    (!turned && !crossing).then_some(out)
}

/// A pitched roof over a mass at about 29°: the sloping edges ([`pitches`]) moved in as far as
/// they go to the ridge (the straight skeleton's roof for a convex footprint, nearly) or
/// [`RUN_MAX`], the gable ends standing up from their walls in `gable`. False where the
/// footprint will not take one; it then gets a flat roof.
fn pitched(
    mesh: &mut TriMesh,
    m: &Mass,
    edges: &[Option<FaceKind>],
    gable: u8,
    size: f64,
    centre: Vec3,
) -> bool {
    let fp = &m.footprint;
    let n = fp.len();
    let Some(w) = pitches(fp, edges) else {
        return false;
    };
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
    let at = |p: DVec2, h: f64| world(size, p, h) - centre;
    for i in 0..n {
        let j = (i + 1) % n;
        let (a, b) = (at(fp[i], m.height), at(fp[j], m.height));
        let (c, e) = (at(ridge[j], m.height + rise), at(ridge[i], m.height + rise));
        let (normal, section) = if w[i] == 0.0 {
            // A gable: the wall carried up, facing out.
            let t = (fp[j] - fp[i]).normalize_or_zero();
            (Vec3::new(t.y as f32, 0.0, -t.x as f32), gable)
        } else {
            let normal = (b - a).cross(c - a).normalize_or_zero();
            (normal * normal.y.signum(), ROOF_TILE)
        };
        triangle_uv(mesh, [a, b, c], [[0.0; 2]; 3], normal, section);
        triangle_uv(mesh, [a, c, e], [[0.0; 2]; 3], normal, section);
    }
    for t in geom::triangulate(&ridge) {
        let p = t.map(|i| at(ridge[i], m.height + rise));
        triangle_uv(mesh, p, [[0.0; 2]; 3], Vec3::Y, top);
    }
    true
}

impl Scenario for PlanScenario {
    fn props(&self) -> Vec<PropSpec> {
        Vec::new()
    }

    fn look(&self) -> Look {
        Look {
            // An afternoon sun from the south-west.
            sun: Some(Vec3::new(-0.45, 0.55, 0.7)),
            ..Look::default()
        }
    }

    fn liquid(&self) -> Option<city_blocks::scenario::LiquidSetup> {
        None
    }

    fn camera(&self) -> FlyCamera {
        self.camera.clone()
    }

    fn build(
        mut self: Box<Self>,
        ctx: &Setup,
        args: &Args,
        _cooked: Cooked,
    ) -> Result<(MeshletScene, Box<dyn Running>)> {
        let start = std::time::Instant::now();
        let mut materials = CityMaterials::new(&ctx.device)?;
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
        let textured = |layer: RenderLayer,
                        (albedo, normal): (TextureId, TextureId),
                        scale: f32,
                        stochastic: bool| RenderLayer {
            albedo_texture: Some(albedo),
            normal_texture: Some(normal),
            texture_scale: scale,
            normal_strength: 1.0,
            hex_tiling: stochastic,
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
            ],
        );
        // The facades: a window bay and a shopfront, read by the walls' coordinates.
        let [facade_albedo, facade_normal, facade_lights] = textures::facade(21, 1024);
        let [shop_albedo, shop_normal] = textures::shopfront(22, 512);
        let shop_lights = textures::shopfront_lights(512);
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
        // them out white, the owner found the lights too strong and all alike). The shading is per unit of the frame's reference
        // illuminance (D-046: the sun's 128 klux by day, the Moon's tenths of a lux at night), so
        // an emissive of L·π / reference gives L. A stopgap for a run held at one time of day
        // (`--time-of-day`): step 5's lights give the renderer emissive light in absolute units.
        let sun = forge_render::starfield::SUN_ILLUMINANCE_1AU;
        let reference = args.time_of_day.map_or(sun, |t| {
            let mut celestial = forge_render::night::Celestial::default();
            if let Some(age) = args.moon_age {
                celestial.moon_age = age;
            }
            forge_render::night::NightLights::new(&celestial.at(t, sun), sun).reference
        });
        let unit = std::f32::consts::PI / reference;
        let glow = 5.0 * unit;
        // The textured boxes beyond the kit's reach glow at 15 % of it. Their windows light as
        // the kit's do, but at the horizon facade stacks on facade with no dark roof or street
        // between, and the far city read as a band of white (the owner, 2026-10-11). A stopgap
        // until a proxy per building (#38) draws the far city as the kit would.
        let far_glow = glow * 0.15;
        let mapped =
            |rgb: [f32; 3], (albedo, normal, lights): (TextureId, TextureId, TextureId)| {
                RenderLayer {
                    albedo_texture: Some(albedo),
                    normal_texture: Some(normal),
                    normal_strength: 1.0,
                    emissive: [far_glow; 3],
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
        let tiles_row = RenderLayer {
            albedo_texture: Some(materials.textures.add(&tile_albedo)?),
            normal_texture: Some(materials.textures.add(&tile_normal)?),
            texture_scale: 2.5,
            normal_strength: 0.8,
            ..colour([1.75, 2.0, 2.1], 10.0, 0.04)
        };
        materials.add_rows(
            "plan-mass",
            vec![
                // The render tints of the palette: sand, ochre, pale rose, off-white.
                (
                    "plan wall (sand)".into(),
                    mapped([0.94, 0.88, 0.76], facade),
                ),
                (
                    "plan wall (ochre)".into(),
                    mapped([0.92, 0.80, 0.62], facade),
                ),
                (
                    "plan wall (rose)".into(),
                    mapped([0.90, 0.76, 0.68], facade),
                ),
                (
                    "plan wall (white)".into(),
                    mapped([0.95, 0.93, 0.88], facade),
                ),
                ("plan shopfront".into(), mapped([1.0, 1.0, 1.0], shop)),
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
            emissive: light.map_or([0.0; 3], |(l, c)| c.map(|v| v * l * unit)),
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
                    (
                        format!("{group} glass"),
                        glass((k % 2 == 1).then_some(LIT[k / 2])),
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
        // Lamp heads of about 1 000 cd/m²: shielded LEDs at 2 700 K behind a frosted diffuser (the
        // style bible's lighting). At 3 000 their halos read too strong (the owner).
        let head = 1000.0 * unit;
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
                        ..colour([0.8, 0.8, 0.78], 40.0, 0.2)
                    },
                ),
            ],
        );
        let lamp_row = materials.of("plan-lamp");
        let ground_row = materials.of("plan-ground");
        let mass_row = materials.of("plan-mass");
        let mut builder = MeshletSceneBuilder::new();
        materials.apply(&mut builder, &[], &[]);
        builder.set_ray_traced(!args.no_shadows);
        builder.set_origin(scene_origin(args));
        let mut meshes = 0;
        for tile in &self.tiles {
            let at = Mat4::from_translation(tile.centre);
            for (mesh, row) in [(&tile.ground, ground_row), (&tile.masses, mass_row)] {
                if mesh.indices.is_empty() {
                    continue;
                }
                // Normals weighed in the LOD: a pavement's kerb and back faces folded into its
                // top at the coarser levels, long tilted triangles where the paving streaked (the
                // owner saw two patterns blend on the pavements).
                let id = builder.add_mesh(&MeshletMesh::build_with(
                    mesh,
                    forge_geom::meshlet::CookOptions { normal_weight: 2.0 },
                ));
                builder.add_instance_with_material(id, at, row);
                meshes += 1;
            }
        }
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
        let kit_ids: Vec<_> = self
            .kit_meshes
            .iter()
            .map(|m| builder.add_mesh(&MeshletMesh::build(m)))
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
        // The trams: a cab and a passenger section's meshes a line, in its livery; their
        // instances are movers, the table's last.
        let mut tram_movers = Vec::new();
        for (i, run) in self.trams.iter().enumerate() {
            let row = tram_rows[i % tram_rows.len()];
            let cab = builder.add_mesh(&MeshletMesh::build(&vehicles::tram_section(true)));
            let middle = builder.add_mesh(&MeshletMesh::build(&vehicles::tram_section(false)));
            builder.set_mesh_material(cab, row);
            builder.set_mesh_material(middle, row);
            let n = run.timetable.vehicles() as u32;
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
            tracing::info!(taxis = t.len(), "the taxi fleet");
            let cab = builder.add_mesh(&MeshletMesh::build(&vehicles::taxi_cab()));
            builder.set_mesh_material(cab, materials_taxi);
            tram_movers.push((cab, t.len() as u32));
        }
        if !tram_movers.is_empty() {
            builder.reserve_movers(&tram_movers);
        }
        let mut scene = builder.build(&ctx.device)?;
        scene.build_tlas(&ctx.device, &ctx.shaders)?;
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
            taxis.advance(self.clock);
            // Every five minutes of the service, how the taxis keep up.
            if before.floor() != (self.clock / 300.0).floor() {
                let (waiting, wait) = taxis.service();
                tracing::info!(
                    taxis = taxis.len(),
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
            let vehicles: Vec<_> = (0..run.timetable.vehicles())
                .map(|k| run.sections(k, self.clock, self.size))
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
            for figures in crowd.figures(self.camera, self.clock) {
                for figure in figures {
                    still.push(!figure.is_some_and(|f| f.kept));
                    transforms.push(place(figure.map_or(parked, |f| (f.at, f.turn))));
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
                let Some(wait) = self.trams[s.line]
                    .timetable
                    .next_arrival(s.stop, s.outbound, self.clock)
                else {
                    continue;
                };
                let row = if wait < 30.0 {
                    0
                } else {
                    ((wait / 60.0).ceil() as usize).min(signs::ROWS - 1)
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
            for (p, yaw) in taxis.poses() {
                still.push(false);
                transforms.push(place((taxis::world(self.size, p), yaw)));
            }
        }
        scene.set_movers_still(&transforms, |k| still.get(k).copied().unwrap_or(false));
    }

    fn title(&mut self) -> String {
        self.title.clone()
    }
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

/// The paths across the open blocks' greens: from each gap between the buildings (a street lot
/// left open) to the court's middle, paved, 2.4 m wide (the owner's photos of open blocks: a
/// shared green with paths through it). Off the streets only: the block's outline runs down the
/// street's middle, and the paths' first metres lay over the carriageway (the owner).
fn court_paths(plan: &CityPlan, masses: &[Mass]) -> Vec<Vec<DVec2>> {
    let built: std::collections::HashSet<u32> = masses.iter().map(|m| m.lot).collect();
    let mut paths = Vec::new();
    for (i, lot) in plan.lots.iter().enumerate() {
        let block = &plan.blocks[lot.block as usize];
        let Some(court) = &block.courtyard else {
            continue;
        };
        if !lot.frontage || built.contains(&(i as u32)) || block.use_ != BlockUse::Built {
            continue;
        }
        // From the block's edge nearest the gap's middle to the court's.
        let middle = geom::centroid(&lot.polygon);
        let n = block.polygon.len();
        let from = (0..n)
            .map(|k| {
                let (a, b) = (block.polygon[k], block.polygon[(k + 1) % n]);
                a + (b - a) * geom::closest_on_segment(middle, a, b)
            })
            .min_by(|p, q| p.distance(middle).total_cmp(&q.distance(middle)));
        let Some(from) = from else {
            continue;
        };
        let line = geom::resample(&[from, geom::centroid(court)], 2.0);
        if line.len() < 2 {
            continue;
        }
        let mut outline = offset_line(&line, 1.2);
        outline.extend(offset_line(&line, -1.2).into_iter().rev());
        paths.extend(ground::simple_polygons(&plan.ground.off_streets(&outline)));
    }
    paths
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
    for tri in icosphere() {
        let p = tri.map(place);
        smooth_triangle(mesh, p.map(|q| q.0), p.map(|q| q.1), 1);
    }
}

/// An icosahedron's faces, each cut in four: 80 triangles of unit directions.
fn icosphere() -> Vec<[Vec3; 3]> {
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
    let mut out = Vec::with_capacity(80);
    for f in faces {
        let (a, b, c) = (v[f[0]], v[f[1]], v[f[2]]);
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

    #[test]
    fn the_court_paths_stay_off_the_carriageways() {
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
            let paths = court_paths(&city, &masses);
            // Every half metre inside each path: none on a carriageway (a path's first metres
            // lay over the road, the owner).
            let (mut samples, mut on_road) = (0, 0);
            for polygon in &paths {
                let (lo, hi) = polygon.iter().fold(
                    (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                    |(lo, hi), p| (lo.min(*p), hi.max(*p)),
                );
                let mut y = lo.y + 0.25;
                while y < hi.y {
                    let mut x = lo.x + 0.25;
                    while x < hi.x {
                        let p = DVec2::new(x, y);
                        if geom::contains(polygon, p) {
                            samples += 1;
                            on_road += usize::from(city.ground.on_carriageway(p));
                        }
                        x += 0.5;
                    }
                    y += 0.5;
                }
            }
            println!(
                "seed {seed:?}: {} court paths, {samples} samples, {on_road} on a carriageway",
                paths.len()
            );
            assert_eq!(
                on_road, 0,
                "seed {seed:?}: {on_road} of {samples} path samples on a carriageway"
            );
        }
    }
}
