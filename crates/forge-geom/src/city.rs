//! Procedural props for the city-blocks demo (issue #34): buildings whose facades carry
//! recessed windows, floor ledges and cornices in the geometry itself, boulders and rubble,
//! and lathe-turned columns, fountains and lamp posts. Each is dense on purpose (0.5 to 3 M
//! triangles): the demo exists to push that much geometry through the cluster DAG.
//!
//! Units are metres, +Y up, the prop standing on the ground plane with its footprint centred
//! on the origin. Everything is a function of its parameters, so a prop's cache key is its
//! parameters (see `crate::cache`).

use std::collections::HashMap;

use forge_core::Seed;
use forge_core::hash::unit_f32;
use glam::Vec3;

use crate::meshlet::CookOptions;
use crate::procedural::{TriMesh, asteroid, fbm};

/// A prop of the city set: its name (unique within the set) and how to generate it.
#[derive(Clone, Debug, PartialEq)]
pub struct PropSpec {
    /// Name, for logs and the cache file.
    pub name: String,
    /// What to generate.
    pub kind: PropKind,
}

/// The generators and their parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum PropKind {
    /// A building (see [`building`]).
    Building(Building),
    /// A single boulder: seed, radius (m), grid segments per cube face.
    Boulder {
        /// Noise seed.
        seed: u64,
        /// Mean radius, metres.
        radius: f32,
        /// Quads per cube-face side (triangles = 12 × segments²).
        segments: u32,
    },
    /// A pile of `pieces` boulders (see [`rubble`]).
    Rubble {
        /// Noise seed.
        seed: u64,
        /// Boulders in the pile.
        pieces: u32,
        /// Quads per cube-face side of each boulder.
        segments: u32,
    },
    /// A surface of revolution (see [`lathe`]).
    Lathe(Lathe),
    /// The ground (see [`terrain_mesh`]).
    Terrain(Terrain),
}

impl PropSpec {
    /// Generates the mesh.
    pub fn generate(&self) -> TriMesh {
        match &self.kind {
            PropKind::Building(b) => building(b),
            PropKind::Boulder {
                seed,
                radius,
                segments,
            } => boulder(*seed, *radius, *segments),
            PropKind::Rubble {
                seed,
                pieces,
                segments,
            } => rubble(*seed, *pieces, *segments),
            PropKind::Lathe(l) => lathe(l),
            PropKind::Terrain(t) => terrain_mesh(t),
        }
    }

    /// How to cook it: hard-surface props weigh their normals in the simplification error
    /// (their windows and flutes are shallow in depth but not in shading), rocks do not.
    pub fn cook_options(&self) -> CookOptions {
        CookOptions {
            normal_weight: match self.kind {
                PropKind::Building(_) => 1.0,
                PropKind::Lathe(_) => 0.5,
                PropKind::Boulder { .. } | PropKind::Rubble { .. } | PropKind::Terrain(_) => 0.0,
            },
        }
    }

    /// A stable text of the parameters and the cook options: the cache key's input.
    pub fn key_text(&self) -> String {
        format!("{self:?} {:?}", self.cook_options())
    }
}

/// A building: a box whose side faces are dense grids displaced into a facade.
#[derive(Clone, Debug, PartialEq)]
pub struct Building {
    /// Footprint along x and z, metres.
    pub width: f32,
    /// Footprint along z, metres.
    pub depth: f32,
    /// Height, metres.
    pub height: f32,
    /// Grid cell of the faces, metres: sets the triangle count (2 × the box's surface /
    /// cell², see [`building_triangles`]).
    pub cell: f32,
    /// Height of the ground floor (shop fronts), metres.
    pub ground_floor: f32,
    /// Height of the other floors, metres.
    pub floor_height: f32,
    /// Target width of a window bay, metres.
    pub bay: f32,
    /// Share of a bay the window takes.
    pub window_ratio: f32,
    /// Width of the flush corner pilasters, metres.
    pub corner: f32,
    /// Rooftop units (boxes), placed from `seed`.
    pub roof_units: u32,
    /// Seed of the rooftop layout.
    pub seed: u64,
}

/// A surface of revolution around +Y: a profile polyline `(radius, height)` from the
/// bottom centre to the top centre, resampled evenly along its length, with optional
/// flutes cut around it.
#[derive(Clone, Debug, PartialEq)]
pub struct Lathe {
    /// The profile, `(radius, height)` in metres; the first and last points should sit on
    /// the axis (radius 0) to close the surface.
    pub profile: Vec<(f32, f32)>,
    /// Vertices around the axis.
    pub around: u32,
    /// Vertices along the profile.
    pub along: u32,
    /// Flutes around the axis (0: none).
    pub flutes: u32,
    /// Flute depth, as a share of the radius.
    pub flute_depth: f32,
    /// Heights between which the flutes are cut.
    pub flute_span: (f32, f32),
}

/// The ground of the city (issue #35): a square heightfield centred on the origin, flat
/// where the city stands and rising into hills around it.
#[derive(Clone, Debug, PartialEq)]
pub struct Terrain {
    /// Side of the square, metres.
    pub size: f32,
    /// Distance between samples, metres.
    pub spacing: f32,
    /// Half the side of the flat city square, metres.
    pub city_half: f32,
    /// Distance over which the hills rise from the city's edge, metres.
    pub rise: f32,
    /// Height of the highest hills, metres.
    pub hill_height: f32,
    /// Noise seed.
    pub seed: u64,
}

impl Terrain {
    /// The city-blocks ground: 4 km across, a sample every 2 m (8 M triangles), a 2.4 km
    /// city square rising over 300 m into hills of up to 90 m.
    pub fn city() -> Self {
        Self {
            size: 4000.0,
            spacing: 2.0,
            city_half: 1200.0,
            rise: 300.0,
            hill_height: 90.0,
            seed: 35,
        }
    }

    /// Samples per side.
    pub fn samples(&self) -> u32 {
        (self.size / self.spacing).round() as u32 + 1
    }

    /// Height of the ground at (x, z), metres.
    pub fn height(&self, x: f32, z: f32) -> f32 {
        let hills = {
            let n = fbm(self.seed, Vec3::new(x, 0.0, z) / 700.0, 5);
            let t = (0.5 + 0.5 * n).clamp(0.0, 1.0);
            self.hill_height * t * t
        };
        let flat = 0.4 * fbm(self.seed ^ 0x5eed, Vec3::new(x, 0.0, z) / 90.0, 3);
        // 0 inside the city square, 1 from `rise` beyond it, smooth between.
        let d = ((x.abs().max(z.abs()) - self.city_half) / self.rise).clamp(0.0, 1.0);
        let w = d * d * (3.0 - 2.0 * d);
        flat + (hills - flat) * w
    }

    /// The heightfield: [`Terrain::height`] at sample `j × samples + i`, x = −size/2 +
    /// i × spacing, z = −size/2 + j × spacing. The terrain mesh's vertices are these samples,
    /// so what stands on the grid's heights stands on the drawn ground.
    pub fn heights(&self) -> Vec<f32> {
        let n = self.samples() as usize;
        let mut heights = vec![0.0; n * n];
        self.heights_into(0, &mut heights);
        heights
    }

    /// Rows `first_row..` of [`Terrain::heights`] into `out` (whole rows), so that callers
    /// can split the grid between threads.
    pub fn heights_into(&self, first_row: u32, out: &mut [f32]) {
        let n = self.samples() as usize;
        let half = self.size * 0.5;
        for (r, row) in out.chunks_exact_mut(n).enumerate() {
            let z = -half + (first_row as usize + r) as f32 * self.spacing;
            for (i, h) in row.iter_mut().enumerate() {
                *h = self.height(-half + i as f32 * self.spacing, z);
            }
        }
    }
}

/// The terrain as a grid mesh: vertex `j × samples + i` at the sample of
/// [`Terrain::heights`], two counter-clockwise triangles per cell seen from above.
pub fn terrain_mesh(t: &Terrain) -> TriMesh {
    let n = t.samples();
    let half = t.size * 0.5;
    let mut mesh = TriMesh::default();
    mesh.positions.reserve((n * n) as usize);
    let heights = t.heights();
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (-half + i as f32 * t.spacing, -half + j as f32 * t.spacing);
            mesh.positions.push([x, heights[(j * n + i) as usize], z]);
        }
    }
    mesh.indices.reserve(((n - 1) * (n - 1) * 6) as usize);
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let a = j * n + i;
            let (b, c, d) = (a + 1, a + n, a + n + 1);
            mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    mesh.recompute_normals();
    mesh
}

/// Six faces of a box as (normal, up, right), with `up × right = normal` so that the grid's
/// triangles wind counter-clockwise seen from outside (the convention of `asteroid`).
const BOX_FACES: [(Vec3, Vec3, Vec3); 6] = [
    (Vec3::X, Vec3::Y, Vec3::Z),
    (Vec3::NEG_X, Vec3::Y, Vec3::NEG_Z),
    (Vec3::Y, Vec3::Z, Vec3::X),
    (Vec3::NEG_Y, Vec3::Z, Vec3::NEG_X),
    (Vec3::Z, Vec3::Y, Vec3::NEG_X),
    (Vec3::NEG_Z, Vec3::Y, Vec3::X),
];

/// A closed box `size` (x, y, z) standing on y = 0 around `origin`, each face a grid of cells
/// of about `cell` metres, every vertex moved along its face's normal by
/// `displace(face, s, t, face_width, face_height)` (s, t: metres along the face's right and up
/// axes from its corner), which also gives the vertex's section. Edge vertices are shared
/// between faces and never move, so the box stays closed. A triangle is in a vertex section
/// only when all three of its vertices are; `mesh.sections` gets one entry per triangle.
fn displaced_box(
    mesh: &mut TriMesh,
    origin: Vec3,
    size: Vec3,
    cell: f32,
    displace: &dyn Fn(usize, f32, f32, f32, f32) -> (f32, u8),
) {
    // Keep `sections` one per triangle even when the mesh so far had none.
    mesh.sections.resize(mesh.indices.len() / 3, 0);
    let mut vertex_section: HashMap<u32, u8> = HashMap::new();
    let segments = (size / cell).ceil().max(Vec3::ONE);
    let half = size * 0.5;
    let center = origin + Vec3::new(0.0, half.y, 0.0);
    let mut welded: HashMap<[i64; 3], u32> = HashMap::new();
    let quantize = |p: Vec3| {
        [
            (f64::from(p.x) * 1e4).round() as i64,
            (f64::from(p.y) * 1e4).round() as i64,
            (f64::from(p.z) * 1e4).round() as i64,
        ]
    };
    let along = |axis: Vec3| axis.abs().dot(size);
    let count = |axis: Vec3| axis.abs().dot(segments) as u32;
    for (face, (normal, up, right)) in BOX_FACES.into_iter().enumerate() {
        let (face_w, face_h) = (along(right), along(up));
        let (nu, nv) = (count(right), count(up));
        let base = center + normal * along(normal) * 0.5;
        let mut grid = Vec::with_capacity(((nu + 1) * (nv + 1)) as usize);
        for j in 0..=nv {
            for i in 0..=nu {
                let s = face_w * i as f32 / nu as f32;
                let t = face_h * j as f32 / nv as f32;
                let flat = base + right * (s - face_w * 0.5) + up * (t - face_h * 0.5);
                let key = quantize(flat);
                let index = *welded.entry(key).or_insert_with(|| {
                    let border = i == 0 || j == 0 || i == nu || j == nv;
                    let (d, section) = if border {
                        (0.0, 0)
                    } else {
                        displace(face, s, t, face_w, face_h)
                    };
                    mesh.positions.push((flat + normal * d).to_array());
                    let index = (mesh.positions.len() - 1) as u32;
                    if section != 0 {
                        vertex_section.insert(index, section);
                    }
                    index
                });
                grid.push(index);
            }
        }
        let stride = nu + 1;
        for j in 0..nv {
            for i in 0..nu {
                let a = grid[(j * stride + i) as usize];
                let b = grid[(j * stride + i + 1) as usize];
                let c = grid[((j + 1) * stride + i) as usize];
                let d = grid[((j + 1) * stride + i + 1) as usize];
                mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
                let section = |tri: [u32; 3]| {
                    let s = tri.map(|v| vertex_section.get(&v).copied().unwrap_or(0));
                    if s[0] == s[1] && s[1] == s[2] {
                        s[0]
                    } else {
                        0
                    }
                };
                mesh.sections.push(section([a, c, b]));
                mesh.sections.push(section([b, c, d]));
            }
        }
    }
}

/// The section of a building's window panes (issue #41): its instances draw it with the material
/// row after theirs.
pub const GLASS: u8 = 1;

/// 0 outside `[lo, hi]`, 1 inside it more than `bevel` from either end, linear between.
fn window(x: f32, lo: f32, hi: f32, bevel: f32) -> f32 {
    ((x - lo).min(hi - x) / bevel).clamp(0.0, 1.0)
}

/// A building: side faces carry flush corner pilasters, a plinth, shop fronts on the ground
/// floor, a ledge at every floor line, recessed windows with bevelled reveals in regular
/// bays, and a cornice under the roof line; the flat roof carries a few boxy units. The
/// window panes, the flat backs of the recesses, are section [`GLASS`]; the rest is section 0.
pub fn building(b: &Building) -> TriMesh {
    let mut mesh = TriMesh::default();
    // The recess at `w` of a window `depth` deep: glass where the reveal's bevel has ended.
    let recess = |w: f32, depth: f32| (-depth * w, if w >= 1.0 { GLASS } else { 0 });
    let facade = |face: usize, s: f32, t: f32, face_w: f32, face_h: f32| -> (f32, u8) {
        if face == 2 || face == 3 {
            return (0.0, 0); // roof and floor
        }
        let top = face_h;
        if s < b.corner || s > face_w - b.corner || t < 0.35 {
            return (0.0, 0); // pilasters and plinth, flush with the edges
        }
        if t > top - 0.9 {
            // Cornice: a band standing out, stepping back to the roof line.
            return (if t < top - 0.2 { 0.3 } else { 0.1 }, 0);
        }
        let usable = face_w - 2.0 * b.corner;
        let bays = (usable / b.bay).floor().max(1.0);
        let bay = usable / bays;
        let x = (s - b.corner) % bay;
        if t < b.ground_floor {
            // Shop fronts: wide, deep windows over a low sill.
            let w = window(x, bay * 0.08, bay * 0.92, 0.08)
                * window(t, 0.6, b.ground_floor - 0.7, 0.08);
            return recess(w, 0.4);
        }
        let floor_t = (t - b.ground_floor) % b.floor_height;
        if floor_t < 0.25 || floor_t > b.floor_height - 0.05 {
            return (0.12, 0); // the floor line's ledge
        }
        let half = bay * b.window_ratio * 0.5;
        let sill = 0.9;
        let lintel = (b.floor_height - 0.45).max(sill + 0.5);
        let w = window(x, bay * 0.5 - half, bay * 0.5 + half, 0.07)
            * window(floor_t, sill, lintel, 0.07);
        recess(w, 0.25)
    };
    displaced_box(
        &mut mesh,
        Vec3::ZERO,
        Vec3::new(b.width, b.height, b.depth),
        b.cell,
        &facade,
    );
    // Rooftop units: plain boxes at seeded spots, clear of the parapet line.
    let seed = Seed::new(b.seed);
    for unit in 0..b.roof_units {
        let r = |k: u32| unit_f32(seed.derive(u64::from(unit * 8 + k)).value());
        let size = Vec3::new(1.5 + 3.0 * r(0), 1.0 + 2.0 * r(1), 1.5 + 3.0 * r(2));
        let x = (r(3) - 0.5) * (b.width - size.x - 2.0).max(0.0);
        let z = (r(4) - 0.5) * (b.depth - size.z - 2.0).max(0.0);
        displaced_box(
            &mut mesh,
            Vec3::new(x, b.height, z),
            size,
            (b.cell * 2.0).max(0.1),
            &|_, _, _, _, _| (0.0, 0),
        );
    }
    mesh.recompute_normals();
    mesh
}

/// A boulder on the ground: the asteroid generator, flattened a little and sunk by a tenth
/// of its radius.
pub fn boulder(seed: u64, radius: f32, segments: u32) -> TriMesh {
    let mut mesh = asteroid(Seed::new(seed), segments, radius, 0.3);
    for p in &mut mesh.positions {
        p[1] = p[1] * 0.75 + radius * 0.65;
    }
    mesh.recompute_normals();
    mesh
}

/// A pile of boulders of decreasing size around the origin.
pub fn rubble(seed: u64, pieces: u32, segments: u32) -> TriMesh {
    let root = Seed::new(seed);
    let mut mesh = TriMesh::default();
    for piece in 0..pieces {
        let r = |k: u64| unit_f32(root.derive(u64::from(piece) * 16 + k).value());
        let radius = 0.4 + 1.6 * r(0) * (1.0 - piece as f32 / pieces as f32 * 0.6);
        let angle = r(1) * std::f32::consts::TAU;
        let distance = 3.0 * r(2).sqrt();
        let offset = Vec3::new(
            angle.cos() * distance,
            r(3) * radius * 0.8,
            angle.sin() * distance,
        );
        let rock = boulder(
            root.derive(100 + u64::from(piece)).value(),
            radius,
            segments,
        );
        let base = mesh.positions.len() as u32;
        mesh.positions.extend(
            rock.positions
                .iter()
                .map(|p| (Vec3::from(*p) + offset).to_array()),
        );
        mesh.indices.extend(rock.indices.iter().map(|i| i + base));
    }
    mesh.recompute_normals();
    mesh
}

/// A surface of revolution (see [`Lathe`]).
pub fn lathe(l: &Lathe) -> TriMesh {
    // Resample the profile evenly along its length.
    let lengths: Vec<f32> = l
        .profile
        .windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .collect();
    let total: f32 = lengths.iter().sum();
    let along = l.along.max(2);
    let around = l.around.max(3);
    let mut samples = Vec::with_capacity(along as usize);
    let (mut segment, mut start) = (0, 0.0);
    for k in 0..along {
        let d = total * k as f32 / (along - 1) as f32;
        while segment + 1 < lengths.len() && d > start + lengths[segment] {
            start += lengths[segment];
            segment += 1;
        }
        let f = ((d - start) / lengths[segment].max(1e-6)).clamp(0.0, 1.0);
        let (a, b) = (l.profile[segment], l.profile[segment + 1]);
        samples.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
    }
    // A row per sample: a ring of `around` vertices, or one vertex where the profile meets
    // the axis (a ring there would be `around` copies of a point, and degenerate triangles
    // stall the simplifier).
    let mut mesh = TriMesh::default();
    let mut rows = Vec::with_capacity(samples.len());
    for &(radius, y) in &samples {
        let first = mesh.positions.len() as u32;
        if radius < 1e-6 {
            mesh.positions.push([0.0, y, 0.0]);
            rows.push((first, true));
            continue;
        }
        rows.push((first, false));
        for i in 0..around {
            let theta = std::f32::consts::TAU * i as f32 / around as f32;
            let fluted = l.flutes > 0 && y >= l.flute_span.0 && y <= l.flute_span.1;
            let cut = if fluted {
                // Rounded channels: the cosine's positive lobes, squared.
                let c = (theta * l.flutes as f32).cos().max(0.0);
                1.0 - l.flute_depth * c * c
            } else {
                1.0
            };
            let r = radius * cut;
            mesh.positions.push([r * theta.cos(), y, r * theta.sin()]);
        }
    }
    let at = |(first, pole): (u32, bool), i: u32| if pole { first } else { first + i % around };
    for k in 0..rows.len() - 1 {
        let (low, high) = (rows[k], rows[k + 1]);
        for i in 0..around {
            let (a, b) = (at(low, i), at(low, i + 1));
            let (c, d) = (at(high, i), at(high, i + 1));
            // Counter-clockwise from outside for a profile rising with the angle increasing
            // from +X towards +Z; a pole row keeps the one triangle that is not degenerate.
            if !low.1 {
                mesh.indices.extend_from_slice(&[a, c, b]);
            }
            if !high.1 {
                mesh.indices.extend_from_slice(&[b, c, d]);
            }
        }
    }
    mesh.recompute_normals();
    mesh
}

/// Triangles `building` makes for `b` (the grid's quads twice; rooftop units left out).
pub fn building_triangles(b: &Building) -> u64 {
    let n = |x: f32| u64::from((x / b.cell).ceil().max(1.0) as u32);
    let (x, y, z) = (n(b.width), n(b.height), n(b.depth));
    2 * 2 * (x * y + z * y + x * z)
}

/// The twenty props of the city-blocks demo: ten buildings of three to fourteen floors, two
/// towers, three boulders, two rubble piles, a column, a fountain and a lamp post, from 0.5
/// to 3 M triangles each (about 26 M in all).
pub fn city_props() -> Vec<PropSpec> {
    let mut props = Vec::new();
    let building = |name: &str, width: f32, depth: f32, height: f32, target_m: f32, seed: u64| {
        let r = |k: u64| unit_f32(Seed::new(seed).derive(k).value());
        let mut b = Building {
            width,
            depth,
            height,
            cell: 1.0,
            ground_floor: 4.2 + r(0),
            floor_height: 3.1 + r(1),
            bay: 2.6 + 1.2 * r(2),
            window_ratio: 0.45 + 0.25 * r(3),
            corner: 0.6 + 0.8 * r(4),
            roof_units: 2 + (r(5) * 4.0) as u32,
            seed,
        };
        // Cell size for the target triangle count: 2 triangles per cell of the box's surface.
        let surface = 2.0 * (width + depth) * height + 2.0 * width * depth;
        b.cell = (2.0 * surface / (target_m * 1e6)).sqrt();
        PropSpec {
            name: name.to_owned(),
            kind: PropKind::Building(b),
        }
    };
    props.push(building("house-narrow", 8.0, 12.0, 16.0, 0.5, 1));
    props.push(building("house-wide", 18.0, 12.0, 14.0, 0.6, 2));
    props.push(building("corner-block", 24.0, 24.0, 22.0, 1.2, 3));
    props.push(building("terrace", 36.0, 10.0, 12.0, 0.8, 4));
    props.push(building("apartments", 20.0, 16.0, 32.0, 1.5, 5));
    props.push(building("office", 26.0, 20.0, 40.0, 2.0, 6));
    props.push(building("hotel", 22.0, 22.0, 48.0, 2.2, 7));
    props.push(building("warehouse", 40.0, 28.0, 10.0, 1.0, 8));
    props.push(building("school", 30.0, 18.0, 15.0, 0.9, 9));
    props.push(building("clinic", 16.0, 14.0, 18.0, 0.6, 10));
    props.push(building("tower-slim", 18.0, 18.0, 110.0, 2.8, 11));
    props.push(building("tower-wide", 30.0, 24.0, 90.0, 3.0, 12));
    for (i, (radius, segments)) in [(2.0, 210), (3.5, 250), (1.2, 205)].into_iter().enumerate() {
        props.push(PropSpec {
            name: format!("boulder-{}", i + 1),
            kind: PropKind::Boulder {
                seed: 40 + i as u64,
                radius,
                segments,
            },
        });
    }
    for (i, pieces) in [9, 14].into_iter().enumerate() {
        props.push(PropSpec {
            name: format!("rubble-{}", i + 1),
            kind: PropKind::Rubble {
                seed: 60 + i as u64,
                pieces,
                segments: 72,
            },
        });
    }
    // Profiles from the bottom centre to the top centre, (radius, height).
    props.push(PropSpec {
        name: "column".to_owned(),
        kind: PropKind::Lathe(Lathe {
            profile: vec![
                (0.0, 0.0),
                (0.75, 0.0),
                (0.75, 0.3),
                (0.6, 0.45),
                (0.5, 0.6),
                (0.42, 7.2),
                (0.55, 7.5),
                (0.7, 7.7),
                (0.7, 8.0),
                (0.0, 8.0),
            ],
            around: 1024,
            along: 900,
            flutes: 20,
            flute_depth: 0.12,
            flute_span: (0.7, 7.1),
        }),
    });
    props.push(PropSpec {
        name: "fountain".to_owned(),
        kind: PropKind::Lathe(Lathe {
            profile: vec![
                (0.0, 0.0),
                (4.0, 0.0),
                (4.2, 0.8),
                (3.9, 0.9),
                (3.7, 0.4),
                (0.6, 0.4),
                (0.35, 1.0),
                (0.3, 2.2),
                (1.4, 2.5),
                (1.5, 2.8),
                (1.3, 2.75),
                (0.25, 2.6),
                (0.2, 3.4),
                (0.35, 3.6),
                (0.0, 3.8),
            ],
            around: 1400,
            along: 1000,
            flutes: 0,
            flute_depth: 0.0,
            flute_span: (0.0, 0.0),
        }),
    });
    props.push(PropSpec {
        name: "lamp-post".to_owned(),
        kind: PropKind::Lathe(Lathe {
            profile: vec![
                (0.0, 0.0),
                (0.3, 0.0),
                (0.3, 0.25),
                (0.14, 0.5),
                (0.08, 4.2),
                (0.12, 4.3),
                (0.35, 4.6),
                (0.3, 5.0),
                (0.1, 5.2),
                (0.0, 5.3),
            ],
            around: 512,
            along: 600,
            flutes: 16,
            flute_depth: 0.2,
            flute_span: (0.6, 4.0),
        }),
    });
    props
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_building_is_closed_and_its_count_as_estimated() {
        let b = Building {
            width: 10.0,
            depth: 8.0,
            height: 12.0,
            cell: 0.5,
            ground_floor: 4.0,
            floor_height: 3.2,
            bay: 3.0,
            window_ratio: 0.5,
            corner: 0.8,
            roof_units: 0,
            seed: 1,
        };
        let mesh = building(&b);
        assert_eq!(mesh.triangle_count() as u64, building_triangles(&b));
        // Closed: every edge is shared by exactly two triangles, in opposite directions.
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for tri in mesh.indices.as_chunks::<3>().0 {
            for k in 0..3 {
                let (a, c) = (tri[k], tri[(k + 1) % 3]);
                *edges.entry((a.min(c), a.max(c))).or_default() += if a < c { 1 } else { -1 };
            }
        }
        assert!(edges.values().all(|&v| v == 0), "an edge without its twin");
    }

    #[test]
    fn window_panes_are_glass_and_keep_their_section_up_the_dag() {
        let b = Building {
            width: 10.0,
            depth: 8.0,
            height: 12.0,
            cell: 0.25,
            ground_floor: 4.0,
            floor_height: 3.2,
            bay: 3.0,
            window_ratio: 0.5,
            corner: 0.8,
            roof_units: 1,
            seed: 1,
        };
        let mesh = building(&b);
        assert_eq!(mesh.sections.len(), mesh.triangle_count());
        let glass = mesh.sections.iter().filter(|&&s| s == GLASS).count();
        assert!(
            glass > 100 && glass < mesh.triangle_count() / 2,
            "{glass} glass triangles"
        );
        let built = crate::MeshletMesh::build_with(
            &mesh,
            crate::meshlet::CookOptions { normal_weight: 1.0 },
        );
        let at_level = |level: u32, section: u32| -> usize {
            built
                .meshlets
                .iter()
                .filter(|m| m.lod_level == level)
                .map(|m| {
                    (0..m.triangle_count)
                        .filter(|&t| crate::meshlet::triangle_section(m.section, t) == section)
                        .count()
                })
                .sum()
        };
        // Level 0 holds exactly the glass triangles in glass clusters, and the coarser levels
        // still have glass clusters (the panes simplify, their borders stay).
        assert_eq!(at_level(0, u32::from(GLASS)), glass);
        assert_eq!(at_level(0, 0), mesh.triangle_count() - glass);
        assert!(at_level(1, u32::from(GLASS)) > 0);
        // Clusters mix the facade and its glass instead of splitting at every pane.
        assert!(built.meshlets.iter().any(|m| (m.section >> 16) & 0xFF != 0));
        assert!(
            built
                .meshlets
                .iter()
                .all(|m| (m.section & 0xFF) <= u32::from(GLASS)
                    && (m.section >> 8 & 0xFF) <= u32::from(GLASS))
        );
    }

    #[test]
    fn a_building_faces_outwards() {
        let b = Building {
            width: 6.0,
            depth: 6.0,
            height: 6.0,
            cell: 1.0,
            ground_floor: 4.0,
            floor_height: 3.0,
            bay: 3.0,
            window_ratio: 0.5,
            corner: 0.8,
            roof_units: 0,
            seed: 1,
        };
        let mesh = building(&b);
        // The normals point away from the box's centre (on average: a window reveal's
        // vertices face sideways).
        let centre = Vec3::new(0.0, 3.0, 0.0);
        let outward: f32 = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .map(|(p, n)| Vec3::from(*n).dot((Vec3::from(*p) - centre).normalize()))
            .sum::<f32>()
            / mesh.positions.len() as f32;
        assert!(outward > 0.5, "mean outward cosine {outward}");
    }

    #[test]
    fn a_lathe_faces_outwards() {
        let mesh = lathe(&Lathe {
            profile: vec![(0.0, 0.0), (1.0, 0.0), (1.0, 2.0), (0.0, 2.0)],
            around: 32,
            along: 64,
            flutes: 0,
            flute_depth: 0.0,
            flute_span: (0.0, 0.0),
        });
        // On the side wall the normals point away from the axis.
        for (p, n) in mesh.positions.iter().zip(&mesh.normals) {
            if p[1] > 0.2 && p[1] < 1.8 {
                assert!(n[0] * p[0] + n[2] * p[2] > 0.0);
            }
        }
    }

    #[test]
    fn the_terrain_faces_up_and_is_flat_in_the_city() {
        let t = Terrain {
            size: 64.0,
            spacing: 4.0,
            city_half: 16.0,
            rise: 8.0,
            hill_height: 30.0,
            seed: 1,
        };
        let mesh = terrain_mesh(&t);
        assert_eq!(mesh.positions.len() as u32, t.samples() * t.samples());
        assert!(
            mesh.normals.iter().all(|n| n[1] > 0.0),
            "every normal faces up"
        );
        // The vertices are the heightfield, in grid order.
        let n = t.samples() as usize;
        let p = mesh.positions[3 * n + 5];
        assert_eq!(
            p,
            [-32.0 + 5.0 * 4.0, t.height(p[0], p[2]), -32.0 + 3.0 * 4.0]
        );
        assert!(t.height(0.0, 0.0).abs() < 0.5, "the city is flat");
    }

    #[test]
    fn the_city_set_holds_twenty_distinct_props() {
        let props = city_props();
        assert_eq!(props.len(), 20);
        let mut names: Vec<_> = props.iter().map(|p| p.name.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), 20);
    }
}
