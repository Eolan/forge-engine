//! Procedural props for the city-blocks demo (issue #34): buildings whose facades carry
//! recessed windows, floor ledges and cornices in the geometry itself, boulders and rubble,
//! and lathe-turned columns, fountains and lamp posts. Each is dense on purpose (0.5 to 3 M
//! triangles): the demo exists to push that much geometry through the cluster DAG.
//!
//! Units are metres, +Y up, the prop standing on the ground plane with its footprint centred
//! on the origin. Everything is a function of its parameters, so a prop's cache key is its
//! parameters (see `crate::cache`).

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use forge_core::Seed;
use forge_core::hash::unit_f32;
use glam::Vec3;

use crate::meshlet::CookOptions;
use crate::procedural::{TriMesh, asteroid, fbm};
use crate::stone::{Stone, stone};

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
    /// A stone of the island (see [`stone`], #130).
    Stone(Stone),
    /// A surface of revolution (see [`lathe`]).
    Lathe(Lathe),
    /// A mesh made elsewhere (see [`Imported`]): a model through glTF.
    Imported(Imported),
    /// A box with rounded edges (see [`block`]): crates, blocks, a floor.
    Block(Block),
    /// The ground (see [`terrain_mesh`]).
    Terrain(Terrain),
    /// A ground made elsewhere (the island of `forge-procgen`; see [`heightfield_mesh`]).
    Heightfield(Heightfield),
}

/// A heightfield generated outside this crate, meshed like the city's ground. Its samples
/// come through `source`, called only when the mesh is not in the cache, so a field that
/// takes a minute to generate costs nothing on a warm start; `key` names its parameters and
/// is the cache key's input (with the samples per side and the spacing).
#[derive(Clone)]
pub struct Heightfield {
    /// The parameters as text, unique per field (a seed, a spacing, the generator's settings).
    pub key: String,
    /// Samples per side.
    pub samples: u32,
    /// Metres between samples.
    pub spacing: f32,
    /// The samples, row-major, `samples × samples` of them, as [`Terrain::heights`] lays them
    /// out.
    pub source: Arc<dyn Fn() -> Arc<[f32]> + Send + Sync>,
    /// The cells drawn finer, from the samples (the island's river channels, #105), or none:
    /// every cell two triangles. Part of the field, so `key` names its parameters too.
    pub detail: Option<Arc<DetailSource>>,
    /// The cells this mesh draws, a tile of the field (#106), or none: all of them. A tile
    /// is drawn in the whole field's frame, its vertices those of the whole field's mesh (see
    /// [`heightfield_window_mesh`]).
    pub window: Option<CellWindow>,
}

/// A rectangle of a heightfield's cells: a tile of it (#106).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellWindow {
    /// The first cell's `(i, j)`: the cell whose first corner is sample `(i, j)`.
    pub first: [u32; 2],
    /// Cells along x and along z.
    pub cells: [u32; 2],
}

impl CellWindow {
    /// Whether cell `(i, j)` is inside.
    pub fn contains(&self, i: u32, j: u32) -> bool {
        (self.first[0]..self.first[0] + self.cells[0]).contains(&i)
            && (self.first[1]..self.first[1] + self.cells[1]).contains(&j)
    }
}

/// What makes a heightfield's [`HeightfieldDetail`] from its samples.
pub type DetailSource = dyn Fn(&[f32]) -> Arc<HeightfieldDetail> + Send + Sync;

/// Cells of a heightfield drawn finer than its samples (the island's river channels, #105):
/// each split into `split × split` quads whose heights are given, the cells around them
/// stitched to their edges without a crack (see [`refined_heightfield_mesh`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HeightfieldDetail {
    /// Quads a side of a refined cell.
    pub split: u32,
    /// The refined cells, `j × (samples − 1) + i` for the cell whose first corner is sample
    /// `(i, j)`, ascending.
    pub cells: Vec<u32>,
    /// Per refined cell in that order, `(split + 1)²` heights row-major along +z: its fine
    /// vertices, corners included. Where two refined cells meet they give the same heights,
    /// and on an edge a refined cell shares with a coarse one they lie on the coarse edge.
    pub heights: Vec<f32>,
}

/// A mesh made outside this crate (a model from Blender through glTF, #138): its triangles,
/// and the key that names them for the cache (the model's file and a hash of its bytes, so a
/// new export cooks again).
#[derive(Clone)]
pub struct Imported {
    /// What names the mesh: unique per model and version.
    pub key: String,
    /// The triangles, in the prop's frame.
    pub mesh: Arc<TriMesh>,
    /// How much its normals weigh in the simplification error ([`CookOptions::normal_weight`]):
    /// `None` for a hard-surface prop's 0.5. A smooth body the camera stays on wants more (the
    /// lab's rocket, whose shading lines popped along its length as it turned).
    pub normal_weight: Option<f32>,
}

impl fmt::Debug for Imported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Imported").field("key", &self.key).finish()
    }
}

impl PartialEq for Imported {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key && self.normal_weight == other.normal_weight
    }
}

impl fmt::Debug for Heightfield {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut s = f.debug_struct("Heightfield");
        s.field("key", &self.key)
            .field("samples", &self.samples)
            .field("spacing", &self.spacing)
            .field("detail", &self.detail.is_some());
        // Only a tile names its window: the whole field's text, and its cache key, stay as
        // they were.
        if let Some(window) = &self.window {
            s.field("window", window);
        }
        s.finish()
    }
}

impl PartialEq for Heightfield {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
            && self.samples == other.samples
            && self.spacing == other.spacing
            && self.detail.is_some() == other.detail.is_some()
            && self.window == other.window
    }
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
            PropKind::Stone(s) => stone(s),
            PropKind::Lathe(l) => lathe(l),
            PropKind::Block(b) => block(b),
            PropKind::Imported(m) => (*m.mesh).clone(),
            PropKind::Terrain(t) => terrain_mesh(t),
            PropKind::Heightfield(h) => {
                let heights = (h.source)();
                match (&h.detail, h.window) {
                    (Some(detail), None) => {
                        refined_heightfield_mesh(h.samples, h.spacing, &heights, &detail(&heights))
                    }
                    (None, None) => heightfield_mesh(h.samples, h.spacing, &heights),
                    (detail, Some(window)) => {
                        let detail = detail.as_ref().map(|d| d(&heights)).unwrap_or_default();
                        heightfield_window_mesh(h.samples, h.spacing, &heights, &detail, window)
                    }
                }
            }
        }
    }

    /// How to cook it: hard-surface props weigh their normals in the simplification error
    /// (their windows and flutes are shallow in depth but not in shading), and so do the
    /// island's smooth stones by their own weight (#131); the city's lumpy rocks and the grounds
    /// do not.
    pub fn cook_options(&self) -> CookOptions {
        CookOptions {
            normal_weight: match self.kind {
                PropKind::Building(_) => 1.0,
                PropKind::Lathe(_) | PropKind::Block(_) => 0.5,
                PropKind::Imported(ref i) => i.normal_weight.unwrap_or(0.5),
                PropKind::Stone(ref s) => s.normal_weight,
                PropKind::Boulder { .. }
                | PropKind::Rubble { .. }
                | PropKind::Terrain(_)
                | PropKind::Heightfield(_) => 0.0,
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
    heightfield_mesh(t.samples(), t.spacing, &t.heights())
}

/// A heightfield of `samples × samples` heights (row-major) as a grid mesh centred on the
/// origin: vertex `j × samples + i` at `(−half + i × spacing, height, −half + j × spacing)`
/// with `half` the grid's half side, two counter-clockwise triangles per cell seen from above.
pub fn heightfield_mesh(samples: u32, spacing: f32, heights: &[f32]) -> TriMesh {
    let n = samples;
    assert_eq!(
        heights.len(),
        (n as usize) * (n as usize),
        "a heightfield of {n} × {n} samples"
    );
    let half = (n - 1) as f32 * spacing * 0.5;
    let mut mesh = TriMesh::default();
    mesh.positions.reserve((n * n) as usize);
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (-half + i as f32 * spacing, -half + j as f32 * spacing);
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

/// [`heightfield_mesh`] with `detail`'s cells drawn finer: each split into `split × split`
/// quads at the heights it gives (two triangles each, split along the same diagonal as the
/// coarse cells). A coarse cell next to a refined one is a fan from its centre over its edges'
/// fine vertices, which draws its two triangles as before when those vertices lie on its edges
/// (its centre sits on the diagonal between them). The vertices on an edge are shared by the
/// cells on either side, so the mesh has no T-junction and no crack.
pub fn refined_heightfield_mesh(
    samples: u32,
    spacing: f32,
    heights: &[f32],
    detail: &HeightfieldDetail,
) -> TriMesh {
    let side = samples.saturating_sub(1);
    let window = CellWindow {
        first: [0, 0],
        cells: [side, side],
    };
    heightfield_window_mesh(samples, spacing, heights, detail, window)
}

/// The cells of `window` of [`refined_heightfield_mesh`]'s mesh: a tile of it (#106), in the
/// whole field's frame. Its vertices are the whole mesh's to the bit, normals included: the
/// cells around the window are built too, so that the vertices on its outline get the normals
/// of every triangle around them, then dropped. Tiles side by side share their outline's
/// vertices and shade alike across it.
pub fn heightfield_window_mesh(
    samples: u32,
    spacing: f32,
    heights: &[f32],
    detail: &HeightfieldDetail,
    window: CellWindow,
) -> TriMesh {
    let n = samples as usize;
    assert_eq!(heights.len(), n * n, "a heightfield of {n} × {n} samples");
    let side = n - 1;
    let k = detail.split.max(1) as usize;
    let per = (k + 1) * (k + 1);
    assert_eq!(
        detail.heights.len(),
        detail.cells.len() * per,
        "{per} heights per refined cell"
    );
    let [x0, z0] = window.first.map(|c| c as usize);
    let (x1, z1) = (x0 + window.cells[0] as usize, z0 + window.cells[1] as usize);
    assert!(x1 <= side && z1 <= side, "the window inside the field");
    let half = side as f32 * spacing * 0.5;
    let fine = spacing / k as f32;
    // The cells built: the window and a cell around it, within the field.
    let (hx0, hz0, hx1, hz1) = (
        x0.saturating_sub(1),
        z0.saturating_sub(1),
        (x1 + 1).min(side),
        (z1 + 1).min(side),
    );
    // The refined cells looked up: those and a cell more around them (their neighbours).
    let (lx0, lz0, lx1, lz1) = (
        hx0.saturating_sub(1),
        hz0.saturating_sub(1),
        (hx1 + 1).min(side),
        (hz1 + 1).min(side),
    );
    let lw = lx1 - lx0;
    let mut slot = vec![u32::MAX; lw * (lz1 - lz0)];
    let rows = detail.cells.partition_point(|&c| (c as usize) < lz0 * side)
        ..detail.cells.partition_point(|&c| (c as usize) < lz1 * side);
    for s in rows.clone() {
        let c = detail.cells[s] as usize;
        let (i, j) = (c % side, c / side);
        if (lx0..lx1).contains(&i) {
            slot[(j - lz0) * lw + i - lx0] = s as u32;
        }
    }
    let slot_of = |i: usize, j: usize| {
        let s = if (lx0..lx1).contains(&i) && (lz0..lz1).contains(&j) {
            slot[(j - lz0) * lw + i - lx0]
        } else {
            u32::MAX
        };
        (s != u32::MAX).then_some(s as usize)
    };
    let refined = |i: usize, j: usize| slot_of(i, j).is_some();
    let width = hx1 - hx0 + 1;
    let vertex = |i: usize, j: usize| ((j - hz0) * width + i - hx0) as u32;
    let mut mesh = TriMesh::default();
    mesh.positions.reserve(width * (hz1 - hz0 + 1));
    for j in hz0..=hz1 {
        for i in hx0..=hx1 {
            let (x, z) = (-half + i as f32 * spacing, -half + j as f32 * spacing);
            mesh.positions.push([x, heights[j * n + i], z]);
        }
    }
    // The refined cells' corners at the detail's heights (the samples' where they meet a
    // coarse cell), in the cells' order.
    for s in rows {
        let c = detail.cells[s] as usize;
        let (i, j) = (c % side, c / side);
        for (u, v) in [(0, 0), (k, 0), (0, k), (k, k)] {
            let (si, sj) = (i + u / k, j + v / k);
            if (hx0..=hx1).contains(&si) && (hz0..=hz1).contains(&sj) {
                mesh.positions[vertex(si, sj) as usize][1] =
                    detail.heights[s * per + v * (k + 1) + u];
            }
        }
    }
    // An edge's `k − 1` inner vertices, made from the first refined cell that meets it (the
    // one below or to its left before the one above or to its right, as the cells' order
    // meets them): edge `2 (j n + i)` from sample (i, j) along +x, `2 (j n + i) + 1` along +z.
    let mut edges: HashMap<usize, u32> = HashMap::new();
    let make_edge = |mesh: &mut TriMesh, key: usize| -> u32 {
        let (s, along_z) = (key / 2, key % 2 == 1);
        let (i, j) = (s % n, s / n);
        let candidates: [(usize, usize, [usize; 2]); 2] = if along_z {
            [(i.wrapping_sub(1), j, [k, 0]), (i, j, [0, 0])]
        } else {
            [(i, j.wrapping_sub(1), [0, k]), (i, j, [0, 0])]
        };
        let step = if along_z { [0, 1] } else { [1, 0] };
        let (ci, cj, from, s) = candidates
            .into_iter()
            .find_map(|(ci, cj, from)| slot_of(ci, cj).map(|s| (ci, cj, from, s)))
            .expect("an edge made for a refined cell");
        let cell_heights = &detail.heights[s * per..(s + 1) * per];
        let (cx0, cz0) = (-half + ci as f32 * spacing, -half + cj as f32 * spacing);
        let start = mesh.positions.len() as u32;
        for t in 1..k {
            let (u, v) = (from[0] + t * step[0], from[1] + t * step[1]);
            mesh.positions.push([
                cx0 + u as f32 * fine,
                cell_heights[v * (k + 1) + u],
                cz0 + v as f32 * fine,
            ]);
        }
        start
    };
    // Per triangle, whether it is the window's (the cells around it only lend their normals).
    let mut keep: Vec<bool> = Vec::new();
    let mut local = vec![0u32; per];
    mesh.indices.reserve(6 * (hx1 - hx0) * (hz1 - hz0));
    for j in hz0..hz1 {
        for i in hx0..hx1 {
            let Some(s) = slot_of(i, j) else {
                continue;
            };
            let cell_heights = &detail.heights[s * per..(s + 1) * per];
            let cx0 = -half + i as f32 * spacing;
            let cz0 = -half + j as f32 * spacing;
            // Bottom, top, left, right.
            let keys = [
                2 * (j * n + i),
                2 * ((j + 1) * n + i),
                2 * (j * n + i) + 1,
                2 * (j * n + i + 1) + 1,
            ];
            let mut starts = [0u32; 4];
            for (e, &key) in keys.iter().enumerate() {
                starts[e] = match edges.get(&key) {
                    Some(&start) => start,
                    None => {
                        let start = make_edge(&mut mesh, key);
                        edges.insert(key, start);
                        start
                    }
                };
            }
            let inner = mesh.positions.len() as u32;
            for v in 1..k {
                for u in 1..k {
                    mesh.positions.push([
                        cx0 + u as f32 * fine,
                        cell_heights[v * (k + 1) + u],
                        cz0 + v as f32 * fine,
                    ]);
                }
            }
            for v in 0..=k {
                for u in 0..=k {
                    let corner = (u == 0 || u == k) && (v == 0 || v == k);
                    local[v * (k + 1) + u] = if corner {
                        vertex(i + u / k, j + v / k)
                    } else if v == 0 {
                        starts[0] + (u - 1) as u32
                    } else if v == k {
                        starts[1] + (u - 1) as u32
                    } else if u == 0 {
                        starts[2] + (v - 1) as u32
                    } else if u == k {
                        starts[3] + (v - 1) as u32
                    } else {
                        inner + ((v - 1) * (k - 1) + (u - 1)) as u32
                    };
                }
            }
            let mine = window.contains(i as u32, j as u32);
            for v in 0..k {
                for u in 0..k {
                    let a = local[v * (k + 1) + u];
                    let (b, c, d) = (
                        local[v * (k + 1) + u + 1],
                        local[(v + 1) * (k + 1) + u],
                        local[(v + 1) * (k + 1) + u + 1],
                    );
                    mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
                    keep.extend([mine, mine]);
                }
            }
        }
    }
    // The coarse cells: two triangles, or a fan where a neighbour across an edge is refined.
    let mut ring: Vec<u32> = Vec::with_capacity(4 * k);
    for j in hz0..hz1 {
        for i in hx0..hx1 {
            if refined(i, j) {
                continue;
            }
            let mine = window.contains(i as u32, j as u32);
            let a = vertex(i, j);
            let (b, c, d) = (vertex(i + 1, j), vertex(i, j + 1), vertex(i + 1, j + 1));
            let below = j > 0 && refined(i, j - 1);
            let above = refined(i, j + 1);
            let left = i > 0 && refined(i - 1, j);
            let right = refined(i + 1, j);
            if !(below || above || left || right) {
                mesh.indices.extend_from_slice(&[a, c, b, b, c, d]);
                keep.extend([mine, mine]);
                continue;
            }
            // An edge a refined cell outside the cells built meets is made here.
            let mut inner = |mesh: &mut TriMesh, key: usize, t: usize| {
                let start = match edges.get(&key) {
                    Some(&start) => start,
                    None => {
                        let start = make_edge(mesh, key);
                        edges.insert(key, start);
                        start
                    }
                };
                start + (t - 1) as u32
            };
            // Round the cell counter-clockwise seen from above: up the left edge, along the
            // top, down the right edge, back along the bottom.
            ring.clear();
            ring.push(a);
            if left {
                for t in 1..k {
                    ring.push(inner(&mut mesh, 2 * (j * n + i) + 1, t));
                }
            }
            ring.push(c);
            if above {
                for t in 1..k {
                    ring.push(inner(&mut mesh, 2 * ((j + 1) * n + i), t));
                }
            }
            ring.push(d);
            if right {
                for t in (1..k).rev() {
                    ring.push(inner(&mut mesh, 2 * (j * n + i + 1) + 1, t));
                }
            }
            ring.push(b);
            if below {
                for t in (1..k).rev() {
                    ring.push(inner(&mut mesh, 2 * (j * n + i), t));
                }
            }
            let (pb, pc) = (mesh.positions[b as usize], mesh.positions[c as usize]);
            let centre = mesh.positions.len() as u32;
            mesh.positions.push([
                0.5 * (pb[0] + pc[0]),
                0.5 * (pb[1] + pc[1]),
                0.5 * (pb[2] + pc[2]),
            ]);
            for m in 0..ring.len() {
                let next = ring[(m + 1) % ring.len()];
                mesh.indices.extend_from_slice(&[centre, ring[m], next]);
                keep.push(mine);
            }
        }
    }
    mesh.recompute_normals();
    if keep.iter().all(|&mine| mine) {
        return mesh;
    }
    // The window's triangles and the vertices they use, in their order.
    let mut used = vec![false; mesh.positions.len()];
    let mut indices = Vec::with_capacity(mesh.indices.len());
    for (tri, &mine) in mesh.indices.as_chunks::<3>().0.iter().zip(&keep) {
        if mine {
            indices.extend_from_slice(tri);
            for &v in tri {
                used[v as usize] = true;
            }
        }
    }
    let mut remap = vec![u32::MAX; used.len()];
    let mut out = TriMesh::default();
    for (v, _) in used.iter().enumerate().filter(|&(_, &u)| u) {
        remap[v] = out.positions.len() as u32;
        out.positions.push(mesh.positions[v]);
        out.normals.push(mesh.normals[v]);
    }
    out.indices = indices.into_iter().map(|v| remap[v as usize]).collect();
    out
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

/// A box with rounded edges, centred on the origin (issue #136: `physics-lab`'s blocks and
/// floor).
#[derive(Clone, Debug, PartialEq)]
pub struct Block {
    /// Half its size along x, y and z, metres.
    pub half: [f32; 3],
    /// The edges' radius, metres (at most the smallest half size).
    pub radius: f32,
    /// Quads along each side of a face, besides the two rows on its rounded rims.
    pub segments: u32,
}

/// A rounded box (see [`Block`]): each face a grid whose outer rows are the rims, each vertex
/// on the box shrunk by the radius and pushed out by it along the way from that inner box, so
/// the faces stay flat and the edges and corners are quarter cylinders and spheres in one
/// step of the grid.
pub fn block(b: &Block) -> TriMesh {
    let half = Vec3::from(b.half);
    // At least a millimetre: the rims are rows of their own, and a zero radius would make them
    // degenerate triangles.
    let r = b.radius.clamp(1e-3, half.min_element());
    let inner = half - Vec3::splat(r);
    let n = b.segments.max(1) + 2;
    // Along a face's side from −1 to 1: the rim's row, the flat part evenly, the other rim's.
    let along = |k: u32, h: f32| -> f32 {
        match k {
            0 => -h,
            k if k == n => h,
            k => -(h - r) + 2.0 * (h - r) * (k - 1) as f32 / (n - 2) as f32,
        }
    };
    let mut mesh = TriMesh::default();
    // Each face: its normal axis and sign, and the two axes along it, ordered so that
    // `u × v` points out of the box.
    for (axis, sign) in [
        (0, 1.0),
        (0, -1.0),
        (1, 1.0),
        (1, -1.0),
        (2, 1.0),
        (2, -1.0),
    ] {
        let (u, v) = if sign > 0.0 {
            ((axis + 1) % 3, (axis + 2) % 3)
        } else {
            ((axis + 2) % 3, (axis + 1) % 3)
        };
        let first = mesh.positions.len() as u32;
        for j in 0..=n {
            for i in 0..=n {
                let mut p = Vec3::ZERO;
                p[axis] = sign * half[axis];
                p[u] = along(i, half[u]);
                p[v] = along(j, half[v]);
                let core = p.clamp(-inner, inner);
                let out = (p - core).normalize_or(Vec3::ZERO);
                let normal = if out == Vec3::ZERO {
                    let mut f = Vec3::ZERO;
                    f[axis] = sign;
                    f
                } else {
                    out
                };
                mesh.positions.push((core + normal * r).to_array());
                mesh.normals.push(normal.to_array());
            }
        }
        let at = |i: u32, j: u32| first + j * (n + 1) + i;
        for j in 0..n {
            for i in 0..n {
                let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
                mesh.indices.extend_from_slice(&[a, b, d, a, d, c]);
            }
        }
    }
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
    fn a_block_fills_its_box_with_its_faces_outward() {
        let b = Block {
            half: [0.4, 0.25, 0.6],
            radius: 0.03,
            segments: 4,
        };
        let mesh = block(&b);
        let (mut low, mut high) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in &mesh.positions {
            low = low.min(Vec3::from(*p));
            high = high.max(Vec3::from(*p));
        }
        assert!(high.abs_diff_eq(Vec3::from(b.half), 1e-6), "{high}");
        assert!(low.abs_diff_eq(-Vec3::from(b.half), 1e-6), "{low}");
        for t in mesh.indices.as_chunks::<3>().0 {
            let [a, c, d] = t.map(|i| Vec3::from(mesh.positions[i as usize]));
            let normal = (c - a).cross(d - a);
            assert!(normal.length() > 0.0, "a degenerate triangle");
            assert!(normal.dot(a + c + d) > 0.0, "a face turned inward");
        }
        for n in &mesh.normals {
            assert!((Vec3::from(*n).length() - 1.0).abs() < 1e-5);
        }
        // Six faces of (4 + 2) × (4 + 2) quads.
        assert_eq!(mesh.triangle_count(), 6 * 6 * 6 * 2);
    }

    #[test]
    fn a_refined_heightfield_has_no_crack_and_keeps_its_coarse_cells() {
        // 6 × 6 samples 8 m apart; three cells split into 4 × 4 quads, their inner vertices a
        // metre down (a channel), those on the region's outline on the coarse surface.
        let (n, spacing, k) = (6usize, 8.0f32, 4usize);
        let height = |i: usize, j: usize| 1.3 * i as f32 + 0.5 * (j * j) as f32;
        let heights: Vec<f32> = (0..n * n).map(|s| height(s % n, s / n)).collect();
        // The coarse surface at fine vertex (fx, fz) of the grid, split along the diagonal.
        let drawn = |fx: usize, fz: usize| {
            let (i, j) = ((fx / k).min(n - 2), (fz / k).min(n - 2));
            let (tx, tz) = (
                (fx - i * k) as f32 / k as f32,
                (fz - j * k) as f32 / k as f32,
            );
            let (a, b, c, d) = (
                height(i, j),
                height(i + 1, j),
                height(i, j + 1),
                height(i + 1, j + 1),
            );
            if tx + tz <= 1.0 {
                a + tx * (b - a) + tz * (c - a)
            } else {
                d + (1.0 - tx) * (c - d) + (1.0 - tz) * (b - d)
            }
        };
        let cells = [(1, 1), (2, 1), (2, 2)];
        let inside = |fx: usize, fz: usize| {
            let covered = |x: usize, z: usize| cells.contains(&(x, z));
            // A fine vertex is inside when every cell around it is refined.
            let (x0, z0) = ((fx.max(1) - 1) / k, (fz.max(1) - 1) / k);
            let (x1, z1) = (fx / k, fz / k);
            covered(x0, z0) && covered(x1, z0) && covered(x0, z1) && covered(x1, z1)
        };
        let mut detail = HeightfieldDetail {
            split: k as u32,
            ..HeightfieldDetail::default()
        };
        let mut sorted = cells.map(|(i, j)| (j * (n - 1) + i) as u32);
        sorted.sort_unstable();
        for &c in &sorted {
            let (i, j) = (c as usize % (n - 1), c as usize / (n - 1));
            for v in 0..=k {
                for u in 0..=k {
                    let (fx, fz) = (i * k + u, j * k + v);
                    let down = if inside(fx, fz) { 1.0 } else { 0.0 };
                    detail.heights.push(drawn(fx, fz) - down);
                }
            }
        }
        detail.cells = sorted.to_vec();
        let mesh = refined_heightfield_mesh(n as u32, spacing, &heights, &detail);
        // Every inner edge has its twin; the border's edges are the field's outline.
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        let mut area = 0.0;
        for tri in mesh.indices.as_chunks::<3>().0 {
            for e in 0..3 {
                let (a, c) = (tri[e], tri[(e + 1) % 3]);
                *edges.entry((a.min(c), a.max(c))).or_default() += if a < c { 1 } else { -1 };
            }
            let p = tri.map(|t| Vec3::from(mesh.positions[t as usize]));
            let normal = (p[1] - p[0]).cross(p[2] - p[0]);
            assert!(normal.y > 0.0, "a triangle facing down");
            area += 0.5 * normal.y;
        }
        let open = edges.values().filter(|&&v| v != 0).count();
        assert_eq!(open, 4 * (n - 1), "only the outline's edges are open");
        let side = (n - 1) as f32 * spacing;
        assert!((area - side * side).abs() < 1e-2, "{area}");
        // The fans draw the coarse cells as their two triangles: every vertex of the mesh lies
        // on the coarse surface except the channel's.
        let half = side * 0.5;
        let fine = spacing / k as f32;
        let channel = mesh
            .positions
            .iter()
            .filter(|p| {
                let (fx, fz) = ((p[0] + half) / fine, (p[2] + half) / fine);
                let (ix, iz) = (fx.round() as usize, fz.round() as usize);
                let on_grid = (fx - ix as f32).abs() < 1e-3 && (fz - iz as f32).abs() < 1e-3;
                let expected = if on_grid { drawn(ix, iz) } else { p[1] };
                (p[1] - expected).abs() > 1e-4
            })
            .count();
        let inner = (0..=(n - 1) * k)
            .flat_map(|fz| (0..=(n - 1) * k).map(move |fx| (fx, fz)))
            .filter(|&(fx, fz)| inside(fx, fz))
            .count();
        assert_eq!(channel, inner);
    }

    #[test]
    fn tiles_of_a_refined_heightfield_are_the_whole_mesh_to_the_bit() {
        // A field of 10 × 10 cells with a band of refined cells crossing the tiles' borders,
        // and tiles of uneven sizes: their triangles together are the whole mesh's, and every
        // vertex of a tile has the whole mesh's position and normal, bit for bit (#106).
        let (n, spacing, k) = (11usize, 8.0f32, 4usize);
        let fine = |fx: usize, fz: usize| {
            let (x, z) = (fx as f32 / k as f32, fz as f32 / k as f32);
            (0.7 * x).sin() * 3.0 + 0.4 * z * z - 0.9 * (x * z * 0.3).cos()
        };
        let heights: Vec<f32> = (0..n * n).map(|s| fine(s % n * k, s / n * k)).collect();
        let mut cells: Vec<u32> = [
            (1, 2),
            (2, 2),
            (2, 3),
            (3, 3),
            (4, 3),
            (4, 4),
            (5, 4),
            (6, 5),
            (6, 6),
            (7, 6),
            (9, 9),
            (0, 9),
        ]
        .iter()
        .map(|&(i, j)| (j * (n - 1) + i) as u32)
        .collect();
        cells.sort_unstable();
        let mut detail = HeightfieldDetail {
            split: k as u32,
            cells: cells.clone(),
            heights: Vec::new(),
        };
        for &c in &cells {
            let (i, j) = (c as usize % (n - 1), c as usize / (n - 1));
            for v in 0..=k {
                for u in 0..=k {
                    detail.heights.push(fine(i * k + u, j * k + v) - 0.5);
                }
            }
        }
        let whole = refined_heightfield_mesh(n as u32, spacing, &heights, &detail);
        let bits = |p: [f32; 3]| p.map(f32::to_bits);
        // A triangle as its positions' bits, from its smallest corner (keeping its winding).
        let triangles = |mesh: &TriMesh| {
            let mut out: Vec<[[u32; 3]; 3]> = mesh
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .map(|t| {
                    let p = t.map(|v| bits(mesh.positions[v as usize]));
                    let m = (0..3).min_by_key(|&r| p[r]).expect("three corners");
                    [p[m], p[(m + 1) % 3], p[(m + 2) % 3]]
                })
                .collect();
            out.sort_unstable();
            out
        };
        let normal_of: HashMap<[u32; 3], [u32; 3]> = whole
            .positions
            .iter()
            .zip(&whole.normals)
            .map(|(&p, &q)| (bits(p), bits(q)))
            .collect();
        let mut together = TriMesh::default();
        for (tx, tz) in [(0, 0), (3, 0), (0, 4), (3, 4)] {
            let window = CellWindow {
                first: [tx, tz],
                cells: [if tx == 0 { 3 } else { 7 }, if tz == 0 { 4 } else { 6 }],
            };
            let tile = heightfield_window_mesh(n as u32, spacing, &heights, &detail, window);
            for (&p, &q) in tile.positions.iter().zip(&tile.normals) {
                assert_eq!(normal_of.get(&bits(p)), Some(&bits(q)), "at {p:?}");
            }
            let base = together.positions.len() as u32;
            together.positions.extend_from_slice(&tile.positions);
            together
                .indices
                .extend(tile.indices.iter().map(|v| v + base));
        }
        assert_eq!(triangles(&together), triangles(&whole));
    }

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

// Derived data (#208): the island's drawn ground keeps its detail between starts.
forge_core::stored!(HeightfieldDetail {
    split,
    cells,
    heights
});
