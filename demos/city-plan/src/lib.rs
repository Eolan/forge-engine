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
use forge_core::material::{RenderLayer, SoundLayer, SoundSurface, TextureId, UvMapping};
use forge_geom::MeshletMesh;
use forge_geom::city::PropSpec;
use forge_geom::flora::{limb, lump};
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
mod fauna;
mod footing;
mod mission;
mod pursuers;
mod residents;
mod signs;
mod sounds;
mod taxis;
mod walk;

// The code digests of the derived products, `CODE_*` (build.rs).
include!(concat!(env!("OUT_DIR"), "/code_digests.rs"));

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
/// How far behind a house's facade its backing stands ([`insides`]), metres: behind the kit's
/// faces, in front of any reveal's lining.
const BACKING: f64 = 0.03;
/// The insides' floors: the stone's colour, smooth (its pits read as squiggles a step away).
const KIT_INSIDE_FLOOR: u8 = 16;
/// The insides' joinery: the cupboards' doors under the flights, a waxed oak.
const KIT_INSIDE_WOOD: u8 = 17;
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
/// [`KIT_GROUPS`] for the houses whose insides are drawn ([`OPEN_GROUP`] onwards): the same rows,
/// their glass drawn after the resolve (D-065's part 4).
const KIT_OPEN_GROUPS: [&str; 8] = [
    "plan-kit-open-0",
    "plan-kit-open-1",
    "plan-kit-open-2",
    "plan-kit-open-3",
    "plan-kit-open-4",
    "plan-kit-open-5",
    "plan-kit-open-6",
    "plan-kit-open-7",
];
/// Each lit building's light, by the group's colour: its luminance (cd/m²) and colour. People
/// choose dim, diffuse light of different colours (the owner): a dim warm lamp, a brighter warm
/// room, a neutral one, a television's blue. Until the residents light their own windows by
/// what they do (D-060's step 3).
const LIT: [(f32, [f32; 3]); 4] = [
    (3.0, [1.0, 0.68, 0.40]),
    (8.0, [1.0, 0.80, 0.55]),
    (6.0, [1.0, 0.92, 0.80]),
    (2.5, [0.70, 0.82, 1.0]),
];
/// The share of a lit room's light ([`LIT`]) its window's clear glass glows with past the frame's
/// lamps (D-065's part 4, `forge_render::GlassPanes::glow`): about what the faked rooms showed.
const GLASS_GLOW: f32 = 0.5;

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

/// Paved ground per stain on the paving (D-074), m²: few, so that they read as wear here and there.
const STAIN_EVERY: f64 = 90.0;
/// The least a grown plant's sideways scale may be as a share of its height's, and its height's
/// as a share of its sideways: squeezed to a narrow planned crown, its trunk thinned with it and
/// its leaves turned to slivers.
const SQUEEZE: f32 = 0.85;
/// Paved ground per crack on the paving (D-074), m².
const CRACKS_EVERY: f64 = 150.0;

/// Stains and cracks on the paved walking ground within `detail` (D-074): stamps of decal kinds
/// 1 (a stain, [`textures::decal_stain`]) and 2 (cracks, [`textures::decal_cracks`]) scattered
/// over `paved` (triangles in plan coordinates, each at its level), one stain to
/// [`STAIN_EVERY`] m² and one crack to [`CRACKS_EVERY`] m², each at a turn and a size of its own
/// and wholly on the paving (its middle and its footprint's corners): none spills onto a lawn.
/// None without `detail`.
fn paving_marks(
    size: f64,
    paved: &[([DVec2; 3], f64)],
    detail: Option<&Detail>,
) -> Vec<forge_render::Decal> {
    const CELL: f64 = 4.0;
    let Some(d) = detail else {
        return Vec::new();
    };
    // The triangles by the cells of CELL their boxes cover, to find the one under a point.
    let cell = |p: DVec2| ((p.x / CELL).floor() as i64, (p.y / CELL).floor() as i64);
    let mut cells: std::collections::HashMap<(i64, i64), Vec<usize>> =
        std::collections::HashMap::new();
    for (i, (t, _)) in paved.iter().enumerate() {
        let lo = cell(t[0].min(t[1]).min(t[2]));
        let hi = cell(t[0].max(t[1]).max(t[2]));
        for x in lo.0..=hi.0 {
            for y in lo.1..=hi.1 {
                cells.entry((x, y)).or_default().push(i);
            }
        }
    }
    let inside = |t: &[DVec2; 3], p: DVec2| {
        let side = |a: DVec2, b: DVec2| (b - a).perp_dot(p - a);
        let s = [side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0])];
        s.iter().all(|&v| v >= 0.0) || s.iter().all(|&v| v <= 0.0)
    };
    let level_at = |p: DVec2| {
        cells.get(&cell(p)).and_then(|list| {
            list.iter()
                .find(|&&i| inside(&paved[i].0, p))
                .map(|&i| paved[i].1)
        })
    };
    let unit = |h: u64| (h >> 11) as f64 / (1u64 << 53) as f64;
    let mut out = Vec::new();
    for (i, (t, level)) in paved.iter().enumerate() {
        let area = (t[1] - t[0]).perp_dot(t[2] - t[0]).abs() * 0.5;
        if t[0].distance(d.centre) > d.radius + 30.0 {
            continue;
        }
        for (kind, every) in [(1u16, STAIN_EVERY), (2, CRACKS_EVERY)] {
            let seed = mix((i as u64) << 8 | u64::from(kind));
            // As many as its area holds, the fraction by a draw.
            let count = (area / every + unit(seed)).floor() as u64;
            for k in 0..count {
                let h = |s: u64| unit(mix(seed ^ (k << 16) ^ s));
                // A point of the triangle, uniform in its area.
                let (mut a, mut b) = (h(1), h(2));
                if a + b > 1.0 {
                    (a, b) = (1.0 - a, 1.0 - b);
                }
                let p = t[0] + (t[1] - t[0]) * a + (t[2] - t[0]) * b;
                let (half_x, half_z) = if kind == 1 {
                    (0.7 + 0.9 * h(3), 0.35 + 0.45 * h(4))
                } else {
                    (0.4 + 0.4 * h(3), 0.4 + 0.4 * h(3))
                };
                let turn = h(5) * std::f64::consts::TAU;
                let (s, c) = turn.sin_cos();
                let corners = [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)].map(|(u, w)| {
                    p + DVec2::new(
                        c * u * half_x + s * w * half_z,
                        -s * u * half_x + c * w * half_z,
                    )
                });
                let on_paving = std::iter::once(p)
                    .chain(corners)
                    .all(|q| level_at(q).is_some_and(|l| (l - level).abs() < 0.05));
                if !on_paving {
                    continue;
                }
                out.push(forge_render::Decal {
                    centre: world(size, p, *level),
                    rotation: glam::Quat::from_rotation_y(turn as f32),
                    half: Vec3::new(half_x as f32, 0.15, half_z as f32),
                    kind,
                    repeat: 1.0,
                    phase: 0.0,
                });
            }
        }
    }
    out
}

/// The grime at the foot of the outer walls of the masses `near` (D-074): one strip of decal
/// kind 0 along each stretch of a street or court wall that meets open air at the ground storey,
/// broken at its doors, shopfronts and passages, from 0.1 m under the walking level to 1 m over it.
fn wall_foot_grime(
    size: f64,
    structures: &[Structure],
    near: &[usize],
) -> Vec<forge_render::Decal> {
    let mut decals = Vec::new();
    for s in near.iter().map(|&i| &structures[i]) {
        for (w, wall) in s.walls.iter().enumerate() {
            if !matches!(wall.role, WallRole::Street | WallRole::Court) || wall.length() < 0.3 {
                continue;
            }
            let t = (wall.b - wall.a) / wall.length();
            // Out of the wall; the box's `x` turned so that its `z` runs down the wall and its
            // texture's dark `v` = 1 meets the ground.
            let out = DVec2::new(t.y, -t.x);
            let rotation = glam::Quat::from_mat3(&glam::Mat3::from_cols(
                Vec3::new(-t.x as f32, 0.0, -t.y as f32),
                Vec3::new(out.x as f32, 0.0, out.y as f32),
                -Vec3::Y,
            ));
            let gaps: Vec<(f64, f64)> = s
                .openings
                .iter()
                .filter(|o| o.wall == w && o.storey == 0 && o.sill < 0.3)
                .map(|o| o.span())
                .collect();
            let open = wall.open.first().map_or(&[][..], |o| &o[..]);
            for &(from, to) in open {
                // The stretch less the openings across it.
                let mut cuts = vec![(from, to)];
                for &(g0, g1) in &gaps {
                    cuts = cuts
                        .into_iter()
                        .flat_map(|(a, b)| [(a, b.min(g0)), (a.max(g1), b)])
                        .filter(|(a, b)| b - a > 0.3)
                        .collect();
                }
                for (a, b) in cuts {
                    let mid = wall.a + t * ((a + b) * 0.5);
                    let length = (b - a) as f32;
                    decals.push(forge_render::Decal {
                        centre: world(size, mid, ground::WALK_LEVEL + 0.45),
                        rotation,
                        half: Vec3::new(length * 0.5, 0.3, 0.55),
                        kind: 0,
                        repeat: (length * 0.5).round().max(1.0),
                        // Each strip from a place of its own: neighbouring walls do not repeat it.
                        phase: (decals.len() as f32 * 0.618_034).fract(),
                    });
                }
            }
        }
    }
    decals
}

/// The grime at the foot of the village's garden walls (`city::walls`, D-074): a strip of decal
/// kind 0 along each face of each wall within `detail`, as [`wall_foot_grime`] lays it along the
/// houses' walls.
fn garden_wall_grime(
    size: f64,
    plan: &CityPlan,
    detail: Option<&Detail>,
    phase_from: usize,
) -> Vec<forge_render::Decal> {
    let Some(d) = detail else {
        return Vec::new();
    };
    let mut decals = Vec::new();
    for w in &plan.walls {
        let ring = w.ring();
        if geom::signed_area(&ring) <= 0.0 || ring[0].distance(d.centre) > d.radius {
            continue;
        }
        for (k, &a) in ring.iter().enumerate() {
            let b = ring[(k + 1) % ring.len()];
            let length = a.distance(b);
            if length < 0.3 {
                continue;
            }
            let t = (b - a) / length;
            let out = DVec2::new(t.y, -t.x);
            decals.push(forge_render::Decal {
                centre: world(size, (a + b) * 0.5, ground::WALK_LEVEL + 0.45),
                rotation: glam::Quat::from_mat3(&glam::Mat3::from_cols(
                    Vec3::new(-t.x as f32, 0.0, -t.y as f32),
                    Vec3::new(out.x as f32, 0.0, out.y as f32),
                    -Vec3::Y,
                )),
                half: Vec3::new(length as f32 * 0.5, 0.3, 0.55),
                kind: 0,
                repeat: (length as f32 * 0.5).round().max(1.0),
                phase: ((phase_from + decals.len()) as f32 * 0.618_034).fract(),
            });
        }
    }
    decals
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
    /// Its decals (D-074): the grime strips at the kit's walls' feet (kind 0), the stains (1) and
    /// the cracks (2) on the paving.
    decals: Vec<forge_render::Decal>,
    /// The style kit's instanced modules and their instances (module, transform, group).
    kit_meshes: Vec<TriMesh>,
    kit_instances: Vec<(u32, Mat4, u8)>,
    /// The kit's modules that are the houses' furniture, and the size on screen, pixels across,
    /// under which they fade away (`--fade-furniture`, #343; none by default).
    home_kit: Vec<u32>,
    fade_furniture: Option<f32>,
    /// The tram lines' runs, and the service's clock at the start, seconds.
    trams: Vec<TramRun>,
    /// The river shuttles, one a crossing.
    boats: Vec<BoatRun>,
    clock: f64,
    /// The buildings' walls as convex pieces (the world's metres), for the walker.
    walls: Vec<Vec<Vec3>>,
    /// What the walker may cross (#316): the garden walls, and the openings of the houses with
    /// insides.
    crossings: Vec<forge_nav::Passage>,
    /// How many passages into courts, and where to look into one, for the log.
    passage_note: String,
    /// What each mass holds (D-060 step 3): its homes' households and residents, its jobs, its
    /// net floor area.
    holds: Vec<Occupancy>,
    walk_on_start: bool,
    /// When to hide every other clear pane and raise the rest (`--glass-test`, #325), seconds.
    glass_test: Option<f32>,
    /// When to hide the forest (`--static-test`, #323), seconds.
    static_test: Option<f32>,
    /// How a tree is promoted to a mover one second in (`--promote-test`, #338).
    promote_test: Option<PromoteTest>,
    /// The names the fingerposts' blades carry, a row of their atlas each.
    sign_names: Vec<String>,
    /// Seconds into the run at which to board the nearest tram (scripted rides).
    ride_after: Option<f64>,
    crowd: Option<crowd::Crowd>,
    /// The pursuers going for a house (`--pursuers`, #317).
    pursuers: Option<pursuers::Pursuers>,
    /// The village's test mission (`--mission`, #287) and whether its autopilot walks it.
    mission: Option<(mission::VillageMission, bool)>,
    /// The village's animals (`--fauna`, #269).
    fauna: Option<fauna::FaunaRun>,
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
    cooked: Option<Vec<(MeshletMesh, [f32; 3])>>,
    /// The houses' rooms for the probes (D-068, [`probe_regions`]), in the world frame.
    regions: Vec<forge_render::RegionVolume>,
    /// The houses' lamps that are on (D-065, [`home_lamps`]), in the world frame.
    home_lamps: Vec<forge_render::lamps::Lamp>,
    /// The clear glass of the houses whose insides are drawn (D-065's part 4), in the world
    /// frame: left out of their windows' modules.
    glass: forge_render::GlassPanes,
    /// The gardens' and parks' grass map (#300, `Detail::grass`), in the world frame.
    grass: Option<forge_render::grass::GrassMap>,
    /// The ground's sections under the plan, for what a foot lands on (#318).
    footing: Option<footing::Footing>,
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
    /// Make a grass map for the blades near the camera (#300, city-blocks' `--grass`).
    pub grass: bool,
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
        let mut structures = structure::structures(plan, masses);
        // The windows the rules shape themselves fitted to their modules (D-066's holes and
        // fills): a balcony's French window to the floor, an attic's window under the cornice,
        // so the insides open round them and the probes' regions take their reveals.
        if let Some(d) = detail {
            let wanted: Vec<usize> = (0..masses.len())
                .filter(|&i| geom::centroid(&masses[i].footprint).distance(d.centre) < d.radius)
                .collect();
            let (dressed, _) =
                facades::dress(d.grammar, &structures, masses, &wanted, d.top, d.seed);
            let fills = facades::fills(d.grammar, &structures, &dressed);
            facades::fit_windows(&mut structures, &fills);
        }
        let structures = structures;
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
        // blocks' order: one thread took 13 s over the city's. Kept in the derived cache (#345):
        // made, the city's took 1.1 s of 16 threads, one park's 0.9 s alone.
        let walled = walled_lots(plan, masses);
        let key = blocks_ground_key(plan, &paths, &middles, &walled);
        let made = forge_core::derived::DerivedCache::new(forge_core::derived::cache_dir(
            forge_core::derived::CacheKind::World,
        ))
        .get_or_make("city-blocks-ground", key, || {
            blocks_ground(plan, &paths, &middles, &walled)
                .into_iter()
                .flatten()
                .map(|(first, triangles, level, section)| {
                    (
                        first.to_array(),
                        triangles
                            .iter()
                            .map(|t| t.map(|p| p.to_array()))
                            .collect::<Vec<_>>(),
                        (level, section),
                    )
                })
                .collect::<Vec<_>>()
        });
        tracing::debug!(
            from_cache = made.from_cache,
            ms = made.ms,
            "the blocks' ground (#345)"
        );
        for (first, triangles, (level, section)) in &made.value {
            let triangles: Vec<[DVec2; 3]> = triangles.iter().map(|t| t.map(DVec2::from)).collect();
            tiles.flat_triangles(DVec2::from(*first), &triangles, *level, *section, false);
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
        // The residents at home and awake at the hour light their own windows, and their lamps.
        let residents = detail.and_then(|d| {
            d.lit_hour.map(|h| {
                residents::Residents::new(plan, masses, &structures, &holds, &d.mix, h, &near)
            })
        });
        let kit = detail.map(|d| {
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
        // And the probes' regions (D-068), where the insides are drawn, and their lamps (D-065).
        let (regions, halls) = if detail.is_some() {
            probe_regions(size, masses, &structures, &near)
        } else {
            (Vec::new(), Vec::new())
        };
        let home_lamps = home_lamps(
            size,
            masses,
            &structures,
            &halls,
            &regions,
            residents.as_ref(),
        );
        let grime = wall_foot_grime(size, &structures, &near);
        // Stains and cracks on the paving (D-074): the blocks' paved pieces and the streets'
        // walking ground.
        let mut paved: Vec<([DVec2; 3], f64)> = made
            .value
            .iter()
            .filter(|p| p.2.1 == PAVING)
            .flat_map(|(_, triangles, (level, _))| {
                triangles.iter().map(move |t| (t.map(DVec2::from), *level))
            })
            .collect();
        for t in &plan.ground.tiles {
            for polygon in ground::simple_polygons(&t.walk) {
                paved.extend(
                    ground::triangles(&polygon)
                        .into_iter()
                        .map(|t| (t, WALK_LEVEL)),
                );
            }
        }
        let mut grime = grime;
        grime.extend(paving_marks(size, &paved, detail));
        let walls = garden_wall_grime(size, plan, detail, grime.len());
        grime.extend(walls);
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
        let lamps: Vec<(Vec3, f32, f32, bool)> = lamps
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
        let (mut kit_meshes, mut kit_instances, kit_report, mut glass) =
            kit.map_or((Vec::new(), Vec::new(), None, Default::default()), |k| {
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
                let report = format!(
                    "{report}; the clear glass over their insides: {} triangles",
                    k.glass.triangle_count()
                );
                seats = k.seats;
                (k.meshes, k.instances, Some(report), k.glass)
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
        let mut home_kit = Vec::new();
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
                home_kit.push(id);
            }
        }
        // Their lamps (D-065 step 2): a globe on its cord, stem or arm, a mesh for each kind and
        // reach (to the millimetre), glowing where its household has it on.
        // Per kind, globe and reach in millimetres: each one's place and whether it is on.
        type Globes<'a> = std::collections::BTreeMap<(&'a str, u32, u32), Vec<(Mat4, bool)>>;
        let mut globes = Globes::default();
        for l in &home_lamps {
            let name = match l.kind {
                furnish::LampKind::Pendant => "pendant",
                furnish::LampKind::Ceiling => "ceiling",
                furnish::LampKind::Sconce => "sconce",
            };
            let mm = |v: f32| (v * 1000.0).round() as u32;
            globes
                .entry((name, mm(l.lamp.radius), mm(l.reach)))
                .or_default()
                .push((
                    Mat4::from_rotation_translation(
                        glam::Quat::from_rotation_y(l.turn),
                        l.lamp.position,
                    ),
                    l.on,
                ));
        }
        for ((name, radius, reach), at) in globes {
            let (radius, reach) = (radius as f32 / 1000.0, reach as f32 / 1000.0);
            if let Some(mesh) = forge_geom::furniture::lamp(name, radius, reach) {
                kit_meshes.push(mesh);
                let id = (kit_meshes.len() - 1) as u32;
                kit_instances.extend(
                    at.into_iter()
                        .map(|(at, on)| (id, at, if on { LAMP_GROUP } else { LAMP_OFF_GROUP })),
                );
            }
        }
        let placed = home_lamps;
        let mut home_lamps: Vec<forge_render::lamps::Lamp> =
            placed.iter().filter(|l| l.on).map(|l| l.lamp).collect();
        if !home_lamps.is_empty() {
            eprintln!("the houses' lamps: {} on", home_lamps.len());
        }
        // And the street lamps' after dark, as the residents' lamps go on (D-065).
        if detail.and_then(|d| d.lit_hour).is_some_and(street_lamps_on) {
            let street = forge_render::lamps::street_lights(&lamps);
            eprintln!("the street lamps: {} on", street.len());
            home_lamps.extend(street);
        }
        // The clear panes glow with their rooms' own lamps (#284).
        let lamped: std::collections::HashSet<u32> =
            placed.iter().map(|l| l.lamp.regions[0]).collect();
        room_lamps(&mut glass, &regions, &lamped, &home_lamps);
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
        // The grass map (#300), when asked for: the gardens' and parks' ground at half a metre.
        let grass = detail
            .filter(|d| d.grass)
            .and_then(|_| grass_map(size, &tiles, masses, &plan.walls));
        laps.mark("the rest");
        // What a foot lands on outdoors (#318): the ground's sections at a metre.
        let footing = footing_map(size, &tiles);
        laps.mark("footing");
        eprintln!(
            "the plan laid out in its tiles, its steps (ms): {}",
            laps.report()
        );
        Self {
            regions,
            home_lamps,
            glass,
            grass,
            footing,
            tiles: tiles.tiles,
            trees,
            flora,
            lamps,
            decals: grime,
            kit_meshes,
            home_kit,
            fade_furniture: None,
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
            crossings: village_passages(size, plan, masses, &structures)
                .into_iter()
                .filter(|p| p.piece >= GARDEN_WALL_PIECE || near.contains(&(p.piece as usize)))
                .collect(),
            passage_note,
            holds: holds.clone(),
            walk_on_start: false,
            glass_test: None,
            static_test: None,
            promote_test: None,
            ride_after: None,
            games,
            stalls: stalls.iter().map(|&(at, _)| at).collect(),
            sign_names,
            crowd: None,
            pursuers: None,
            mission: None,
            fauna: None,
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

    /// One second in, every other clear pane hidden and the rest raised 0.4 m: the check of
    /// panes changed at run time (#325).
    pub fn glass_test(mut self) -> Self {
        self.glass_test = Some(1.0);
        self
    }

    /// The houses' furniture fading away under `px` pixels across on screen (#343, a ladder to
    /// nothing): a few tens of metres off, the culls and the raster skip it.
    pub fn fade_furniture(mut self, px: f32) -> Self {
        self.fade_furniture = Some(px);
        self
    }

    /// One second in, the village's forest hidden, its trees, their stand-ins and their shadows: the
    /// check of static instances changed at run time (#323).
    pub fn static_test(mut self) -> Self {
        self.static_test = Some(1.0);
        self
    }

    /// One second in, the tree before the camera promoted to a mover of a pool, and demoted a
    /// second later, as `mode` says: the check of static instances promoted while they move
    /// (#338).
    pub fn promote_test(mut self, mode: PromoteTest) -> Self {
        self.promote_test = Some(mode);
        self
    }

    /// Four pursuers on the street before the first house with insides, 3 m apart along its
    /// front, going for its hall by its door or over a window's sill (#317, D-076's proof as a
    /// scene). The log names a view of the house's front.
    pub fn with_pursuers(mut self, plan: &CityPlan, masses: &[Mass]) -> Self {
        let structures = structure::structures(plan, masses);
        let passages =
            forge_nav::Passages::new(village_passages(self.size, plan, masses, &structures));
        let Some((i, door, windows)) = pursued_house(masses, &structures, &passages) else {
            tracing::warn!("no house for the pursuers");
            return self;
        };
        let field = house_field(self.size, masses, &structures, &passages, i, false);
        self.pursuers = pursuers::Pursuers::new(field, &door, windows, 3.0);
        let eye = door.middle() - door.across * 8.0;
        let look = door.across;
        tracing::info!(
            "the pursuers' house (lot {}): --view={:.1},1.7,{:.1},{:.1},-5 ({} pursuers)",
            masses[i].lot,
            eye.x,
            eye.z,
            (-look.x).atan2(-look.z).to_degrees(),
            if self.pursuers.is_some() {
                pursuers::COUNT
            } else {
                0
            }
        );
        self
    }

    /// The village's animals (#269's fauna, `--fauna`): its sparrows, pigeons, swifts and gulls,
    /// cats, and dogs walked by their owners, from `hour` (0 to 24) of a June day, stepped with the
    /// player and the pursuers as people they flee. The log counts them.
    pub fn with_fauna(mut self, site: &Site, plan: &CityPlan, masses: &[Mass], hour: f64) -> Self {
        let structures = structure::structures(plan, masses);
        let half = self.size * 0.5;
        let on_plan = |p: Vec3| DVec2::new(f64::from(p.x) + half, f64::from(p.z) + half);
        let trees: Vec<(DVec2, f64, f64)> =
            self.trees
                .iter()
                .map(|&(at, scale, ..)| {
                    (
                        on_plan(at),
                        f64::from(scale.y) * 10.0,
                        f64::from(scale.x) * 4.2,
                    )
                })
                .chain(self.flora.iter().map(|&(at, height, crown, ..)| {
                    (on_plan(at), f64::from(height), f64::from(crown))
                }))
                .collect();
        let parts = fauna::parts(site, plan, masses, &structures, &trees);
        self.fauna = Some(fauna::FaunaRun::new(parts, hour, 1));
        self
    }

    /// The village's test mission (#287's proof on the first map): written from the first house
    /// with a stair (`mission::village`), played from the street before its front door, the
    /// walker on from the start; `autopilot` walks it on the house's field. The log names the
    /// house and each change.
    pub fn with_mission(mut self, plan: &CityPlan, masses: &[Mass], autopilot: bool) -> Self {
        let Some(m) = mission::village(self.size, plan, masses) else {
            tracing::warn!("no house for the village's mission");
            return self;
        };
        tracing::info!(
            house = m.house,
            lot = m.lot,
            "the village's mission: --view={:.1},1.7,{:.1},{:.1},-3",
            m.start.x,
            m.start.z,
            (-m.facing.x).atan2(-m.facing.z).to_degrees()
        );
        self.camera.position = m.start + Vec3::Y * walk::EYES;
        self.camera.yaw = (-m.facing.x).atan2(-m.facing.z);
        self.camera.pitch = -0.05;
        self.walk_on_start = true;
        self.mission = Some((m, autopilot));
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
/// The houses' lamps' groups (D-065): their globes lit, and unlit.
const LAMP_GROUP: u8 = 11;
const LAMP_OFF_GROUP: u8 = 12;
/// The kit's groups again ([`KIT_OPEN_GROUPS`]) for the houses whose insides are drawn: their
/// windows' glass left out of the visibility pass, drawn over the rooms after the resolve
/// (D-065's part 4).
const OPEN_GROUP: u8 = 13;
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
    /// The clear glass of the houses whose insides are drawn, left out of their modules (D-065's
    /// part 4), in the world frame.
    glass: forge_render::GlassPanes,
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

/// A module's clear glass (its [`modules::GLASS`] triangles) as a mesh of its own, for the houses
/// whose insides are drawn (D-065's part 4): drawn over the room behind it after the resolve
/// (`forge_render::Glass`). The module keeps them: their row leaves them out of the visibility
/// pass ([`OPEN_GROUP`]), and the rays still meet them.
fn module_panes(mesh: &TriMesh) -> TriMesh {
    let mut panes = TriMesh::default();
    for t in (0..mesh.triangle_count()).filter(|&t| mesh.section(t) == modules::GLASS) {
        let base = panes.positions.len() as u32;
        for &i in &mesh.indices[t * 3..t * 3 + 3] {
            panes.positions.push(mesh.positions[i as usize]);
            panes.normals.push(mesh.normals[i as usize]);
        }
        panes.indices.extend([base, base + 1, base + 2]);
    }
    panes
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
    // Each instanced module's clear glass, by its mesh, for the houses whose insides are drawn
    // (D-065's part 4): drawn over their rooms, not in the module.
    let mut panes_of: std::collections::HashMap<u32, TriMesh> = std::collections::HashMap::new();
    let mut glass = forge_render::GlassPanes::default();
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
                    let panes = module_panes(&mesh);
                    meshes.push(mesh);
                    let id = (meshes.len() - 1) as u32;
                    keys.insert(key, id);
                    cards.insert(id, railing);
                    panes_of.insert(id, panes);
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
            // A house whose insides are drawn: its windows' glass drawn over its rooms after the
            // resolve (D-065's part 4), not in the visibility pass, and the panes laid out for it.
            let clear = entered[piece.mass as usize];
            let group = colour * 2 + u8::from(lit);
            instances.push((
                id,
                transform,
                if clear { OPEN_GROUP + group } else { group },
            ));
            // Its clear glass, glowing as its lit room past the frame's lamps.
            if let Some(p) = panes_of.get(&id).filter(|p| clear && !p.indices.is_empty()) {
                let glow = if lit {
                    let (l, c) = LIT[usize::from(colour)];
                    c.map(|v| v * l * GLASS_GLOW)
                } else {
                    [0.0; 3]
                };
                // Each window's glass one pane (#325), numbered as laid out.
                let pane = glass.pane.last().map_or(0, |&n| n + 1);
                for &i in &p.indices {
                    glass.pane.push(pane);
                    let (q, n) = (
                        Vec3::from(p.positions[i as usize]),
                        Vec3::from(p.normals[i as usize]),
                    );
                    glass.positions.push(transform.transform_point3(q));
                    glass
                        .normals
                        .push(transform.transform_vector3(n).normalize_or(z));
                    glass.glow.push(glow);
                }
            }
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
        glass,
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
/// The houses' insides as the probes' regions (D-068, `research/probe-leaks-indoors.md`), in
/// the world frame: each room of each storey a region of its own, the hall one through every
/// storey (its flight rises through them), each doorway in a partition or an outer wall a ramp
/// from one side's region to the other's (outside, 0), and each window's inner reveal, from the
/// glass to the room, its room's. A probe then lights only its own region's surfaces: a thin
/// partition passed a sunlit room's light, and a sunlit probe outside lit a window's head.
/// With them, each house's (its index into `masses`) hall's region: its rooms' follow, in
/// the interior's order.
fn probe_regions(
    size: f64,
    masses: &[Mass],
    structures: &[Structure],
    near: &[usize],
) -> (Vec<forge_render::RegionVolume>, Vec<(usize, u32)>) {
    use forge_render::{RegionKind, RegionVolume};
    let mut out = Vec::new();
    let mut next = 1u32;
    let mut halls = Vec::new();
    for &i in near {
        let (m, s) = (&masses[i], &structures[i]);
        let Some(inner) = interior::house(m, s) else {
            continue;
        };
        let n = s.storeys.len();
        if n == 0 {
            continue;
        }
        let corner = |p: DVec2| {
            let w = world(size, p, 0.0);
            glam::Vec2::new(w.x, w.z)
        };
        let floor = |k: usize| floor_level(s, k) as f32;
        let top = (s.storeys[n - 1].level + s.storeys[n - 1].height) as f32;
        // A storey's rooms reach up to 3 cm under the next floor, inside its slab: the ceiling
        // under it lower, the floor over it the next storey's.
        let ceiling = |k: usize| if k + 1 < n { floor(k + 1) - 0.03 } else { top };
        // The house's body round its rooms, last: a face whose point 2 cm off it missed its
        // room's outline (a corner, a ceiling's edge) is lit by every probe, not taken for the
        // outside's (white specks down a corner). Its footprint's triangles, each a quad with a
        // corner twice.
        // Up to just over the top ceiling ([`insides`]' under the roof): reaching 0.3 m over
        // the walls, it held the roof's lowest course and the parapet's top, which took every
        // lamp of the frame unshadowed, as the body does, and glowed as a band round the roof at
        // night (the owner, 2026-10-24: "light is leaking on the roof").
        let last = &s.storeys[n - 1];
        let body_top = ((last.level + last.height - last.slab).min(m.height - 0.05) + 0.05) as f32;
        for t in geom::triangulate(&m.footprint) {
            let [a, b, c] = t.map(|k| m.footprint[k]);
            out.push(RegionVolume {
                corners: [a, b, c, c].map(corner),
                floor: floor(0) - 0.5,
                ceiling: body_top,
                region: 0,
                other: 0,
                kind: RegionKind::Solid,
            });
        }
        let hall = next;
        halls.push((i, hall));
        next += 1;
        out.push(RegionVolume {
            corners: inner.hall.map(corner),
            floor: floor(0) - 0.15,
            ceiling: top,
            region: hall,
            other: hall,
            kind: RegionKind::Room,
        });
        let mut rooms: Vec<(usize, [DVec2; 4], u32)> = Vec::new();
        for r in &inner.rooms {
            rooms.push((r.storey, r.outline, next));
            out.push(RegionVolume {
                corners: r.outline.map(corner),
                floor: floor(r.storey) - 0.15,
                ceiling: ceiling(r.storey),
                region: next,
                other: next,
                kind: RegionKind::Room,
            });
            next += 1;
        }
        // The region holding a point of storey `k`: a room of it, else the hall, else outside.
        let region_of = |p: DVec2, k: usize| -> u32 {
            rooms
                .iter()
                .find(|(rk, o, _)| *rk == k && geom::contains(o, p))
                .map(|r| r.2)
                .or_else(|| geom::contains(&inner.hall, p).then_some(hall))
                .unwrap_or(0)
        };
        for part in &inner.partitions {
            let t = (part.b - part.a).normalize_or_zero();
            let left = DVec2::new(-t.y, t.x);
            let h = part.thickness * 0.5 + 0.01;
            let at = |along: f64, across: f64| part.a + t * along + left * across;
            for &(lo, hi) in &part.doors {
                let mid = (lo + hi) * 0.5;
                out.push(RegionVolume {
                    corners: [at(lo, -h), at(hi, -h), at(hi, h), at(lo, h)].map(corner),
                    floor: floor(part.storey) - 0.1,
                    ceiling: floor(part.storey) + interior::DOORWAY_HEAD as f32 + 0.05,
                    region: region_of(at(mid, -h - 0.15), part.storey),
                    other: region_of(at(mid, h + 0.15), part.storey),
                    kind: RegionKind::Doorway,
                });
            }
        }
        for o in &s.openings {
            let wall = &s.walls[o.wall];
            let t = (wall.b - wall.a).normalize_or_zero();
            let inward = DVec2::new(-t.y, t.x);
            let at = |along: f64, depth: f64| wall.a + t * along + inward * depth;
            let (lo, hi) = o.span();
            let deep = wall.thickness + 0.01;
            let inside = region_of(at((lo + hi) * 0.5, wall.thickness + 0.15), o.storey);
            let level = s.storeys[o.storey].level;
            let head = (level + o.head) as f32 + 0.05;
            match o.kind {
                OpeningKind::Window => {
                    // From the window's glass to the inner face. The module stands two thirds
                    // into the wall (`kit`), its glass 5 cm before its back (`modules::window`):
                    // the box starts 6 cm before it, so the glass's outer face, 2 cm off it, stays
                    // outside's, its sky's reflection lit by the street's probes. What stands
                    // before the glass (the frame's outer bars) is outside the room, lit by
                    // every probe as the walls are.
                    let from = (wall.thickness * 0.65).clamp(0.12, 0.40) - 0.06;
                    out.push(RegionVolume {
                        corners: [at(lo, from), at(hi, from), at(hi, deep), at(lo, deep)]
                            .map(corner),
                        floor: (level + o.sill) as f32 - 0.05,
                        ceiling: head,
                        region: inside,
                        other: inside,
                        kind: RegionKind::Recess,
                    });
                }
                OpeningKind::Door | OpeningKind::Shopfront => out.push(RegionVolume {
                    corners: [at(lo, -0.01), at(hi, -0.01), at(hi, deep), at(lo, deep)].map(corner),
                    floor: floor(o.storey) - 0.1,
                    ceiling: head,
                    region: 0,
                    other: inside,
                    kind: RegionKind::Doorway,
                }),
                OpeningKind::Passage => {}
            }
        }
    }
    (out, halls)
}

/// A house's lamp as the scene has it (D-065 step 2, #270): the renderer's lamp in the world
/// frame, whether its household has it on, and what draws it.
#[derive(Clone, Copy, Debug)]
struct PlacedLamp {
    lamp: forge_render::lamps::Lamp,
    on: bool,
    kind: furnish::LampKind,
    /// A sconce's turn about y (its arm back towards −z); 0 for the others.
    turn: f32,
    /// How far its fitting reaches from the globe's middle: up to the ceiling, or back to the
    /// wall.
    reach: f32,
}

/// A lit room's luminance seen through its window from afar, per candela of its strongest lamp
/// (#284), cd/m² per cd: what the resolve shows of the room when the frame lights that lamp,
/// matched against every lamp lit (`--all-lamps`) at 80 m.
const ROOM_GLOW: f32 = 0.02;

/// The clear panes' rooms' lamps (#284): for each pane of `glass`, the room behind it among
/// `regions` (30 cm off it, either side), and of `lamps` (the frame's, all on) the strongest whose
/// own room it is and whose storey band holds the pane (a house lamp lights from 2.7 m under it
/// to 0.8 m over it, `lamp_weight_in`: the hall is one region through every storey). The pane
/// then glows with [`ROOM_GLOW`] of that lamp's light in its colour, by the share of it the frame
/// leaves unlit (`GlassPanes::lamp`), at any distance; a pane of a room with a lamp (`lamped`, on
/// or off) but none on in its band glows with nothing. A pane whose room has no lamp, or none is
/// found, keeps its window's glow faded in by distance.
fn room_lamps(
    glass: &mut forge_render::GlassPanes,
    regions: &[forge_render::RegionVolume],
    lamped: &std::collections::HashSet<u32>,
    lamps: &[forge_render::lamps::Lamp],
) {
    let mut by_room: std::collections::HashMap<u32, Vec<usize>> = std::collections::HashMap::new();
    for (i, l) in lamps.iter().enumerate() {
        if !l.outside() && l.regions[0] != forge_render::lamps::NO_REGION {
            by_room.entry(l.regions[0]).or_default().push(i);
        }
    }
    glass.lamp = vec![forge_render::NO_LAMP; glass.positions.len()];
    for t in 0..glass.triangle_count() {
        let corners = 3 * t..3 * t + 3;
        let c = glass.positions[corners.clone()].iter().sum::<Vec3>() / 3.0;
        let n = glass.normals[3 * t];
        let Some(room) = [0.3_f32, -0.3]
            .into_iter()
            .map(|s| forge_render::region_at(regions, c + n * s))
            .find(|h| h.region != 0 && h.kind != forge_render::RegionKind::Solid)
            .map(|h| h.region)
            .filter(|r| lamped.contains(r))
        else {
            continue;
        };
        let strongest = by_room
            .get(&room)
            .into_iter()
            .flatten()
            .copied()
            .filter(|&i| (-2.7..=0.8).contains(&(c.y - lamps[i].position.y)))
            .max_by(|&a, &b| lamps[a].candela.total_cmp(&lamps[b].candela));
        let (glow, lamp) = match strongest {
            Some(i) => {
                let l = &lamps[i];
                let c = forge_render::lamps::lamp_rgb(l.kelvin) * (l.candela * ROOM_GLOW);
                (c.to_array(), i as u32)
            }
            None => ([0.0; 3], forge_render::NO_LAMP),
        };
        for k in corners {
            glass.glow[k] = glow;
            glass.lamp[k] = lamp;
        }
    }
}

/// The lamps of the houses `halls` names ([`probe_regions`]: each house and its hall's region)
/// among their furniture ([`furnish::lamps`]), each lighting its room's region and those its
/// storey's doorways open into, the street's left out (`regions`'s doorways; D-068); on where
/// the household is home and awake and has the room lit ([`residents::Residents::lit`], the
/// faked windows' rule), its sconces in one room lit in two, the hall's light where a room of
/// its storey is lit. All off without `residents`.
fn home_lamps(
    size: f64,
    masses: &[Mass],
    structures: &[Structure],
    halls: &[(usize, u32)],
    regions: &[forge_render::RegionVolume],
    residents: Option<&residents::Residents>,
) -> Vec<PlacedLamp> {
    use forge_render::lamps::{LAMP_REGIONS, Lamp, NO_REGION};
    let mut out = Vec::new();
    for &(i, hall) in halls {
        let (m, s) = (&masses[i], &structures[i]);
        let Some(inner) = interior::house(m, s) else {
            continue;
        };
        let pieces = forge_procgen::city::walk::furnished(m, s, &inner);
        let region_of = |room: Option<usize>| room.map_or(hall, |r| hall + 1 + r as u32);
        // Whether room `r` of the house is lit: its window's rule, its own draw.
        let lit = |r: usize| {
            let room = &inner.rooms[r];
            let middle = geom::centroid(&room.outline);
            let unit = residents::unit_at(s, room.storey, middle);
            let spot = mix((i as u64) << 20 | r as u64);
            residents
                .and_then(|res| res.lit(s, unit, spot))
                .unwrap_or(false)
        };
        let rooms_lit: Vec<bool> = (0..inner.rooms.len()).map(lit).collect();
        for l in furnish::lamps(s, &inner, &pieces) {
            let own = region_of(l.room);
            // Its regions: its own, then those across its storey's doorways (not the street).
            let level = floor_level(s, l.storey) as f32;
            let mut lights = vec![own];
            for v in regions {
                if v.kind != forge_render::RegionKind::Doorway
                    || (v.floor + 0.1 - level).abs() > 0.3
                {
                    continue;
                }
                let across = if v.region == own {
                    v.other
                } else if v.other == own {
                    v.region
                } else {
                    continue;
                };
                if across != 0 && !lights.contains(&across) {
                    lights.push(across);
                }
            }
            let mut slots = [NO_REGION; LAMP_REGIONS];
            for (slot, r) in slots.iter_mut().zip(&lights) {
                *slot = *r;
            }
            let on = match (l.kind, l.room) {
                (furnish::LampKind::Sconce, Some(r)) => {
                    rooms_lit[r] && mix((i as u64) << 20 | r as u64 | 1 << 19).is_multiple_of(2)
                }
                (_, Some(r)) => rooms_lit[r],
                (_, None) => inner
                    .rooms
                    .iter()
                    .zip(&rooms_lit)
                    .any(|(room, &on)| on && room.storey == l.storey),
            };
            let clear = s.storeys[l.storey].clear();
            let (turn, reach) = match l.kind {
                furnish::LampKind::Sconce => ((l.facing.x as f32).atan2(l.facing.y as f32), 0.1),
                _ => (0.0, (clear - l.height) as f32),
            };
            out.push(PlacedLamp {
                lamp: Lamp {
                    // A ceiling's lamp from its storey's level, as `furnish::lamps` measures it
                    // under the ceiling: from the ground floor's, 0.32 m higher over the walk, a
                    // pendant's canopy came 2 cm out of the floor above, a little brass square
                    // on the bedroom's floor (the owner, 2026-10-24: "small things on the
                    // floor"). A sconce from the floor its bed or sofa stands on.
                    position: world(
                        size,
                        l.at,
                        match l.kind {
                            furnish::LampKind::Sconce => f64::from(level),
                            _ => s.storeys[l.storey].level,
                        } + l.height,
                    ),
                    radius: l.radius as f32,
                    candela: l.candela as f32,
                    kelvin: l.kelvin as f32,
                    regions: slots,
                    emission: forge_render::lamps::Emission::Every,
                },
                on,
                kind: l.kind,
                turn,
                reach,
            });
        }
    }
    out
}

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
            // The face from `from` to `to` along the wall, `y0` to `y1`, its plaster mapped by
            // the place along the wall and the height, so it runs on from face to face (each face
            // began its own at its corner).
            let mut face = |from: f64, to: f64, y0: f64, y1: f64| {
                let start = if from <= 0.0 { run_a } else { 0.0 };
                let end = if to >= wall.length() { run_b } else { 0.0 };
                let (s0, s1) = (from - start, to + end);
                let p = wall.a + t * s0 + inward * deep;
                let q = wall.a + t * s1 + inward * deep;
                let (u0, u1) = ((s0 / 3.0) as f32, (s1 / 3.0) as f32);
                let (v0, v1) = ((-y0 / 3.0) as f32, (-y1 / 3.0) as f32);
                triangle_uv(
                    mesh,
                    [at(p, y0), at(q, y0), at(q, y1)],
                    [[u0, v0], [u1, v0], [u1, v1]],
                    normal,
                    plaster,
                );
                triangle_uv(
                    mesh,
                    [at(p, y0), at(q, y1), at(p, y1)],
                    [[u0, v0], [u1, v1], [u0, v1]],
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
            // Every band's face is cut at every height a band of the wall stops at (the holes'
            // sills and heads), so two bands side by side share their corners: a band cut round a
            // window beside one face the wall's height left T-junctions on their common edge,
            // pinholes in a line up the wall that showed the light behind it (2026-10-17).
            let mut cuts: Vec<f64> = holes
                .iter()
                .flat_map(|h| [h.2.max(0.0), h.3.min(top)])
                .collect();
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|x, y| (*x - *y).abs() < 0.005);
            let mut face = |x0: f64, x1: f64, y0: f64, y1: f64| {
                let mut lo = y0;
                for &c in cuts.iter().filter(|&&c| c > y0 + 0.005 && c < y1 - 0.005) {
                    face(x0, x1, lo, c);
                    lo = c;
                }
                face(x0, x1, lo, y1);
            };
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
            // Behind the facade, [`BACKING`] into the wall, a face turned out to the street in the
            // facade's plaster, cut at the openings: the kit's pieces meet at the storeys' lines
            // and the corners on edges that never quite close (each piece placed on its own),
            // and at night the lit rooms showed through as white flecks in a line (the owner,
            // 2026-10-24). Hidden but through those cracks. Its ends meet the walls' beside it
            // on their mitre, so a corner's crack shows it too.
            let mitre_at = |c: DVec2, u: DVec2, v: DVec2, from_a: bool| {
                // Where this wall's line, moved in, meets the other's, moved in as far: its place
                // along this wall from `wall.a`.
                let (n_u, n_v) = (DVec2::new(-u.y, u.x), DVec2::new(-v.y, v.x));
                let denom = u.perp_dot(v);
                if denom.abs() < 1e-6 {
                    return if from_a { 0.0 } else { length };
                }
                // `u` this wall's way, `v` the other's: c + n_u d + u x = c + n_v d + v s.
                let w = (n_v - n_u) * BACKING;
                let x = w.perp_dot(v) / denom;
                (c - wall.a).dot(u) + x
            };
            let (xa, xb) = (
                mitre_at(wall.a, t, along(prev), true),
                mitre_at(wall.b, t, along(next), false),
            );
            let outward = -normal;
            let facade = KIT_PLASTER + tint_of(m.lot);
            let mut backing = |x0: f64, x1: f64, y0: f64, y1: f64| {
                let x0 = if x0 <= 0.0 { xa } else { x0.max(xa) };
                let x1 = if x1 >= length { xb } else { x1.min(xb) };
                if x1 < x0 + 0.005 {
                    return;
                }
                let p = wall.a + t * x0 + inward * BACKING;
                let q = wall.a + t * x1 + inward * BACKING;
                let (u0, u1) = ((x0 / 3.0) as f32, (x1 / 3.0) as f32);
                let (v0, v1) = ((-y0 / 3.0) as f32, (-y1 / 3.0) as f32);
                triangle_uv(
                    mesh,
                    [at(p, y0), at(q, y0), at(q, y1)],
                    [[u0, v0], [u1, v0], [u1, v1]],
                    outward,
                    facade,
                );
                triangle_uv(
                    mesh,
                    [at(p, y0), at(q, y1), at(p, y1)],
                    [[u0, v0], [u1, v1], [u0, v1]],
                    outward,
                    facade,
                );
            };
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
                        backing(x0, x1, y, c0);
                    }
                    y = y.max(c1);
                }
                if top > y + 0.005 {
                    backing(x0, x1, y, top);
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
                let tx = Vec3::new(t.x as f32, 0.0, t.y as f32);
                let r = ((deep - from) / 3.0) as f32;
                // The reveals cut where the face round the hole is, so the two share their
                // corners along the inner edge: the jambs at the heights the bands beside them
                // stop at, the head and sill at the bands' edges across the hole. A head whole
                // under a face cut by the window over the door left T-junctions along their
                // edge, white flakes in a line in low sun (the owner, 2026-10-21).
                let up: Vec<f64> = std::iter::once(y0)
                    .chain(
                        cuts.iter()
                            .copied()
                            .filter(|&c| c > y0 + 0.005 && c < y1 - 0.005),
                    )
                    .chain(std::iter::once(y1))
                    .collect();
                let across: Vec<f64> = std::iter::once(lo)
                    .chain(
                        edges
                            .iter()
                            .copied()
                            .filter(|&e| e > lo + 0.005 && e < hi - 0.005),
                    )
                    .chain(std::iter::once(hi))
                    .collect();
                for (x, n) in [(lo, tx), (hi, -tx)] {
                    let (a, b) = (p(x, from), p(x, deep));
                    for pair in up.windows(2) {
                        let (ya, yb) = (pair[0], pair[1]);
                        let (va, vb) = ((-(ya - y0) / 3.0) as f32, (-(yb - y0) / 3.0) as f32);
                        triangle_uv(
                            mesh,
                            [at(a, ya), at(b, ya), at(b, yb)],
                            [[0.0, va], [r, va], [r, vb]],
                            n,
                            plaster,
                        );
                        triangle_uv(
                            mesh,
                            [at(a, ya), at(b, yb), at(a, yb)],
                            [[0.0, va], [r, vb], [0.0, vb]],
                            n,
                            plaster,
                        );
                    }
                }
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
                let bottom = caps[1].0;
                for &(y, n, section) in &caps {
                    for pair in across.windows(2) {
                        let (xa, xb) = (pair[0], pair[1]);
                        let (a, b, c, d) = (p(xa, from), p(xb, from), p(xb, deep), p(xa, deep));
                        let (ua, ub) = (((xa - lo) / 3.0) as f32, ((xb - lo) / 3.0) as f32);
                        triangle_uv(
                            mesh,
                            [at(a, y), at(b, y), at(c, y)],
                            [[ua, 0.0], [ub, 0.0], [ub, r]],
                            n,
                            section,
                        );
                        triangle_uv(
                            mesh,
                            [at(a, y), at(c, y), at(d, y)],
                            [[ua, 0.0], [ub, r], [ua, r]],
                            n,
                            section,
                        );
                    }
                }
                // Behind the joint with the kit's lining (at `from`), a strip 5 mm into the wall
                // round the hole: the lining and these reveals are two meshes whose common edge
                // never quite closes, and pinholes in a line up the reveal showed the sky
                // (2026-10-17). Hidden but through those pinholes.
                let (j0, j1) = ((from - 0.03).max(0.0), (from + 0.03).min(deep));
                let (b0, b1) = (bottom - 0.005, y1 + 0.005);
                for (x, n) in [(lo - 0.005, tx), (hi + 0.005, -tx)] {
                    let (a, b) = (p(x, j0), p(x, j1));
                    triangle_uv(
                        mesh,
                        [at(a, b0), at(b, b0), at(b, b1)],
                        [[0.0; 2]; 3],
                        n,
                        plaster,
                    );
                    triangle_uv(
                        mesh,
                        [at(a, b0), at(b, b1), at(a, b1)],
                        [[0.0; 2]; 3],
                        n,
                        plaster,
                    );
                }
                for &(y, n, section) in &[(b1, Vec3::NEG_Y, plaster), (b0, Vec3::Y, caps[1].2)] {
                    let (a, b, c, d) = (
                        p(lo - 0.005, j0),
                        p(hi + 0.005, j0),
                        p(hi + 0.005, j1),
                        p(lo - 0.005, j1),
                    );
                    triangle_uv(
                        mesh,
                        [at(a, y), at(b, y), at(c, y)],
                        [[0.0; 2]; 3],
                        n,
                        section,
                    );
                    triangle_uv(
                        mesh,
                        [at(a, y), at(c, y), at(d, y)],
                        [[0.0; 2]; 3],
                        n,
                        section,
                    );
                }
            }
        }
        // The floors and the ceilings: the footprint inside its walls, a slab a storey, the
        // ground floor 2 cm over the walking level its door's sill stands on (5 cm up, a garden's
        // lawn showed through it), the top ceiling under the roof.
        let Some(ring) = inner_ring_into(m, s, 0.01) else {
            count += (mesh.indices.len() - before) / 3;
            continue;
        };
        // A piece of a floor or a ceiling, its triangles split at its neighbours' `corners`
        // ([`conforming_triangles`]).
        let cap = |mesh: &mut TriMesh,
                   ring: &[DVec2],
                   corners: &[DVec2],
                   y: f64,
                   normal: Vec3,
                   section: u8| {
            for t in conforming_triangles(ring, corners) {
                triangle_uv(mesh, t.map(|p| at(p, y)), [[0.0; 2]; 3], normal, section);
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
                let corners: Vec<DVec2> = ground.iter().flatten().copied().collect();
                for poly in &ground {
                    cap(
                        mesh,
                        poly,
                        &corners,
                        st.level.max(WALK_LEVEL + 0.02),
                        Vec3::Y,
                        KIT_INSIDE_FLOOR,
                    );
                }
            } else {
                let pieces = hole(k).map_or_else(|| vec![ring.clone()], |h| without(&ring, &h));
                let corners: Vec<DVec2> = pieces.iter().flatten().copied().collect();
                for poly in &pieces {
                    cap(mesh, poly, &corners, st.level, Vec3::Y, KIT_INSIDE_FLOOR);
                    cap(mesh, poly, &corners, st.level - st.slab, -Vec3::Y, plaster);
                }
                // The opening's four edges through the slab, facing into it: with the slab's top
                // and underside alone, the opening's rim was a slot one saw through from the hall
                // (the owner, 2026-10-17: "a kind of black rectangle ... it looks like a gap in
                // the geometry").
                if let Some(h) = hole(k) {
                    let middle = (h[0] + h[1] + h[2] + h[3]) * 0.25;
                    for i in 0..4 {
                        let (a, b) = (h[i], h[(i + 1) % 4]);
                        let t = (b - a).normalize_or_zero();
                        let mut n = DVec2::new(-t.y, t.x);
                        if n.dot(middle - a) < 0.0 {
                            n = -n;
                        }
                        let normal = Vec3::new(n.x as f32, 0.0, n.y as f32);
                        let (y0, y1) = (st.level - st.slab, st.level);
                        let (l, d) = ((a.distance(b) / 3.0) as f32, (st.slab / 3.0) as f32);
                        triangle_uv(
                            mesh,
                            [at(a, y0), at(b, y0), at(b, y1)],
                            [[0.0, 0.0], [l, 0.0], [l, -d]],
                            normal,
                            plaster,
                        );
                        triangle_uv(
                            mesh,
                            [at(a, y0), at(b, y1), at(a, y1)],
                            [[0.0, 0.0], [l, -d], [0.0, -d]],
                            normal,
                            plaster,
                        );
                    }
                }
            }
        }
        if let Some(last) = s.storeys.last() {
            let ceiling = (last.level + last.height - last.slab).min(top - 0.05);
            cap(mesh, &ring, &[], ceiling, -Vec3::Y, plaster);
            // Its top, 2 cm over it: seen from above through the gap between the cornice and
            // the roof's edge, the ceiling faced away and the lit room below showed as a bright
            // strip along the eaves at night (the owner, 2026-10-24: "light is leaking on the
            // roof as well").
            cap(mesh, &ring, &[], ceiling + 0.02, Vec3::Y, plaster);
        }
        // The walls' tops, 2 cm under the roof's edge, from the building line to their inner
        // faces: the walls are hollow, and looking down between the parapet and the roof one
        // saw into the lit rooms through the inner faces' backs, a bright band round the roof
        // at night (the owner, 2026-10-24: "light is leaking on the roof").
        if let Some(inner) = inner_ring_into(m, s, 0.0).filter(|r| r.len() == m.footprint.len()) {
            let n = m.footprint.len();
            let y = top - 0.02;
            for k in 0..n {
                let quad = [
                    m.footprint[k],
                    m.footprint[(k + 1) % n],
                    inner[(k + 1) % n],
                    inner[k],
                ];
                for t in [[0, 1, 2], [0, 2, 3]] {
                    triangle_uv(
                        mesh,
                        t.map(|i| at(quad[i], y)),
                        [[0.0; 2]; 3],
                        Vec3::Y,
                        KIT_PLASTER + tint_of(m.lot),
                    );
                }
            }
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
    inner_ring_into(m, s, 0.0)
}

/// [`inner_ring`] reaching `into` metres into the walls: the floors and the ceilings 1 cm into
/// them, so their edges hide inside the walls (on the walls' faces, pinholes along every edge
/// showed the light behind them).
fn inner_ring_into(m: &Mass, s: &Structure, into: f64) -> Option<Vec<DVec2>> {
    let d: Vec<f64> = s
        .walls
        .iter()
        .map(|w| {
            if w.role == WallRole::Inner {
                0.05 - into
            } else {
                w.thickness.max(0.2) - into
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

/// How thick a flight's balustrade is, metres.
const RAIL: f64 = 0.075;
/// How high a flight's balustrade stands over the steps' nosings, metres.
const RAIL_HEIGHT: f64 = 0.9;
/// How high the steps stand where a flight's balustrade begins, metres: below, its side is open
/// to the hall beside it, so its foot is entered from there as well as from its end (#285: the
/// balustrade from the first step boxed the foot in against the back wall, a pocket a walker
/// 0.6 m across turned into with centimetres to spare).
const RAIL_FROM: f64 = 0.45;

/// The first tread a flight's balustrade stands over, for steps `rise` metres high.
fn rail_start(rise: f64) -> u32 {
    ((RAIL_FROM / rise.max(1e-3)).ceil() as u32).saturating_sub(1)
}

/// The balustrade along a flight's open side: a plastered half wall [`RAIL`] thick and
/// [`RAIL_HEIGHT`] over the steps' nosings, on the side away from the walls, its outer face `out`
/// outside the steps' side (the string's plane). Open, a walker fell off the flight's side into
/// the corridor, up to 3.7 m at its head (2026-10-17). Across the flight from `stair[0]`, its
/// outer face, its inner face and the outer face's side (1 towards `stair[1]`, −1 away); none
/// where that side stands within 0.3 m of a wall. `r`: the flight's risers.
fn balustrade(
    s: &Structure,
    stair: [DVec2; 4],
    climb: DVec2,
    r: u32,
    out: f64,
) -> Option<(f64, f64, f64)> {
    let across = stair[1] - stair[0];
    let width = across.length();
    let a = across / width.max(1e-6);
    let to_walls = |q: DVec2| {
        s.walls
            .iter()
            .map(|w| geom::distance_to_segment(q, w.a, w.b))
            .fold(f64::MAX, f64::min)
    };
    let mid = stair[0]
        + climb.normalize_or_zero() * (f64::from(r.saturating_sub(1)) * cores::GOING * 0.5);
    let (outer, inner, sign) = if to_walls(mid + a * width) > to_walls(mid) {
        (width + out, width + out - RAIL, 1.0)
    } else {
        (-out, RAIL - out, -1.0)
    };
    (to_walls(mid + a * (outer + sign * 0.3)) > 0.3).then_some((outer, inner, sign))
}

/// Each flight's soffit and strings (`interior_solids` draws its steps), plastered as the walls:
/// - under its steps, a slope through the steps' back lower corners, from where it leaves the
///   floor to the last step: from below the flight read as every step's flat bottom, a sawtooth;
/// - on each side one face, the steps' profile over that slope (a closed string), 5 mm outside
///   the steps' sides: the steps' own sides met each other half along their edges, and the light
///   showed through the seams (the owner, 2026-10-17: "the side of the stairs have strange
///   patterns").
fn soffits(
    mesh: &mut TriMesh,
    at: &dyn Fn(DVec2, f64) -> Vec3,
    s: &Structure,
    inner: &interior::Interior,
    section: u8,
) {
    /// How far the strings stand out of the steps' sides, metres.
    const OUT: f64 = 0.005;
    let Some(stair) = inner.stair else {
        return;
    };
    let across = stair[1] - stair[0];
    let width = across.length();
    let a = across / width.max(1e-6);
    let c = inner.climb.normalize_or_zero();
    let g = cores::GOING;
    let side = Vec3::new(a.x as f32, 0.0, a.y as f32);
    let along = Vec3::new(c.x as f32, 0.0, c.y as f32);
    let (lo, hi) = (-OUT, width + OUT);
    for (k, &r) in inner.risers.iter().enumerate() {
        if k + 1 >= s.storeys.len() || r < 3 {
            break;
        }
        let base = floor_level(s, k);
        let rise = (s.storeys[k + 1].level - base) / f64::from(r);
        // The underside: two risers and 15 cm under the nosings' line, through each step's
        // back lower corner.
        let under = |v: f64| base + (v / g - 1.0) * rise - 0.15;
        let (v0, v1) = (g * (1.0 + 0.15 / rise), f64::from(r - 1) * g);
        if v1 <= v0 + 0.05 {
            continue;
        }
        let top = base + f64::from(r - 1) * rise;
        let p = |v: f64, u: f64, y: f64| at(stair[0] + c * v + a * u, y);
        let slope = Vec3::new(c.x as f32, (rise / g) as f32, c.y as f32).normalize();
        let mut down = slope.cross(side).normalize();
        if down.y > 0.0 {
            down = -down;
        }
        let quad = |mesh: &mut TriMesh, q: [Vec3; 4], n: Vec3| {
            triangle_uv(mesh, [q[0], q[1], q[2]], [[0.0; 2]; 3], n, section);
            triangle_uv(mesh, [q[0], q[2], q[3]], [[0.0; 2]; 3], n, section);
        };
        quad(
            mesh,
            [
                p(v0, lo, under(v0)),
                p(v1, lo, under(v1)),
                p(v1, hi, under(v1)),
                p(v0, hi, under(v0)),
            ],
            down,
        );
        // Its head, from the underside up to the last step's top.
        quad(
            mesh,
            [
                p(v1, lo, under(v1)),
                p(v1, hi, under(v1)),
                p(v1, hi, top),
                p(v1, lo, top),
            ],
            along,
        );
        // The strings: the steps' profile, then down the head and back along the underside.
        let mut ring: Vec<DVec2> = vec![DVec2::new(0.0, base)];
        for i in 0..r - 1 {
            let y = base + f64::from(i + 1) * rise;
            ring.push(DVec2::new(f64::from(i) * g, y));
            ring.push(DVec2::new(f64::from(i + 1) * g, y));
        }
        ring.push(DVec2::new(v1, under(v1)));
        ring.push(DVec2::new(v0, base));
        if geom::signed_area(&ring) < 0.0 {
            ring.reverse();
        }
        for (u, n) in [(lo, -side), (hi, side)] {
            for t in geom::triangulate(&ring) {
                let q = t.map(|i| p(ring[i].x, u, ring[i].y));
                triangle_uv(mesh, q, [[0.0; 2]; 3], n, section);
            }
        }
        // The balustrade on its open side (`balustrade`): its faces over the steps' profile up
        // to its sloped top, on the string's plane outside, and its top and ends.
        // From its first tread [`RAIL_FROM`] up (`rail_start`): below, the side is open.
        let first = rail_start(rise);
        if first + 1 < r
            && let Some((outer, inner_u, sign)) = balustrade(s, stair, inner.climb, r, OUT)
        {
            let over = |v: f64| base + rise + v / g * rise + RAIL_HEIGHT;
            let (start, last) = (f64::from(first) * g, f64::from(r - 1) * g);
            let mut wall: Vec<DVec2> = Vec::new();
            for i in first..r - 1 {
                let y = base + f64::from(i + 1) * rise;
                wall.push(DVec2::new(f64::from(i) * g, y));
                wall.push(DVec2::new(f64::from(i + 1) * g, y));
            }
            wall.push(DVec2::new(last, over(last)));
            wall.push(DVec2::new(start, over(start)));
            if geom::signed_area(&wall) < 0.0 {
                wall.reverse();
            }
            let n = side * sign as f32;
            for (u, face) in [(outer, n), (inner_u, -n)] {
                for t in geom::triangulate(&wall) {
                    let q = t.map(|i| p(wall[i].x, u, wall[i].y));
                    triangle_uv(mesh, q, [[0.0; 2]; 3], face, section);
                }
            }
            quad(
                mesh,
                [
                    p(start, inner_u, over(start)),
                    p(start, outer, over(start)),
                    p(last, outer, over(last)),
                    p(last, inner_u, over(last)),
                ],
                -down,
            );
            let low = base + f64::from(first + 1) * rise;
            quad(
                mesh,
                [
                    p(start, inner_u, low),
                    p(start, outer, low),
                    p(start, outer, over(start)),
                    p(start, inner_u, over(start)),
                ],
                -along,
            );
            let foot = base + f64::from(r - 1) * rise;
            quad(
                mesh,
                [
                    p(last, inner_u, foot),
                    p(last, outer, foot),
                    p(last, outer, over(last)),
                    p(last, inner_u, over(last)),
                ],
                along,
            );
        }
        // The cupboard under the flight (the owner, 2026-10-17: "Below the stairs there can be
        // different type of cupboard or things to fill up the space"): its side to the corridor
        // closed under the slope, its end under the head, and an oak door in the side where it
        // stands 1.2 m or more, by the head (a coat cupboard by the front door). Under the
        // ground floor's flight only: the flights stack in one box, and under an upper one is the
        // stairwell over the flight below. Its cupboard hung from that floor's level across the
        // lower flight's head, three risers over its steps (the owner, 2026-10-17: "is there
        // enough space for people to pass in this stair case?").
        if k > 0 {
            continue;
        }
        let from_walls = |q: DVec2| {
            s.walls
                .iter()
                .map(|w| geom::distance_to_segment(q, w.a, w.b))
                .fold(f64::MAX, f64::min)
        };
        let middle = (v0 + v1) * 0.5;
        let (u, n, out) = if from_walls(stair[0] + c * middle + a * hi)
            > from_walls(stair[0] + c * middle + a * lo)
        {
            (hi, side, 1.0)
        } else {
            (lo, -side, -1.0)
        };
        triangle_uv(
            mesh,
            [p(v0, u, base), p(v1, u, base), p(v1, u, under(v1))],
            [[0.0; 2]; 3],
            n,
            section,
        );
        quad(
            mesh,
            [
                p(v1, lo, base),
                p(v1, hi, base),
                p(v1, hi, under(v1)),
                p(v1, lo, under(v1)),
            ],
            along,
        );
        let (d1, d0) = (v1 - 0.15, v1 - 0.75);
        let head = (under(d0) - 0.1).min(base + 2.0);
        if d0 > v0 && head - base >= 1.2 {
            let q = |v: f64, w: f64| stair[0] + c * v + a * (u + out * w);
            let mut leaf = vec![q(d0, 0.0), q(d1, 0.0), q(d1, 0.02), q(d0, 0.02)];
            if geom::signed_area(&leaf) < 0.0 {
                leaf.reverse();
            }
            solid(mesh, at, &leaf, base + 0.02, head, KIT_INSIDE_WOOD);
            // Its knob at a hand's height, 8 cm in from its edge away from the head.
            let mut knob = vec![
                q(d0 + 0.06, 0.02),
                q(d0 + 0.10, 0.02),
                q(d0 + 0.10, 0.05),
                q(d0 + 0.06, 0.05),
            ];
            if geom::signed_area(&knob) < 0.0 {
                knob.reverse();
            }
            solid(mesh, at, &knob, base + 0.98, base + 1.02, KIT_INSIDE_WOOD);
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
    // Each partition 1 cm into the walls it meets, the floor and the ceiling: ending on their
    // faces, pinholes along every corner showed the light behind them (2026-10-17).
    const INTO: f64 = 0.01;
    for p in &inner.partitions {
        let st = &s.storeys[p.storey];
        let (y0, y1) = (
            floor_level(s, p.storey) - INTO,
            (st.level + st.height - st.slab).min(top - 0.05) + INTO,
        );
        let t = (p.b - p.a).normalize_or_zero();
        let side = DVec2::new(-t.y, t.x) * (p.thickness * 0.5);
        let piece = |from: f64, to: f64| {
            let (a, b) = (p.a + t * from, p.a + t * to);
            vec![a - side, b - side, b + side, a + side]
        };
        let mut from = -INTO;
        for &(d0, d1) in &p.doors {
            if d0 > from + 0.01 {
                out.push((piece(from, d0), y0, y1, false));
            }
            out.push((piece(d0, d1), y0 + INTO + interior::DOORWAY_HEAD, y1, false));
            from = d1;
        }
        if p.length() > from + 0.01 {
            out.push((piece(from, p.length() + INTO), y0, y1, false));
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
            // The cupboard under the ground floor's flight (`soffits` closes it): a metre of it a
            // walker meets, where the slope stands over that, so no one walks in under the steps.
            // An upper flight stands over the stairwell (`soffits`).
            let (vc, v1) = (
                cores::GOING * (1.0 + 1.15 / rise),
                f64::from(r.saturating_sub(1)) * cores::GOING,
            );
            if k == 0 && v1 > vc + 0.1 {
                let (p0, p1) = (inner.climb * vc, inner.climb * v1);
                let under = vec![stair[0] + p0, stair[1] + p0, stair[1] + p1, stair[0] + p1];
                out.push((under, base, base + 1.0, false));
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
            // No guard across the head where the next flight starts there: the flights stack in
            // one box, and across the foot of the one above it walled that flight off.
            let next = k + 2 < s.storeys.len() && inner.risers.get(k + 1).is_some_and(|&r| r >= 2);
            let guards = if next { vec![side] } else { vec![side, head] };
            for ring in guards {
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
        // The flights' balustrades (`soffits` draws them), a box over each tread.
        if let Some(stair) = inner.stair {
            let across = (stair[1] - stair[0]).normalize_or_zero();
            for (k, &r) in inner.risers.iter().enumerate() {
                if k + 1 >= s.storeys.len() || r < 3 {
                    break;
                }
                let Some((outer, inside, _)) = balustrade(s, stair, inner.climb, r, 0.005) else {
                    continue;
                };
                let base = floor_level(s, k);
                let rise = (s.storeys[k + 1].level - base) / f64::from(r);
                let (u0, u1) = (outer.min(inside), outer.max(inside));
                for t in rail_start(rise)..r - 1 {
                    let (v0, v1) = (f64::from(t) * cores::GOING, f64::from(t + 1) * cores::GOING);
                    let ring = [(v0, u0), (v0, u1), (v1, u1), (v1, u0)]
                        .map(|(v, u)| stair[0] + inner.climb * v + across * u);
                    let ring = if geom::signed_area(&ring) < 0.0 {
                        ring.into_iter().rev().collect::<Vec<_>>()
                    } else {
                        ring.to_vec()
                    };
                    let y = base + f64::from(t + 1) * rise;
                    out.push(solid(&ring, y, y + rise + RAIL_HEIGHT));
                }
            }
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

/// `ring`'s triangles, each split where one of `points` lies inside one of its edges: the upper
/// floors round a flight's opening are pieces cut along its edges' lines, and a piece's corner
/// stands in the middle of its neighbour's edge (a T). Rasterised apart, the rounding opened a
/// crack along that edge and the room below showed through it (the owner, 2026-10-24: "small
/// things on the floor").
fn conforming_triangles(ring: &[DVec2], points: &[DVec2]) -> Vec<[DVec2; 3]> {
    let inside = |p: DVec2, a: DVec2, b: DVec2| {
        let ab = b - a;
        let length = ab.length();
        if length < 1e-9 {
            return false;
        }
        let t = (p - a).dot(ab) / length;
        t > 1e-4 && t < length - 1e-4 && geom::cross(ab, p - a).abs() / length < 1e-5
    };
    let mut todo: Vec<[DVec2; 3]> = geom::triangulate(ring)
        .into_iter()
        .map(|t| t.map(|k| ring[k]))
        .collect();
    let mut out = Vec::new();
    while let Some([a, b, c]) = todo.pop() {
        // Bounded: each split takes a point off an edge, and the pieces are a few dozen points.
        if out.len() + todo.len() > 4096 {
            out.push([a, b, c]);
            continue;
        }
        let split = points.iter().find_map(|&p| {
            if inside(p, a, b) {
                Some([[a, p, c], [p, b, c]])
            } else if inside(p, b, c) {
                Some([[a, b, p], [a, p, c]])
            } else if inside(p, c, a) {
                Some([[a, b, p], [p, b, c]])
            } else {
                None
            }
        });
        match split {
            Some([x, y]) => {
                todo.push(x);
                todo.push(y);
            }
            None => out.push([a, b, c]),
        }
    }
    out
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

/// The village's passages (#316, D-076), the world's metres, made with its pieces: each house's
/// ground-floor windows and doors (side A the street's, side B the house's floor) and the garden
/// walls, low (vaulted, up to 1.1 m) or tall (climbed). `piece` numbers them: a mass's index for
/// its openings, `GARDEN_WALL_PIECE` plus the wall's index for a garden wall.
pub fn village_passages(
    size: f64,
    plan: &CityPlan,
    masses: &[Mass],
    structures: &[Structure],
) -> Vec<forge_nav::Passage> {
    use forge_nav::{Passage, PassageKind, PassageState};
    let mut out = Vec::new();
    let v3 = |p: DVec2, y: f64| world(size, p, y);
    for (i, (m, s)) in masses.iter().zip(structures).enumerate() {
        let centroid = geom::centroid(&m.footprint);
        let inside = floor_level(s, 0);
        for o in s.openings.iter().filter(|o| o.storey == 0) {
            let kind = match o.kind {
                OpeningKind::Window => PassageKind::Window,
                OpeningKind::Door => PassageKind::Door,
                _ => continue,
            };
            let w = &s.walls[o.wall];
            if w.role == WallRole::Inner {
                continue;
            }
            let t = (w.b - w.a).normalize_or_zero();
            let mut inward = DVec2::new(-t.y, t.x);
            if inward.dot(centroid - (w.a + w.b) * 0.5) < 0.0 {
                inward = -inward;
            }
            let depth = w.thickness.max(0.2);
            let (lo, hi) = o.span();
            let mid = |along: f64| w.a + t * along + inward * (depth * 0.5);
            let drop = inside - WALK_LEVEL;
            let across = v3(inward, 0.0) - v3(DVec2::ZERO, 0.0);
            out.push(Passage {
                kind,
                ends: [v3(mid(lo), WALK_LEVEL), v3(mid(hi), WALK_LEVEL)],
                across: across.normalize_or_zero(),
                sill: (o.sill + drop) as f32,
                head: Some((o.head + drop) as f32),
                depth: depth as f32,
                drop: drop as f32,
                both_ways: true,
                state: PassageState::Open,
                piece: i as u32,
            });
        }
    }
    for (k, w) in plan.walls.iter().enumerate() {
        let t = (w.b - w.a).normalize_or_zero();
        let left = DVec2::new(-t.y, t.x);
        let (r, l) = w.band();
        let shift = left * ((l - r) * 0.5);
        let kind = if w.height <= 1.1 {
            PassageKind::LowFence
        } else {
            PassageKind::TallFence
        };
        out.push(Passage {
            kind,
            ends: [v3(w.a + shift, WALK_LEVEL), v3(w.b + shift, WALK_LEVEL)],
            across: (v3(left, 0.0) - v3(DVec2::ZERO, 0.0)).normalize_or_zero(),
            sill: w.height as f32,
            head: None,
            depth: (r + l) as f32,
            drop: 0.0,
            both_ways: true,
            state: PassageState::Open,
            piece: GARDEN_WALL_PIECE + k as u32,
        });
    }
    out
}

/// Where the garden walls' numbers start in [`village_passages`]' `piece`.
const GARDEN_WALL_PIECE: u32 = 1 << 24;

/// Pieces as the walk field's solids: each piece's corners in pairs (bottom, top) round its
/// ring, made a prism of triangles, its top facing up.
fn field_prism(piece: &[Vec3], out: &mut Vec<[Vec3; 3]>) {
    let (y0, y1) = (piece[0].y, piece[1].y);
    let ring: Vec<Vec3> = piece.iter().step_by(2).copied().collect();
    let n = ring.len();
    let at = |p: Vec3, y: f32| Vec3::new(p.x, y, p.z);
    for k in 1..n.saturating_sub(1) {
        for (y, up) in [(y1, true), (y0, false)] {
            let mut t = [at(ring[0], y), at(ring[k], y), at(ring[k + 1], y)];
            if ((t[1] - t[0]).cross(t[2] - t[0]).y > 0.0) != up {
                t.swap(1, 2);
            }
            out.push(t);
        }
    }
    for k in 0..n {
        let (a, b) = (ring[k], ring[(k + 1) % n]);
        out.push([at(a, y0), at(b, y0), at(b, y1)]);
        out.push([at(a, y0), at(b, y1), at(a, y1)]);
    }
}

/// House `i`'s walk field: its lot and 4 m round it, the ground, the buildings' walls, its
/// insides and raised floor, each ground-floor door shut by a leaf when `shut`.
pub fn house_field(
    size: f64,
    masses: &[Mass],
    structures: &[Structure],
    passages: &forge_nav::Passages,
    i: usize,
    shut: bool,
) -> forge_nav::Field {
    use forge_nav::{Field, FieldDesc, PERSON, PassageKind};
    let (m, s) = (&masses[i], &structures[i]);
    let walls = wall_pieces(size, masses, structures);
    let (lo, hi) = m.footprint.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let (lo, hi) = (world(size, lo - 4.0, 0.0), world(size, hi + 4.0, 0.0));
    let desc = FieldDesc {
        corner: glam::Vec2::new(lo.x, lo.z),
        cells: (((hi.x - lo.x).max(hi.z - lo.z) / 0.1).ceil() as u32).min(1024),
        cell: 0.1,
    };
    let mut triangles = Vec::new();
    let y = WALK_LEVEL as f32;
    let g = [
        Vec3::new(lo.x, y, lo.z),
        Vec3::new(hi.x, y, lo.z),
        Vec3::new(hi.x, y, hi.z),
        Vec3::new(lo.x, y, hi.z),
    ];
    triangles.push([g[0], g[2], g[1]]);
    triangles.push([g[0], g[3], g[2]]);
    if let Some(ring) = inner_ring(m, s) {
        let piece: Vec<Vec3> = ring
            .iter()
            .flat_map(|&p| {
                [
                    world(size, p, WALK_LEVEL),
                    world(size, p, floor_level(s, 0)),
                ]
            })
            .collect();
        field_prism(&piece, &mut triangles);
    }
    let inside = |p: &[Vec3]| {
        p.iter()
            .any(|q| q.x >= lo.x && q.x <= hi.x && q.z >= lo.z && q.z <= hi.z)
    };
    for piece in walls
        .iter()
        .chain(&interior_pieces(size, masses, structures, &[i]))
        .filter(|p| inside(p))
    {
        field_prism(piece, &mut triangles);
    }
    // Each ground-floor door shut: a leaf across its doorway, up to its head.
    for d in passages
        .all()
        .iter()
        .filter(|p| shut && p.kind == PassageKind::Door && p.piece == i as u32)
    {
        let side = d.across * (d.depth * 0.5);
        let (a, b) = (d.ends[0], d.ends[1]);
        let top = d.ends[0].y + d.head.unwrap_or(2.1);
        let leaf: Vec<Vec3> = [a - side, b - side, b + side, a + side]
            .into_iter()
            .flat_map(|p| [Vec3::new(p.x, y, p.z), Vec3::new(p.x, top, p.z)])
            .collect();
        field_prism(&leaf, &mut triangles);
    }
    Field::build(desc, PERSON, &triangles)
}

/// The house the pursuers go for (#317, `--pursuers`): the first with insides, a ground-floor
/// door and windows; its index, its first door and its ground-floor windows.
pub fn pursued_house(
    masses: &[Mass],
    structures: &[Structure],
    passages: &forge_nav::Passages,
) -> Option<(usize, forge_nav::Passage, Vec<forge_nav::Passage>)> {
    use forge_nav::PassageKind;
    let of = |i: usize, kind: PassageKind| -> Vec<forge_nav::Passage> {
        passages
            .all()
            .iter()
            .filter(|p| p.kind == kind && p.piece == i as u32)
            .copied()
            .collect()
    };
    (0..masses.len()).find_map(|i| {
        interior::house(&masses[i], &structures[i])?;
        let door = *of(i, PassageKind::Door).first()?;
        let windows = of(i, PassageKind::Window);
        (!windows.is_empty()).then_some((i, door, windows))
    })
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

    /// Ends the step `name`, which the loading screen shows as the last done
    /// (`forge_core::activity`).
    fn mark(&mut self, name: &'static str) {
        let now = std::time::Instant::now();
        self.steps.push((name, (now - self.last).as_millis()));
        forge_core::activity::note("village", name, Some(now - self.last));
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

/// The band of the trees' ladder (D-075, `--tree-ladder`): each tree crosses from its cards to
/// its crown's shells over a quarter of the crossing's size either side of it.
const TREE_LADDER_BAND: f32 = 0.25;

/// The band of the crown shells' fade into the plants' impostors (D-075 step 4, `--impostors`).
const IMPOSTOR_FADE_BAND: f32 = 0.25;

/// The leaf atlas's and its normals' seed and size (`textures::leaves`, `leaf_normals`): part of
/// the impostors' cache key, which only names the textures by index.
const LEAF_ATLAS: (u64, u32) = (41, 1024);

/// With the trees' ladder, the distance from the camera from which the shadow rays meet the
/// plants' shadow proxies in place of their cards (D-075 step 3, `--far-shadows`), metres.
const FAR_SHADOWS: f32 = 60.0;

/// The crown shells' colour as a share of their leaves' (D-075's ladder): opaque and smooth, at
/// the leaves' own colour a far wood stood 1.75 times as bright as its cards.
const CROWN_SHELL_TONE: f32 = 0.55;

/// How strongly the leaf cards' own normals (`textures::leaf_normals`, #346) turn the crown's
/// bent ones: enough for each leaf to catch the light its own way, the crown still lit as a
/// rounded mass (research/leaf-detail.md §2: reduced strength over the bent normal).
const LEAF_NORMAL_STRENGTH: f32 = 0.6;

/// The forest's species in the order their first plants come (D-063): each grows
/// `forge_flora::grow::INDIVIDUALS` individuals, at a quarter, half and all of its maturity,
/// three of each.
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
    /// kit's modules, the tiles' insides), each plant's with its height and crown and followed by
    /// its crown's proxy for the probes' rays ([`forge_geom::flora::grown_proxy`]), its crown's
    /// shells, the next step on its ladder ([`forge_geom::flora::grown_shells`]), and the proxy the
    /// far shadow rays meet ([`forge_geom::flora::grown_shadow_proxy`]). Each is kept
    /// in the derived cache (D-053) under its name (its place in the scene, so a changed tile
    /// replaces its old copy) and a key: its content's digest, its cook's options and the
    /// cooking code's digest (`cook_code`).
    fn cook_meshes(
        &self,
        species: &[forge_flora::species::Species],
    ) -> Vec<(MeshletMesh, [f32; 3])> {
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
            .chain(planted(&self.flora, species).into_iter().flat_map(|sp| {
                (0..forge_flora::grow::INDIVIDUALS).map(move |k| Cook::Plant(sp, k))
            }))
            .chain(self.kit_meshes.iter().enumerate().map(|(m, mesh)| {
                // Hard surfaces: a door or a drawer proud of its face folds onto it otherwise
                // (#309).
                Cook::Mesh(mesh, CookOptions::HARD_SURFACE, format!("{scene}-kit{m}"))
            }))
            .chain(
                self.tiles
                    .iter()
                    .enumerate()
                    .filter(|(_, tile)| !tile.insides.indices.is_empty())
                    .map(|(t, tile)| {
                        let name = format!("{scene}-tile{t}-insides");
                        Cook::Mesh(&tile.insides, CookOptions::HARD_SURFACE, name)
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
            size: [f32; 3],
        }
        let code = forge_geom::cache::cook_code();
        let made: Vec<Made> = par_map(jobs.len(), cores(), None, |i| {
            let meshes = match &jobs[i] {
                Cook::Mesh(mesh, options, name) => {
                    vec![(Cow::Borrowed(*mesh), *options, name.clone(), [0.0; 3])]
                }
                &Cook::Plant(sp, k) => {
                    let s = &species[usize::from(sp)];
                    let plant =
                        forge_geom::flora::grow_plant(s, &forge_flora::grow::individual(s, sp, k));
                    let size = [plant.height, plant.crown, plant.lump];
                    let name = format!("plant-{}-{k}", s.id);
                    let options = CookOptions::default();
                    // And its crown as the probes' rays see it (#297), right after it, then its
                    // crown's shells, the next step on its ladder, and its far shadows' proxy
                    // (D-075, `--tree-ladder`).
                    vec![
                        (Cow::Owned(plant.mesh), options, name.clone(), size),
                        (
                            Cow::Owned(plant.proxy),
                            options,
                            name.clone() + "-proxy",
                            size,
                        ),
                        (
                            Cow::Owned(plant.shells),
                            options,
                            name.clone() + "-shells",
                            size,
                        ),
                        (Cow::Owned(plant.shadow), options, name + "-shadow", size),
                    ]
                }
            };
            meshes
                .into_iter()
                .map(|(mesh, options, name, size)| {
                    let key = format!(
                        "plan mesh {:016x} {options:?} code {code:016x}",
                        mesh_digest(&mesh)
                    );
                    Made {
                        mesh,
                        options,
                        name,
                        key,
                        size,
                    }
                })
                .collect::<Vec<_>>()
        })
        .into_iter()
        .flatten()
        .collect();
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
        // The loading screen's count (`forge_core::activity`): the meshes read or cooked so far.
        let task = forge_core::activity::begin(
            "cooking",
            format!(
                "the meshes: {} cached, {} to cook",
                hits.len(),
                misses.len()
            ),
        );
        let total = (hits.len() + misses.len()) as u64;
        let finished = std::sync::atomic::AtomicU64::new(0);
        let counted = |m: &Made| {
            let done = take(m);
            let n = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            task.progress(n, total);
            done
        };
        let loaded = par_map(hits.len(), cores(), None, |j| counted(&made[hits[j]]));
        let order = largest_first(misses.iter().map(|&i| made[i].mesh.indices.len()));
        let cooked_now = par_map(misses.len(), (cores() / 3).max(2), Some(&order), |j| {
            counted(&made[misses[j]])
        });
        drop(task);
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
            .map(|(mesh, m)| (mesh.expect("every mesh taken"), m.size))
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
            // 3.70 ms): 128. And 1 m apart since, as the models lab's rooms: at 2 m a room
            // 3 m wide had a probe or two across, and a probe's sunlight came through its wall:
            // a bright patch with a hard edge on the wall and the floor by its doorway (gone with
            // `--no-probes`); 1 m apart it is gone, for 0.2–0.4 ms indoors (the coarsest cascade
            // 32 m apart).
            probes: (!self.flora.is_empty()).then_some((128, 1.0, 6)),
            // And metered: at the street's fixed exposure (EV 14) a hall lit through its door
            // and the ground under the holm oaks were black (the owner, 2026-10-16: "insides are
            // way too dark", "light below trees is too dark"); the light was there, the eye
            // adapts to it (the hall metered at EV 9.4, the wood at 11.0, a garden at 13.4).
            metered: !self.flora.is_empty(),
            // Local exposure with it (D-067, #275): a room seen from a lit hall through its door
            // was pitch black; lifted, it reads a bit darker, as the eye sees it.
            local_exposure: !self.flora.is_empty(),
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
        // What the ground sounds like underfoot (D-007): the open land, the gardens and the
        // parks grass, the water and its mirrors water, the rest rock's stone.
        for (section, surface) in [
            (GROUND, SoundSurface::Grass),
            (GARDEN, SoundSurface::Grass),
            (PARK, SoundSurface::Grass),
            (WATER, SoundSurface::Water),
            (MIRROR, SoundSurface::Water),
        ] {
            let sound = SoundLayer {
                surface: Some(surface),
            };
            materials.set_section_sound("plan-ground", section, sound);
        }
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
                // The insides' joinery (`KIT_INSIDE_WOOD`): a waxed oak.
                (
                    "plan inside wood".into(),
                    colour([0.42, 0.28, 0.16], 30.0, 0.08),
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
            // By the modules' sections (`modules::PLASTER` to `modules::FROSTED`).
            let rows: Vec<(String, RenderLayer)> = vec![
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
                // A bathroom's obscured glass (`modules::FROSTED`): milky and rough, no room
                // seen behind it, glowing softly when the room is lit.
                (
                    format!("{group} frosted glass"),
                    RenderLayer {
                        emissive: (k % 2 == 1)
                            .then_some(LIT[k / 2])
                            .map_or([0.0; 3], |(l, c)| c.map(|v| v * l * 0.6)),
                        emissive_absolute: true,
                        // The daylight comes in through it (D-068).
                        glazed: true,
                        ..colour([0.80, 0.82, 0.82], 20.0, 0.04)
                    },
                ),
            ];
            // The same for the houses whose insides are drawn: their clear glass drawn over the
            // rooms after the resolve (D-065's part 4), the rays meeting the faked room as before.
            let open = KIT_OPEN_GROUPS[k];
            let open_rows = rows
                .iter()
                .enumerate()
                .map(|(s, (name, layer))| {
                    let glass = s == usize::from(modules::GLASS);
                    let layer = RenderLayer {
                        drawn_after: glass,
                        ..*layer
                    };
                    (name.replacen(group, open, 1), layer)
                })
                .collect();
            materials.add_rows(group, rows);
            materials.add_rows(open, open_rows);
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
        // The houses' lamps (D-065): frosted globes of about 2 000 cd/m² at 2 700 K where lit (a
        // 60 cd bulb behind a globe 9 cm in radius), and their brass fittings. A lamp's light
        // reaches the rooms as a lamp, so the probes' rays take none of the globe's glow.
        for (group, lit) in [("plan-kit-lamp", true), ("plan-kit-lamp-off", false)] {
            let globe = 2000.0;
            materials.add_rows(
                group,
                vec![
                    (format!("{group} render"), render([0.94, 0.88, 0.76])),
                    (format!("{group} stone"), stone),
                    (
                        format!("{group} frame"),
                        colour([0.35, 0.26, 0.12], 60.0, 0.3),
                    ),
                    (
                        format!("{group} globe"),
                        RenderLayer {
                            emissive: if lit {
                                [globe, globe * 0.78, globe * 0.55]
                            } else {
                                [0.0; 3]
                            },
                            emissive_absolute: true,
                            lamp: true,
                            ..colour([0.86, 0.85, 0.82], 20.0, 0.05)
                        },
                    ),
                    (
                        format!("{group} paint"),
                        colour([0.35, 0.26, 0.12], 60.0, 0.3),
                    ),
                    (
                        format!("{group} brass"),
                        colour([0.35, 0.26, 0.12], 60.0, 0.3),
                    ),
                ],
            );
            kit_rows.push(materials.of(group));
        }
        // The open groups, from OPEN_GROUP on.
        debug_assert_eq!(kit_rows.len(), usize::from(OPEN_GROUP));
        kit_rows.extend(KIT_OPEN_GROUPS.iter().map(|g| materials.of(g)));
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
        let (leaf_atlas, leaf_normals) = if self.flora.is_empty() {
            (None, None)
        } else {
            (
                Some(
                    materials
                        .textures
                        .add(&textures::leaves(LEAF_ATLAS.0, LEAF_ATLAS.1))?,
                ),
                // Each leaf leaning its own way (#346).
                Some(
                    materials
                        .textures
                        .add(&textures::leaf_normals(LEAF_ATLAS.0, LEAF_ATLAS.1))?,
                ),
            )
        };
        // The decals' textures (D-074): the grime at the walls' feet, a strip repeating along u;
        // a stain and cracks on the paving, stamps.
        let decal_textures = if self.decals.is_empty() || args.no_decals {
            None
        } else {
            Some([
                materials.textures.add(&textures::decal_grime(74, 512))?,
                materials.textures.add(&textures::decal_stain(75, 256))?,
                materials.textures.add(&textures::decal_cracks(76, 256))?,
            ])
        };
        let flora_rows: Vec<_> = species
            .iter()
            .map(|s| {
                let [bark, leaves] = [s.look.bark, s.look.leaves].map(|[a, b]| (a, b));
                let foliage = tones(leaves.0, leaves.1, 6.0);
                // The crowns' cores at 0.15 of the leaves' colour: the shade inside a crown (at
                // half, once the metered exposure lit the wood, their sunlit facets read paler
                // than the cards round them; at three tenths, olive balls through the leaves'
                // gaps close up, #346).
                let shade = |c: [f32; 3]| c.map(|k| k * 0.15);
                materials
                    .add_row_block(vec![
                        (format!("{} bark", s.id), tones(bark.0, bark.1, 8.0)),
                        (
                            format!("{} leaves", s.id),
                            tones(shade(leaves.0), shade(leaves.1), 6.0),
                        ),
                        (
                            format!("{} leaf cards", s.id),
                            RenderLayer {
                                albedo_texture: leaf_atlas,
                                normal_texture: leaf_normals,
                                normal_strength: LEAF_NORMAL_STRENGTH,
                                uv: Some(UvMapping {
                                    alpha_cutoff: Some(0.5),
                                    ..UvMapping::default()
                                }),
                                foliage: true,
                                // Their undersides seen from under the crown and the light
                                // through them, yellower than their faces (#346).
                                underside: s.look.underside,
                                scatter: [1.0, 1.1, 0.6].map(|k| k * s.look.through),
                                ..foliage
                            },
                        ),
                        // The crown's proxy that the probes' rays meet (#297), opaque.
                        (format!("{} crown", s.id), foliage),
                        // The crown's shells, the next step on its ladder (D-075): the cards'
                        // look from afar, their leaves' colour through the atlas's and the
                        // shade the cards cast on each other.
                        (
                            format!("{} crown shells", s.id),
                            tones(
                                leaves.0.map(|k| k * CROWN_SHELL_TONE),
                                leaves.1.map(|k| k * CROWN_SHELL_TONE),
                                6.0,
                            ),
                        ),
                    ])
                    .expect("a species' rows")
            })
            .collect();
        // Lamp heads of about 1 000 cd/m²: shielded LEDs at 2 700 K behind a frosted diffuser (the
        // style bible's lighting). At 3 000 their halos read too strong (the owner). Their light
        // is a lamp's (`forge_render::lamps::street_lights`), so the probes leave their glow out, as
        // a house's globes.
        let head = forge_render::lamps::STREET_LENS;
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
                        lamp: true,
                        ..colour([0.8, 0.8, 0.78], 40.0, 0.2)
                    },
                ),
            ],
        );
        // The mission's parcel (`--mission`, #287): cardboard.
        if self.mission.is_some() {
            materials.add_rows(
                "plan-parcel",
                vec![("plan parcel".into(), colour([0.50, 0.36, 0.22], 6.0, 0.03))],
            );
        }
        let parcel_row = self.mission.is_some().then(|| materials.of("plan-parcel"));
        // The animals' coats (`--fauna`, #269): a group a species and coat, its fur and its
        // dark parts.
        let fauna_layout = self.fauna.as_ref().map(fauna::FaunaRun::layout);
        let mut fauna_rows = Vec::new();
        if let (Some(run), Some(layout)) = (&self.fauna, &fauna_layout) {
            for &(s, c, k, _) in &layout.blocks {
                let sp = &run.species()[s];
                let Some((_, coats)) = fauna::body(sp).filter(|_| k == 0) else {
                    continue;
                };
                let (coat, dark) = coats[c];
                // A name for the run (the materials key their groups by static names).
                let group: &'static str = format!("plan-fauna-{}-{c}", sp.id).leak();
                materials.add_rows(
                    group,
                    vec![
                        (format!("{group} coat"), colour(coat, 10.0, 0.04)),
                        (format!("{group} dark"), colour(dark, 20.0, 0.05)),
                    ],
                );
                fauna_rows.push(((s, c), materials.of(group)));
            }
        }
        let lamp_row = materials.of("plan-lamp");
        let ground_row = materials.of("plan-ground");
        // What each ground section sounds like underfoot (#318, D-007).
        let ground_sounds: Vec<Option<forge_audio::Surface>> = (0..=MIRROR)
            .map(|s| {
                materials
                    .section_sound("plan-ground", s)
                    .surface
                    .map(Into::into)
            })
            .collect();
        let mass_row = materials.of("plan-mass");
        let grille_row = materials.of("plan-grille");
        laps.mark("rows and leaves");
        let mut builder = MeshletSceneBuilder::new();
        // The rows an impostor's bake draws a plant with (`--impostors`, `--impostor-bake`, D-075
        // step 4), and the plants drawn as impostors with the trees' ladder.
        let drawn_impostors = args.tree_ladder.and(args.impostors);
        let bake_rows = (args.impostor_bake.is_some() || drawn_impostors.is_some())
            .then(|| materials.gpu_rows());
        let mut impostors = Vec::new();
        let mut impostor_instances = Vec::new();
        materials.apply(&mut builder, &[], &[]);
        laps.mark("materials applied");
        builder.set_ray_traced(!args.no_shadows);
        // The leaf cards' opacity micromaps for the rays (#333): the same picture, the sun's rays
        // 10-21 % cheaper among the trees (`FORGE_OMM=0` builds without).
        builder.set_opacity_micromaps(true);
        builder.set_origin(scene_origin(args));
        if self.static_test.is_some() || self.promote_test.is_some() {
            builder.allow_static_edits();
        }
        if let Some([grime, stain, cracks]) = decal_textures {
            let stamp = |texture, strength: f32, priority| forge_render::DecalKind {
                texture,
                tint: [1.0, 1.0, 1.0, strength],
                strip: false,
                priority,
                depth_fade: 0.5,
                facing: [0.6, 0.85],
                row: None,
            };
            builder.set_decals(forge_render::DecalSet {
                kinds: vec![
                    forge_render::DecalKind {
                        texture: grime,
                        tint: [1.0; 4],
                        strip: true,
                        priority: 0,
                        depth_fade: 0.3,
                        facing: [0.3, 0.6],
                        row: None,
                    },
                    stamp(stain, 1.0, 1),
                    stamp(cracks, 0.9, 2),
                ],
                decals: self.decals.clone(),
            });
        }
        // The street lamps beyond each frame's set, the rest of their light unshadowed (#284), in
        // the frame the frames' lamps are given in.
        if !args.no_lamps && !args.no_far_lamps {
            let origin = scene_origin(args);
            let street: Vec<_> = self
                .home_lamps
                .iter()
                .filter(|l| l.outside())
                .map(|l| forge_render::lamps::Lamp {
                    position: forge_render::CellPos::from_f64(l.position.as_dvec3())
                        .relative_to(origin),
                    ..*l
                })
                .collect();
            builder.set_far_lamps(&street);
        }
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
                let (mesh, _) = cooked.next().expect("a tile's mesh cooked");
                let id = builder.add_mesh(&mesh);
                builder.add_instance_with_material(id, at, rows[kind]);
                meshes += 1;
            }
        }
        laps.mark("tiles");
        // The forest by species (D-063): a few individuals of each grown once (`forge_flora`),
        // their meshes instanced, each plant scaled to its height and crown.
        let flora_first = builder.instance_count() as u32;
        if !self.flora.is_empty() {
            // Each species' individuals, grown at a quarter, half and all of its maturity, three
            // of each; a plant takes the age nearest its height, so a young street tree is a
            // young tree, not a mature one shrunk (a 10 m plane was a ball on a stem), and of
            // that age the individual its hash picks, its crown stretched by up to 8 % along its
            // turn.
            // The probes' rays meet each plant's crown proxy, not its leaf cards (#297), unless
            // `--leaf-probes`.
            let proxies = !args.leaf_probes;
            // With the ladder, the shadow rays from points `far_shadows` metres off and more meet
            // each plant's opaque shadow proxy in place of its cards (D-075 step 3).
            let far_shadows = args
                .tree_ladder
                .map_or(0.0, |_| args.far_shadows.unwrap_or(FAR_SHADOWS));
            if far_shadows > 0.0 {
                builder.set_far_shadows(far_shadows);
            }
            struct Grown {
                id: forge_render::MeshId,
                proxy: Option<forge_render::MeshId>,
                shadow: Option<forge_render::MeshId>,
                height: f32,
                crown: f32,
                /// Its culling sphere, in its own frame, and its ladder's crossing (0 without).
                centre: Vec3,
                radius: f32,
                ladder_px: f32,
            }
            let mut models: std::collections::HashMap<u16, Vec<Grown>> = Default::default();
            // Each species' impostors (D-075 step 4), one for each age: its kind, the middle of its
            // baked sphere and the crown it was baked with.
            let mut impostor_kinds: std::collections::HashMap<(u16, u8), (u32, Vec3, f32)> =
                Default::default();
            let origin = scene_origin(args);
            for &sp in &planted {
                let grown = (0..forge_flora::grow::INDIVIDUALS)
                    .map(|individual| {
                        let (mesh, [height, crown, _]) = cooked.next().expect("a plant cooked");
                        let (proxy, _) = cooked.next().expect("a plant's crown proxy cooked");
                        let (shells, [.., lump]) =
                            cooked.next().expect("a plant's crown shells cooked");
                        let (shadow, _) = cooked.next().expect("a plant's shadow proxy cooked");
                        let proxy = proxies.then(|| builder.add_mesh(&proxy));
                        let id = builder.add_mesh(&mesh);
                        // The species' last seed of each age is the individual its impostor for
                        // that age is baked from (`--impostors`, `--impostor-bake`).
                        let age = individual % forge_flora::grow::AGES;
                        if let Some(rows) = &bake_rows
                            && individual + forge_flora::grow::AGES
                                >= forge_flora::grow::INDIVIDUALS
                        {
                            impostor_kinds.insert(
                                (sp, age),
                                (impostors.len() as u32, Vec3::from(mesh.center), crown),
                            );
                            impostors.push((
                                format!("{}-{age}", species[usize::from(sp)].id),
                                forge_render::impostor::BakeSource {
                                    mesh: mesh.clone(),
                                    rows: rows.clone(),
                                    material: flora_rows[usize::from(sp)],
                                    // The block's third row: its leaf cards.
                                    leaves: forge_core::material::MaterialId(
                                        flora_rows[usize::from(sp)].0 + 2,
                                    ),
                                    // The leaf atlas's and its normals' seed and size.
                                    inputs: forge_core::derived::KeyHasher::new()
                                        .debug(&LEAF_ATLAS)
                                        .key(0)
                                        .digest(),
                                },
                            ));
                        }
                        // With `--tree-ladder` (D-075, #302): the cards near, the crown's shells
                        // once a lump stands under the size it gives, dithered across a band of a
                        // quarter of it. The ladder measures the plant's sphere: as many times the
                        // lumps' size as its diameter holds their mean diameter.
                        // With `--impostors` the shells fade in turn where the plant's impostor
                        // takes their pixels (D-075 step 4).
                        let mut ladder_px = 0.0;
                        if let Some(px) = args.tree_ladder {
                            let next = builder.add_mesh(&shells);
                            let across = 2.0 * mesh.radius / lump;
                            ladder_px = px * across;
                            builder.set_ladder(id, next, ladder_px, TREE_LADDER_BAND);
                            if let Some(fade) = drawn_impostors {
                                builder.set_fade(next, fade, IMPOSTOR_FADE_BAND);
                            }
                        }
                        // Placed with the ladder whatever the distance, so `--far-shadows 0` keeps
                        // the instances' ids (each tree's tones follow its id) for the A/B.
                        let shadow = args.tree_ladder.map(|_| builder.add_mesh(&shadow));
                        Grown {
                            id,
                            proxy,
                            shadow,
                            height,
                            crown,
                            centre: Vec3::from(mesh.center),
                            radius: mesh.radius,
                            ladder_px,
                        }
                    })
                    .collect();
                models.insert(sp, grown);
            }
            for &(at, height, crown, turn, sp, pick) in &self.flora {
                let Some(grown) = models.get(&sp) else {
                    continue;
                };
                let ages = usize::from(forge_flora::grow::AGES);
                let Some(age) = (0..ages.min(grown.len())).min_by(|&a, &b| {
                    (grown[a].height / height)
                        .ln()
                        .abs()
                        .total_cmp(&(grown[b].height / height).ln().abs())
                }) else {
                    continue;
                };
                let seeds = grown.len().div_ceil(ages);
                let k = age + ages * (usize::from(pick) % seeds);
                let Some(model) = grown.get(k).or(grown.get(age)) else {
                    continue;
                };
                let (id, h, w) = (model.id, model.height, model.crown);
                // To the plan's crown, its proportions kept within [`SQUEEZE`]: planned
                // narrower than its model, it is drawn shorter, not thinner. Squeezed sideways
                // to a street's room, a young umbrella pine kept a trunk of a few centimetres
                // under a crown that seemed to float (the owner, 2026-10-24); widened past the
                // plan, a crown reached the facades the plan kept it from (a review). Planned
                // wider, it is never stretched past the inverse.
                let planned = crown * 2.0 / w;
                let tall = (height / h).min(planned / SQUEEZE);
                let across = planned.min(tall / SQUEEZE);
                let stretch = 0.92 + f32::from(pick >> 2) / 63.0 * 0.16;
                let transform = Mat4::from_scale_rotation_translation(
                    Vec3::new(across * stretch, tall, across / stretch),
                    glam::Quat::from_rotation_y(turn),
                    at,
                );
                let row = flora_rows[usize::from(sp)];
                builder.add_instance_with_material(id, transform, row);
                // The cards for the picture, the shadows and the mirrors; the proxy for the
                // probes alone, and the shadow proxy for the far shadow rays alone.
                let plant = builder.instance_count() - 1;
                builder.set_instance_visibility(
                    plant,
                    forge_render::Visibility {
                        probes: model.proxy.is_none(),
                        far_shadows: model.shadow.is_none(),
                        ..forge_render::Visibility::ALL
                    },
                );
                // Its impostor where its shells fade (`--impostors`): of its species at its age, its
                // crown as wide as this plant's, its fade measured as the culls measure it.
                if drawn_impostors.is_some()
                    && let Some(&(kind, middle, baked_crown)) = impostor_kinds.get(&(sp, age as u8))
                {
                    let (scale, rotation, translation) = transform.to_scale_rotation_translation();
                    let place = origin.offset(translation).normalized();
                    let culled = rotation * (model.centre * scale.x);
                    let size = crown * 2.0 * stretch / baked_crown;
                    impostor_instances.push(forge_render::impostor::ImpostorInstance {
                        kind,
                        position: forge_render::CellPos {
                            cell: place.cell,
                            local: place.local + culled,
                        },
                        middle: rotation * (middle * size) - culled,
                        rotation,
                        scale: size,
                        cull_radius: model.radius * scale.x,
                        id: plant as u32,
                        ladder: Some((model.ladder_px, TREE_LADDER_BAND)),
                    });
                }
                if let Some(proxy) = model.proxy {
                    builder.add_instance_with_material(proxy, transform, row);
                    let at = builder.instance_count() - 1;
                    builder.set_instance_visibility(at, forge_render::Visibility::PROBES);
                }
                if let Some(shadow) = model.shadow {
                    builder.add_instance_with_material(shadow, transform, row);
                    let at = builder.instance_count() - 1;
                    builder.set_instance_visibility(at, forge_render::Visibility::FAR_SHADOWS);
                }
            }
        }
        let flora_end = builder.instance_count() as u32;
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
            let models = [false, true].map(|double| {
                builder.add_mesh(&MeshletMesh::build(&forge_geom::street::lamp(double)))
            });
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
        if let Some(px) = self.fade_furniture {
            for &k in &self.home_kit {
                builder.set_fade(kit_ids[k as usize], px, FURNITURE_FADE_BAND);
            }
        }
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
        // The pursuers (#317), the very last: a mesh a stride, a slot of each for each of them.
        if self.pursuers.is_some() {
            for stride in pursuers::STRIDES {
                let figure = builder.add_mesh(&MeshletMesh::build(&forge_geom::figures::walker(
                    1.75, stride,
                )));
                builder.set_mesh_material(figure, people_rows[1 % people_rows.len()]);
                tram_movers.push((figure, pursuers::COUNT as u32));
            }
        }
        // The mission's parcel (#287), after them: one slot, under the ground until it lies.
        if let Some(row) = parcel_row {
            let parcel = builder.add_mesh(&MeshletMesh::build(&mission::parcel_mesh()));
            builder.set_mesh_material(parcel, row);
            tram_movers.push((parcel, 1));
        }
        // The animals (`--fauna`, #269), after it: a block of slots a species, coat and pose,
        // then the dogs' owners, a block a stride.
        if let (Some(run), Some(layout)) = (&self.fauna, &fauna_layout) {
            for &(s, c, k, n) in &layout.blocks {
                let Some((b, _)) = fauna::body(&run.species()[s]) else {
                    continue;
                };
                let mesh = builder.add_mesh(&MeshletMesh::build(&fauna::mesh(b, k)));
                if let Some(&(_, row)) = fauna_rows.iter().find(|(key, _)| *key == (s, c)) {
                    builder.set_mesh_material(mesh, row);
                }
                tram_movers.push((mesh, n));
            }
            if layout.owners > 0 {
                for stride in pursuers::STRIDES {
                    let figure = builder.add_mesh(&MeshletMesh::build(
                        &forge_geom::figures::walker(1.72, stride),
                    ));
                    builder.set_mesh_material(figure, people_rows[2 % people_rows.len()]);
                    tram_movers.push((figure, layout.owners));
                }
            }
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
        if self.promote_test.is_some() {
            builder.reserve_mover_pool(PROMOTE_POOL);
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
                walls: walk::Walls::new(self.walls.clone(), self.crossings.clone()),
                walker: None,
                walk_on_start: self.walk_on_start,
                glass_test: self.glass_test,
                static_test: self
                    .static_test
                    .map(|at| (self.clock + f64::from(at), flora_first..flora_end)),
                promote_test: self.promote_test.map(|mode| PromoteRun {
                    mode,
                    at: self.clock + 1.0,
                    trees: flora_first..flora_end,
                    tree: Vec::new(),
                    pivot: Vec3::ZERO,
                    done: false,
                }),
                view: (Vec3::ZERO, Vec3::NEG_Z),
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
                pursuers: self.pursuers.take(),
                fauna: self
                    .fauna
                    .take()
                    .zip(fauna_layout)
                    .map(|(run, layout)| (run, layout, fauna::Drawn::default())),
                mission: self.mission.take().and_then(|(m, autopilot)| {
                    mission::MissionRun::new(m, autopilot)
                        .inspect_err(|e| tracing::warn!("the village's mission: {e}"))
                        .ok()
                }),
                steps: None,
                walkers: Vec::new(),
                camera: glam::Vec2::ZERO,
                size: self.size,
                origin: scene_origin(args).to_f64().as_vec3(),
                regions: std::mem::take(&mut self.regions),
                footing: self.footing.take(),
                ground_sounds,
                home_lamps: std::mem::take(&mut self.home_lamps),
                glass: std::mem::take(&mut self.glass),
                grass: self
                    .grass
                    .take()
                    .map(|m| (m, ground_row.0 + u32::from(GARDEN))),
                impostors: (!impostors.is_empty()).then(|| forge_render::impostor::ImpostorSet {
                    sources: impostors,
                    instances: impostor_instances,
                    fade_px: args.impostors.unwrap_or(0.0),
                    fade_band: IMPOSTOR_FADE_BAND,
                }),
                canopy: Canopy::new(
                    self.trees
                        .iter()
                        .map(|&(at, scale, ..)| (glam::Vec2::new(at.x, at.z), 4.2 * scale.x))
                        .chain(
                            self.flora
                                .iter()
                                .map(|&(at, _, crown, ..)| (glam::Vec2::new(at.x, at.z), crown)),
                        ),
                ),
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
    /// When to hide every other clear pane and raise the rest (`--glass-test`, #325), seconds.
    glass_test: Option<f32>,
    /// When to hide the forest, on the service clock, and its instances (`--static-test`, #323).
    static_test: Option<(f64, std::ops::Range<u32>)>,
    /// The tree promoted to a mover and demoted (`--promote-test`, #338).
    promote_test: Option<PromoteRun>,
    /// The camera's position and forward after its last update.
    view: (Vec3, Vec3),
    walk_on_start: bool,
    /// The population on foot round the camera, and where the camera last stood (x, z).
    crowd: Option<crowd::Crowd>,
    /// The pursuers going for a house (`--pursuers`, #317).
    pursuers: Option<pursuers::Pursuers>,
    /// The village's test mission (`--mission`, #287).
    mission: Option<mission::MissionRun>,
    /// The village's animals (`--fauna`, #269), their movers' layout and what their drawing
    /// keeps.
    fauna: Option<(fauna::FaunaRun, fauna::Layout, fauna::Drawn)>,
    /// Their footsteps (#318), once the app has a device.
    steps: Option<sounds::Steps>,
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
    /// The houses' rooms for the probes (D-068), in the world frame.
    regions: Vec<forge_render::RegionVolume>,
    /// The ground's sections under the plan and what each sounds like underfoot (#318).
    footing: Option<footing::Footing>,
    ground_sounds: Vec<Option<forge_audio::Surface>>,
    /// The houses' lamps that are on (D-065), in the world frame.
    home_lamps: Vec<forge_render::lamps::Lamp>,
    /// The houses' clear glass (D-065's part 4), in the world frame, handed over once.
    glass: forge_render::GlassPanes,
    /// The grass map (#300) and its blades' row (the gardens'), in the world frame.
    grass: Option<(forge_render::grass::GrassMap, u32)>,
    /// The plants' impostors (`--impostors`, `--impostor-bake`, D-075 step 4), taken once.
    impostors: Option<forge_render::impostor::ImpostorSet>,
    /// The trees' crowns over the plan, for the ambience's habitat (#350).
    canopy: Canopy,
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

/// How `--promote-test` (#338) moves its tree: one second in, for a second.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromoteTest {
    /// Nothing promoted, the pool and the static edits there: the others' reference.
    None,
    /// Promoted and demoted where it stands: the frames the reference's.
    Hold,
    /// Promoted, leant [`PROMOTE_LEAN`] over the second, and demoted there.
    Swing,
    /// Not promoted: leant where the swing ends when it ends, a static edit (the swing's
    /// reference after it).
    Snap,
}

/// The pool's movers (`--promote-test`): room for a tree's instances.
const PROMOTE_POOL: u32 = 16;
/// `--fade-furniture`'s band, a share of its size either side: the furniture dissolves over
/// about half again its distance.
const FURNITURE_FADE_BAND: f32 = 0.25;
/// How far `--promote-test`'s swing leans its tree, degrees.
const PROMOTE_LEAN: f32 = 15.0;

/// `--promote-test` as it runs.
struct PromoteRun {
    mode: PromoteTest,
    /// When the tree is promoted, on the service clock.
    at: f64,
    /// The forest's instances, the tree picked among them.
    trees: std::ops::Range<u32>,
    /// The tree's instances, with their movers once promoted and where each stood.
    tree: Vec<(u32, Option<u32>, MoverTransform)>,
    /// The foot of the tree, which it leans about.
    pivot: Vec3,
    done: bool,
}

impl PromoteRun {
    /// Its frame, before the movers are committed: the tree before the camera (`eye`, `forward`,
    /// relative to the scene's origin) picked and promoted at its time, its pose given, and a
    /// second later demoted, or moved where the swing ends (`Snap`).
    fn step(&mut self, scene: &MeshletScene, clock: f64, (eye, forward): (Vec3, Vec3)) {
        if self.done || clock < self.at {
            return;
        }
        let flat = |a: Vec3, b: Vec3| glam::Vec2::new(a.x - b.x, a.z - b.z).length();
        if self.tree.is_empty() {
            // The instance nearest a point 12 m before the camera, and those standing where it
            // stands: its tree's trunk, cards and stand-ins.
            let target = eye + forward * 12.0;
            let at = |i: u32| scene.static_transform(i).position;
            let Some(nearest) = self
                .trees
                .clone()
                .min_by(|&a, &b| flat(at(a), target).total_cmp(&flat(at(b), target)))
            else {
                self.done = true;
                return;
            };
            let foot = at(nearest);
            self.tree = self
                .trees
                .clone()
                .filter(|&i| flat(at(i), foot) < 0.5)
                .take(PROMOTE_POOL as usize)
                .map(|i| (i, None, scene.static_transform(i)))
                .collect();
            self.pivot = self
                .tree
                .iter()
                .map(|t| t.2.position)
                .min_by(|a, b| a.y.total_cmp(&b.y))
                .unwrap_or(foot);
            if matches!(self.mode, PromoteTest::Hold | PromoteTest::Swing) {
                for (instance, mover, _) in &mut self.tree {
                    *mover = scene.promote(*instance);
                }
            }
            tracing::info!(
                instances = self.tree.len(),
                promoted = self.tree.iter().filter(|t| t.1.is_some()).count(),
                at = ?self.pivot,
                "promote test: a tree picked (#338)"
            );
        }
        let end = clock >= self.at + 1.0;
        let s = match self.mode {
            PromoteTest::Swing => ((clock - self.at) as f32).min(1.0),
            PromoteTest::Snap => 1.0,
            _ => 0.0,
        };
        let lean = glam::Quat::from_rotation_x((PROMOTE_LEAN * s).to_radians());
        let pivot = self.pivot;
        let pose = |t: &MoverTransform| MoverTransform {
            position: pivot + lean * (t.position - pivot),
            rotation: lean * t.rotation,
            scale: t.scale,
        };
        for (instance, mover, t) in &self.tree {
            match (self.mode, mover) {
                (PromoteTest::Hold | PromoteTest::Swing, Some(mover)) if end => {
                    scene.demote(*mover, pose(t));
                }
                (PromoteTest::Hold | PromoteTest::Swing, Some(mover)) => {
                    let still = self.mode == PromoteTest::Hold;
                    scene.set_movers_at(*mover, &[pose(t)], |_| still);
                }
                (PromoteTest::Snap, _) if end => scene.set_static_transform(*instance, pose(t)),
                _ => {}
            }
        }
        if end {
            self.done = true;
            tracing::info!("promote test: demoted (#338)");
        }
    }
}

impl Running for PlanRunning {
    fn probe_regions(&self) -> Vec<forge_render::RegionVolume> {
        self.regions.clone()
    }

    fn lamps(&mut self, _time: f32) -> Vec<forge_render::lamps::Lamp> {
        self.home_lamps.clone()
    }

    fn glass(&self) -> forge_render::GlassPanes {
        self.glass.clone()
    }

    fn impostors(&mut self) -> Option<forge_render::impostor::ImpostorSet> {
        self.impostors.take()
    }

    fn grass(&self) -> Option<(forge_render::grass::GrassMap, u32)> {
        self.grass.clone()
    }

    fn glass_edits(&mut self, time: f32) -> Vec<(u32, Option<Mat4>)> {
        if self.glass_test.is_none_or(|at| time < at) {
            return Vec::new();
        }
        self.glass_test = None;
        let count = self.glass.pane.iter().max().map_or(0, |&p| p + 1);
        let raised = Mat4::from_translation(Vec3::Y * 0.4);
        (0..count)
            .map(|k| (k, (k % 2 == 1).then_some(raised)))
            .collect()
    }

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
            let walk = self
                .mission
                .as_mut()
                .and_then(|m| m.walk(walker.feet()))
                .unwrap_or_else(|| city_blocks::wished_walk(args, camera.yaw, input));
            walker.advance(&self.walls, walk, f64::from(step));
            // The mission (#287), once the walker moved: its feet where they stand.
            if let Some(m) = &mut self.mission {
                m.steer(walk, step);
                m.advance(walker.feet(), f64::from(step));
            }
        }
        if let Some(p) = &mut self.pursuers {
            p.advance(f64::from(step));
        }
        // The animals (#269), the player and the pursuers the people they may flee.
        if let Some((run, ..)) = &mut self.fauna {
            let people: Vec<Vec3> = self
                .walker
                .as_ref()
                .map(walk::Walker::feet)
                .into_iter()
                .chain(self.pursuers.iter().flat_map(|p| p.feet().to_vec()))
                .collect();
            run.advance(f64::from(step), &people);
        }
        None
    }

    fn player(&mut self) -> Option<Vec3> {
        None
    }

    fn follow(&mut self, camera: &mut FlyCamera) {
        self.view = (camera.position, camera.forward());
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
        // In first person: the camera at the walker's eyes, the view where the mouse turns it,
        // or the mission's autopilot where it walks.
        if let Some(walker) = &self.walker {
            camera.position = walker.drawn_feet() + Vec3::Y * walk::EYES;
            if let Some((yaw, pitch)) = self.mission.as_ref().and_then(|m| m.view()) {
                (camera.yaw, camera.pitch) = (yaw, pitch);
            }
        }
    }

    fn movers(&mut self, scene: &mut MeshletScene) {
        // `--static-test` (#323): the forest hidden at its time, its records, cells and place in
        // the rays' structure changed in place.
        if let Some((at, trees)) = &self.static_test
            && self.clock >= *at
        {
            for instance in trees.clone() {
                scene.set_static_visibility(instance, forge_render::meshlet::Visibility::NONE);
            }
            tracing::info!(instances = trees.len(), "static test: the forest hidden");
            self.static_test = None;
        }
        // `--promote-test` (#338): its tree's movers and edits before the movers are committed.
        if let Some(test) = &mut self.promote_test {
            let (eye, forward) = self.view;
            test.step(scene, self.clock, (eye - self.origin, forward));
        }
        if self.trams.is_empty()
            && self.crowd.is_none()
            && self.boats.is_empty()
            && self.taxis.is_none()
            && self.pursuers.is_none()
            && self.mission.is_none()
            && self.fauna.is_none()
        {
            // The pool's movers are committed without others.
            if self.promote_test.is_some() {
                scene.commit_movers();
            }
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
        // Then the pursuers (#317), the very last: each in its pose's mesh, the slots of its
        // other poses (and those none fills) under the ground. A figure that changed mesh has
        // no motion of its own that frame.
        if let Some(p) = &mut self.pursuers {
            still.resize(transforms.len(), false);
            let (feet, facing, poses) = (p.feet(), p.facing().to_vec(), p.poses());
            let parked = (Vec3::new(0.0, -1000.0, 0.0) + origin, 0.0);
            for mesh in 0..pursuers::STRIDES.len() {
                for k in 0..pursuers::COUNT {
                    let figure = feet.get(k).zip(facing.get(k)).zip(poses.get(k));
                    match figure.filter(|(_, (pose, _))| *pose == mesh) {
                        Some(((&at, f), &(_, changed))) => {
                            still.push(changed);
                            transforms.push(place((at, (-f.x).atan2(-f.y))));
                        }
                        None => {
                            still.push(true);
                            transforms.push(place(parked));
                        }
                    }
                }
            }
        }
        // Then the mission's parcel (#287): where it lies, or under the ground.
        if let Some(m) = &self.mission {
            still.resize(transforms.len(), false);
            still.push(true);
            let at = m.parcel().unwrap_or(Vec3::new(0.0, -1000.0, 0.0) + origin);
            transforms.push(place((at, 0.0)));
        }
        // Then the animals (#269), in their blocks.
        if let Some((run, layout, drawn)) = &mut self.fauna {
            still.resize(transforms.len(), false);
            let away = Vec3::new(0.0, -1000.0, 0.0) + origin;
            for (at, yaw, changed) in run.poses(layout, drawn, self.clock, away) {
                still.push(changed);
                transforms.push(place((at, yaw)));
            }
        }
        scene.set_movers_still(&transforms, |k| still.get(k).copied().unwrap_or(false));
    }

    fn title(&mut self) -> String {
        // The mission's objective after the place's name (#287).
        match self.mission.as_ref().map(|m| m.line()) {
            Some(line) if !line.is_empty() => format!("{} — {line}", self.title),
            _ => self.title.clone(),
        }
    }

    fn hud_lines(&self) -> Vec<String> {
        // The mission's objective, or how it ended (#287).
        self.mission
            .as_ref()
            .map(mission::MissionRun::line)
            .filter(|l| !l.is_empty())
            .into_iter()
            .collect()
    }

    fn footing(&self) -> Option<(Vec3, forge_audio::Surface)> {
        let feet = self.walker.as_ref()?.drawn_feet();
        Some((feet, self.ground_under(feet)))
    }

    fn habitat(&self, listener: Vec3) -> Option<forge_audio::ambience::Habitat> {
        // The trees within earshot (30 % of the ground under crowns is a wood), and indoors in
        // a room's volume.
        let green = (self
            .canopy
            .cover(glam::Vec2::new(listener.x, listener.z), HABITAT_REACH)
            / 0.3)
            .min(1.0);
        let indoors = forge_render::region_at(&self.regions, listener).region != 0;
        Some(forge_audio::ambience::Habitat {
            green,
            indoors: if indoors { 1.0 } else { 0.0 },
        })
    }

    fn sounds(&mut self, sound: &mut forge_app::sound::Sound, listener: Vec3) {
        let Some(pursuers) = &self.pursuers else {
            return;
        };
        let steps = self.steps.get_or_insert_with(|| sounds::Steps::new(sound));
        let ground = Ground {
            regions: &self.regions,
            footing: self.footing.as_ref(),
            sounds: &self.ground_sounds,
        };
        let walls = &self.walls;
        steps.update(
            sound,
            &pursuers.feet(),
            listener,
            |at| ground.under(at),
            |a, b| walls.between(a, b),
        );
    }
}

/// How far round the listener the ambience counts the trees, metres (#350).
const HABITAT_REACH: f32 = 32.0;

/// The trees' crowns over the plan (#350): their area in square metres per cell of [`Canopy::CELL`].
struct Canopy {
    area: std::collections::HashMap<(i32, i32), f32>,
}

impl Canopy {
    /// Its cells' side, metres.
    const CELL: f32 = 8.0;

    /// From each crown's centre on the ground and its radius.
    fn new(crowns: impl Iterator<Item = (glam::Vec2, f32)>) -> Self {
        let mut area = std::collections::HashMap::new();
        for (at, radius) in crowns {
            let cell = (at / Self::CELL).floor();
            *area.entry((cell.x as i32, cell.y as i32)).or_insert(0.0) +=
                std::f32::consts::PI * radius * radius;
        }
        Self { area }
    }

    /// The share of the ground within `reach` of `at` under crowns, 0 to 1: the crown area of the
    /// cells whose centre lies within it, over its disc.
    fn cover(&self, at: glam::Vec2, reach: f32) -> f32 {
        let lo = ((at - reach) / Self::CELL).floor();
        let hi = ((at + reach) / Self::CELL).floor();
        let mut sum = 0.0;
        for y in lo.y as i32..=hi.y as i32 {
            for x in lo.x as i32..=hi.x as i32 {
                let centre = (glam::Vec2::new(x as f32, y as f32) + 0.5) * Self::CELL;
                if centre.distance(at) <= reach {
                    sum += self.area.get(&(x, y)).copied().unwrap_or(0.0);
                }
            }
        }
        (sum / (std::f32::consts::PI * reach * reach)).min(1.0)
    }
}

/// What a foot lands on (#318): the floors indoors (inside a room's volume) are boards; outdoors
/// the ground's section under it sounds as its row says (D-007: the lawns and the open land grass,
/// the water mirrors water), stone where no ground was laid or the plan has no map (the city).
struct Ground<'a> {
    regions: &'a [forge_render::RegionVolume],
    footing: Option<&'a footing::Footing>,
    sounds: &'a [Option<forge_audio::Surface>],
}

impl Ground<'_> {
    /// What a foot at `feet` lands on.
    fn under(&self, feet: Vec3) -> forge_audio::Surface {
        let stone = forge_audio::Surface::Stone;
        if forge_render::region_at(self.regions, feet + Vec3::Y * 0.1).region != 0 {
            forge_audio::Surface::Wood
        } else {
            self.footing
                .map_or(stone, |f| f.surface_at(feet, self.sounds, stone))
        }
    }
}

impl PlanRunning {
    /// What a foot at `feet` lands on ([`Ground`]).
    fn ground_under(&self, feet: Vec3) -> forge_audio::Surface {
        Ground {
            regions: &self.regions,
            footing: self.footing.as_ref(),
            sounds: &self.ground_sounds,
        }
        .under(feet)
    }
}

/// The ground's sections under a plan of side `size` (#318), painted from the tiles' ground at a
/// metre; `None` for a plan too large (the city).
fn footing_map(size: f64, tiles: &Tiles) -> Option<footing::Footing> {
    let corner = world(size, DVec2::ZERO, 0.0);
    let mut map = footing::Footing::new(glam::Vec2::new(corner.x, corner.z), size as f32)?;
    for tile in &tiles.tiles {
        let mesh = &tile.ground;
        for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            let p = [0, 1, 2].map(|k| tile.centre + Vec3::from(mesh.positions[tri[k] as usize]));
            map.paint(p, mesh.section(t));
        }
    }
    tracing::info!(
        cells_by_section = ?map.counts(),
        "the ground underfoot, a metre a cell (the last: none laid)"
    );
    Some(map)
}

/// The grass map's cell, metres (#300).
const GRASS_CELL: f32 = 0.5;

/// The grass map of a plan of side `size` (#300, `--grass`): the gardens' lawns and the parks'
/// longer grass where the tiles lay them, cleared under the buildings and along the garden
/// walls; `None` for a plan too large for one map (the city).
fn grass_map(
    size: f64,
    tiles: &Tiles,
    masses: &[Mass],
    walls: &[forge_procgen::city::walls::Wall],
) -> Option<forge_render::grass::GrassMap> {
    let side = (size / f64::from(GRASS_CELL)).ceil() as u32;
    if u64::from(side) * u64::from(side) > u64::from(forge_render::grass::MAX_CELLS) {
        return None;
    }
    let corner = world(size, DVec2::ZERO, 0.0);
    let mut map = forge_render::grass::GrassMap::new(
        glam::Vec2::new(corner.x, corner.z),
        GRASS_CELL,
        side,
        side,
    );
    for tile in &tiles.tiles {
        let mesh = &tile.ground;
        for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            // Blades a cell, their height and their row after the garden's: a lawn's 56 (224 a
            // square metre) of 12 cm, a park's 80 of 22 cm in its own colours.
            let (count, height, row) = match mesh.section(t) {
                GARDEN => (56, 12, 0),
                PARK => (80, 22, PARK - GARDEN),
                _ => continue,
            };
            let p = [0, 1, 2].map(|k| tile.centre + Vec3::from(mesh.positions[tri[k] as usize]));
            map.paint(p, count, height, row);
        }
    }
    let flat = |p: DVec2| {
        let w = world(size, p, 0.0);
        glam::Vec2::new(w.x, w.z)
    };
    for m in masses {
        let ring: Vec<glam::Vec2> = m.footprint.iter().map(|&p| flat(p)).collect();
        map.clear(&ring, 0.15);
    }
    let half = (forge_procgen::city::walls::THICKNESS * 0.5) as f32;
    for w in walls {
        let (a, b) = (flat(w.a), flat(w.b));
        map.clear(&[a, b, b, a], half + 0.05);
    }
    Some(map)
}

/// A block's ground piece: its first corner (the tile it goes in), its triangles, its level and
/// its section.
type GroundPiece = (DVec2, Vec<[DVec2; 3]>, f64, u8);

/// Each block's lots walled on their outline: in a block with garden walls (a village's,
/// `CityPlan::walls`, a wall's middle in it), its houses' lots; none elsewhere.
fn walled_lots(plan: &CityPlan, masses: &[Mass]) -> Vec<Vec<Vec<DVec2>>> {
    let mut walled = vec![false; plan.blocks.len()];
    for w in &plan.walls {
        let mid = (w.a + w.b) * 0.5;
        if let Some(b) = plan
            .blocks
            .iter()
            .position(|block| geom::contains(&block.polygon, mid))
        {
            walled[b] = true;
        }
    }
    let mut out = vec![Vec::new(); plan.blocks.len()];
    for m in masses {
        let lot = &plan.lots[m.lot as usize];
        if walled[lot.block as usize] {
            out[lot.block as usize].push(lot.polygon.clone());
        }
    }
    out
}

/// Each block's ground off the streets, cut and triangulated on the machine's threads
/// ([`PlanScenario::detailed`]): a built block's forecourt paved with its `paths` and its depot's
/// yard, outside its `walled` lots, the rest its gardens; a large square's `middles` cut out of
/// its paving.
fn blocks_ground(
    plan: &CityPlan,
    paths: &[Vec<Vec<DVec2>>],
    middles: &[Option<(Vec<DVec2>, bool)>],
    walled: &[Vec<Vec<DVec2>>],
) -> Vec<Vec<GroundPiece>> {
    par_map(plan.blocks.len(), cores(), None, |b| {
        let block = &plan.blocks[b];
        let mut pieces: Vec<GroundPiece> = Vec::new();
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
            let (front, rest) = plan
                .ground
                .forecourt(&block.polygon, FORECOURT, &ways, &walled[b]);
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
    })
}

/// The derived cache's key of [`blocks_ground`] (#345): a digest of everything it reads (the
/// blocks, their paths, the depots' yards, the squares' middles and the street ground), and the
/// digest of its code.
fn blocks_ground_key(
    plan: &CityPlan,
    paths: &[Vec<Vec<DVec2>>],
    middles: &[Option<(Vec<DVec2>, bool)>],
    walled: &[Vec<Vec<DVec2>>],
) -> forge_core::derived::Key {
    let mut hash = xxhash_rust::xxh3::Xxh3::new();
    let mut put = |x: f64| hash.update(&x.to_bits().to_le_bytes());
    let points = |put: &mut dyn FnMut(f64), p: &[DVec2]| {
        put(p.len() as f64);
        for q in p {
            put(q.x);
            put(q.y);
        }
    };
    put(plan.blocks.len() as f64);
    for (b, block) in plan.blocks.iter().enumerate() {
        put(walled[b].len() as f64);
        for lot in &walled[b] {
            points(&mut put, lot);
        }
        put(match block.use_ {
            BlockUse::Built => 0.0,
            BlockUse::Park => 1.0,
            BlockUse::Square => 2.0,
            BlockUse::Quay => 3.0,
        });
        points(&mut put, &block.polygon);
        put(paths[b].len() as f64);
        for path in &paths[b] {
            points(&mut put, path);
        }
        match &middles[b] {
            Some((middle, mirror)) => {
                put(f64::from(u8::from(*mirror)) + 1.0);
                points(&mut put, middle);
            }
            None => put(0.0),
        }
    }
    put(plan.tram_depots.len() as f64);
    for depot in &plan.tram_depots {
        put(f64::from(depot.block));
        points(&mut put, &depot.yard());
    }
    plan.ground.numbers(&mut put);
    forge_core::derived::KeyHasher::new()
        .number(hash.digest())
        .key(CODE_BLOCKS_GROUND)
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

/// Whether the street lamps are on at `hour` (0 to 24): from 17:00 to 7:00, the middle of the
/// residents' lamps' ramps at dusk and dawn ([`residents::lamps`]).
fn street_lamps_on(hour: f64) -> bool {
    !(7.0..17.0).contains(&hour.rem_euclid(24.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_fauna::Mode;
    use forge_procgen::city::{example, plan};

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

    /// #262's measure (the owner, 2026-10-11: the city's people pass through trees and lamp
    /// posts). A walker is an upright disc of 0.3 m, slid every 0.25 m along each street's two
    /// walking lines: `(half − 3)` m either side of its middle, at least a metre, as
    /// `crowd::Route::side` puts them (its corners at junctions left out). Against the street
    /// trees' trunks (0.3 m at the foot for a crown of 4.2 m) and the lamps' posts (0.1 m), each
    /// obstacle counted once. A control post on the first street's line shows the sweep looks.
    /// The counts only go down: each fix lowers its bound.
    #[test]
    fn walkers_lines_meet_few_trunks_and_posts() {
        use std::collections::{HashMap, HashSet};
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let trees = forge_procgen::city::trees::trees(&site, &city, 5);
        let lamps = forge_procgen::city::lamps::lamps(&site, &city);
        const WALKER: f64 = 0.3;
        let line_offset = |half: f64| (half - 3.0).max(1.0);
        // Upright discs: where, radius, what (0 a trunk, 1 a post, 2 the control).
        let mut discs: Vec<(DVec2, f64, u8)> = Vec::new();
        for t in trees.iter().filter(|t| t.species.is_none()) {
            discs.push((t.pos, 0.3 * t.crown / 4.2, 0));
        }
        for l in &lamps {
            discs.push((l.pos, 0.1, 1));
        }
        if let Some(e) = city.roads.edges.first()
            && e.points.len() >= 2
        {
            let (a, b) = (e.points[0], e.points[1]);
            let dir = (b - a).normalize_or_zero();
            let right = DVec2::new(-dir.y, dir.x);
            let at = (a + b) * 0.5 + right * line_offset(e.profile.width() * 0.5);
            discs.push((at, 0.1, 2));
        }
        const CELL: f64 = 10.0;
        let key = |p: DVec2| ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32);
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, d) in discs.iter().enumerate() {
            cells.entry(key(d.0)).or_default().push(i);
        }
        // A street's walking line on one side (`off` signed, + on the right): its points moved
        // out, their corners mitred where the two segments' lines meet, or their mean where a
        // sharp turn's mitre runs far out, as `Route::corner` turns them.
        let walking_line = |pts: &[DVec2], off: f64| -> Vec<DVec2> {
            let n = pts.len();
            let dir = |k: usize| (pts[k + 1] - pts[k]).normalize_or_zero();
            let right = |d: DVec2| DVec2::new(-d.y, d.x);
            (0..n)
                .map(|i| {
                    if i == 0 {
                        return pts[0] + right(dir(0)) * off;
                    }
                    if i == n - 1 {
                        return pts[n - 1] + right(dir(n - 2)) * off;
                    }
                    let (d0, d1) = (dir(i - 1), dir(i));
                    let (p, q) = (pts[i] + right(d0) * off, pts[i] + right(d1) * off);
                    let (denom, mean) = (d0.perp_dot(d1), (p + q) * 0.5);
                    if denom.abs() < 1e-6 {
                        return mean;
                    }
                    let mitre = p + d0 * ((q - p).perp_dot(d1) / denom);
                    if mitre.distance(pts[i]) > 3.0 * off.abs() {
                        mean
                    } else {
                        mitre
                    }
                })
                .collect()
        };
        let mut met: HashSet<usize> = HashSet::new();
        let mut km = 0.0;
        for e in &city.roads.edges {
            let off = line_offset(e.profile.width() * 0.5);
            for side in [1.0, -1.0] {
                for w in walking_line(&e.points, side * off).windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let length = a.distance(b);
                    let dir = (b - a).normalize_or_zero();
                    km += length / 1000.0;
                    let mut s = 0.0;
                    while s <= length {
                        let p = a + dir * s;
                        let (cx, cz) = key(p);
                        for dx in -1..=1 {
                            for dz in -1..=1 {
                                for &k in cells.get(&(cx + dx, cz + dz)).into_iter().flatten() {
                                    let (at, r, _) = discs[k];
                                    if p.distance(at) < WALKER + r {
                                        met.insert(k);
                                    }
                                }
                            }
                        }
                        s += 0.25;
                    }
                }
            }
        }
        let count = |what: u8| met.iter().filter(|&&k| discs[k].2 == what).count();
        let (trunks, posts, control) = (count(0), count(1), count(2));
        let (all_trunks, all_posts) = (
            discs.iter().filter(|d| d.2 == 0).count(),
            discs.iter().filter(|d| d.2 == 1).count(),
        );
        println!(
            "{km:.1} km of walking lines: {trunks} of {all_trunks} trunks and {posts} of \
             {all_posts} lamp posts met; the control {control}"
        );
        assert_eq!(
            control, 1,
            "the control post on a walking line was never met"
        );
        // The bounds of 2026-10-24, when the measure came in.
        assert!(
            trunks <= TRUNKS_MET && posts <= POSTS_MET,
            "{trunks} trunks, {posts} posts"
        );
    }

    /// #262's bounds: the trunks and lamp posts the walkers' lines met when the measure came in
    /// (2026-10-24, 390 km of lines): one trunk of 29,103 and two posts of 9,307, all on shared
    /// streets at a bend (a sharp turn's line drawn in over the middle's trees, a lamp set by the
    /// next segment's normal). Each fix lowers them.
    const TRUNKS_MET: usize = 1;
    const POSTS_MET: usize = 2;

    /// Every tram runs its line both ways clear of the street trees and the lamps (#276: a
    /// vehicle's box swept along its track). Each section is a box set by its two ends on the
    /// track (the middle of a section swings in on a curve, its ends out), 2.55 m wide, from
    /// 0.2 m to 3.7 m over the street, slid along every line's whole route in steps of 0.5 m.
    /// The obstacles are the street tree model's trunk (0.3 m at its foot for a crown of 4.2 m,
    /// to 44 % of its height) and its crown (its radius, from 43 % of its height up), and each
    /// lamp's post. The point tests (the kerbs keep `SWEPT` from a line) stay the first pass.
    #[test]
    fn trams_run_their_lines_clear_of_the_trees_and_lamps() {
        use forge_geom::vehicles::{SECTION_GAP, SECTION_LENGTH, TRAM_WIDTH};
        use forge_procgen::city::sweep::{self, Cuboid};
        use std::collections::HashMap;
        let size = 4000.0;
        let site = example::site(size, 10.0);
        let city = plan(&site, &example::params(size));
        let trees = forge_procgen::city::trees::trees(&site, &city, 5);
        let lamps = forge_procgen::city::lamps::lamps(&site, &city);
        // Upright discs: where, radius, from, to, and what (0 a trunk, 1 a crown, 2 a lamp).
        let mut discs: Vec<(DVec2, f64, f64, f64, u8)> = Vec::new();
        for t in trees.iter().filter(|t| t.species.is_none()) {
            discs.push((t.pos, 0.3 * t.crown / 4.2, 0.0, 0.44 * t.height, 0));
            discs.push((t.pos, t.crown, 0.43 * t.height, t.height, 1));
        }
        for l in &lamps {
            discs.push((l.pos, 0.1, 0.0, l.height, 2));
        }
        // The control: a post on the first line's outbound track halfway, which every tram
        // passing meets, so a sweep that finds nothing has looked.
        let control = city.tram_lines.first().and_then(|l| {
            let run = TramRun::new(l, &city, &site, &[])?;
            let s = run.along[run.along.len() - 1] * 0.5;
            let (p, t) = run.track(s);
            Some(p + DVec2::new(-t.y, t.x) * run.side(s, true))
        });
        if let Some(at) = control {
            discs.push((at, 0.1, 0.0, 3.0, 3));
        }
        const CELL: f64 = 10.0;
        let key = |p: DVec2| ((p.x / CELL).floor() as i32, (p.y / CELL).floor() as i32);
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, d) in discs.iter().enumerate() {
            cells.entry(key(d.0)).or_default().push(i);
        }
        let body = Cuboid {
            half_length: f64::from(SECTION_LENGTH) * 0.5,
            half_width: f64::from(TRAM_WIDTH) * 0.5,
            bottom: 0.2,
            top: 3.7,
        };
        let pitch = f64::from(SECTION_LENGTH + SECTION_GAP);
        let (mut km, mut met) = (0.0, [0_usize; 4]);
        let mut first: Vec<(usize, f64, u8)> = Vec::new();
        let mut taken: Vec<DVec2> = Vec::new();
        for (line, l) in city.tram_lines.iter().enumerate() {
            let Some(run) = TramRun::new(l, &city, &site, &taken) else {
                continue;
            };
            taken.extend(run.timetable.stops().iter().map(|&s| run.track(s).0));
            let length = run.along[run.along.len() - 1];
            km += 2.0 * length / 1000.0;
            for outbound in [true, false] {
                let dir = if outbound { 1.0 } else { -1.0 };
                let on_track = |s: f64| {
                    let (p, t) = run.track(s);
                    p + DVec2::new(-t.y, t.x) * run.side(s, outbound)
                };
                let mut s = tram_length() * 0.5;
                while s <= length - tram_length() * 0.5 {
                    let nose = s + dir * tram_length() * 0.5;
                    for i in 0..TRAM_SECTIONS {
                        let front = nose - dir * pitch * i as f64;
                        let (a, b) = (
                            on_track(front),
                            on_track(front - dir * f64::from(SECTION_LENGTH)),
                        );
                        let (centre, heading) = ((a + b) * 0.5, (a - b).normalize_or_zero());
                        let (cx, cz) = key(centre);
                        for dx in -1..=1 {
                            for dz in -1..=1 {
                                let Some(near) = cells.get(&(cx + dx, cz + dz)) else {
                                    continue;
                                };
                                for &k in near {
                                    let (at, r, y0, y1, what) = discs[k];
                                    if sweep::cuboid_meets_disc(
                                        &body, centre, heading, 0.0, at, r, y0, y1,
                                    ) {
                                        met[what as usize] += 1;
                                        if what < 3 && first.len() < 12 {
                                            first.push((line, (s * 10.0).round() / 10.0, what));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    s += 0.5;
                }
            }
        }
        println!(
            "{km:.1} km of track both ways: {} trunks, {} crowns, {} lamps met; the control {} times",
            met[0], met[1], met[2], met[3]
        );
        assert!(km > 10.0, "{km}");
        assert!(met[3] > 0, "the control post on the track was never met");
        assert!(
            met[..3].iter().all(|&n| n == 0),
            "trams meet {} trunks, {} crowns and {} lamps (line, metres along, 0 trunk 1 crown 2 \
             lamp): {first:?}",
            met[0],
            met[1],
            met[2]
        );
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

    /// Every flight of the village's houses keeps 2 m clear over each tread, across its width:
    /// no solid of the insides (the flight above, a cupboard, a partition) nor an upper floor in
    /// that height. Under the flight above, the cupboard closed under it hung across a lower
    /// flight's head in the houses of three storeys (the owner, 2026-10-17: "is there enough space
    /// for people to pass in this stair case?").
    #[test]
    fn the_village_flights_keep_two_metres_over_their_treads() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let going = cores::GOING;
        let (mut treads, mut low) = (0, Vec::new());
        for (m, s) in v.masses.iter().zip(&structures) {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let Some(stair) = inner.stair else {
                continue;
            };
            let solids = interior_solids(s, &inner, m.height);
            let floors = interior_floors(m, s, &inner);
            let across = stair[1] - stair[0];
            for (k, &r) in inner.risers.iter().enumerate() {
                if k + 1 >= s.storeys.len() || r == 0 {
                    break;
                }
                let base = floor_level(s, k);
                let rise = (s.storeys[k + 1].level - base) / f64::from(r);
                for i in 0..r - 1 {
                    let y = base + f64::from(i + 1) * rise;
                    for u in [0.2, 0.5, 0.8] {
                        let p =
                            stair[0] + inner.climb * ((f64::from(i) + 0.5) * going) + across * u;
                        treads += 1;
                        let (from, to) = (y + 0.05, y + 2.0);
                        let blocked = solids
                            .iter()
                            .map(|(ring, y0, y1, _)| (ring, *y0, *y1))
                            .chain(floors.iter().map(|(ring, y0, y1)| (ring, *y0, *y1)))
                            .any(|(ring, y0, y1)| y0 < to && y1 > from && geom::contains(ring, p));
                        if blocked {
                            low.push((m.lot, k, i));
                        }
                    }
                }
            }
        }
        assert!(treads > 1000, "{treads}");
        assert!(
            low.is_empty(),
            "{} of {treads} tread points under 2 m: {:?}",
            low.len(),
            &low[..low.len().min(8)]
        );
    }

    /// Every flight of the village's houses can be stepped on: the 0.6 m of floor before its
    /// first riser, across its width, is clear 2 m up. The guard across an upper floor's
    /// stairwell walled off the flight that starts there in the houses of three storeys.
    #[test]
    fn the_village_flights_feet_are_clear() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let (mut feet, mut blocked) = (0, Vec::new());
        for (m, s) in v.masses.iter().zip(&structures) {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let Some(stair) = inner.stair else {
                continue;
            };
            let solids = interior_solids(s, &inner, m.height);
            let across = stair[1] - stair[0];
            for (k, &r) in inner.risers.iter().enumerate() {
                if k + 1 >= s.storeys.len() || r == 0 {
                    break;
                }
                feet += 1;
                let y = floor_level(s, k);
                let clear = [0.1, 0.35, 0.6].iter().all(|&back| {
                    [0.2, 0.5, 0.8].iter().all(|&u| {
                        let p = stair[0] - inner.climb * back + across * u;
                        !solids.iter().any(|(ring, y0, y1, _)| {
                            *y0 < y + 2.0 && *y1 > y + 0.05 && geom::contains(ring, p)
                        })
                    })
                });
                if !clear {
                    blocked.push((m.lot, k));
                }
            }
        }
        assert!(feet > 50, "{feet}");
        assert!(
            blocked.is_empty(),
            "{} of {feet} flights walled at their foot: {blocked:?}",
            blocked.len()
        );
    }

    /// A walker's capsule climbs every flight of the village's houses without meeting a solid
    /// (#276: the owner's "test with the shape the navigation places are supposed to be
    /// navigated by"): up the flight's middle from 0.6 m before its first riser to 0.7 m onto the
    /// landing above, on the floor, each tread and the landing in turn, against the insides'
    /// solids (the flight's own steps are what it climbs) and the upper floors round their
    /// stairwells. The point tests above stay the fast first pass.
    #[test]
    fn the_village_flights_pass_a_walkers_capsule() {
        use forge_procgen::city::sweep::{self, Prism};
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let going = cores::GOING;
        let (mut flights, mut met) = (0, Vec::new());
        for (m, s) in v.masses.iter().zip(&structures) {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let Some(stair) = inner.stair else {
                continue;
            };
            let solids = interior_solids(s, &inner, m.height);
            let floors = interior_floors(m, s, &inner);
            let prisms: Vec<Prism<'_>> = solids
                .iter()
                .filter(|(_, _, _, step)| !step)
                .map(|(ring, y0, y1, _)| Prism {
                    ring,
                    y0: *y0,
                    y1: *y1,
                })
                .chain(floors.iter().map(|(ring, y0, y1)| Prism {
                    ring,
                    y0: *y0,
                    y1: *y1,
                }))
                .collect();
            let middle = 0.5 * (stair[0] + stair[1]);
            let climb = inner.climb.normalize_or_zero();
            for (k, &r) in inner.risers.iter().enumerate() {
                if k + 1 >= s.storeys.len() || r == 0 {
                    break;
                }
                flights += 1;
                let base = floor_level(s, k);
                let top = floor_level(s, k + 1);
                let rise = (s.storeys[k + 1].level - base) / f64::from(r);
                let run = f64::from(r - 1) * going;
                let (before, after) = (0.6, 0.7);
                let path = [middle - climb * before, middle + climb * (run + after)];
                // The ground under the walker: the floor, the tread it stands on, the landing.
                let ground = |along: f64, _: DVec2| {
                    let v = along - before;
                    if v < 0.0 {
                        base
                    } else if v >= run {
                        top
                    } else {
                        base + ((v / going).floor() + 1.0) * rise
                    }
                };
                for hit in sweep::sweep(&sweep::WALKER, &path, 0.05, ground, &prisms) {
                    met.push((m.lot, k, (hit.along * 100.0).round() / 100.0, hit.solid));
                }
            }
        }
        println!("{flights} flights climbed by a walker's capsule");
        assert!(flights > 50, "{flights}");
        assert!(
            met.is_empty(),
            "{} capsule samples meet a solid on {flights} flights (lot, storey, metres along, \
             solid): {:?}",
            met.len(),
            &met[..met.len().min(12)]
        );
    }

    /// D-076's walk field (#317, `forge-nav`) over the village's houses as the walker meets them
    /// (their walls open at their front doors, partitions, flights, floors and furniture: the
    /// walker's convex pieces, each a prism) and the ground: from the street a metre and a half
    /// outside each front door to the middle of every room, up the flights. The field, 0.10 m
    /// columns over the house and 4 m round it, is built from the pieces' triangles.
    #[test]
    fn forge_nav_walks_from_the_street_into_every_room() {
        use forge_nav::{Field, FieldDesc, PERSON};
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let walls = wall_pieces(size, &v.masses, &structures);
        // A piece's prism (its corners in pairs, at its foot and its top) as triangles, its top
        // facing up and its underside down.
        let prism = |piece: &[Vec3], out: &mut Vec<[Vec3; 3]>| {
            let (y0, y1) = (piece[0].y, piece[1].y);
            let ring: Vec<Vec3> = piece.iter().step_by(2).copied().collect();
            let n = ring.len();
            let at = |p: Vec3, y: f32| Vec3::new(p.x, y, p.z);
            for k in 1..n.saturating_sub(1) {
                for (y, up) in [(y1, true), (y0, false)] {
                    let mut t = [at(ring[0], y), at(ring[k], y), at(ring[k + 1], y)];
                    let ny = (t[1] - t[0]).cross(t[2] - t[0]).y;
                    if (ny > 0.0) != up {
                        t.swap(1, 2);
                    }
                    out.push(t);
                }
            }
            for k in 0..n {
                let (a, b) = (ring[k], ring[(k + 1) % n]);
                out.push([at(a, y0), at(b, y0), at(b, y1)]);
                out.push([at(a, y0), at(b, y1), at(a, y1)]);
            }
        };
        let (mut houses, mut rooms, mut reached, mut upstairs) = (0, 0, 0, 0);
        let (mut build, mut search) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let mut missed = Vec::new();
        for (i, (m, s)) in v.masses.iter().zip(&structures).enumerate() {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let Some(door) = s
                .openings
                .iter()
                .find(|o| o.kind == OpeningKind::Door && o.storey == 0)
            else {
                continue;
            };
            houses += 1;
            if houses > 24 {
                break;
            }
            let (lo, hi) = m.footprint.iter().fold(
                (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                |(lo, hi), &p| (lo.min(p), hi.max(p)),
            );
            let (lo, hi) = (world(size, lo - 4.0, 0.0), world(size, hi + 4.0, 0.0));
            let cell = 0.1_f32;
            let cells = (((hi.x - lo.x).max(hi.z - lo.z) / cell).ceil() as u32).min(1024);
            let desc = FieldDesc {
                corner: glam::Vec2::new(lo.x, lo.z),
                cells,
                cell,
            };
            let inside = |p: &[Vec3]| {
                p.iter()
                    .any(|q| q.x >= lo.x && q.x <= hi.x && q.z >= lo.z && q.z <= hi.z)
            };
            let mut triangles = Vec::new();
            let ground = [
                Vec3::new(lo.x, WALK_LEVEL as f32, lo.z),
                Vec3::new(hi.x, WALK_LEVEL as f32, lo.z),
                Vec3::new(hi.x, WALK_LEVEL as f32, hi.z),
                Vec3::new(lo.x, WALK_LEVEL as f32, hi.z),
            ];
            triangles.push([ground[0], ground[2], ground[1]]);
            triangles.push([ground[0], ground[3], ground[2]]);
            // The ground floor's slab, up to its level.
            if let Some(ring) = inner_ring(m, s) {
                let y = floor_level(s, 0);
                let piece: Vec<Vec3> = ring
                    .iter()
                    .flat_map(|&p| [world(size, p, WALK_LEVEL), world(size, p, y)])
                    .collect();
                prism(&piece, &mut triangles);
            }
            for piece in walls
                .iter()
                .chain(&interior_pieces(size, &v.masses, &structures, &[i]))
                .filter(|p| inside(p))
            {
                prism(piece, &mut triangles);
            }
            let started = std::time::Instant::now();
            let field = Field::build(desc, PERSON, &triangles);
            build += started.elapsed();
            // Outside the front door, a metre and a half out from its wall's middle.
            let w = &s.walls[door.wall];
            let t = (w.b - w.a).normalize_or_zero();
            let mut out = DVec2::new(t.y, -t.x);
            let centroid = geom::centroid(&m.footprint);
            let on = w.a + t * door.along;
            if out.dot(on - centroid) < 0.0 {
                out = -out;
            }
            let from = world(size, on + out * 1.5, WALK_LEVEL);
            // What a walker reaches from the street on the field alone: a room counts when some
            // of its floor is reached (as the walk grid counts, `walk::house`). A path to the
            // reached place nearest its middle times the search.
            let Some(door_place) = field.place_near(from, 1.0) else {
                missed.push((m.lot, "front door".to_owned(), 0));
                continue;
            };
            let seen = field.reach(door_place);
            for room in &inner.rooms {
                rooms += 1;
                let level = floor_level(s, room.storey) as f32;
                let middle = room.outline.iter().fold(DVec2::ZERO, |a, &p| a + p) * 0.25;
                let middle = world(size, middle, f64::from(level));
                let ring: Vec<DVec2> = room
                    .outline
                    .iter()
                    .map(|&p| {
                        let w = world(size, p, 0.0);
                        DVec2::new(f64::from(w.x), f64::from(w.z))
                    })
                    .collect();
                let d = field.desc();
                let nearest = (0..d.cells)
                    .flat_map(|j| (0..d.cells).map(move |i| (i, j)))
                    .filter(|&(i, j)| {
                        let c = d.centre(i, j);
                        geom::contains(&ring, DVec2::new(f64::from(c.x), f64::from(c.y)))
                    })
                    .flat_map(|(i, j)| field.column(i, j))
                    .filter(|&q| {
                        seen[q as usize] && (field.spans()[q as usize].floor - level).abs() < 0.2
                    })
                    .min_by(|&a, &b| {
                        let (pa, pb) = (field.point(a), field.point(b));
                        pa.distance(middle)
                            .total_cmp(&pb.distance(middle))
                            .then(a.cmp(&b))
                    });
                let Some(q) = nearest else {
                    missed.push((m.lot, format!("{:?}", room.kind), room.storey));
                    continue;
                };
                reached += 1;
                upstairs += usize::from(room.storey > 0);
                let started = std::time::Instant::now();
                let path = field.path(from, field.point(q), &[], 400_000);
                search += started.elapsed();
                assert!(
                    path.is_some_and(|p| p.complete),
                    "lot {}: no path to a place its flood reached",
                    m.lot
                );
            }
        }
        println!(
            "{} houses, {reached} of {rooms} rooms reached ({upstairs} upstairs); fields \
             built in {:.1} ms each, paths in {:.2} ms each; not reached (lot, kind, storey): \
             {missed:?}",
            houses.min(24),
            build.as_secs_f64() * 1000.0 / houses.min(24) as f64,
            search.as_secs_f64() * 1000.0 / reached.max(1) as f64
        );
        assert!(houses > 10, "{houses} houses");
        // Every room since 2026-10-23 (#285): the flights' first steps are open on the hall's side
        // (`RAIL_FROM`). Their foot stands in a pocket by the back wall, which the rail from the
        // first step boxed in: a walker 0.6 m across turned into it with centimetres to spare,
        // and the field's clearance, a lower bound, counted it shut (147 of 152 rooms before).
        assert!(
            missed.is_empty(),
            "{} rooms of {rooms} not reached (lot, kind, storey): {:?}",
            missed.len(),
            &missed[..missed.len().min(12)]
        );
    }

    /// The village's passages (#316, D-076): with a house's front door shut, a walker no longer
    /// gets in from the street, and a climber does through a ground-floor window's link; walking
    /// straight through a window from the street meets that window (the player's automatic
    /// crossing); a garden wall is a fence by its height.
    #[test]
    fn a_shut_door_is_passed_by_a_window_for_climbers() {
        use forge_nav::{PERSON, PassageKind, Passages};
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let passages = Passages::new(village_passages(size, &v.plan, &v.masses, &structures));
        let count = |k: PassageKind| passages.all().iter().filter(|p| p.kind == k).count();
        let (windows, doors) = (count(PassageKind::Window), count(PassageKind::Door));
        let fences = count(PassageKind::LowFence) + count(PassageKind::TallFence);
        assert!(
            windows > 100 && doors > 50 && fences > 50,
            "{windows} {doors} {fences}"
        );
        let (mut shut, mut climbed) = (0, 0);
        for (i, (m, s)) in v.masses.iter().zip(&structures).enumerate() {
            if interior::house(m, s).is_none() {
                continue;
            }
            let Some(door) = passages
                .all()
                .iter()
                .find(|p| p.kind == PassageKind::Door && p.piece == i as u32)
            else {
                continue;
            };
            if !passages
                .all()
                .iter()
                .any(|p| p.kind == PassageKind::Window && p.piece == i as u32)
            {
                continue;
            }
            if shut == 8 {
                break;
            }
            shut += 1;
            let field = house_field(size, &v.masses, &structures, &passages, i, true);
            let from = door.middle() - door.across * 1.5;
            let hall = door.middle() + door.across * 1.5 + Vec3::Y * door.drop;
            // Shut: no way in on foot.
            let walking = field
                .path(from, hall, &[], 400_000)
                .expect("a place outside");
            assert!(!walking.complete, "lot {}: in through a shut door", m.lot);
            // A climber's links: the ground-floor windows, two seconds each.
            let links = passages.links(0.5, |p| {
                (p.kind == PassageKind::Window).then_some((2.0, PassageKind::Window as u16))
            });
            let climbing = field
                .path(from, hall, &links, 400_000)
                .expect("a place outside");
            if climbing.complete {
                climbed += 1;
                let (_, k) = climbing.links[0];
                assert_eq!(links[k].kind, PassageKind::Window as u16);
            }
            // Walking straight at a window's middle from the street meets that window.
            let w = passages
                .all()
                .iter()
                .position(|p| p.kind == PassageKind::Window && p.piece == i as u32)
                .expect("a window");
            let p = passages.get(w);
            let met = passages.met_by(
                p.middle() - p.across * 0.6,
                p.middle() + p.across * 0.6,
                PERSON.step,
            );
            assert_eq!(met.map(|m| m.0), Some(w), "lot {}", m.lot);
        }
        println!("{shut} houses shut, {climbed} entered through a window");
        assert!(shut == 8 && climbed * 4 >= shut * 3, "{climbed} of {shut}");
    }

    /// The village's animals (#269's fauna on the first map): the village's plan given as the
    /// fauna's parts, twenty minutes of an evening from 18:00 (the swifts' parties, the dogs'
    /// walks): no flyer ends a step in a wall, a roof or the ground, none on foot stands in a
    /// house, all stay on the plan, and the same seed gives the same animals.
    #[test]
    fn the_village_animals_keep_out_of_its_walls() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let trees: Vec<(DVec2, f64, f64)> = forge_procgen::city::trees::trees(&site, &v.plan, 5)
            .iter()
            .map(|t| (t.pos, t.height, t.crown))
            .collect();
        let run = || {
            let parts = fauna::parts(&site, &v.plan, &v.masses, &structures, &trees);
            let mut r = fauna::FaunaRun::new(parts, 18.0, 7);
            for _ in 0..20 * 60 * 10 {
                r.advance(0.1, &[]);
            }
            let f = r.fauna();
            let indoors = (0..f.len())
                .filter(|&i| matches!(f.mode[i], Mode::Ground | Mode::Walk | Mode::Rest))
                .filter(|&i| f.pos[i].z < 0.5 && f.plan.building_at(f.pos[i].truncate()).is_some())
                .count();
            let off = f.pos.iter().filter(|p| {
                !(0.0..=size as f32).contains(&p.x) || !(0.0..=size as f32).contains(&p.y)
            });
            println!(
                "{} animals in {} groups, {} sites; contacts {}, on foot in a house {indoors}, \
                 off the plan {}; digest {:016x}",
                f.len(),
                f.groups.len(),
                f.plan.sites.len(),
                f.contacts,
                off.clone().count(),
                f.digest()
            );
            assert!(f.len() > 50, "{} animals", f.len());
            assert_eq!(f.contacts, 0, "{:?}", f.contacts_by);
            assert_eq!(indoors, 0);
            assert_eq!(off.count(), 0);
            f.digest()
        };
        assert_eq!(run(), run());
    }

    /// The village's test mission (#287's proof on the first map) is won on its autopilot: the
    /// walker against the house's walls and insides up to the bedroom, to the parcel in the
    /// kitchen and back out to the street, the same ticks and state twice.
    #[test]
    fn the_village_mission_is_won_on_its_autopilot() {
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let run = || {
            let m = mission::village(size, &v.plan, &v.masses).expect("a house for the mission");
            let (i, lot) = (m.house, m.lot);
            let pieces = wall_pieces(size, &v.masses, &structures)
                .into_iter()
                .chain(interior_pieces(size, &v.masses, &structures, &[i]))
                .collect();
            let passages = village_passages(size, &v.plan, &v.masses, &structures)
                .into_iter()
                .filter(|p| p.piece as usize == i)
                .collect();
            let walls = walk::Walls::new(pieces, passages);
            let mut walker = walk::Walker::new(&walls, m.start).expect("a walker on the street");
            let mut run = mission::MissionRun::new(m, true).expect("a valid mission");
            let mut lines = vec![run.line()];
            for _ in 0..60 * 180 {
                let wish = run.walk(walker.feet()).expect("the autopilot walks");
                walker.advance(&walls, wish, 1.0 / 60.0);
                run.advance(walker.feet(), 1.0 / 60.0);
                if lines.last() != Some(&run.line()) {
                    lines.push(run.line());
                }
                if run.outcome() != forge_mission::Outcome::Running {
                    break;
                }
            }
            println!(
                "lot {lot}: {:?} at tick {} ({:.1} s): {lines:?}",
                run.outcome(),
                run.tick(),
                run.tick() as f64 / 60.0
            );
            assert_eq!(
                run.outcome(),
                forge_mission::Outcome::Succeeded,
                "lot {lot}: {lines:?}, the walker at {:?}",
                walker.feet()
            );
            (run.tick(), run.digest())
        };
        assert_eq!(run(), run());
    }

    /// D-076's proof (#317): four pursuers on the street before a house reach its insides, by
    /// its open door, or with its doors shut through its ground-floor windows, carried over the
    /// sills (`CrossingMotion`); on the house's field, following their paths, keeping clear of
    /// one another.
    #[test]
    fn pursuers_get_into_a_house_by_its_door_or_through_its_windows() {
        use forge_nav::Passages;
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let passages = Passages::new(village_passages(size, &v.plan, &v.masses, &structures));
        let (i, door, windows) = pursued_house(&v.masses, &structures, &passages).expect("a house");
        let hall = door.middle() + door.across * 1.5 + Vec3::Y * door.drop;
        // Inside: the places a walker reaches from the hall with the doors shut.
        let shut_field = house_field(size, &v.masses, &structures, &passages, i, true);
        let inside = shut_field.reach(shut_field.place(hall).expect("the hall"));
        let is_in = |p: Vec3| shut_field.place(p).is_some_and(|s| inside[s as usize]);
        for shut in [false, true] {
            let field = house_field(size, &v.masses, &structures, &passages, i, shut);
            let mut pursuers =
                pursuers::Pursuers::new(field, &door, windows.clone(), 1.0).expect("ways in");
            let mut least = f32::INFINITY;
            for _ in 0..60 * 60 {
                pursuers.advance(1.0 / 60.0);
                let (feet, climbing) = (pursuers.feet(), pursuers.climbing());
                for a in 0..feet.len() {
                    for b in a + 1..feet.len() {
                        if !climbing[a] && !climbing[b] {
                            let (p, q) = (feet[a], feet[b]);
                            let gap = glam::Vec2::new(p.x - q.x, p.z - q.z).length() - 0.6;
                            least = least.min(gap);
                        }
                    }
                }
            }
            let got_in = pursuers.feet().iter().filter(|&&p| is_in(p)).count();
            println!(
                "lot {}, doors {}: {got_in} of 4 in, {} through windows, least gap {least:.3} m",
                v.masses[i].lot,
                if shut { "shut" } else { "open" },
                pursuers.crossings
            );
            assert_eq!(got_in, 4, "doors shut: {shut}");
            assert!(least > -0.05, "{least}");
            if shut {
                assert!(pursuers.crossings >= 4, "{}", pursuers.crossings);
            }
        }
    }

    /// A walker's capsule passes every doorway of the village's houses' insides (#276): through
    /// the doorway's middle, 0.4 m either side of its partition, against the partitions (their
    /// heads over the doorways included), the furniture as drawn ([`walk::furnished`]) and the
    /// upper floors.
    #[test]
    fn the_village_doorways_pass_a_walkers_capsule() {
        use forge_procgen::city::sweep::{self, Prism};
        use forge_procgen::city::walk;
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let (mut doorways, mut met) = (0, Vec::new());
        for (m, s) in v.masses.iter().zip(&structures) {
            let Some(inner) = interior::house(m, s) else {
                continue;
            };
            let solids = interior_solids(s, &inner, m.height);
            let floors = interior_floors(m, s, &inner);
            let furniture: Vec<(Vec<DVec2>, f64, f64)> = walk::furnished(m, s, &inner)
                .iter()
                .map(|p| {
                    let y = floor_level(s, p.storey);
                    (p.outline().to_vec(), y, y + p.piece.height())
                })
                .collect();
            let prisms: Vec<Prism<'_>> = solids
                .iter()
                .filter(|(_, _, _, step)| !step)
                .map(|(ring, y0, y1, _)| (ring, *y0, *y1))
                .chain(floors.iter().map(|(ring, y0, y1)| (ring, *y0, *y1)))
                .chain(furniture.iter().map(|(ring, y0, y1)| (ring, *y0, *y1)))
                .map(|(ring, y0, y1)| Prism { ring, y0, y1 })
                .collect();
            for p in &inner.partitions {
                let t = (p.b - p.a).normalize_or_zero();
                let n = DVec2::new(-t.y, t.x);
                let ground = floor_level(s, p.storey);
                for &(d0, d1) in &p.doors {
                    doorways += 1;
                    let middle = p.a + t * ((d0 + d1) * 0.5);
                    let path = [middle - n * 0.4, middle + n * 0.4];
                    let hits = sweep::sweep(&sweep::WALKER, &path, 0.05, |_, _| ground, &prisms);
                    if let Some(hit) = hits.first() {
                        met.push((m.lot, p.storey, d1 - d0, hit.solid));
                    }
                }
            }
        }
        println!("{doorways} doorways passed by a walker's capsule");
        assert!(doorways > 200, "{doorways}");
        assert!(
            met.is_empty(),
            "{} of {doorways} doorways meet a walker's capsule (lot, storey, width, solid): {:?}",
            met.len(),
            &met[..met.len().min(12)]
        );
    }

    /// The village houses' probe regions (D-068), as the lookup finds them: each room's middle
    /// its own room, each window's reveal a room's (not the outside's), each doorway two
    /// different regions with its ramp half way across at its middle.
    #[test]
    fn the_village_houses_rooms_reveals_and_doorways_find_their_regions() {
        use forge_render::{RegionKind, region_at};
        let size = 600.0;
        let site = example::site(size, 10.0);
        let v = forge_procgen::city::village::village(
            &site,
            &forge_procgen::city::village::params(size),
            1,
        );
        let structures = structure::structures(&v.plan, &v.masses);
        let near: Vec<usize> = (0..v.masses.len()).collect();
        let (volumes, _) = probe_regions(size, &v.masses, &structures, &near);
        let middle = |r: &forge_render::RegionVolume, y: f32| {
            let c = r.corners.iter().fold(glam::Vec2::ZERO, |s, &c| s + c) * 0.25;
            glam::Vec3::new(c.x, y, c.y)
        };
        let (mut rooms, mut reveals, mut doorways) = (0, 0, 0);
        for r in &volumes {
            match r.kind {
                RegionKind::Room => {
                    let p = middle(r, r.floor + 1.2);
                    let hit = region_at(&volumes, p);
                    assert_eq!((hit.kind, hit.region), (RegionKind::Room, r.region), "{p}");
                    rooms += 1;
                }
                RegionKind::Recess => {
                    let p = middle(r, (r.floor + r.ceiling) * 0.5);
                    let hit = region_at(&volumes, p);
                    assert_eq!(hit.kind, RegionKind::Recess, "{p}");
                    assert_ne!(hit.region, 0, "a window into no room at {p}");
                    reveals += 1;
                }
                RegionKind::Doorway => {
                    assert_ne!(r.region, r.other, "{r:?}");
                    let p = middle(r, r.floor + 1.0);
                    let hit = region_at(&volumes, p);
                    assert_eq!(hit.kind, RegionKind::Doorway, "{p}");
                    assert!((hit.share(r.region) - 0.5).abs() < 0.05, "{p}: {hit:?}");
                    assert!((hit.share(r.other) - 0.5).abs() < 0.05, "{p}: {hit:?}");
                    doorways += 1;
                }
                RegionKind::Solid => {}
            }
        }
        assert!(
            rooms > 200 && reveals > 200 && doorways > 100,
            "{rooms} {reveals} {doorways}"
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

    /// A floor piece whose neighbour's corner stands in the middle of its edge (a T) is split
    /// there, so the two share that edge's points and no crack opens between them (the owner's
    /// "small things on the floor", 2026-10-24).
    #[test]
    fn floor_pieces_share_their_points_where_one_meets_anothers_edge() {
        let a = [
            DVec2::new(0.0, 0.0),
            DVec2::new(2.0, 0.0),
            DVec2::new(2.0, 1.0),
            DVec2::new(0.0, 1.0),
        ];
        let b = [
            DVec2::new(0.0, 1.0),
            DVec2::new(1.0, 1.0),
            DVec2::new(1.0, 2.0),
            DVec2::new(0.0, 2.0),
        ];
        let corners: Vec<DVec2> = a.iter().chain(&b).copied().collect();
        let tris = conforming_triangles(&a, &corners);
        let area: f64 = tris.iter().map(|t| geom::signed_area(t).abs()).sum();
        assert!((area - 2.0).abs() < 1e-9, "{area}");
        let t_point = DVec2::new(1.0, 1.0);
        assert!(tris.iter().any(|t| t.contains(&t_point)));
        for t in &tris {
            for i in 0..3 {
                let (p, q) = (t[i], t[(i + 1) % 3]);
                let inside = geom::distance_to_segment(t_point, p, q) < 1e-9
                    && p.distance(t_point) > 1e-9
                    && q.distance(t_point) > 1e-9;
                assert!(!inside, "the T point inside the edge {p} {q}");
            }
        }
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
                let (front, rest) =
                    city.ground
                        .forecourt(&block.polygon, FORECOURT, &paths[b], &[]);
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
