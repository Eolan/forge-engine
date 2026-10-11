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
use forge_core::material::RenderLayer;
use forge_geom::MeshletMesh;
use forge_geom::city::PropSpec;
use forge_geom::procedural::TriMesh;
use forge_procgen::city::mass::{Mass, Roof};
use forge_procgen::city::trees::Tree;
use forge_procgen::city::{BlockUse, CityPlan, LAGOON, SEA, Site, geom};
use forge_render::MeshletScene;
use forge_render::MeshletSceneBuilder;
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
/// Mass sections.
const WALL: u8 = 0;
const ROOF_GREEN: u8 = 1;
const ROOF_SOLAR: u8 = 2;
const ROOF_TERRACE: u8 = 3;

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
    /// Each tree's place in the world, its scale (its height over the model's 10 m), its turn and
    /// which of the two models it uses.
    trees: Vec<(Vec3, f32, f32, usize)>,
    camera: FlyCamera,
    title: String,
    triangles: usize,
}

impl PlanScenario {
    /// The scene of `plan` on `site` with its `masses` and `trees`, the camera starting at
    /// `camera`.
    pub fn new(
        site: &Site,
        plan: &CityPlan,
        masses: &[Mass],
        trees: &[Tree],
        camera: FlyCamera,
        title: &str,
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
            ribbon(&mut tiles, &e.points, e.profile.width(), 0.16, STREET);
        }
        for line in &plan.tram_lines {
            for offset in [-0.7, 0.7] {
                let rail = offset_line(&line.points, offset);
                ribbon(&mut tiles, &rail, 0.25, 0.22, TRACK);
            }
        }
        for m in masses {
            mass(&mut tiles, m);
        }
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
                    (t.height / 10.0) as f32,
                    turn,
                    i % 2,
                )
            })
            .collect();
        Self {
            tiles: tiles.tiles,
            trees,
            camera,
            title: format!(
                "{title}: {} masses, {} trees, {:.0} km of streets, {} tram stops",
                masses.len(),
                tree_count,
                st.main_km + st.street_km,
                st.stops
            ),
            triangles,
        }
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

/// A mass: its walls and its roof.
fn mass(tiles: &mut Tiles, m: &Mass) {
    let size = tiles.size;
    let tile = tiles.at(m.footprint[0]);
    let centre = tile.centre;
    let mesh = &mut tile.masses;
    let n = m.footprint.len();
    for i in 0..n {
        let (a, b) = (m.footprint[i], m.footprint[(i + 1) % n]);
        let t = (b - a).normalize_or_zero();
        if t == DVec2::ZERO {
            continue;
        }
        // Outward: right of the edge for a positive outline.
        let out = Vec3::new(t.y as f32, 0.0, -t.x as f32);
        let (a0, b0) = (world(size, a, 0.0) - centre, world(size, b, 0.0) - centre);
        let (a1, b1) = (
            world(size, a, m.height) - centre,
            world(size, b, m.height) - centre,
        );
        triangle(mesh, [a0, b0, b1], out, WALL);
        triangle(mesh, [a0, b1, a1], out, WALL);
    }
    let roof = match m.roof {
        Roof::Green => ROOF_GREEN,
        Roof::Solar => ROOF_SOLAR,
        Roof::Terrace => ROOF_TERRACE,
    };
    for t in geom::triangulate(&m.footprint) {
        let p = t.map(|i| world(size, m.footprint[i], m.height) - centre);
        triangle(mesh, p, Vec3::Y, roof);
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
        materials.add_rows(
            "plan-ground",
            vec![
                ("plan ground".into(), colour([0.52, 0.49, 0.44], 12.0, 0.03)),
                ("plan garden".into(), colour([0.30, 0.38, 0.22], 6.0, 0.02)),
                ("plan park".into(), colour([0.22, 0.36, 0.14], 6.0, 0.02)),
                ("plan paving".into(), colour([0.60, 0.57, 0.52], 16.0, 0.04)),
                ("plan street".into(), colour([0.42, 0.41, 0.40], 14.0, 0.04)),
                ("plan track".into(), colour([0.16, 0.16, 0.17], 60.0, 0.3)),
                ("plan water".into(), colour([0.05, 0.12, 0.15], 900.0, 0.9)),
            ],
        );
        materials.add_rows(
            "plan-mass",
            vec![
                ("plan wall".into(), colour([0.72, 0.66, 0.58], 10.0, 0.03)),
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
            ],
        );
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
                    Vec3::splat(scale),
                    glam::Quat::from_rotation_y(turn),
                    at,
                );
                builder.add_instance_with_material(model, transform, tree_rows[kind]);
            }
        }
        let mut scene = builder.build(&ctx.device)?;
        scene.build_tlas(&ctx.device, &ctx.shaders)?;
        tracing::info!(
            meshes,
            trees = self.trees.len(),
            triangles = self.triangles,
            ms = start.elapsed().as_millis(),
            "the plan's scene"
        );
        Ok((
            scene,
            Box::new(PlanRunning {
                title: self.title.clone(),
            }),
        ))
    }
}

/// The plan's scene while it runs: nothing moves.
struct PlanRunning {
    title: String,
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
        _step: f32,
    ) -> Option<f64> {
        None
    }

    fn player(&mut self) -> Option<Vec3> {
        None
    }

    fn follow(&mut self, _camera: &mut FlyCamera) {}

    fn movers(&mut self, _scene: &mut MeshletScene) {}

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

/// A tree 10 m tall: a hexagonal trunk to 4.5 m (section 0) and a crown (section 1), an
/// icosahedron subdivided once and roughened, 4.2 m across its widest, its normals radial.
fn tree_model() -> TriMesh {
    let mut mesh = TriMesh::default();
    // The trunk.
    let sides = 6;
    for k in 0..sides {
        let a = k as f32 / sides as f32 * std::f32::consts::TAU;
        let b = (k + 1) as f32 / sides as f32 * std::f32::consts::TAU;
        let (pa, pb) = (
            Vec3::new(a.cos() * 0.28, 0.0, a.sin() * 0.28),
            Vec3::new(b.cos() * 0.28, 0.0, b.sin() * 0.28),
        );
        let up = Vec3::Y * 5.0;
        let out = ((pa + pb) * 0.5).normalize();
        triangle(&mut mesh, [pa, pb, pb + up], out, 0);
        triangle(&mut mesh, [pa, pb + up, pa + up], out, 0);
    }
    // The crown: an icosahedron's faces, each cut in four, pushed onto an ellipsoid.
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
    let centre = Vec3::new(0.0, 7.0, 0.0);
    let place = |d: Vec3| {
        let d = d.normalize();
        // A little roughness, the same for the same direction.
        let bump = 1.0 + 0.08 * (d.x * 7.3 + d.y * 3.1 + d.z * 5.7).sin();
        (
            centre + Vec3::new(d.x * 4.2, d.y * 3.6, d.z * 4.2) * bump,
            d,
        )
    };
    for f in faces {
        let (a, b, c) = (v[f[0]], v[f[1]], v[f[2]]);
        let (ab, bc, ca) = ((a + b) * 0.5, (b + c) * 0.5, (c + a) * 0.5);
        for tri in [[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]] {
            let p = tri.map(place);
            let normal = (p[0].1 + p[1].1 + p[2].1).normalize();
            triangle(&mut mesh, [p[0].0, p[1].0, p[2].0], normal, 1);
        }
    }
    mesh
}
