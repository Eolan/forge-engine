//! `city-plan`: a city's plan in 3D (D-059's step 1 seen from the street, before step 2's
//! modules). Any [`CityPlan`] from `forge_procgen::city` and its masses become meshes in tiles
//! of [`TILE`] metres, drawn by `city-blocks`' renderer through its scene interface (#216), with
//! its sky, sun, ray-traced shadows, probes and anti-aliasing:
//! - the ground from the site's mask: land, and water a little below it;
//! - gardens and courtyards, parks and squares from the blocks;
//! - streets as ribbons of their profile's width, tram tracks along the lines;
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
use forge_procgen::city::mass::{FLOOR, GROUND_FLOOR, Mass, Roof};
use forge_procgen::city::trees::Tree;
use forge_procgen::city::{BlockUse, CityPlan, LAGOON, SEA, Site, geom};
use forge_procgen::grammar::Grammar;
use forge_render::MeshletSceneBuilder;
use forge_render::textures;
use forge_render::{MeshletScene, MoverTransform};
use forge_sim::transit::{Service, Timetable};
use glam::{DVec2, Mat4, Vec3};
use winit::keyboard::KeyCode;

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
        for t in geom::triangulate(polygon) {
            let p = t.map(|i| world(size, polygon[i], height) - centre);
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
    lamps: Vec<(Vec3, f32, f32)>,
    /// The style kit's instanced modules and their instances (module, transform, group).
    kit_meshes: Vec<TriMesh>,
    kit_instances: Vec<(u32, Mat4, u8)>,
    /// The tram lines' runs, and the service's clock at the start, seconds.
    trams: Vec<TramRun>,
    clock: f64,
    size: f64,
    /// What the kit built, logged once the app logs.
    kit_report: Option<String>,
    camera: FlyCamera,
    title: String,
    triangles: usize,
}

/// Buildings drawn from a style set's grammar and kit (D-039's modules) within `radius` of
/// `centre`; the masses beyond stay walls with facade textures.
pub struct Detail<'a> {
    /// The style set's rules.
    pub grammar: &'a Grammar,
    /// Their start shape's rule.
    pub start: &'a str,
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
        ground_and_water(site, &mut tiles);
        for block in &plan.blocks {
            let section = match block.use_ {
                BlockUse::Built => GARDEN,
                BlockUse::Park => PARK,
                BlockUse::Square => PAVING,
                // A quay's face holds its river: the land under it is the ground's paving.
                BlockUse::Quay => continue,
            };
            tiles.flat(&block.polygon, 0.08, section, false);
        }
        // The rivers as smooth strips over the ground, their beds drawn as land: the cells'
        // staircase would show along the quays.
        for river in &site.rivers {
            let line = geom::resample(&river.centreline, 15.0);
            let half = river.width * 0.5;
            strip(
                &mut tiles,
                &offset_line(&line, half),
                &offset_line(&line, -half),
                0.04,
                WATER,
            );
        }
        for e in &plan.roads.edges {
            // Asphalt on the avenues' carriageways; stone on shared streets, lanes and paths.
            let section = match e.profile {
                forge_procgen::city::Profile::TramAvenue | forge_procgen::city::Profile::Avenue => {
                    STREET
                }
                _ => PAVING,
            };
            ribbon(&mut tiles, &e.points, e.profile.width(), 0.16, section);
        }
        pavements(&mut tiles, plan);
        // Two tracks a line, one each way, on the right of the way they run.
        for line in &plan.tram_lines {
            for track in [-TRACK_OFFSET, TRACK_OFFSET] {
                for rail in [-0.72, 0.72] {
                    let rail = offset_line(&line.points, track + rail);
                    ribbon(&mut tiles, &rail, 0.25, 0.22, TRACK);
                }
            }
        }
        let trams: Vec<TramRun> = plan.tram_lines.iter().filter_map(TramRun::new).collect();
        let mut near = Vec::new();
        for (i, m) in masses.iter().enumerate() {
            match detail {
                Some(d) if geom::centroid(&m.footprint).distance(d.centre) < d.radius => {
                    near.push(i);
                }
                _ => mass(&mut tiles, m),
            }
        }
        let kit = detail.map(|d| kit(&mut tiles, plan, masses, &near, d));
        let triangles = tiles
            .tiles
            .iter()
            .map(|t| t.ground.triangle_count() + t.masses.triangle_count())
            .sum();
        let st = &plan.stats;
        let tree_count = trees.len();
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
        let lamps = lamps
            .iter()
            .map(|l| {
                let turn = (-l.facing.x).atan2(-l.facing.y) as f32;
                (world(size, l.pos, 0.0), (l.height / 8.0) as f32, turn)
            })
            .collect();
        let (kit_meshes, kit_instances, kit_report) =
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
                (k.meshes, k.instances, Some(report))
            });
        Self {
            tiles: tiles.tiles,
            trees,
            lamps,
            kit_meshes,
            kit_instances,
            kit_report,
            trams,
            clock: 3600.0,
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

/// How far each of a line's two tracks lies from its centreline, metres.
const TRACK_OFFSET: f64 = 1.6;
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
#[derive(Clone)]
struct TramRun {
    points: Vec<DVec2>,
    /// Distance along `points` at each point, metres.
    along: Vec<f64>,
    timetable: Timetable,
    colour: [u8; 3],
}

impl TramRun {
    /// A line's runs, its stops at their distances along it; `None` with fewer than two.
    fn new(line: &forge_procgen::city::TramLine) -> Option<Self> {
        let points = line.points.clone();
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along[along.len() - 1] + w[0].distance(w[1]));
        }
        // Each stop at the distance of its nearest point on the route, 60 m apart at least.
        let mut stops: Vec<f64> = line
            .stops
            .iter()
            .map(|&stop| {
                let mut best = (f64::MAX, 0.0);
                for (i, w) in points.windows(2).enumerate() {
                    let t = geom::closest_on_segment(stop, w[0], w[1]);
                    let q = w[0] + (w[1] - w[0]) * t;
                    let d = q.distance(stop);
                    if d < best.0 {
                        best = (d, along[i] + w[0].distance(q));
                    }
                }
                best.1
            })
            .collect();
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

/// The masses `near` built from `d`'s grammar and kit: each face derived, the walls, bands and
/// cornices merged into the tiles, the openings instanced; and their roofs.
fn kit(tiles: &mut Tiles, plan: &CityPlan, masses: &[Mass], near: &[usize], d: &Detail) -> Kit {
    let start = std::time::Instant::now();
    let size = tiles.size;
    let wanted: std::collections::HashSet<u32> = near.iter().map(|&i| i as u32).collect();
    let faces: Vec<facades::Face> = facades::faces(plan, masses)
        .into_iter()
        .filter(|f| wanted.contains(&f.mass))
        .collect();
    let (terminals, _stats) = facades::derive(d.grammar, d.start, &faces, masses, d.top, d.seed);
    let mut keys: std::collections::HashMap<(u16, i32, i32), u32> =
        std::collections::HashMap::new();
    let mut meshes = Vec::new();
    let mut instances = Vec::new();
    let (mut run, mut longest_run, mut last) = (0, 0, None);
    for &(f, t) in &terminals {
        let face = &faces[f as usize];
        let m = &masses[face.mass as usize];
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
            let key = (
                t.module,
                (w * 100.0).round() as i32,
                (h * 100.0).round() as i32,
            );
            let id = match keys.get(&key) {
                Some(&id) => id,
                None => {
                    let Some(mesh) = modules::module(name, w, h) else {
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
            // Runs of one opening along a floor of a street front.
            if face.kind == FaceKind::Street {
                let row = (f, (o.y * 10.0).round() as i64);
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
        roof(tiles, &masses[i]);
    }
    Kit {
        meshes,
        instances,
        faces: faces.len(),
        terminals: terminals.len(),
        longest_run,
        ms: start.elapsed().as_secs_f64() * 1e3,
    }
}

/// The site's land and water as runs of cells along each row, in the tiles the runs start in
/// (a run is cut at tile edges).
fn ground_and_water(site: &Site, tiles: &mut Tiles) {
    let water = &site.water;
    let s = water.spacing;
    let n = water.size;
    let per_tile = (TILE / s).round().max(1.0) as u32;
    for y in 0..n {
        let mut x = 0;
        while x < n {
            let wet = is_open_water(water.get(x, y));
            let start = x;
            let tile_end = ((x / per_tile) + 1) * per_tile;
            while x < n && x < tile_end && is_open_water(water.get(x, y)) == wet {
                x += 1;
            }
            let (x0, x1) = (start as f64 * s, x as f64 * s);
            let (y0, y1) = (y as f64 * s, (y + 1) as f64 * s);
            let quad = [
                DVec2::new(x0, y0),
                DVec2::new(x1, y0),
                DVec2::new(x1, y1),
                DVec2::new(x0, y1),
            ];
            if wet {
                tiles.flat(&quad, -0.6, WATER, false);
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
/// above, one of four tints by its lot) and its roof.
fn mass(tiles: &mut Tiles, m: &Mass) {
    let size = tiles.size;
    let tile = tiles.at(m.footprint[0]);
    let centre = tile.centre;
    let mesh = &mut tile.masses;
    let n = m.footprint.len();
    let tint = tint_of(m.lot);
    // Shops along avenues and in pedestrian lanes.
    let shops = m.street_width >= 20.0 || m.street_width <= 8.0;
    let ground = GROUND_FLOOR as f32;
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
        let (lower, floor, repeat) = if shops {
            (SHOP, ground, 1.0)
        } else {
            (WALL + tint, FLOOR as f32, cells)
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
                FLOOR as f32,
                cells,
                out,
                WALL + tint,
            );
        }
    }
    roof(tiles, m);
}

/// A mass's roof: green, solar or a terrace.
fn roof(tiles: &mut Tiles, m: &Mass) {
    let size = tiles.size;
    let tile = tiles.at(m.footprint[0]);
    let centre = tile.centre;
    let section = match m.roof {
        Roof::Green => ROOF_GREEN,
        Roof::Solar => ROOF_SOLAR,
        Roof::Terrace => ROOF_TERRACE,
    };
    for t in geom::triangulate(&m.footprint) {
        let p = t.map(|i| world(size, m.footprint[i], m.height) - centre);
        triangle_uv(&mut tile.masses, p, [[0.0; 2]; 3], Vec3::Y, section);
    }
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
        self: Box<Self>,
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
        let gravel = ground(textures::gravel(31, 512))?;
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
                    textured(colour([1.9, 1.8, 1.65], 12.0, 0.03), gravel, 1.25, true),
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
                ("plan track".into(), colour([0.16, 0.16, 0.17], 60.0, 0.3)),
                ("plan water".into(), colour([0.05, 0.12, 0.15], 900.0, 0.9)),
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
        // Lit windows of about 12 cd/m² (behind curtains, seen from the street). The shading is per unit of the frame's reference
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
        let glow = 12.0 * std::f32::consts::PI / reference;
        let mapped =
            |rgb: [f32; 3], (albedo, normal, lights): (TextureId, TextureId, TextureId)| {
                RenderLayer {
                    albedo_texture: Some(albedo),
                    normal_texture: Some(normal),
                    normal_strength: 1.0,
                    emissive: [glow; 3],
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
        let glass = |lit: bool| RenderLayer {
            emissive: if lit {
                // The same room as the facade texture's lit windows, warm.
                [glow, glow * 0.78, glow * 0.5]
            } else {
                [0.0; 3]
            },
            reflectance: 0.06,
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
                    (format!("{group} glass"), glass(k % 2 == 1)),
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
        let kit_rows: Vec<_> = KIT_GROUPS.iter().map(|g| materials.of(g)).collect();
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
                    (format!("{group} glass"), glass(false)),
                    (format!("{group} stripe"), colour(stripe, 60.0, 0.25)),
                    (
                        format!("{group} running gear"),
                        colour([0.07, 0.07, 0.08], 30.0, 0.1),
                    ),
                ],
            );
        }
        let tram_rows: Vec<_> = TRAM_GROUPS.iter().map(|g| materials.of(g)).collect();
        let trunk = colour([0.20, 0.15, 0.11], 8.0, 0.02);
        materials.add_rows(
            "plan-tree-a",
            vec![
                ("plan trunk".into(), trunk),
                ("plan leaves".into(), colour([0.10, 0.22, 0.07], 6.0, 0.02)),
            ],
        );
        materials.add_rows(
            "plan-tree-b",
            vec![
                ("plan trunk (b)".into(), trunk),
                (
                    "plan leaves (b)".into(),
                    colour([0.15, 0.25, 0.08], 6.0, 0.02),
                ),
            ],
        );
        let tree_rows = [materials.of("plan-tree-a"), materials.of("plan-tree-b")];
        // Lamp heads of about 3 000 cd/m², shielded LEDs at 2 700 K (the style bible's lighting).
        let head = 3000.0 * std::f32::consts::PI / reference;
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
                let id = builder.add_mesh(&MeshletMesh::build(mesh));
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
            let model = builder.add_mesh(&MeshletMesh::build(&lamp_model()));
            for &(at, scale, turn) in &self.lamps {
                let transform = Mat4::from_scale_rotation_translation(
                    Vec3::splat(scale),
                    glam::Quat::from_rotation_y(turn),
                    at,
                );
                builder.add_instance_with_material(model, transform, lamp_row);
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
        Ok((
            scene,
            Box::new(PlanRunning {
                title: self.title.clone(),
                trams: self.trams.clone(),
                clock: self.clock,
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
    /// The service's clock, seconds.
    clock: f64,
    size: f64,
    /// The scene's origin, which the movers' positions are relative to.
    origin: Vec3,
}

impl Running for PlanRunning {
    fn key_pressed(&mut self, _code: KeyCode, _camera: &FlyCamera) -> bool {
        false
    }

    fn update(
        &mut self,
        _args: &Args,
        _input: &Input,
        _camera: &FlyCamera,
        _frame: u64,
        _dt: f32,
        step: f32,
    ) -> Option<f64> {
        self.clock += f64::from(step);
        None
    }

    fn player(&mut self) -> Option<Vec3> {
        None
    }

    fn follow(&mut self, _camera: &mut FlyCamera) {}

    fn movers(&mut self, scene: &mut MeshletScene) {
        if self.trams.is_empty() {
            return;
        }
        // In the order the movers were reserved: a line's cabs (front and rear of each
        // vehicle), then its passenger sections.
        let mut transforms = Vec::new();
        let place = |(at, yaw): (Vec3, f32)| MoverTransform {
            position: at - self.origin,
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
        scene.set_movers(&transforms);
    }

    fn title(&mut self) -> String {
        self.title.clone()
    }
}

/// Whether a water code is the sea or a lagoon, drawn from the site's cells (rivers are drawn as
/// strips).
fn is_open_water(code: u8) -> bool {
    code == SEA || code == LAGOON
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

/// Raised pavements with kerbs along the avenues (shared streets, lanes and paths are level
/// surfaces): each side a strip of the profile's pavement 14 cm over the carriageway and a kerb
/// face towards it, cut back at each end by the widest street meeting there.
fn pavements(tiles: &mut Tiles, plan: &CityPlan) {
    use forge_procgen::city::Profile;
    let size = tiles.size;
    let mut widest = vec![0.0_f64; plan.roads.nodes.len()];
    for e in &plan.roads.edges {
        let half = e.profile.width() * 0.5;
        widest[e.a as usize] = widest[e.a as usize].max(half);
        widest[e.b as usize] = widest[e.b as usize].max(half);
    }
    for e in &plan.roads.edges {
        let pavement = match e.profile {
            Profile::TramAvenue => 5.0,
            Profile::Avenue => 4.5,
            _ => continue,
        };
        let length = e.length();
        let (start, end) = (
            widest[e.a as usize] + 1.0,
            length - widest[e.b as usize] - 1.0,
        );
        if end - start < 4.0 {
            continue;
        }
        let steps = ((end - start) / 4.0).ceil() as usize;
        let line: Vec<DVec2> = (0..=steps)
            .map(|i| geom::point_at(&e.points, start + (end - start) * i as f64 / steps as f64))
            .collect();
        let half = e.profile.width() * 0.5;
        for side in [1.0, -1.0] {
            let outer = offset_line(&line, side * half);
            let inner = offset_line(&line, side * (half - pavement));
            let (left, right) = if side > 0.0 {
                (&outer, &inner)
            } else {
                (&inner, &outer)
            };
            strip(tiles, left, right, 0.30, PAVING);
            // The kerb: from the carriageway's top up to the pavement's, facing the road.
            for w in inner.windows(2) {
                let tile = tiles.at(w[0]);
                let centre = tile.centre;
                let (a, b) = (w[0], w[1]);
                let t = (b - a).normalize_or_zero();
                let towards = DVec2::new(-t.y, t.x) * -side;
                let normal = Vec3::new(towards.x as f32, 0.0, towards.y as f32);
                let p = |q: DVec2, h: f64| world(size, q, h) - centre;
                triangle(
                    &mut tile.ground,
                    [p(a, 0.16), p(b, 0.16), p(b, 0.30)],
                    normal,
                    PAVING,
                );
                triangle(
                    &mut tile.ground,
                    [p(a, 0.16), p(b, 0.30), p(a, 0.30)],
                    normal,
                    PAVING,
                );
            }
        }
    }
}

/// A street lamp 8 m tall: a hexagonal pole and an arm reaching 1.4 m along −z (section 0), and a
/// lamp head under the arm's end (section 1, emissive).
fn lamp_model() -> TriMesh {
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
    boxed(
        &mut mesh,
        Vec3::new(-0.04, 7.85, -1.45),
        Vec3::new(0.04, 7.95, 0.0),
        0,
    );
    boxed(
        &mut mesh,
        Vec3::new(-0.14, 7.70, -1.70),
        Vec3::new(0.14, 7.85, -1.10),
        1,
    );
    mesh
}
