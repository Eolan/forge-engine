//! Growing a plant: space colonisation (Runions, Lane and Prusinkiewicz 2007) for the free
//! space each bud perceives, and the self-organising tree of Pałubicki et al. 2009 for how the
//! vigour is shared: the light gathered from the buds down to the base, then the vigour handed
//! from the base up to the buds by Borchert and Honda's rule (1984), each bud growing a shoot of
//! as many metamers as its vigour allows. Branches that stay in the dark are shed; the radii
//! follow the pipe model (Shinozaki et al. 1964).
//!
//! The crown's envelope grows with the plant's age along a Chapman–Richards curve, from the
//! species' young crown to its grown one, so an older plant has filled a larger space with more
//! branches. Pruning (a lifted crown, a topped crown, an open centre) and a prevailing wind
//! (the crown leans and the shoots bend downwind) shape the space it may grow into.
//!
//! Everything is deterministic from the seed (D-016): one random stream drawn in a fixed order,
//! `forge_core::dmath` for the transcendental functions.

use forge_core::{Seed, SplitMix64, dmath};
use glam::{DVec2, DVec3};

use crate::graph::{LeafCluster, NO_PARENT, Node, TreeGraph};
use crate::species::{Crown, Habit, Species, growth_share};

/// What was cut or trained.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pruning {
    /// The crown lifted: no branch starts below this height, metres (a street tree's, an
    /// olive's clear trunk).
    pub lift: f64,
    /// The crown topped: nothing grows above this height, metres (an olive kept low for the
    /// picking).
    pub top: f64,
    /// An open centre: the leader cut above the scaffold branches and the crown kept clear
    /// within this radius of the trunk, metres (an olive's vase); 0 for none.
    pub open_centre: f64,
}

impl Default for Pruning {
    fn default() -> Self {
        Self {
            lift: 0.0,
            top: f64::INFINITY,
            open_centre: 0.0,
        }
    }
}

/// What a plant grows from and in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditions {
    /// Its age, years.
    pub age: f64,
    /// Its seed: the individual.
    pub seed: u64,
    /// The site's quality, 0 to 1: 1 grows a plant in the published size range, poorer sites
    /// smaller ones (a holm oak on bare limestone stays a shrub).
    pub site: f64,
    /// What was cut.
    pub pruning: Pruning,
    /// The prevailing wind's direction on the ground (X, Z) with its strength, 0 to 1, as the
    /// length: the crown leans that way.
    pub wind: DVec2,
}

impl Conditions {
    /// A plant of `age` years from `seed` on a good site, unpruned, in still air.
    pub fn new(age: f64, seed: u64) -> Self {
        Self {
            age,
            seed,
            site: 1.0,
            pruning: Pruning::default(),
            wind: DVec2::ZERO,
        }
    }
}

/// The ages each species is grown at for a forest (#296): a quarter, half and all of its
/// maturity.
pub const AGES: u8 = 3;
/// The individuals grown at each age, each from its own seed, in its own breeze and on its own
/// site: one an age made every holm oak of a wood's edge the same tree (the owner, 2026-10-16:
/// "trees are too much identical, they need variations").
pub const SEEDS: u8 = 3;
/// How many individuals of each species a forest grows: individual `k` is age `k % AGES`, seed
/// `k / AGES` ([`individual`]).
pub const INDIVIDUALS: u8 = AGES * SEEDS;

/// The growing conditions of individual `k` of species `s`, `index` its place in its list (#296,
/// city-plan's forest's): a quarter, half and all of its maturity, the first seed on a good site in
/// still air, the others leaning in a breeze each its own way on a poorer site; the crown lifted on
/// the species' `planted_lift` share of the height it has grown to by then, so a tree scaled to a
/// street's height keeps its clear trunk in proportion (lifted in metres, a 9 m plane's crown hung
/// at the eyes). A wild plant's: set `pruning.lift` to 0.
pub fn individual(s: &Species, index: u16, k: u8) -> Conditions {
    let variant = usize::from(k / AGES) % usize::from(SEEDS);
    let k = usize::from(k);
    let age = s.maturity * [0.25, 0.5, 1.0][k % usize::from(AGES)];
    let tall = (s.height[0] + s.height[1]) * 0.5 * growth_share(age, s.maturity);
    Conditions {
        age,
        seed: 1000 + u64::from(index) * 10 + k as u64,
        site: [1.0, 0.8, 0.9][variant],
        pruning: Pruning {
            lift: tall * s.planted_lift,
            ..Default::default()
        },
        wind: [DVec2::ZERO, DVec2::new(0.3, 0.0), DVec2::new(-0.1, -0.25)][variant],
    }
}

/// Cycles of growth at most, whatever the age (one a year below it): each grows the envelope
/// by an equal step of size.
const MAX_CYCLES: u32 = 28;
/// Cycles of growth at least: a young plant's growth is split finer.
const MIN_CYCLES: u32 = 14;
/// Cycles grown at the final envelope after the last year's, to fill it.
const SETTLE_CYCLES: u32 = 2;
/// Segments closer than this many steps along the graph meet at a fork: not checked for
/// passing through each other.
pub const FORK_HOPS: u32 = 4;
/// How far the crown leans, metres per metre of height, in a wind of strength 1.
const LEAN: f64 = 0.35;

/// The most lobes a crown has.
const MAX_LOBES: usize = 16;
/// The sine below its widest over which a shrub's envelope goes from its lobes to the smooth
/// profile its stems rise through ([`Lobes::reach`]).
const SHRUB_BLEND: f64 = 0.3;
/// The golden angle, radians: the lobes turn by it one after another round the crown.
const GOLDEN: f64 = 2.399_963_229_728_653;

/// An individual crown's lobes, the sub-crowns its main limbs carry. With the species' smooth
/// profile alone the markers filled one ellipsoid evenly, and every crown grew into the same
/// round ball, a holm oak's as a plane's (the owner, 2026-10-24: "as if the generation stopped
/// too early"). Each lobe is a ball inside the envelope touching it, about half its size; the
/// crown is their union over a core, so its outline is a cluster of domes, deepest where two
/// lobes leave a gap. The first lobe stands over the trunk, so the crown keeps its height; the
/// others go round it on a golden-angle spiral from the top to a little below the middle.
#[derive(Clone, Copy, Debug)]
struct Lobes {
    /// Each lobe's direction from the crown's middle and its radius, a share of the envelope's
    /// reach that way.
    lobes: [(DVec3, f64); MAX_LOBES],
    count: usize,
    /// The core's reach: the least of the envelope's anywhere.
    core: f64,
    /// A shrub's: below its widest it keeps the smooth profile, where its stems rise from the
    /// ground ([`SHRUB_BLEND`]).
    shrub: bool,
}

impl Lobes {
    /// The plant's lobes, from a stream of their own: the markers and the growth draw what
    /// they drew before.
    fn new(sp: &Species, cond: &Conditions) -> Self {
        let mut rng = Seed::new(cond.seed)
            .derive_str(&sp.id)
            .derive_str("lobes")
            .rng();
        let [lo, hi] = sp.lobes;
        let count = (u64::from(lo) + rng.below(u64::from(hi.max(lo) - lo + 1)))
            .min(MAX_LOBES as u64) as usize;
        let phase = rng.range_f64(0.0, std::f64::consts::TAU);
        let mut lobes = [(DVec3::Y, 1.0); MAX_LOBES];
        for (k, lobe) in lobes.iter_mut().enumerate().take(count) {
            // The height of the lobe's direction, its sine: the top's over the trunk, then
            // down the crown to a little under its middle.
            let up = if k == 0 {
                rng.range_f64(0.9, 1.0)
            } else {
                let down = (k as f64 - 0.5) / (count as f64 - 1.0).max(1.0);
                (0.8 - 1.1 * down + rng.range_f64(-0.12, 0.12)).clamp(-0.4, 0.95)
            };
            let flat = (1.0 - up * up).sqrt();
            let turn = phase + GOLDEN * k as f64 + rng.range_f64(-0.3, 0.3);
            let (s, c) = dmath::sin_cos(turn);
            let size = sp.lobe_size * rng.range_f64(0.8, 1.2);
            *lobe = (DVec3::new(c * flat, up, s * flat), size.clamp(0.1, 1.0));
        }
        Self {
            lobes,
            count,
            // The limbs' sub-crowns come with age: a young plant's crown is smoother, at a
            // quarter of its maturity a third as deep (a young umbrella pine's, lobed as a grown
            // one's, kept half its width in a narrow clump on a stick).
            core: 1.0
                - sp.lobe_depth.clamp(0.0, 0.8) * growth_share(cond.age, sp.maturity).min(1.0),
            shrub: sp.habit == Habit::Shrub,
        }
    }

    /// The envelope's reach along the unit direction `dir` from the crown's middle, as a share
    /// of the smooth profile's: the farthest a lobe's ball reaches along that ray (a ball of
    /// radius `r` touching the envelope stands `1 - r` out), the core's at least.
    fn reach(&self, dir: DVec3) -> f64 {
        if self.count == 0 || self.core >= 1.0 {
            return 1.0;
        }
        let mut reach = self.core;
        for &(axis, r) in &self.lobes[..self.count] {
            let a = 1.0 - r;
            let along = a * dir.dot(axis);
            let off = a * a - along * along;
            if off < r * r {
                reach = reach.max(along + (r * r - off).sqrt());
            }
        }
        // A shrub's smooth below its widest, blended in over the first SHRUB_BLEND of the sine down:
        // switched there, it stood out as a shelf a quarter of its reach deep (a review).
        let full = if self.shrub {
            (-dir.y / SHRUB_BLEND).clamp(0.0, 1.0)
        } else {
            0.0
        };
        (reach + (1.0 - reach) * full).min(1.0)
    }
}

/// The crown's envelope at one age.
#[derive(Clone, Copy, Debug)]
struct Envelope {
    height: f64,
    radius: f64,
    crown: Crown,
    lean: DVec2,
    pruning: Pruning,
    lobes: Lobes,
    /// A lifted crown's lift as a share of the grown plant's height, 0 for none.
    lift_share: f64,
}

impl Envelope {
    fn axis(&self, y: f64) -> DVec2 {
        self.lean * y
    }

    /// Where the crown starts: the species' base, or a lifted crown's lift above it, as a share
    /// of the envelope's height (`lift_share`). A planted tree's crown grows its whole shape over
    /// its clear stem; cut at the lift instead, a young umbrella pine's cone, widest near the
    /// ground, kept only its narrow top half, a spire on a stick (the owner, 2026-10-24: "a bit
    /// of a strange canopy"). In metres, the young envelope stood all above the lift, and a
    /// leader with no free space round it starved at 5 m.
    fn crown_base(&self) -> f64 {
        self.crown.base.max(self.lift_share) * self.height
    }

    /// Whether `p` lies in the space the crown may fill, grown by `margin`.
    fn contains(&self, p: DVec3, margin: f64) -> bool {
        if p.y < 0.0 || p.y > self.height + margin || p.y > self.pruning.top {
            return false;
        }
        let base = self.crown_base();
        let depth = (self.height - base).max(1e-6);
        if self.pruning.open_centre > 0.0 && p.y > self.pruning.lift + 0.3 {
            let c = self.axis(p.y);
            if DVec2::new(p.x - c.x, p.z - c.y).length() < self.pruning.open_centre {
                return false;
            }
        }
        // The point taken out from the crown's middle by the lobe it stands towards
        // ([`Lobes::reach`]), then tested against the smooth profile.
        let mid = base + self.crown.widest * depth;
        let o = self.axis(mid);
        let half = 0.5 * depth;
        let across = self.radius.max(1e-6);
        let q = DVec3::new(
            (p.x - o.x) / across,
            (p.y - mid) / half,
            (p.z - o.y) / across,
        );
        let s = q.try_normalize().map_or(1.0, |dir| self.lobes.reach(dir));
        let p = DVec3::new(
            o.x + (p.x - o.x) / s,
            mid + (p.y - mid) / s,
            o.y + (p.z - o.y) / s,
        );
        let t = (p.y - base) / depth;
        let slack = margin / depth;
        if t < -slack || t > 1.0 + slack {
            return false;
        }
        let c = self.axis(p.y);
        let d = DVec2::new(p.x - c.x, p.z - c.y).length();
        d <= self.radius * self.crown.profile(t.clamp(0.0, 1.0)) + margin
    }
}

/// A dense grid of indices over a box, cubes of `cell`.
struct Grid {
    lo: DVec3,
    cell: f64,
    dims: [i32; 3],
    cells: Vec<Vec<u32>>,
}

impl Grid {
    fn new(lo: DVec3, hi: DVec3, cell: f64) -> Self {
        let dims = [0, 1, 2].map(|k| (((hi[k] - lo[k]) / cell).floor() as i32 + 1).clamp(1, 512));
        Self {
            lo,
            cell,
            dims,
            cells: vec![Vec::new(); (dims[0] * dims[1] * dims[2]) as usize],
        }
    }

    fn coords(&self, p: DVec3) -> [i32; 3] {
        let c = (p - self.lo) / self.cell;
        [0, 1, 2].map(|k| (c[k].floor() as i32).clamp(0, self.dims[k] - 1))
    }

    fn key(&self, c: [i32; 3]) -> usize {
        ((c[2] * self.dims[1] + c[1]) * self.dims[0] + c[0]) as usize
    }

    fn insert(&mut self, p: DVec3, i: u32) {
        let k = self.key(self.coords(p));
        self.cells[k].push(i);
    }

    fn clear(&mut self) {
        for c in &mut self.cells {
            c.clear();
        }
    }

    /// Calls `f` with the indices in the cells within one cell of `p`'s.
    fn near(&self, p: DVec3, mut f: impl FnMut(u32)) {
        let c = self.coords(p);
        for z in (c[2] - 1).max(0)..=(c[2] + 1).min(self.dims[2] - 1) {
            for y in (c[1] - 1).max(0)..=(c[1] + 1).min(self.dims[1] - 1) {
                for x in (c[0] - 1).max(0)..=(c[0] + 1).min(self.dims[0] - 1) {
                    for &i in &self.cells[self.key([x, y, z])] {
                        f(i);
                    }
                }
            }
        }
    }
}

/// A node while growing.
#[derive(Clone, Copy, Debug)]
struct GrowNode {
    pos: DVec3,
    parent: u32,
    /// The child continuing this node's axis, or `NO_PARENT`.
    main: u32,
    order: u8,
    starts_branch: bool,
    /// Its place along its axis, for the phyllotaxis.
    rank: u32,
    /// Its axis' turn of phase, radians.
    phase: f64,
    alive: bool,
    /// The last cycle its subtree had light.
    lit: u32,
}

/// A bud.
#[derive(Clone, Copy, Debug)]
struct Bud {
    node: u32,
    dir: DVec3,
    /// A terminal bud continues its node's axis; a lateral one starts a branch.
    terminal: bool,
    /// The order of the shoot it grows.
    order: u8,
    alive: bool,
    starved: u32,
    /// Markers it perceived this cycle, and their summed directions.
    count: u32,
    towards: DVec3,
    light: f64,
    vigour: f64,
}

/// A unit vector perpendicular to `d`.
fn perpendicular(d: DVec3) -> DVec3 {
    let a = if d.y.abs() < 0.9 { DVec3::Y } else { DVec3::X };
    d.cross(a).normalize()
}

/// `v` turned by `angle` radians about the unit `axis` (Rodrigues).
fn rotate(v: DVec3, axis: DVec3, angle: f64) -> DVec3 {
    let (s, c) = dmath::sin_cos(angle);
    v * c + axis.cross(v) * s + axis * axis.dot(v) * (1.0 - c)
}

/// A random unit vector.
fn random_unit(rng: &mut SplitMix64) -> DVec3 {
    let z = rng.range_f64(-1.0, 1.0);
    let a = rng.range_f64(0.0, std::f64::consts::TAU);
    let r = (1.0 - z * z).max(0.0).sqrt();
    let (s, c) = dmath::sin_cos(a);
    DVec3::new(r * c, z, r * s)
}

/// Grows one plant of `species` under `conditions`.
pub fn grow(species: &Species, conditions: &Conditions) -> TreeGraph {
    // A plant whose crown fills under MIN_FILL of its envelope's width, or grows under MIN_LEAVES
    // of leaf clusters for it, is grown again from a seed derived from its own, the fullest of the
    // tries kept: a young umbrella pine on a poorer site grew 16 leaf clusters, a few tufts on a
    // stick (the owner, 2026-10-24: "a bit of a strange canopy"), where its age's others grew 40
    // to 80. A nursery sells the full ones. Of the village's 90 individuals it alone regrows.
    let grown = |k: u64| {
        let c = Conditions {
            seed: if k == 0 {
                conditions.seed
            } else {
                Seed::new(conditions.seed).derive(k).value()
            },
            ..*conditions
        };
        let grower = Grower::new(species, &c);
        let width = 2.0
            * grower
                .envelope(growth_share(c.age, species.maturity))
                .radius;
        let g = grower.run();
        // Its crown's width and its leaves, each against what the envelope asks: 1 or more both.
        let wide = f64::from(g.crown_width()) / (MIN_FILL * width).max(1e-6);
        let leafy = g.leaves.len() as f64 / (MIN_LEAVES * width * width).max(1e-6);
        (wide.min(leafy), g)
    };
    let mut best = grown(0);
    for k in 1..=REGROWTHS {
        if best.0 >= 1.0 {
            break;
        }
        let next = grown(k);
        if next.0 > best.0 {
            best = next;
        }
    }
    best.1
}

/// The least share of its envelope's width a plant's crown fills before it is grown again from
/// another seed ([`grow`]).
const MIN_FILL: f64 = 0.6;
/// The fewest leaf clusters a plant grows per square metre of its envelope's width squared
/// before it is grown again ([`grow`]): the young umbrella pine of 16 had 1.3, its age's others
/// 3 to 4, the dense species tens.
const MIN_LEAVES: f64 = 2.0;
/// The most times a plant is grown again ([`MIN_FILL`]).
const REGROWTHS: u64 = 3;

struct Grower<'a> {
    sp: &'a Species,
    cond: Conditions,
    rng: SplitMix64,
    /// The individual's grown height and crown radius.
    height: f64,
    radius: f64,
    /// Its crown's lobes.
    lobes: Lobes,
    nodes: Vec<GrowNode>,
    buds: Vec<Bud>,
    markers: Vec<DVec3>,
    taken: Vec<bool>,
    marker_grid: Grid,
    segment_grid: Grid,
    bud_grid: Grid,
    cycles: u32,
    /// Each node's radius by the pipe model at the last cycle's end, metres.
    girth: Vec<f64>,
    /// The pipe model's radius by the number of tips carried.
    pipe: Vec<f64>,
}

impl<'a> Grower<'a> {
    fn new(sp: &'a Species, cond: &Conditions) -> Self {
        let mut rng = Seed::new(cond.seed).derive_str(&sp.id).rng();
        let size = 0.35 + 0.65 * cond.site.clamp(0.0, 1.0);
        // The individual's size inside the published range, away from its ends: the plant's
        // measured size comes out a little under its envelope's.
        let pick =
            |rng: &mut SplitMix64, r: [f64; 2]| r[0] + (r[1] - r[0]) * rng.range_f64(0.25, 0.8);
        let height = pick(&mut rng, sp.height) * size;
        let radius = pick(&mut rng, sp.crown_width) * 0.5 * size;
        let l = sp.internode;
        let lean = cond.wind * LEAN;
        let margin = 2.0 * l + sp.perception * l;
        let lo = DVec3::new(
            -radius + (lean.x * height).min(0.0) - margin,
            -margin,
            -radius + (lean.y * height).min(0.0) - margin,
        );
        let hi = DVec3::new(
            radius + (lean.x * height).max(0.0) + margin,
            height + margin,
            radius + (lean.y * height).max(0.0) + margin,
        );
        // Markers fill the box round the grown envelope at about one per occupancy cube.
        let rho = sp.occupancy * l;
        let volume = (hi.x - lo.x - 2.0 * margin)
            * (hi.y - lo.y - 2.0 * margin).max(rho)
            * (hi.z - lo.z - 2.0 * margin);
        let count = ((volume / (rho * rho * rho)) * sp.density).clamp(64.0, 60_000.0) as usize;
        let mut markers = Vec::with_capacity(count);
        let mut marker_grid = Grid::new(lo, hi, rho);
        for i in 0..count {
            let p = DVec3::new(
                rng.range_f64(lo.x + margin, hi.x - margin),
                rng.range_f64(0.0, height),
                rng.range_f64(lo.z + margin, hi.z - margin),
            );
            markers.push(p);
            marker_grid.insert(p, i as u32);
        }
        let years = cond.age.max(0.0).ceil() as u32;
        Self {
            sp,
            cond: *cond,
            rng,
            height,
            radius,
            lobes: Lobes::new(sp, cond),
            nodes: Vec::new(),
            buds: Vec::new(),
            taken: vec![false; markers.len()],
            markers,
            marker_grid,
            segment_grid: Grid::new(lo, hi, 3.0 * l),
            bud_grid: Grid::new(lo, hi, sp.perception * l),
            cycles: years.clamp(MIN_CYCLES, MAX_CYCLES),
            girth: Vec::new(),
            pipe: Vec::new(),
        }
    }

    /// The envelope at `share` of the grown size.
    fn envelope(&self, share: f64) -> Envelope {
        Envelope {
            height: self.height * share,
            radius: self.radius * dmath::powf(share, 0.9),
            crown: self.sp.crown_at(share),
            lean: self.cond.wind * LEAN,
            pruning: self.cond.pruning,
            lobes: self.lobes,
            // The lift against the height the plant reaches by its age, the envelope's last: against
            // its grown height, the crown started at the lift times its share of growth, a young
            // one's under it, and was cut there at its widest (a review).
            lift_share: (self.cond.pruning.lift
                / (self.height * growth_share(self.cond.age, self.sp.maturity)).max(1e-6))
            .clamp(0.0, 0.9),
        }
    }

    fn run(mut self) -> TreeGraph {
        self.seed_plant();
        // The envelope grows by equal steps of size to its share at this age: the cycles go
        // where the plant changes, not to its slow old years.
        let share = growth_share(self.cond.age, self.sp.maturity);
        let total = self.cycles + SETTLE_CYCLES;
        for c in 0..total {
            let step = f64::from((c + 1).min(self.cycles)) / f64::from(self.cycles);
            let env = self.envelope(share * step);
            self.cycle(c, &env);
        }
        self.finish()
    }

    /// The root and its first buds: one trunk, or a shrub's stems.
    fn seed_plant(&mut self) {
        self.nodes.push(GrowNode {
            pos: DVec3::ZERO,
            parent: NO_PARENT,
            main: NO_PARENT,
            order: 0,
            starts_branch: false,
            rank: 0,
            phase: 0.0,
            alive: true,
            lit: 0,
        });
        let up =
            (DVec3::Y + DVec3::new(self.cond.wind.x, 0.0, self.cond.wind.y) * LEAN).normalize();
        match self.sp.habit {
            Habit::Tree => self.buds.push(Bud::new(0, up, true, 0)),
            Habit::Shrub => {
                let [lo, hi] = self.sp.stems;
                let n = lo + self.rng.below(u64::from(hi.max(lo) - lo + 1)) as u32;
                let phase = self.rng.range_f64(0.0, std::f64::consts::TAU);
                let spread = self.sp.stem_spread.to_radians();
                for k in 0..n {
                    let az = phase
                        + std::f64::consts::TAU * f64::from(k) / f64::from(n)
                        + self.rng.range_f64(-0.3, 0.3);
                    let tilt = spread * self.rng.range_f64(0.3, 1.0);
                    let axis = rotate(DVec3::X, DVec3::Y, az);
                    let dir = rotate(up, axis, tilt);
                    // Stems are order-0 shoots that start their own branch.
                    self.buds.push(Bud::new(0, dir, false, 0));
                }
            }
        }
    }

    fn cycle(&mut self, c: u32, env: &Envelope) {
        let sp = self.sp;
        let l = sp.internode;
        let reach = sp.perception * l;
        let cos_half = dmath::cos((sp.perception_angle * 0.5).to_radians());
        // 1. The free space each bud perceives: every marker in the envelope goes to the
        // nearest bud that sees it within its cone.
        self.bud_grid.clear();
        for (i, b) in self.buds.iter_mut().enumerate() {
            b.count = 0;
            b.towards = DVec3::ZERO;
            b.light = 0.0;
            b.vigour = 0.0;
            if b.alive {
                self.bud_grid
                    .insert(self.nodes[b.node as usize].pos, i as u32);
            }
        }
        for (m, &p) in self.markers.iter().enumerate() {
            if self.taken[m] || !env.contains(p, 0.0) {
                continue;
            }
            let mut best = (f64::MAX, u32::MAX);
            self.bud_grid.near(p, |i| {
                let b = &self.buds[i as usize];
                let to = p - self.nodes[b.node as usize].pos;
                let d2 = to.length_squared();
                if d2 > reach * reach || d2 >= best.0 {
                    return;
                }
                if to.dot(b.dir) < cos_half * d2.sqrt() {
                    return;
                }
                best = (d2, i);
            });
            if best.1 != u32::MAX {
                let b = &mut self.buds[best.1 as usize];
                let to = p - self.nodes[b.node as usize].pos;
                b.count += 1;
                b.towards += to / to.length().max(1e-9);
            }
        }
        // The trunk's and the stems' leaders climb towards the crown they have not reached.
        let top = (env.height - reach).max(env.height - l);
        for b in &mut self.buds {
            if !b.alive {
                continue;
            }
            if b.count > 0 {
                b.light = 1.0;
                b.towards = b.towards.normalize_or_zero();
            } else if b.order == 0 && self.nodes[b.node as usize].pos.y < top {
                b.light = 1.0;
                b.towards = DVec3::Y;
            }
        }
        // 2. The light gathered from the buds down to the base.
        let n = self.nodes.len();
        let mut qmain = vec![0.0f64; n];
        let mut qlat = vec![0.0f64; n];
        for b in self.buds.iter().filter(|b| b.alive) {
            if b.terminal {
                qmain[b.node as usize] += b.light;
            } else {
                qlat[b.node as usize] += b.light;
            }
        }
        for i in (1..n).rev() {
            let node = &self.nodes[i];
            if !node.alive {
                continue;
            }
            let s = qmain[i] + qlat[i];
            let p = node.parent as usize;
            if self.nodes[p].main == i as u32 {
                qmain[p] += s;
            } else {
                qlat[p] += s;
            }
        }
        // 3. The vigour handed from the base up (Borchert and Honda): at each node the main
        // axis takes λ·Qm / (λ·Qm + (1−λ)·Ql) of it.
        let lambda = sp.apical_control;
        let mut vmain = vec![0.0f64; n];
        let mut vlat = vec![0.0f64; n];
        for i in 0..n {
            let node = self.nodes[i];
            if !node.alive {
                continue;
            }
            let qsub = qmain[i] + qlat[i];
            if qsub > 0.0 {
                self.nodes[i].lit = c;
            }
            let vin = if i == 0 {
                sp.vigour * qsub
            } else {
                let p = node.parent as usize;
                if self.nodes[p].main == i as u32 {
                    vmain[p]
                } else if qlat[p] > 0.0 {
                    vlat[p] * qsub / qlat[p]
                } else {
                    0.0
                }
            };
            let denom = lambda * qmain[i] + (1.0 - lambda) * qlat[i];
            if vin > 0.0 && denom > 0.0 {
                vmain[i] = vin * lambda * qmain[i] / denom;
                vlat[i] = vin * (1.0 - lambda) * qlat[i] / denom;
            }
        }
        for b in self.buds.iter_mut().filter(|b| b.alive && b.light > 0.0) {
            let i = b.node as usize;
            b.vigour = if b.terminal {
                vmain[i]
            } else if qlat[i] > 0.0 {
                vlat[i] * b.light / qlat[i]
            } else {
                0.0
            };
        }
        // 4. Each bud grows a shoot of ⌊vigour⌋ metamers, at most `max_shoot`, clear of the
        // wood's girth as it stood at the last cycle's end.
        let count = self.buds.len();
        for k in 0..count {
            let b = self.buds[k];
            if !b.alive {
                continue;
            }
            if b.light > 0.0 {
                self.buds[k].starved = 0;
            } else {
                self.buds[k].starved += 1;
                // A leader waits for its crown; other buds die in the dark.
                if self.buds[k].starved > sp.shed_after && !(b.terminal && b.order == 0) {
                    self.buds[k].alive = false;
                }
                continue;
            }
            let metamers = (b.vigour.floor() as u32).min(sp.max_shoot);
            if metamers > 0 {
                self.shoot(k, metamers, env, c);
            }
        }
        // 5. Branches in the dark for `shed_after` cycles are shed, and the pruning cut.
        self.shed(c, env);
        // 6. Where thickening wood has come to pass through other wood, the thinner branch
        // breaks off and the crown grows back into the space it held.
        self.break_crossings();
    }

    /// Breaks the thinner of each pair of branches passing through each other (an axis inside
    /// the other's wood, by the girth as it stands), away from forks, and frees the space the
    /// broken wood held.
    fn break_crossings(&mut self) {
        self.girths();
        let n = self.nodes.len();
        let mut cut = vec![false; n];
        let mut any = false;
        // Growth kept every new segment `clearance` clear of the others, so only wood that has
        // since grown thicker than that can have come to cross: each such segment is checked
        // against its thinner neighbours.
        let clearance = 0.3 * self.sp.internode;
        for i in 1..n {
            let a = self.nodes[i];
            if !a.alive || a.parent == NO_PARENT || self.girth[i] <= clearance {
                continue;
            }
            let (a0, a1) = (self.nodes[a.parent as usize].pos, a.pos);
            self.segment_grid.near((a0 + a1) * 0.5, |j| {
                let j = j as usize;
                let b = &self.nodes[j];
                if j == i || !b.alive || b.parent == NO_PARENT {
                    return;
                }
                if (self.girth[j], j) > (self.girth[i], i) {
                    return;
                }
                let (thick, thin) = (i, j);
                let d = crate::graph::segment_distance(
                    a0.as_vec3(),
                    a1.as_vec3(),
                    self.nodes[b.parent as usize].pos.as_vec3(),
                    b.pos.as_vec3(),
                );
                if f64::from(d) >= self.girth[thick] || self.within_hops(i as u32, j as u32) {
                    return;
                }
                // Keep the stems: break the other one.
                let victim = if self.nodes[thin].parent == 0 {
                    thick
                } else {
                    thin
                };
                cut[victim] = true;
                any = true;
            });
        }
        if !any {
            return;
        }
        let rho = self.sp.occupancy * self.sp.internode;
        let mut freed = Vec::new();
        for i in 1..n {
            let p = self.nodes[i].parent as usize;
            if !self.nodes[i].alive {
                continue;
            }
            if cut[p] || !self.nodes[p].alive {
                cut[i] = true;
            }
            if cut[i] {
                self.nodes[i].alive = false;
                let pos = self.nodes[i].pos;
                let (markers, taken) = (&self.markers, &mut self.taken);
                self.marker_grid.near(pos, |m| {
                    if taken[m as usize] && markers[m as usize].distance_squared(pos) < rho * rho {
                        taken[m as usize] = false;
                        freed.push(m);
                    }
                });
            }
        }
        for b in &mut self.buds {
            if b.alive && !self.nodes[b.node as usize].alive {
                b.alive = false;
            }
        }
        for i in 0..n {
            let m = self.nodes[i].main;
            if m != NO_PARENT && !self.nodes[m as usize].alive {
                self.nodes[i].main = NO_PARENT;
            }
        }
        // The freed markers that living wood still occupies stay taken.
        for m in freed {
            let p = self.markers[m as usize];
            let mut held = false;
            self.segment_grid.near(p, |j| {
                let node = &self.nodes[j as usize];
                if node.alive && node.pos.distance_squared(p) < rho * rho {
                    held = true;
                }
            });
            self.taken[m as usize] = held;
        }
    }

    /// Whether two nodes are within [`FORK_HOPS`] steps of each other along the graph.
    fn within_hops(&self, a: u32, b: u32) -> bool {
        let mut ups = [(NO_PARENT, 0u32); FORK_HOPS as usize + 1];
        let mut n = a;
        for (k, up) in ups.iter_mut().enumerate() {
            *up = (n, k as u32);
            n = self.nodes[n as usize].parent;
            if n == NO_PARENT {
                break;
            }
        }
        let mut n = b;
        for k in 0..=FORK_HOPS {
            if let Some(&(_, s)) = ups.iter().find(|(m, _)| *m == n) {
                return s + k <= FORK_HOPS;
            }
            n = self.nodes[n as usize].parent;
            if n == NO_PARENT {
                break;
            }
        }
        false
    }

    /// Each living node's radius by the pipe model, as the plant stands.
    fn girths(&mut self) {
        let sp = self.sp;
        let n = self.nodes.len();
        // With every tip of one radius, a node's radius is the tip's times its tips counted to
        // the power 1/n: a table, not a power per node.
        let mut tips = vec![0u32; n];
        self.girth.clear();
        self.girth.resize(n, 0.0);
        for i in (1..n).rev() {
            if !self.nodes[i].alive {
                continue;
            }
            let t = tips[i].max(1);
            while self.pipe.len() <= t as usize {
                let k = self.pipe.len() as f64;
                self.pipe
                    .push(sp.tip_radius * dmath::powf(k, 1.0 / sp.pipe_exponent));
            }
            self.girth[i] = self.pipe[t as usize];
            tips[self.nodes[i].parent as usize] += t;
        }
    }

    /// Grows `metamers` metamers from bud `k`.
    fn shoot(&mut self, k: usize, metamers: u32, env: &Envelope, c: u32) {
        let sp = self.sp;
        let l = sp.internode;
        let b = self.buds[k];
        let mut at = b.node;
        let mut dir = b.dir;
        let wind = DVec3::new(self.cond.wind.x, 0.0, self.cond.wind.y);
        let clearance = 0.3 * l;
        let mut grown = 0;
        for m in 0..metamers {
            let wander = random_unit(&mut self.rng) * sp.wander;
            let d = dir + b.towards * sp.environment + DVec3::Y * sp.tropism + wind * 0.25 + wander;
            let d = if d.length_squared() > 1e-12 {
                d.normalize()
            } else {
                dir
            };
            let from = self.nodes[at as usize].pos;
            let p = from + d * l;
            // Inside the crown's space, or the trunk climbing to it.
            let below_crown = p.y < env.crown_base() + l;
            let trunk = b.order == 0 && {
                let a = env.axis(p.y);
                DVec2::new(p.x - a.x, p.z - a.y).length() < 1.5 * l + p.y * 0.08
            };
            let inside = env.contains(p, 0.5 * l) || (trunk && (below_crown || p.y < env.height));
            if !inside || p.y < 0.0 || p.y > env.pruning.top {
                break;
            }
            if b.order > 0 && p.y < env.pruning.lift {
                break;
            }
            // Never through other wood: clear of each segment by its radius as it stands, with
            // room for it to thicken, except at the fork the new segment grows from.
            let mut ups = [(NO_PARENT, 0u32); FORK_HOPS as usize];
            let mut n = at;
            for (k, up) in ups.iter_mut().enumerate() {
                *up = (n, k as u32 + 1);
                n = self.nodes[n as usize].parent;
                if n == NO_PARENT {
                    break;
                }
            }
            let mut blocked = false;
            self.segment_grid.near((from + p) * 0.5, |j| {
                if blocked {
                    return;
                }
                let other = &self.nodes[j as usize];
                if !other.alive || other.parent == NO_PARENT {
                    return;
                }
                // Within FORK_HOPS steps of the new segment along the graph: the same fork.
                let mut n = j;
                for k in 0..FORK_HOPS {
                    if let Some(&(_, s)) = ups.iter().find(|(m, _)| *m == n)
                        && s + k <= FORK_HOPS
                    {
                        return;
                    }
                    n = self.nodes[n as usize].parent;
                    if n == NO_PARENT {
                        break;
                    }
                }
                let q = self.nodes[other.parent as usize].pos;
                let dist = crate::graph::segment_distance(
                    from.as_vec3(),
                    p.as_vec3(),
                    q.as_vec3(),
                    other.pos.as_vec3(),
                );
                let girth = self.girth.get(j as usize).copied().unwrap_or(sp.tip_radius);
                if f64::from(dist) < clearance + 1.5 * girth {
                    blocked = true;
                }
            });
            if blocked {
                break;
            }
            let first = m == 0;
            let parent_node = self.nodes[at as usize];
            let continues = b.terminal || !first;
            let order = if first { b.order } else { parent_node.order };
            let (rank, phase) = if continues {
                (parent_node.rank + 1, parent_node.phase)
            } else {
                (0, self.rng.range_f64(0.0, std::f64::consts::TAU))
            };
            let idx = self.nodes.len() as u32;
            self.nodes.push(GrowNode {
                pos: p,
                parent: at,
                main: NO_PARENT,
                order,
                starts_branch: (first && !b.terminal) || (first && at == 0),
                rank,
                phase,
                alive: true,
                lit: c,
            });
            if continues {
                self.nodes[at as usize].main = idx;
            }
            self.segment_grid.insert((from + p) * 0.5, idx);
            // Markers within the occupancy radius are taken.
            let rho = sp.occupancy * l;
            let (markers, taken) = (&self.markers, &mut self.taken);
            self.marker_grid.near(p, |m| {
                if !taken[m as usize] && markers[m as usize].distance_squared(p) < rho * rho {
                    taken[m as usize] = true;
                }
            });
            // A lateral bud at the new node, turned by the phyllotaxis.
            if order < sp.max_order && p.y >= env.pruning.lift {
                let az = phase + f64::from(rank) * sp.divergence.to_radians();
                let axis = rotate(perpendicular(d), d, az);
                let lat = rotate(d, axis, sp.branch_angle.to_radians());
                self.buds.push(Bud::new(idx, lat, false, order + 1));
            }
            at = idx;
            dir = d;
            grown += 1;
        }
        if grown == 0 {
            return;
        }
        if b.terminal {
            self.buds[k].node = at;
            self.buds[k].dir = dir;
        } else {
            // The lateral bud became a branch: its tip carries the branch's terminal bud.
            self.buds[k].alive = false;
            let order = self.nodes[at as usize].order;
            self.buds.push(Bud::new(at, dir, true, order));
        }
    }

    /// Sheds the branches left in the dark and the pruned wood.
    fn shed(&mut self, c: u32, env: &Envelope) {
        let sp = self.sp;
        let lift = env.pruning.lift;
        let open = env.pruning.open_centre > 0.0;
        let n = self.nodes.len();
        // Each node's living subtree, in nodes: a limb is not shed by the shade.
        let mut size = vec![0u32; n];
        for i in (1..n).rev() {
            if self.nodes[i].alive {
                size[i] += 1;
                size[self.nodes[i].parent as usize] += size[i];
            }
        }
        let mut cut = vec![false; n];
        for (i, node) in self.nodes.iter().enumerate().skip(1) {
            if !node.alive {
                continue;
            }
            let dark =
                node.starts_branch && node.order > 0 && c.saturating_sub(node.lit) >= sp.shed_after;
            // Lifted by hand, or by the shade as the crown's base rises (self-pruning).
            let junction = self.nodes[node.parent as usize].pos.y;
            let lifted = node.starts_branch
                && node.order > 0
                && (junction < lift
                    || (junction < 0.8 * env.crown_base() && size[i] <= sp.limb_nodes));
            let topped = node.pos.y > env.pruning.top;
            // An open centre: the trunk's leader cut above the scaffold branches.
            let leader = open
                && node.order == 0
                && node.pos.y > lift + 3.0 * sp.internode
                && self.sp.habit == Habit::Tree;
            if dark || lifted || topped || leader {
                cut[i] = true;
            }
        }
        for i in 1..n {
            let p = self.nodes[i].parent as usize;
            if cut[p] || !self.nodes[p].alive {
                cut[i] = true;
            }
            if cut[i] {
                self.nodes[i].alive = false;
            }
        }
        for b in &mut self.buds {
            if b.alive && !self.nodes[b.node as usize].alive {
                b.alive = false;
            }
            if b.alive && !b.terminal && b.order > 0 && self.nodes[b.node as usize].pos.y < lift {
                b.alive = false;
            }
        }
        // A cut leader leaves its node without a main child.
        for i in 0..n {
            let m = self.nodes[i].main;
            if m != NO_PARENT && !self.nodes[m as usize].alive {
                self.nodes[i].main = NO_PARENT;
            }
        }
    }

    /// The living wood as a graph, its radii by the pipe model, crossings cut, leaves placed.
    fn finish(mut self) -> TreeGraph {
        let sp = self.sp;
        let mut graph = self.compact();
        for _ in 0..6 {
            pipe_radii(&mut graph, sp);
            let cut = self.crossing_cuts(&graph);
            if cut.is_empty() {
                break;
            }
            graph = remove_subtrees(&graph, &cut);
        }
        pipe_radii(&mut graph, sp);
        // Leaves on the young wood: the nodes carrying few twig tips.
        let mut tips = vec![0u32; graph.nodes.len()];
        for i in (1..graph.nodes.len()).rev() {
            if tips[i] == 0 {
                tips[i] = 1;
            }
            let p = graph.nodes[i].parent as usize;
            tips[p] += tips[i];
        }
        for (i, n) in graph.nodes.iter().enumerate().skip(1) {
            if tips[i] > sp.foliage_tips {
                continue;
            }
            let p = graph.nodes[n.parent as usize].pos;
            let dir = (n.pos - p).normalize_or_zero();
            let size = (sp.leaf_cluster * self.rng.range_f64(0.8, 1.2)) as f32;
            graph.leaves.push(LeafCluster {
                pos: n.pos,
                dir,
                size,
                node: i as u32,
            });
        }
        graph
    }

    fn compact(&self) -> TreeGraph {
        let mut index = vec![NO_PARENT; self.nodes.len()];
        let mut nodes = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            if !n.alive {
                continue;
            }
            index[i] = nodes.len() as u32;
            nodes.push(Node {
                pos: n.pos.as_vec3(),
                parent: if n.parent == NO_PARENT {
                    NO_PARENT
                } else {
                    index[n.parent as usize]
                },
                radius: 0.0,
                order: n.order,
                starts_branch: n.starts_branch,
            });
        }
        TreeGraph {
            species: self.sp.id.clone(),
            age: self.cond.age as f32,
            seed: self.cond.seed,
            nodes,
            leaves: Vec::new(),
        }
    }

    /// The segments to cut where wood passes through wood: of each crossing pair, the thinner.
    fn crossing_cuts(&self, g: &TreeGraph) -> Vec<u32> {
        let mut cut = Vec::new();
        let segs: Vec<crate::graph::Segment> = g.segments().collect();
        if segs.is_empty() {
            return cut;
        }
        let reach = segs.iter().map(|s| s.a.distance(s.b)).fold(0.0, f32::max)
            + 2.0 * segs.iter().map(|s| s.radius).fold(0.0, f32::max);
        let grid = crate::graph::SegmentGrid::new(&segs, reach.max(0.05));
        for (i, s) in segs.iter().enumerate() {
            grid.near(s, |j| {
                if j <= i {
                    return;
                }
                let t = &segs[j];
                let d = crate::graph::segment_distance(s.a, s.b, t.a, t.b);
                let (thick, thin) = if s.radius >= t.radius { (s, t) } else { (t, s) };
                if d < thick.radius
                    && crate::graph::hop_distance(g, s.node, t.node, FORK_HOPS) > FORK_HOPS
                {
                    // Keep the stems: cut the other one.
                    let victim = if g.nodes[thin.node as usize].parent == 0 {
                        thick.node
                    } else {
                        thin.node
                    };
                    cut.push(victim);
                }
            });
        }
        cut.sort_unstable();
        cut.dedup();
        cut
    }
}

impl Bud {
    fn new(node: u32, dir: DVec3, terminal: bool, order: u8) -> Self {
        Self {
            node,
            dir,
            terminal,
            order,
            alive: true,
            starved: 0,
            count: 0,
            towards: DVec3::ZERO,
            light: 0.0,
            vigour: 0.0,
        }
    }
}

/// The radii by the pipe model: a twig's is the species' tip radius, and a parent's to the
/// power `n` is its children's summed.
fn pipe_radii(g: &mut TreeGraph, sp: &Species) {
    let n = sp.pipe_exponent;
    let mut sum = vec![0.0f64; g.nodes.len()];
    for i in (0..g.nodes.len()).rev() {
        let r = if sum[i] > 0.0 {
            dmath::powf(sum[i], 1.0 / n)
        } else {
            sp.tip_radius
        };
        g.nodes[i].radius = r as f32;
        let p = g.nodes[i].parent;
        if p != NO_PARENT {
            sum[p as usize] += dmath::powf(r, n);
        }
    }
    // The root carries its thickest stem's radius.
    let base = g
        .nodes
        .iter()
        .filter(|n| n.parent == 0)
        .map(|n| n.radius)
        .fold(0.0, f32::max);
    if let Some(root) = g.nodes.first_mut() {
        root.radius = base.max(sp.tip_radius as f32);
    }
}

/// The graph without the subtrees under `cut`.
fn remove_subtrees(g: &TreeGraph, cut: &[u32]) -> TreeGraph {
    let mut dead = vec![false; g.nodes.len()];
    for &c in cut {
        dead[c as usize] = true;
    }
    let mut index = vec![NO_PARENT; g.nodes.len()];
    let mut nodes = Vec::with_capacity(g.nodes.len());
    for (i, n) in g.nodes.iter().enumerate() {
        if n.parent != NO_PARENT && dead[n.parent as usize] {
            dead[i] = true;
        }
        if dead[i] {
            continue;
        }
        index[i] = nodes.len() as u32;
        let mut m = *n;
        if m.parent != NO_PARENT {
            m.parent = index[m.parent as usize];
        }
        nodes.push(m);
    }
    TreeGraph {
        species: g.species.clone(),
        age: g.age,
        seed: g.seed,
        nodes,
        leaves: Vec::new(),
    }
}
