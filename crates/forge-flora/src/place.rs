//! Placing plants on the ground by rules (#269's second point): the ground's fields (slope, the
//! sun's heat by aspect, soil depth, moisture from the flow), each species' habitat as bands over
//! them, the light under the canopy already placed, and spacing by pairs of species, a
//! deterministic dart throwing in priority order (Bridson 2007's grid, Wei 2010's classes). The
//! planted rules (rows along a line, groves in a polygon) are the city's street, park and
//! courtyard trees (`forge-procgen::city::trees`) as one system with the wild ones.
//!
//! The order: the planted rows and groves, then the wild canopy, then the understorey in the light
//! the canopy leaves. Each wild layer throws one dart per cell of a grid (2.5 m for the trees,
//! 0.75 m for the shrubs), its place and its random bits hashed from the cell and the seed, and
//! takes the darts in the order of their hashed priority. It runs over a whole region at once, a
//! pure function of the ground, the rules and the seed, never in a window that follows a player
//! (`docs/research/flora.md`, the patents).
//!
//! Plan coordinates: `x` east, `y` south (world +Z; north, −Z, is up on the maps), metres.

use forge_core::{SplitMix64, dmath, hash};
use glam::DVec2;
use serde::Deserialize;

use crate::species::{Species, growth_share};

/// A square heightfield: `side × side` samples `cell` metres apart, row by row from the
/// north-west corner.
#[derive(Clone, Debug)]
pub struct Ground {
    /// Samples per side.
    pub side: usize,
    /// The samples' spacing, metres.
    pub cell: f64,
    /// The heights, metres, row by row (`y` then `x`).
    pub heights: Vec<f32>,
}

impl Ground {
    /// A heightfield from its samples.
    pub fn new(side: usize, cell: f64, heights: Vec<f32>) -> Self {
        assert_eq!(heights.len(), side * side, "a square of samples");
        Self {
            side,
            cell,
            heights,
        }
    }

    /// Its side, metres.
    pub fn size(&self) -> f64 {
        (self.side - 1) as f64 * self.cell
    }

    /// The sample at column `i`, row `j` (clamped to the field).
    pub fn at(&self, i: i64, j: i64) -> f64 {
        let n = self.side as i64 - 1;
        f64::from(self.heights[(j.clamp(0, n) * (n + 1) + i.clamp(0, n)) as usize])
    }

    /// The height at `p`, bilinear.
    pub fn height(&self, p: DVec2) -> f64 {
        let q = p / self.cell;
        let (i, j) = (q.x.floor() as i64, q.y.floor() as i64);
        let (fx, fy) = (q.x - i as f64, q.y - j as f64);
        let a = self.at(i, j) + (self.at(i + 1, j) - self.at(i, j)) * fx;
        let b = self.at(i, j + 1) + (self.at(i + 1, j + 1) - self.at(i, j + 1)) * fx;
        a + (b - a) * fy
    }

    /// The example hillside: an east–west valley with a stream, a south-facing slope to its
    /// north, a north-facing one to its south, ridges at the north and south edges, `size`
    /// metres a side.
    pub fn example(size: f64, cell: f64, seed: u64) -> Self {
        let side = (size / cell).round() as usize + 1;
        let mut heights = Vec::with_capacity(side * side);
        for j in 0..side {
            for i in 0..side {
                let p = DVec2::new(i as f64 * cell, j as f64 * cell);
                heights.push(example_height(p, size, seed) as f32);
            }
        }
        Self::new(side, cell, heights)
    }
}

/// The example's valley line: where the stream runs, `y` at `x`.
pub fn example_valley(x: f64, size: f64) -> f64 {
    size * 0.52 + size * 0.06 * dmath::sin(x / size * std::f64::consts::TAU * 0.8)
}

fn example_height(p: DVec2, size: f64, seed: u64) -> f64 {
    let v = example_valley(p.x, size);
    // The north side rises to its ridge faster than the south side.
    let d = p.y - v;
    let reach = if d < 0.0 { v } else { size - v };
    let t = (d.abs() / reach).min(1.2);
    let profile = 85.0 * dmath::powf(t, 1.35);
    // A spur or two and the ground's roughness.
    let rough = fbm(p / 90.0, seed) * 9.0 + fbm(p / 25.0, seed ^ 0x9e37) * 2.0;
    18.0 + profile + rough - p.x / size * 12.0
}

/// Value noise on an integer lattice, smoothed, three octaves, in about −1 to 1.
fn fbm(p: DVec2, seed: u64) -> f64 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut q = p;
    for o in 0..3 {
        sum += value(q, seed.wrapping_add(o)) * amp;
        q *= 2.03;
        amp *= 0.5;
    }
    sum / 0.875
}

fn value(p: DVec2, seed: u64) -> f64 {
    let (i, j) = (p.x.floor(), p.y.floor());
    let (fx, fy) = (p.x - i, p.y - j);
    let s = |t: f64| t * t * (3.0 - 2.0 * t);
    let h = |a: f64, b: f64| {
        hash::unit_f32(hash::hash_cell2(seed, a as i32, b as i32)) as f64 * 2.0 - 1.0
    };
    let a = h(i, j) + (h(i + 1.0, j) - h(i, j)) * s(fx);
    let b = h(i, j + 1.0) + (h(i + 1.0, j + 1.0) - h(i, j + 1.0)) * s(fx);
    a + (b - a) * s(fy)
}

/// What the ground offers a plant at one place.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Site {
    /// The slope, degrees.
    pub slope: f64,
    /// The sun's heat by aspect, 0 (a steep north face) to 1 (a steep south face), 0.5 flat.
    pub heat: f64,
    /// The soil's depth, 0 (bare rock) to 1 (deep).
    pub soil: f64,
    /// The moisture from the flow, 0 (dry) to 1 (wet).
    pub moisture: f64,
}

/// The ground's fields per sample.
#[derive(Clone, Debug)]
pub struct Fields {
    /// Samples per side, as the ground's.
    pub side: usize,
    /// Their spacing, metres.
    pub cell: f64,
    /// Each sample's site.
    pub sites: Vec<Site>,
    /// Each sample's height above the mean of the ground within 40 m, metres: ridges above 0,
    /// valleys below.
    pub relief: Vec<f32>,
    /// Each sample's drained area, square metres.
    pub area: Vec<f32>,
}

/// The radius the relief is measured over, metres.
const RELIEF_RADIUS: f64 = 40.0;

impl Fields {
    /// The fields of `ground`.
    pub fn of(ground: &Ground) -> Self {
        let n = ground.side;
        let c = ground.cell;
        let at = |i: usize, j: usize| f64::from(ground.heights[j * n + i]);
        // Slopes by central differences.
        let mut grad = vec![DVec2::ZERO; n * n];
        for j in 0..n {
            for i in 0..n {
                let (i0, i1) = (i.saturating_sub(1), (i + 1).min(n - 1));
                let (j0, j1) = (j.saturating_sub(1), (j + 1).min(n - 1));
                grad[j * n + i] = DVec2::new(
                    (at(i1, j) - at(i0, j)) / ((i1 - i0) as f64 * c),
                    (at(i, j1) - at(i, j0)) / ((j1 - j0) as f64 * c),
                );
            }
        }
        // The relief: the height above the mean within a box of 2·40 m.
        let heights: Vec<f64> = ground.heights.iter().map(|&h| f64::from(h)).collect();
        let mean = box_mean(&heights, n, (RELIEF_RADIUS / c).round().max(1.0) as usize);
        let relief: Vec<f32> = (0..n * n).map(|k| (heights[k] - mean[k]) as f32).collect();
        // The drained area: each sample passes its own and what it received to its steepest
        // lower neighbour (D8), highest first; ties by index.
        let mut order: Vec<u32> = (0..(n * n) as u32).collect();
        order.sort_by(|&a, &b| {
            ground.heights[b as usize]
                .total_cmp(&ground.heights[a as usize])
                .then(a.cmp(&b))
        });
        let mut area = vec![(c * c) as f32; n * n];
        for &k in &order {
            let (i, j) = ((k as usize) % n, (k as usize) / n);
            let h = at(i, j);
            let mut best = (0.0f64, usize::MAX);
            for (di, dj) in [
                (-1i64, -1i64),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (x, y) = (i as i64 + di, j as i64 + dj);
                if x < 0 || y < 0 || x >= n as i64 || y >= n as i64 {
                    continue;
                }
                let dist = c * if di != 0 && dj != 0 {
                    std::f64::consts::SQRT_2
                } else {
                    1.0
                };
                let drop = (h - at(x as usize, y as usize)) / dist;
                if drop > best.0 {
                    best = (drop, y as usize * n + x as usize);
                }
            }
            if best.1 != usize::MAX {
                area[best.1] += area[k as usize];
            }
        }
        // The topographic wetness index, ln(a / tan β), a the area drained per metre of contour,
        // spread over 6 m: a stream's banks share its water.
        let twi: Vec<f64> = (0..n * n)
            .map(|k| dmath::ln(f64::from(area[k]) / c / grad[k].length().max(0.01)))
            .collect();
        let twi = box_mean(&twi, n, (6.0 / c).round().max(1.0) as usize);
        let tan25 = dmath::tan(25f64.to_radians());
        let sites = (0..n * n)
            .map(|k| {
                let g = grad[k];
                let steep = g.length();
                let slope = dmath::atan(steep).to_degrees();
                // South is +y: the ground faces south when it falls towards +y.
                let southness = if steep > 1e-9 { -g.y / steep } else { 0.0 };
                let heat = (0.5 + 0.5 * southness * (steep / tan25).min(1.0)).clamp(0.0, 1.0);
                // Wetter where the water gathers, in the hollows, and on a shaded slope that
                // keeps more of its water than a sunny one (an estimate of Forge's).
                let hollow = (-f64::from(relief[k]) / 25.0).clamp(0.0, 0.3);
                let moisture =
                    ((twi[k] - 4.5) / 6.0 + hollow + 0.35 * (0.5 - heat)).clamp(0.0, 1.0);
                // Soil: thin on the ridges, the steep and the sun-baked ground, deep in the
                // hollows and where the water gathers (an estimate of Forge's).
                let soil = (0.55 - f64::from(relief[k]) / 14.0 - (slope - 12.0) / 50.0
                    + 0.35 * (moisture - 0.3)
                    - 0.15 * (heat - 0.5))
                    .clamp(0.0, 1.0);
                Site {
                    slope,
                    heat,
                    soil,
                    moisture,
                }
            })
            .collect();
        Self {
            side: n,
            cell: c,
            sites,
            relief,
            area,
        }
    }

    /// The relief at `p` (the nearest sample), metres.
    pub fn relief_at(&self, p: DVec2) -> f64 {
        let n = self.side as i64 - 1;
        let i = ((p.x / self.cell).round() as i64).clamp(0, n);
        let j = ((p.y / self.cell).round() as i64).clamp(0, n);
        f64::from(self.relief[(j * (n + 1) + i) as usize])
    }

    /// The site at `p` (the nearest sample).
    pub fn site(&self, p: DVec2) -> Site {
        let n = self.side as i64 - 1;
        let i = ((p.x / self.cell).round() as i64).clamp(0, n);
        let j = ((p.y / self.cell).round() as i64).clamp(0, n);
        self.sites[(j * (n + 1) + i) as usize]
    }
}

/// The mean of `values` (an `n × n` grid) over the box of `r` samples round each, through a
/// summed-area table; the box is cut at the grid's edges.
fn box_mean(values: &[f64], n: usize, r: usize) -> Vec<f64> {
    let w = n + 1;
    let mut sat = vec![0.0f64; w * w];
    for j in 0..n {
        for i in 0..n {
            sat[(j + 1) * w + i + 1] =
                values[j * n + i] + sat[j * w + i + 1] + sat[(j + 1) * w + i] - sat[j * w + i];
        }
    }
    let mut out = vec![0.0; n * n];
    for j in 0..n {
        for i in 0..n {
            let (i0, i1) = (i.saturating_sub(r), (i + r + 1).min(n));
            let (j0, j1) = (j.saturating_sub(r), (j + r + 1).min(n));
            let sum = sat[j1 * w + i1] - sat[j0 * w + i1] - sat[j1 * w + i0] + sat[j0 * w + i0];
            out[j * n + i] = sum / ((i1 - i0) * (j1 - j0)) as f64;
        }
    }
    out
}

/// A plant's layer: the canopy is placed first and shades the understorey.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Layer {
    /// Trees.
    Canopy,
    /// Shrubs.
    Understorey,
}

/// Where a species grows wild, as bands over the site: full suitability inside each band,
/// falling to none over a soft edge outside it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Habitat {
    /// Its layer.
    pub layer: Layer,
    /// Whether it grows wild; planted species (the olive, the cypress) only by rows and groves.
    #[serde(default = "yes")]
    pub wild: bool,
    /// The soil depths it does best on.
    pub soil: [f64; 2],
    /// The moistures it does best at.
    pub moisture: [f64; 2],
    /// The heats it does best at.
    pub heat: [f64; 2],
    /// The slopes it does best on, degrees.
    pub slope: [f64; 2],
    /// The least light it stands under the canopy, 0 to 1.
    pub light: f64,
    /// How close it stands to its own layer, in summed crown radii.
    pub spacing: f64,
    /// Its weight where it does best.
    pub abundance: f64,
    /// How much light its crown stops, 0 to 1.
    pub opacity: f64,
}

fn yes() -> bool {
    true
}

/// 1 inside `[lo, hi]`, falling linearly to 0 at `soft` outside.
fn band(v: f64, r: [f64; 2], soft: f64) -> f64 {
    if v < r[0] {
        (1.0 - (r[0] - v) / soft).max(0.0)
    } else if v > r[1] {
        (1.0 - (v - r[1]) / soft).max(0.0)
    } else {
        1.0
    }
}

impl Habitat {
    /// How well a site suits it, 0 to 1.
    pub fn suitability(&self, s: &Site) -> f64 {
        band(s.soil, self.soil, 0.2)
            * band(s.moisture, self.moisture, 0.2)
            * band(s.heat, self.heat, 0.2)
            * band(s.slope, self.slope, 10.0)
    }
}

/// A placed plant.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plant {
    /// Its trunk's place on the plan, metres.
    pub pos: DVec2,
    /// Its species' index in the species list.
    pub species: u16,
    /// Its age, years.
    pub age: f32,
    /// Its site's quality for it, 0 to 1 (the grower's `Conditions::site`).
    pub site: f32,
    /// Its height, metres.
    pub height: f32,
    /// Its crown's radius, metres.
    pub crown: f32,
    /// Its seed, for the grower.
    pub seed: u64,
    /// Whether it was planted (a row or a grove) rather than grew wild.
    pub planted: bool,
}

/// A rule that plants or clears: the city's settings and the countryside's planted trees.
#[derive(Clone, Debug, PartialEq)]
pub enum Rule {
    /// A row along a line, one every `spacing` metres from half a spacing in, `offset` metres to
    /// the line's left (a street's trees, a cypress alley).
    Row {
        /// The species' index.
        species: u16,
        /// The line, metres on the plan.
        line: Vec<DVec2>,
        /// Metres between two trees.
        spacing: f64,
        /// Metres to the line's left (negative to its right).
        offset: f64,
        /// The trees' age, years.
        age: f64,
    },
    /// A grove on a jittered grid inside a polygon, each cell kept with probability `fill` (a
    /// park, a courtyard, an olive grove).
    Grove {
        /// The species' index.
        species: u16,
        /// The polygon, metres on the plan.
        polygon: Vec<DVec2>,
        /// The grid's step, metres.
        spacing: f64,
        /// The share of cells planted.
        fill: f64,
        /// The trees' ages, years.
        age: [f64; 2],
    },
    /// Nothing grows within `half_width` metres of the line (a track, a stream).
    Clear {
        /// The line, metres on the plan.
        line: Vec<DVec2>,
        /// Metres each side kept clear.
        half_width: f64,
    },
}

/// The canopy's candidate grid, metres: one dart per cell.
const CANOPY_STEP: f64 = 2.5;
/// The understorey's candidate grid, metres.
const UNDERSTOREY_STEP: f64 = 0.75;
/// The share of the canopy's darts kept where its best species does best.
const CANOPY_DENSITY: f64 = 0.6;
/// The share of the understorey's darts kept where its best species does best.
const UNDERSTOREY_DENSITY: f64 = 0.55;
/// A plant's clearance from a trunk of the other layer, metres.
const TRUNK_CLEAR: f64 = 0.4;

/// A species' height and crown radius for placement at `age` on a site of quality `site`, as
/// the grower makes them (the middle of its published ranges).
pub fn plant_size(sp: &Species, age: f64, site: f64) -> (f64, f64) {
    let share = growth_share(age, sp.maturity);
    let quality = 0.35 + 0.65 * site.clamp(0.0, 1.0);
    let h = (sp.height[0] + sp.height[1]) * 0.5 * quality * share;
    let r = (sp.crown_width[0] + sp.crown_width[1]) * 0.25 * quality * dmath::powf(share, 0.9);
    (h, r)
}

/// Plants binned in square cells, for neighbour queries.
struct PlantGrid {
    cell: f64,
    side: i64,
    cells: Vec<Vec<u32>>,
}

impl PlantGrid {
    fn new(size: f64, cell: f64) -> Self {
        let side = (size / cell).ceil() as i64 + 1;
        Self {
            cell,
            side,
            cells: vec![Vec::new(); (side * side) as usize],
        }
    }

    fn coord(&self, v: f64) -> i64 {
        ((v / self.cell).floor() as i64).clamp(0, self.side - 1)
    }

    fn insert(&mut self, p: DVec2, k: u32) {
        let (i, j) = (self.coord(p.x), self.coord(p.y));
        self.cells[(j * self.side + i) as usize].push(k);
    }

    /// Calls `f` with every plant in the cells within `radius` of `p`.
    fn near(&self, p: DVec2, radius: f64, mut f: impl FnMut(u32)) {
        for y in self.coord(p.y - radius)..=self.coord(p.y + radius) {
            for x in self.coord(p.x - radius)..=self.coord(p.x + radius) {
                for &k in &self.cells[(y * self.side + x) as usize] {
                    f(k);
                }
            }
        }
    }
}

/// The light at `p` under the canopy in `grid`, 0 to 1: each crown's shade a disc of 1.1 times
/// its radius, cast north of its trunk by 0.4 of its height (the sun to the south), stopping its
/// species' opacity of the light (an estimate of Forge's). `reach` bounds a shade's distance
/// from its trunk.
fn light(p: DVec2, plants: &[Plant], grid: &PlantGrid, reach: f64, species: &[Species]) -> f64 {
    let mut light = 1.0;
    grid.near(p, reach, |k| {
        let t = &plants[k as usize];
        let Some(h) = species[t.species as usize].habitat.as_ref() else {
            return;
        };
        if h.layer != Layer::Canopy {
            return;
        }
        let centre = t.pos - DVec2::new(0.0, 0.4 * f64::from(t.height));
        let r = f64::from(t.crown) * 1.1;
        let d = p.distance(centre);
        if d < r {
            let cover = ((r - d) / (0.3 * r)).min(1.0);
            light *= 1.0 - h.opacity * cover;
        }
    });
    light
}

/// The distance from `p` to a polyline.
pub fn to_line(p: DVec2, line: &[DVec2]) -> f64 {
    line.windows(2)
        .map(|w| {
            let ab = w[1] - w[0];
            let t = ((p - w[0]).dot(ab) / ab.length_squared().max(1e-12)).clamp(0.0, 1.0);
            p.distance(w[0] + ab * t)
        })
        .fold(f64::MAX, f64::min)
}

/// Whether `p` is inside a polygon (even-odd).
pub fn inside(p: DVec2, polygon: &[DVec2]) -> bool {
    let n = polygon.len();
    let mut c = false;
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        if (a.y > p.y) != (b.y > p.y) && p.x < a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x) {
            c = !c;
        }
    }
    c
}

/// A dart: a candidate place, its priority and its random bits.
struct Dart {
    priority: u64,
    pos: DVec2,
    bits: u64,
}

/// One layer's darts over the ground: one per cell of `step`, jittered, in priority order. Each
/// dart's place and bits hash from its cell, so a cell's dart is the same whatever the ground's
/// size.
fn darts(size: f64, step: f64, seed: u64) -> Vec<Dart> {
    let n = (size / step).floor() as i32;
    let mut out = Vec::with_capacity((n.max(0) * n.max(0)) as usize);
    for j in 0..n {
        for i in 0..n {
            let mut rng = SplitMix64::new(hash::hash_cell2(seed, i, j));
            let pos = DVec2::new(
                (f64::from(i) + rng.next_f64()) * step,
                (f64::from(j) + rng.next_f64()) * step,
            );
            out.push(Dart {
                priority: rng.next_u64(),
                pos,
                bits: rng.next_u64(),
            });
        }
    }
    out.sort_by_key(|d| d.priority);
    out
}

/// What the placement made.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Placement {
    /// The plants: the planted first, then the wild canopy, then the understorey.
    pub plants: Vec<Plant>,
    /// Darts thrown per wild layer (canopy, understorey).
    pub darts: [usize; 2],
}

/// Places the plants of `species` on `ground` by `rules` and the species' habitats, from
/// `seed`. `keep_out` says where nothing grows (water, roads, buildings: the caller's map).
pub fn place(
    ground: &Ground,
    fields: &Fields,
    species: &[Species],
    rules: &[Rule],
    seed: u64,
    keep_out: &dyn Fn(DVec2) -> bool,
) -> Placement {
    let extent = ground.size();
    let mut out = Placement::default();
    // The trees (planted and wild) in cells of 8 m, the shrubs in cells of 3 m.
    let mut trees = PlantGrid::new(extent, 8.0);
    let mut shrubs = PlantGrid::new(extent, 3.0);
    let layer_of = |s: u16| {
        species[s as usize]
            .habitat
            .as_ref()
            .map_or(Layer::Canopy, |h| h.layer)
    };
    let spacing_of = |s: u16| {
        species[s as usize]
            .habitat
            .as_ref()
            .map_or(1.0, |h| h.spacing)
    };
    // How far a tree's shade reaches from its trunk, and the widest crowns.
    let widest = species
        .iter()
        .map(|s| s.crown_width[1] * 0.5)
        .fold(0.5, f64::max);
    let tallest = species.iter().map(|s| s.height[1]).fold(1.0, f64::max);
    let shade = widest * 1.1 + 0.4 * tallest;
    let clear: Vec<(&[DVec2], f64)> = rules
        .iter()
        .filter_map(|r| match r {
            Rule::Clear { line, half_width } => Some((line.as_slice(), *half_width)),
            _ => None,
        })
        .collect();
    let blocked = |p: DVec2| {
        p.x < 0.0
            || p.y < 0.0
            || p.x > extent
            || p.y > extent
            || keep_out(p)
            || clear.iter().any(|(l, w)| to_line(p, l) < *w)
    };
    let mut rng = forge_core::Seed::new(seed).derive_str("planted").rng();
    // 1. The planted rows and groves.
    let mut planted: Vec<(DVec2, u16, f64)> = Vec::new();
    for rule in rules {
        match rule {
            Rule::Row {
                species: s,
                line,
                spacing,
                offset,
                age,
            } => {
                let mut along = spacing * 0.5;
                let mut walked = 0.0;
                for w in line.windows(2) {
                    let len = w[0].distance(w[1]);
                    let dir = (w[1] - w[0]) / len.max(1e-9);
                    let left = DVec2::new(dir.y, -dir.x);
                    while along < walked + len {
                        planted.push((w[0] + dir * (along - walked) + left * *offset, *s, *age));
                        along += spacing;
                    }
                    walked += len;
                }
            }
            Rule::Grove {
                species: s,
                polygon,
                spacing,
                fill,
                age,
            } => {
                let (lo, hi) = polygon.iter().fold(
                    (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
                    |(lo, hi), &p| (lo.min(p), hi.max(p)),
                );
                let mut y = lo.y + spacing * 0.5;
                while y < hi.y {
                    let mut x = lo.x + spacing * 0.5;
                    while x < hi.x {
                        let p = DVec2::new(
                            x + rng.range_f64(-0.12, 0.12) * spacing,
                            y + rng.range_f64(-0.12, 0.12) * spacing,
                        );
                        let keep = rng.next_f64() < *fill;
                        let a = rng.range_f64(age[0], age[1]);
                        if keep && inside(p, polygon) {
                            planted.push((p, *s, a));
                        }
                        x += spacing;
                    }
                    y += spacing;
                }
            }
            Rule::Clear { .. } => {}
        }
    }
    for (pos, s, age) in planted {
        if blocked(pos) {
            continue;
        }
        let (h, r) = plant_size(&species[s as usize], age, 1.0);
        let k = out.plants.len() as u32;
        out.plants.push(Plant {
            pos,
            species: s,
            age: age as f32,
            site: 1.0,
            height: h as f32,
            crown: r as f32,
            seed: rng.next_u64(),
            planted: true,
        });
        if layer_of(s) == Layer::Canopy {
            trees.insert(pos, k);
        } else {
            shrubs.insert(pos, k);
        }
    }
    // 2. The wild canopy, then the understorey in the light it leaves.
    for (layer, step, density, tag) in [
        (Layer::Canopy, CANOPY_STEP, CANOPY_DENSITY, "canopy"),
        (
            Layer::Understorey,
            UNDERSTOREY_STEP,
            UNDERSTOREY_DENSITY,
            "understorey",
        ),
    ] {
        let members: Vec<(u16, &Habitat)> = species
            .iter()
            .enumerate()
            .filter_map(|(k, s)| s.habitat.as_ref().map(|h| (k as u16, h)))
            .filter(|(_, h)| h.layer == layer && h.wild)
            .collect();
        if members.is_empty() {
            continue;
        }
        // The widest crown of this layer: how far a neighbour may count.
        let reach = members
            .iter()
            .map(|(s, _)| species[*s as usize].crown_width[1] * 0.5)
            .fold(0.1, f64::max);
        let darts = darts(
            extent,
            step,
            forge_core::Seed::new(seed).derive_str(tag).value(),
        );
        out.darts[usize::from(layer == Layer::Understorey)] = darts.len();
        let mut weights = vec![0.0; members.len()];
        for d in &darts {
            if blocked(d.pos) {
                continue;
            }
            let mut bits = SplitMix64::new(d.bits);
            let site = fields.site(d.pos);
            let mut total = 0.0;
            for (w, (_, h)) in weights.iter_mut().zip(&members) {
                *w = h.suitability(&site) * h.abundance;
                total += *w;
            }
            if total <= 0.0 || bits.next_f64() >= total.min(1.0) * density {
                continue;
            }
            // The species by its weight here.
            let mut pick = bits.next_f64() * total;
            let mut m = 0;
            while m + 1 < members.len() && pick >= weights[m] {
                pick -= weights[m];
                m += 1;
            }
            let (s, h) = members[m];
            let sp = &species[s as usize];
            let quality = h.suitability(&site);
            let age = sp.maturity * bits.range_f64(0.3, 1.4);
            let (height, crown) = plant_size(sp, age, quality);
            // Spacing within the layer: the two crowns summed times the closer-standing
            // species' spacing; a shrub clear of the trees' trunks.
            let mut ok = true;
            let (own, other) = match layer {
                Layer::Canopy => (&trees, &shrubs),
                Layer::Understorey => (&shrubs, &trees),
            };
            own.near(d.pos, crown + reach, |k| {
                let o = &out.plants[k as usize];
                let need = (crown + f64::from(o.crown)) * h.spacing.min(spacing_of(o.species));
                if ok && o.pos.distance(d.pos) < need {
                    ok = false;
                }
            });
            if !ok {
                continue;
            }
            other.near(d.pos, crown + reach, |k| {
                let o = &out.plants[k as usize];
                let shrub = if layer == Layer::Understorey {
                    crown
                } else {
                    f64::from(o.crown)
                };
                if ok && o.pos.distance(d.pos) < shrub * 0.5 + TRUNK_CLEAR {
                    ok = false;
                }
            });
            if !ok || light(d.pos, &out.plants, &trees, shade, species) < h.light {
                continue;
            }
            let k = out.plants.len() as u32;
            out.plants.push(Plant {
                pos: d.pos,
                species: s,
                age: age as f32,
                site: quality as f32,
                height: height as f32,
                crown: crown as f32,
                seed: bits.next_u64(),
                planted: false,
            });
            match layer {
                Layer::Canopy => trees.insert(d.pos, k),
                Layer::Understorey => shrubs.insert(d.pos, k),
            }
        }
    }
    out
}

/// The example hillside's planted rules and its lines.
#[derive(Clone, Debug, PartialEq)]
pub struct Example {
    /// The rules: the stream and the track kept clear, a cypress alley along the track, an olive
    /// grove on the lower sunny slope.
    pub rules: Vec<Rule>,
    /// The stream.
    pub stream: Vec<DVec2>,
    /// The track.
    pub track: Vec<DVec2>,
}

/// The example hillside's rules for `species` (the village's) on a ground of `size` metres.
pub fn example_rules(species: &[Species], size: f64) -> Example {
    let index = |id: &str| {
        species
            .iter()
            .position(|s| s.id == id)
            .expect("a village species") as u16
    };
    let line = |offset: f64, x0: f64, x1: f64| -> Vec<DVec2> {
        let n = 48;
        (0..=n)
            .map(|k| {
                let x = x0 + (x1 - x0) * f64::from(k) / f64::from(n);
                DVec2::new(x, example_valley(x, size) + offset)
            })
            .collect()
    };
    let stream = line(0.0, 0.0, size);
    let track = line(-22.0, 0.0, size);
    let v = |x: f64| example_valley(x, size);
    let (x0, x1) = (size * 0.5, size * 0.85);
    let grove = vec![
        DVec2::new(x0, v(x0) - 38.0),
        DVec2::new(x1, v(x1) - 38.0),
        DVec2::new(x1, v(x1) - 105.0),
        DVec2::new(x0, v(x0) - 105.0),
    ];
    let rules = vec![
        Rule::Clear {
            line: stream.clone(),
            half_width: 3.0,
        },
        Rule::Clear {
            line: track.clone(),
            half_width: 2.5,
        },
        Rule::Row {
            species: index("italian-cypress"),
            line: line(-22.0, size * 0.08, size * 0.42),
            spacing: 7.0,
            offset: 4.0,
            age: 35.0,
        },
        Rule::Grove {
            species: index("olive"),
            polygon: grove,
            spacing: 8.0,
            fill: 0.92,
            age: [30.0, 90.0],
        },
    ];
    Example {
        rules,
        stream,
        track,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::species::mediterranean;

    fn example(seed: u64) -> (Ground, Fields, Vec<Species>, Example, Placement) {
        let size = 400.0;
        let ground = Ground::example(size, 2.0, 7);
        let fields = Fields::of(&ground);
        let species = mediterranean();
        let ex = example_rules(&species, size);
        let placed = place(&ground, &fields, &species, &ex.rules, seed, &|_| false);
        (ground, fields, species, ex, placed)
    }

    fn mean(v: impl Iterator<Item = f64>) -> f64 {
        let (s, n) = v.fold((0.0, 0usize), |(s, n), x| (s + x, n + 1));
        s / n.max(1) as f64
    }

    #[test]
    fn the_fields_read_the_ground() {
        let size = 400.0;
        let ground = Ground::example(size, 2.0, 7);
        let fields = Fields::of(&ground);
        let x = size * 0.3;
        let v = example_valley(x, size);
        let north = fields.site(DVec2::new(x, v - 90.0));
        let south = fields.site(DVec2::new(x, v + 90.0));
        // The valley's floor: the lowest ground within 30 m of the valley's line.
        let floor = (-15..=15)
            .map(|k| DVec2::new(x, v + f64::from(k) * 2.0))
            .min_by(|a, b| ground.height(*a).total_cmp(&ground.height(*b)))
            .unwrap();
        let valley = fields.site(floor);
        let ridge = fields.site(DVec2::new(x, 4.0));
        assert!(north.heat > 0.6, "the north side faces south: {north:?}");
        assert!(south.heat < 0.4, "the south side faces north: {south:?}");
        assert!(
            valley.moisture > ridge.moisture + 0.3,
            "{valley:?} {ridge:?}"
        );
        assert!(valley.soil > ridge.soil + 0.3, "{valley:?} {ridge:?}");
    }

    #[test]
    fn placement_is_the_same_every_time() {
        let (.., a) = example(3);
        let (.., b) = example(3);
        assert_eq!(a, b);
        let (.., c) = example(4);
        assert_ne!(a.plants, c.plants);
        assert!(a.plants.len() > 1000, "{} plants", a.plants.len());
    }

    #[test]
    fn plants_keep_their_spacing_and_the_clear_lines() {
        let (_, _, species, ex, placed) = example(3);
        let layer = |p: &Plant| {
            species[p.species as usize]
                .habitat
                .as_ref()
                .map_or(Layer::Canopy, |h| h.layer)
        };
        let spacing = |p: &Plant| {
            species[p.species as usize]
                .habitat
                .as_ref()
                .map_or(1.0, |h| h.spacing)
        };
        let plants = &placed.plants;
        for (i, a) in plants.iter().enumerate() {
            for b in &plants[i + 1..] {
                if a.planted && b.planted {
                    continue;
                }
                let d = a.pos.distance(b.pos);
                if d > 30.0 {
                    continue;
                }
                let need = if layer(a) == layer(b) {
                    f64::from(a.crown + b.crown) * spacing(a).min(spacing(b))
                } else {
                    let shrub = if layer(a) == Layer::Understorey { a } else { b };
                    f64::from(shrub.crown) * 0.5 + TRUNK_CLEAR
                };
                // The planted came first and the wild kept from them.
                assert!(
                    d >= need - 1e-6,
                    "{a:?} and {b:?} {d:.2} m apart, {need:.2} needed"
                );
            }
        }
        for p in plants {
            assert!(to_line(p.pos, &ex.stream) >= 3.0 && to_line(p.pos, &ex.track) >= 2.5);
        }
    }

    #[test]
    fn the_garrigue_takes_the_sunny_slope_and_the_oaks_the_valley() {
        let (ground, fields, species, _, placed) = example(3);
        let size = ground.size();
        let id = |p: &Plant| species[p.species as usize].id.as_str();
        let wild = |name: &'static str| {
            placed
                .plants
                .iter()
                .filter(move |p| !p.planted && id(p) == name)
        };
        let garrigue = ["rosemary", "cistus"];
        // The garrigue's shrubs on hot ground, the holm oaks on cool ground.
        let heat = |name| mean(wild(name).map(|p| fields.site(p.pos).heat));
        for g in garrigue {
            assert!(heat(g) > 0.6, "{g} at heat {:.2}", heat(g));
        }
        assert!(
            heat("holm-oak") < 0.5,
            "holm oaks at heat {:.2}",
            heat("holm-oak")
        );
        // The oaks on deeper soil than the pines, the pines higher in the relief.
        let soil = |name| mean(wild(name).map(|p| fields.site(p.pos).soil));
        let relief = |name| mean(wild(name).map(|p| fields.relief_at(p.pos)));
        assert!(soil("holm-oak") > soil("aleppo-pine") + 0.2);
        assert!(relief("aleppo-pine") > relief("holm-oak") + 1.0);
        // Most of the valley's floor trees are holm oaks.
        let near_valley = |p: &&Plant| (p.pos.y - example_valley(p.pos.x, size)).abs() < 25.0;
        let oaks = wild("holm-oak").filter(near_valley).count();
        let pines = wild("aleppo-pine").filter(near_valley).count();
        assert!(
            oaks > 3 * pines,
            "{oaks} oaks and {pines} pines by the stream"
        );
    }

    #[test]
    fn shrubs_stand_in_the_light_they_need() {
        let (ground, _, species, _, placed) = example(3);
        let mut grid = PlantGrid::new(ground.size(), 8.0);
        let canopy: Vec<Plant> = placed
            .plants
            .iter()
            .filter(|p| {
                species[p.species as usize]
                    .habitat
                    .as_ref()
                    .is_none_or(|h| h.layer == Layer::Canopy)
            })
            .copied()
            .collect();
        for (k, p) in canopy.iter().enumerate() {
            grid.insert(p.pos, k as u32);
        }
        for p in placed.plants.iter().filter(|p| !p.planted) {
            let h = species[p.species as usize].habitat.as_ref().unwrap();
            if h.layer != Layer::Understorey {
                continue;
            }
            let l = light(p.pos, &canopy, &grid, 40.0, &species);
            assert!(
                l >= h.light - 1e-9,
                "a {} in {l:.2} light",
                species[p.species as usize].id
            );
        }
    }

    #[test]
    fn rows_and_groves_plant_where_told() {
        let (_, _, species, ex, placed) = example(3);
        let cypress = species
            .iter()
            .position(|s| s.id == "italian-cypress")
            .unwrap() as u16;
        let olive = species.iter().position(|s| s.id == "olive").unwrap() as u16;
        let Rule::Row { line, spacing, .. } = &ex.rules[2] else {
            panic!("the third rule is the alley")
        };
        let length: f64 = line.windows(2).map(|w| w[0].distance(w[1])).sum();
        let row = placed
            .plants
            .iter()
            .filter(|p| p.species == cypress)
            .count();
        assert!(
            row as f64 >= length / spacing - 2.0,
            "{row} cypresses over {length:.0} m"
        );
        assert!(
            placed
                .plants
                .iter()
                .filter(|p| p.species == cypress)
                .all(|p| p.planted)
        );
        let Rule::Grove { polygon, .. } = &ex.rules[3] else {
            panic!("the fourth rule is the grove")
        };
        let olives: Vec<&Plant> = placed
            .plants
            .iter()
            .filter(|p| p.species == olive)
            .collect();
        assert!(olives.len() > 30, "{} olives", olives.len());
        assert!(olives.iter().all(|p| inside(p.pos, polygon) && p.planted));
    }
}
