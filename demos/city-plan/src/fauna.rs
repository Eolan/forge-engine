//! The village's animals (#269's fauna, D-062's core, `--fauna`): the plan given to
//! `forge_fauna` as its [`Parts`], the animals stepped at its rate with the player and the
//! pursuers as what they flee, and drawn as movers, a mesh a species and pose
//! (`forge_geom::animals`).
//!
//! The first version keeps the village's everyday animals: sparrows, pigeons and swifts, cats,
//! and dogs walked by their owners. The water's (gulls, mullet) wait for the shore as outlines,
//! the goats for a pasture.

use forge_fauna::plan::{Building, Garden, Parts, Street, Sun, Tree, Wall};
use forge_fauna::{Activity, Behaviour, Disturber, Fauna, Mode, Plan, Roster, Species, Threat};
use forge_geom::animals::{Bird, Gait, Quadruped, Wings};
use forge_procgen::city::{BlockUse, CityPlan, RoadClass, geom};
use glam::{DVec2, Vec2, Vec3};

use super::{Mass, Structure};

/// How near a house a forest plant stands to count as the village's (a perch, a roost), metres.
const NEAR_HOUSES: f64 = 40.0;
/// The streets' polylines cut into straight streets at most this long, metres.
const PIECE: f64 = 12.0;
/// A main road's limit, km/h; the lanes are the people's.
const MAIN_LIMIT: f32 = 30.0;

/// What the animals see of the village of `plan` and `masses` (a side of `size` metres): its
/// houses with their eaves and front doors, its garden walls, each house's lot as its garden, its
/// squares and greens, its streets, and its trees: `trees` (the plan's, `(x, y, height, crown)`
/// on the plan) and the forest's plants within [`NEAR_HOUSES`] of a house.
pub fn parts(
    plan: &CityPlan,
    masses: &[Mass],
    structures: &[Structure],
    trees: &[(DVec2, f64, f64)],
) -> Parts {
    let v = |p: DVec2| Vec2::new(p.x as f32, p.y as f32);
    let buildings: Vec<Building> = masses
        .iter()
        .zip(structures)
        .map(|(m, s)| {
            // Just outside its front door (the core's), else out of its footprint's first edge.
            let door = s
                .cores
                .iter()
                .find(|c| c.house)
                .and_then(|c| s.openings.get(c.door))
                .map(|o| {
                    let w = &s.walls[o.wall];
                    let t = (w.b - w.a).normalize_or_zero();
                    w.a + t * o.along - DVec2::new(-t.y, t.x) * 0.6
                })
                .unwrap_or_else(|| {
                    // 0.6 m out of the edge's middle, away from the house.
                    let (a, b) = (m.footprint[0], m.footprint[1 % m.footprint.len()]);
                    let (mid, t) = ((a + b) * 0.5, (b - a).normalize_or_zero());
                    let n = DVec2::new(-t.y, t.x);
                    let away = if n.dot(mid - geom::centroid(&m.footprint)) < 0.0 {
                        -n
                    } else {
                        n
                    };
                    mid + away * 0.6
                });
            Building {
                footprint: m.footprint.iter().map(|&p| v(p)).collect(),
                height: m.height as f32,
                door: v(door),
            }
        })
        .collect();
    let walls = plan
        .walls
        .iter()
        .map(|w| Wall {
            a: v(w.a),
            b: v(w.b),
            height: w.height as f32,
        })
        .collect();
    let gardens = masses
        .iter()
        .enumerate()
        .filter_map(|(i, m)| {
            let lot = plan.lots.get(m.lot as usize)?;
            let garden = behind(&lot.polygon, &m.footprint)?;
            Some(Garden {
                polygon: garden.iter().map(|&p| v(p)).collect(),
                house: i,
            })
        })
        .collect();
    let squares = plan
        .blocks
        .iter()
        .filter(|b| matches!(b.use_, BlockUse::Square | BlockUse::Park))
        .map(|b| b.polygon.iter().map(|&p| v(p)).collect())
        .collect();
    // The street graph, each polyline cut into straight pieces at its points, then at most
    // [`PIECE`] long.
    let mut nodes: Vec<Vec2> = plan.roads.nodes.iter().map(|n| v(n.pos)).collect();
    let mut streets = Vec::new();
    for e in &plan.roads.edges {
        let limit = if e.class == RoadClass::Main {
            MAIN_LIMIT
        } else {
            0.0
        };
        let mut points: Vec<DVec2> = Vec::new();
        for w in e.points.windows(2) {
            let n = (w[0].distance(w[1]) / PIECE).ceil().max(1.0) as usize;
            points.extend((0..n).map(|k| w[0] + (w[1] - w[0]) * (k as f64 / n as f64)));
        }
        let mut prev = e.a;
        for &p in points.iter().skip(1) {
            nodes.push(v(p));
            let id = nodes.len() as u32 - 1;
            streets.push(Street {
                a: prev,
                b: id,
                limit,
            });
            prev = id;
        }
        streets.push(Street {
            a: prev,
            b: e.b,
            limit,
        });
    }
    let near = |p: DVec2| {
        masses.iter().any(|m| {
            m.footprint
                .first()
                .is_some_and(|&c| c.distance(p) < NEAR_HOUSES + 15.0)
        })
    };
    let trees = trees
        .iter()
        .filter(|&&(p, _, _)| near(p))
        .map(|&(p, height, crown)| Tree {
            pos: v(p),
            height: height as f32,
            crown: crown as f32,
        })
        .collect();
    Parts {
        size: plan.size as f32,
        urban: false,
        buildings,
        walls,
        gardens,
        squares,
        nodes,
        streets,
        water: Vec::new(),
        quay: Vec::new(),
        pasture: Vec::new(),
        fold: Vec::new(),
        trees,
        shrubs: Vec::new(),
        // The scene's day: its sun up from 6:00 to 18:00 (`hour_of_day`).
        sun: Sun {
            rise: 6.0 * 3600.0,
            set: 18.0 * 3600.0,
        },
    }
}

/// The least garden kept behind a house, square metres.
const LEAST_GARDEN: f64 = 4.0;

/// The part of `lot` (its street side first) behind the house on it (`footprint`): the lot cut
/// along the house's edge farthest from the street, on the far side; none under
/// [`LEAST_GARDEN`]. The whole lot put a cat's garden spots inside its house.
fn behind(lot: &[DVec2], footprint: &[DVec2]) -> Option<Vec<DVec2>> {
    if lot.len() < 3 || footprint.len() < 3 {
        return None;
    }
    let (s0, s1) = (lot[0], lot[1]);
    let street = |p: DVec2| geom::distance_to_segment(p, s0, s1);
    let n = footprint.len();
    let (a, b) = (0..n)
        .map(|k| (footprint[k], footprint[(k + 1) % n]))
        .max_by(|x, y| street((x.0 + x.1) * 0.5).total_cmp(&street((y.0 + y.1) * 0.5)))?;
    // The side of the back edge's line away from the street.
    let along = (b - a).normalize_or_zero();
    let mut out_n = DVec2::new(-along.y, along.x);
    if out_n.dot((s0 + s1) * 0.5 - a) > 0.0 {
        out_n = -out_n;
    }
    let side = |p: DVec2| (p - a).dot(out_n);
    let mut clipped = Vec::new();
    for k in 0..lot.len() {
        let (p, q) = (lot[k], lot[(k + 1) % lot.len()]);
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            clipped.push(p);
        }
        if (sp >= 0.0) != (sq >= 0.0) {
            clipped.push(p + (q - p) * (sp / (sp - sq)));
        }
    }
    (clipped.len() >= 3 && geom::signed_area(&clipped).abs() >= LEAST_GARDEN).then_some(clipped)
}

/// The groups the village keeps: the example village's (`Roster::village`), without the water's
/// and the hillside's species.
pub fn roster(species: &[Species]) -> Roster {
    let mut r = Roster::village(species);
    r.groups.retain(|&(s, _)| {
        !matches!(
            species[s as usize].behaviour,
            Behaviour::Gull | Behaviour::Goat | Behaviour::Fish
        )
    });
    r
}

/// The animals under way: stepped at `forge_fauna`'s rate, drawn between their last two steps.
pub struct FaunaRun {
    fauna: Fauna<'static>,
    /// Each animal's place at the step before the last, to draw between.
    before: Vec<Vec3>,
    /// Seconds into the step under way.
    owed: f64,
    /// The people's feet at the last call, for their velocities.
    seen: Vec<Vec3>,
    /// The plan's half side: the world's frame is the plan's moved by it.
    half: f32,
}

/// Seconds a step of `forge_fauna` takes.
const STEP: f64 = 1.0 / forge_fauna::sim::HZ as f64;
/// The most steps a frame runs; a longer frame's rest is dropped.
const CATCH_UP: u32 = 4;
/// How long the animals live before the run starts, seconds: a cat stays in 20 to 60 minutes
/// at the start, and the dogs leave home on their walks' hours, so a run that began with them
/// showed none.
const WARM_UP: f64 = 2400.0;

impl FaunaRun {
    /// The village's animals from `seed`, at `hour` of the day (0 to 24), [`WARM_UP`] into their
    /// routine; the plan's `parts` and the species kept for the run's length (the simulation
    /// borrows them).
    pub fn new(parts: Parts, hour: f64, seed: u64) -> Self {
        let half = parts.size * 0.5;
        let plan: &'static Plan = Box::leak(Box::new(Plan::new(parts)));
        let species: &'static [Species] = Box::leak(forge_fauna::village().into_boxed_slice());
        let start = (hour * 3600.0 - WARM_UP).rem_euclid(86_400.0);
        let mut fauna = Fauna::new(plan, species, &roster(species), seed, start as f32);
        fauna.run(forge_fauna::sim::ticks(WARM_UP as f32), None);
        tracing::info!(
            animals = fauna.len(),
            groups = fauna.groups.len(),
            sites = plan.sites.len(),
            "the village's animals"
        );
        // A view of each species' first group out of doors: 3 m south of it and 4 m over it,
        // looking north and down over the gardens' shrubs.
        for (s, sp) in species.iter().enumerate() {
            let Some(g) = (0..fauna.groups.len()).find(|&g| {
                let gr = &fauna.groups[g];
                gr.species as usize == s
                    && (gr.first..gr.first + gr.len).any(|i| fauna.mode[i as usize] != Mode::Hidden)
            }) else {
                continue;
            };
            let c = fauna.centroid(g, None);
            tracing::info!(
                "the village's {}: --view={:.1},{:.1},{:.1},0,-50",
                sp.id,
                c.x - half,
                c.z + 4.0,
                c.y - half + 3.0
            );
        }
        Self {
            before: fauna.pos.clone(),
            fauna,
            owed: 0.0,
            seen: Vec::new(),
            half,
        }
    }

    /// Runs the steps `dt` seconds owe, with `people` (their feet, the world's frame) as the
    /// people on foot the animals may flee besides their own; their velocities from where they
    /// stood at the last call.
    pub fn advance(&mut self, dt: f64, people: &[Vec3]) {
        let around: Vec<Disturber> = people
            .iter()
            .enumerate()
            .map(|(k, &at)| {
                let vel = match self.seen.get(k) {
                    Some(&was) if people.len() == self.seen.len() && dt > 0.0 => {
                        (at - was) / dt as f32
                    }
                    _ => Vec3::ZERO,
                };
                Disturber {
                    pos: at + Vec3::Y,
                    vel,
                    threat: Threat::Person,
                }
            })
            .collect();
        self.seen = people.to_vec();
        self.owed += dt;
        let mut ran = 0;
        while self.owed >= STEP {
            self.owed -= STEP;
            ran += 1;
            if ran > CATCH_UP {
                self.owed = 0.0;
                break;
            }
            let outside: Vec<Disturber> = around
                .iter()
                .map(|d| Disturber {
                    pos: self.plan_point(d.pos),
                    vel: Vec3::new(d.vel.x, d.vel.z, d.vel.y),
                    ..*d
                })
                .collect();
            self.before.clone_from(&self.fauna.pos);
            self.fauna.step(&outside);
        }
    }

    /// The species, as the simulation indexes them.
    pub fn species(&self) -> &'static [Species] {
        self.fauna.species
    }

    /// Animal `i`: its species, its mode, where it is drawn (the world's frame, its height over
    /// the ground) and the way it moves (the world's x and z).
    pub fn animal(&self, i: usize) -> (&Species, Mode, Vec3, Vec2) {
        let t = (self.owed / STEP).clamp(0.0, 1.0) as f32;
        let p = self.before[i].lerp(self.fauna.pos[i], t);
        let vel = self.fauna.vel[i];
        (
            self.fauna.species_of(i),
            self.fauna.mode[i],
            Vec3::new(p.x - self.half, p.z, p.y - self.half),
            Vec2::new(vel.x, vel.y),
        )
    }

    /// A point of the world's frame on the plan (x east, y south, z up).
    fn plan_point(&self, p: Vec3) -> Vec3 {
        Vec3::new(p.x + self.half, p.z + self.half, p.y)
    }

    /// The simulation, for checks.
    #[cfg(test)]
    pub fn fauna(&self) -> &Fauna<'static> {
        &self.fauna
    }
}

/// A species' body: a bird's or a four-legged animal's build.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Body {
    /// A bird.
    Bird(Bird),
    /// A cat or a dog.
    Quad(Quadruped),
}

/// The poses a body is drawn in, a mesh each: a bird folded, then its wings up, level and down;
/// a four-legged animal its near legs forward, square, back, and lying.
pub const POSES: usize = 4;

/// A coat: its fur's or plumage's colour and its dark parts', linear.
pub type Coat = ([f32; 3], [f32; 3]);

/// The body `s` is drawn with, and its coats.
pub fn body(s: &Species) -> Option<(Body, &'static [Coat])> {
    let bird = |span: f32, tail: f32| {
        Body::Bird(Bird {
            body: s.body,
            span,
            tail,
        })
    };
    let quad = |shoulder: f32, tail: (f32, f32), ears: f32| {
        Body::Quad(Quadruped {
            body: s.body,
            shoulder,
            tail,
            ears,
        })
    };
    const BEAK: [f32; 3] = [0.06, 0.05, 0.04];
    const SPARROW: &[Coat] = &[([0.30, 0.20, 0.12], BEAK)];
    const PIGEON: &[Coat] = &[([0.36, 0.38, 0.42], [0.55, 0.25, 0.25])];
    const SWIFT: &[Coat] = &[([0.07, 0.06, 0.06], BEAK)];
    const CAT: &[Coat] = &[
        ([0.55, 0.30, 0.12], [0.35, 0.15, 0.12]),
        ([0.32, 0.31, 0.30], [0.12, 0.11, 0.11]),
        ([0.05, 0.05, 0.05], [0.25, 0.12, 0.12]),
    ];
    const DOG: &[Coat] = &[
        ([0.38, 0.24, 0.13], [0.05, 0.04, 0.04]),
        ([0.70, 0.62, 0.48], [0.08, 0.06, 0.05]),
    ];
    Some(match s.behaviour {
        Behaviour::Sparrow => (bird(1.6, 0.35), SPARROW),
        Behaviour::Pigeon => (bird(2.0, 0.3), PIGEON),
        Behaviour::Swift => (bird(2.5, 0.4), SWIFT),
        Behaviour::Cat => (quad(0.55, (0.6, 0.8), 0.07), CAT),
        Behaviour::Dog => (quad(0.75, (0.35, 0.5), 0.05), DOG),
        _ => return None,
    })
}

/// Body `b`'s mesh in pose `k` (of [`POSES`]).
pub fn mesh(b: Body, k: usize) -> forge_geom::procedural::TriMesh {
    match b {
        Body::Bird(build) => {
            let wings = [
                Wings::Folded,
                Wings::Spread(0.6),
                Wings::Spread(0.0),
                Wings::Spread(-0.5),
            ][k];
            forge_geom::animals::bird(build, wings)
        }
        Body::Quad(build) => {
            let gait = [
                Gait::Stride(0.35),
                Gait::Stride(0.0),
                Gait::Stride(-0.35),
                Gait::Lying,
            ][k];
            forge_geom::animals::quadruped(build, gait)
        }
    }
}

/// The movers the animals are drawn with, in their reserved order: for each species and coat, a
/// block of slots a pose, a slot each of its animals; then the dogs' owners, a block a stride, a
/// slot each.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    /// Each block: its species, coat and pose, and its slots.
    pub blocks: Vec<(usize, usize, usize, u32)>,
    /// Each animal's first block (its species' and coat's, in its first pose) and its slot in
    /// each of them; none for an animal not drawn.
    pub slot: Vec<Option<(usize, u32)>>,
    /// The owners' slots a stride.
    pub owners: u32,
}

/// What the drawing keeps from frame to frame: each animal's way and pose, each owner's stride.
#[derive(Clone, Debug, Default)]
pub struct Drawn {
    facing: Vec<f32>,
    pose: Vec<Option<usize>>,
    stride: Vec<Option<usize>>,
}

/// How fast each species beats its wings, beats a second (a pose a quarter beat).
fn wingbeat(b: Behaviour) -> f32 {
    match b {
        Behaviour::Sparrow => 12.0,
        Behaviour::Pigeon => 6.0,
        _ => 7.0,
    }
}

impl FaunaRun {
    /// The movers the animals need: a block per species, coat and pose, as many slots as that
    /// species has animals of that coat; the owners' as many as there are dogs.
    pub fn layout(&self) -> Layout {
        let mut out = Layout {
            slot: vec![None; self.fauna.len()],
            ..Layout::default()
        };
        for (s, sp) in self.fauna.species.iter().enumerate() {
            let Some((_, coats)) = body(sp) else {
                continue;
            };
            for c in 0..coats.len() {
                let first = out.blocks.len();
                let mut n = 0;
                for i in 0..self.fauna.len() {
                    if self.species_index(i) == s && i % coats.len() == c {
                        out.slot[i] = Some((first, n));
                        n += 1;
                    }
                }
                if n > 0 {
                    out.blocks.extend((0..POSES).map(|k| (s, c, k, n)));
                }
            }
        }
        out.owners = self
            .fauna
            .groups
            .iter()
            .filter(|g| g.leash.is_some())
            .count() as u32;
        out
    }

    /// Animal `i`'s species' index.
    fn species_index(&self, i: usize) -> usize {
        self.fauna.groups[self.fauna.group[i] as usize].species as usize
    }

    /// The movers' places, turns about y (from +x) and whether each moved from elsewhere this
    /// frame (an animal that changed pose, a slot that changed hands: no motion of its own), in
    /// `layout`'s order. Each animal in its slot of its pose's block, turned the way it goes
    /// (still, the way it last went), its wings beating at `time` seconds; the owners in their
    /// strides; the slots no one fills at `away`.
    pub fn poses(
        &self,
        layout: &Layout,
        drawn: &mut Drawn,
        time: f64,
        away: Vec3,
    ) -> Vec<(Vec3, f32, bool)> {
        let ground = Vec3::Y * super::ground::WALK_LEVEL as f32;
        let n = self.fauna.len();
        drawn.facing.resize(n, 0.0);
        drawn.pose.resize(n, None);
        let mut out = Vec::new();
        let mut starts = Vec::with_capacity(layout.blocks.len());
        for &(_, _, _, slots) in &layout.blocks {
            starts.push(out.len());
            out.extend((0..slots).map(|_| (away, 0.0, true)));
        }
        for i in 0..n {
            let Some((first, rank)) = layout.slot[i] else {
                continue;
            };
            let (sp, mode, at, vel) = self.animal(i);
            let Some((b, _)) = body(sp) else {
                continue;
            };
            if vel.length() > 0.05 {
                drawn.facing[i] = (-vel.y).atan2(vel.x);
            }
            let pose = match (b, mode) {
                (_, Mode::Hidden) => None,
                (Body::Bird(_), Mode::Rest | Mode::Ground | Mode::Walk) => Some(0),
                (Body::Bird(_), _) => {
                    let beat = wingbeat(sp.behaviour) * time as f32 + i as f32 * 0.37;
                    Some(1 + (beat * 4.0) as usize % 3)
                }
                (Body::Quad(_), Mode::Rest) => Some(3),
                (Body::Quad(q), _) => {
                    let pace = self.fauna.travelled[i] / (0.35 * q.body);
                    Some([0, 1, 2, 1][pace as usize % 4])
                }
            };
            let changed = pose != drawn.pose[i];
            drawn.pose[i] = pose;
            if let Some(k) = pose {
                out[starts[first + k] + rank as usize] = (at + ground, drawn.facing[i], changed);
            }
        }
        // The owners, a block a stride: walking, the crowd's gait at their pace; still, square.
        let owners = self.owners_of_dogs();
        drawn.stride.resize(owners.len(), None);
        let gait = super::pursuers::GAIT;
        let strides = super::pursuers::STRIDES.len();
        let base = out.len();
        out.extend((0..strides * layout.owners as usize).map(|_| (away, 0.0, true)));
        for (j, owner) in owners.iter().enumerate().take(layout.owners as usize) {
            let stride = owner.map(|(_, vel)| {
                if vel.length() > 0.1 {
                    gait[(time * 4.0 + j as f64 * 0.5) as usize % gait.len()]
                } else {
                    1
                }
            });
            let changed = stride != drawn.stride[j];
            drawn.stride[j] = stride;
            if let (Some((at, vel)), Some(k)) = (owner, stride) {
                let yaw = if vel.length() > 0.05 {
                    (-vel.y).atan2(vel.x)
                } else {
                    0.0
                };
                out[base + k * layout.owners as usize + j] = (at + ground, yaw, changed);
            }
        }
        out
    }

    /// Each dog's owner in the groups' order: where they walk (the world's frame) and their
    /// velocity (x and z) while out with the dog, else none.
    fn owners_of_dogs(&self) -> Vec<Option<(Vec3, Vec2)>> {
        self.fauna
            .groups
            .iter()
            .filter_map(|g| g.leash.as_ref().map(|l| (g.activity, l)))
            .map(|(activity, l)| {
                (activity == Activity::Walk).then(|| {
                    (
                        Vec3::new(l.owner.x - self.half, 0.0, l.owner.y - self.half),
                        Vec2::new(l.vel.x, l.vel.y),
                    )
                })
            })
            .collect()
    }
}
