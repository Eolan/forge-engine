//! The animals stepped at a fixed rate: groups that decide (where to feed, rest, roost, when to
//! walk the dog), members that move (flocking in the air and in the water, walking on the
//! ground), and reactions to the people, vehicles, dogs and cats about them.
//!
//! Everyone is a few columns of a structure of arrays, as `forge-sim`'s population. A group (a
//! flock, a herd, a school, a cat, a dog with its owner) holds the shared decision: its activity,
//! where, and until when. Its members follow it: a flock flies to its next site in Couzin's
//! zones over a topological neighbourhood (Ballerini et al. 2008), each bird at a constant cruise
//! with a turn it cannot exceed, looking ahead for walls and roofs (Reynolds 1999); a herd
//! browses shrub to shrub inside its fence; a school swims along the quay.
//!
//! Every choice is a hash of the group's seed and a counter, every position the result of
//! IEEE-exact arithmetic and `forge_core::dmath`, in a fixed order: the same seed and the same
//! passers give the same animals to the bit on every machine (D-016).

use std::f32::consts::FRAC_1_SQRT_2;

use forge_core::dmath;
use forge_core::hash::mix64;
use glam::{Vec2, Vec3};

use crate::passers::{Disturber, Threat};
use crate::plan::{self, PatchKind, Plan, SiteKind, URBAN};
use crate::species::{Behaviour, Species};

/// Steps a second.
pub const HZ: u32 = 10;
/// The fixed step, seconds.
pub const STEP: f32 = 1.0 / HZ as f32;
/// Steps a day.
pub const DAY: u64 = 86_400 * HZ as u64;
/// No index.
pub const NONE: u32 = u32::MAX;
/// How much closer a threat that is not coming at the animal may pass (a passer-by against an
/// approach; Forge's estimate: flight distances are measured on direct approaches).
pub const PASSING: f32 = 0.6;
/// Birds this high or higher ignore threats on the ground, metres.
pub const SAFE_BIRD: f32 = 2.5;
/// A cat this high (a wall's top) ignores threats on the ground, metres.
pub const SAFE_CAT: f32 = 1.5;
/// Turns tried to get past an obstacle, nearest first: the cosines and sines of ±30°, ±60°,
/// ±90° and ±135°.
const TURNS: [(f32, f32); 8] = [
    (0.866_025_4, 0.5),
    (0.866_025_4, -0.5),
    (0.5, 0.866_025_4),
    (0.5, -0.866_025_4),
    (0.0, 1.0),
    (0.0, -1.0),
    (-FRAC_1_SQRT_2, FRAC_1_SQRT_2),
    (-FRAC_1_SQRT_2, -FRAC_1_SQRT_2),
];

/// Steps in `seconds`.
pub fn ticks(seconds: f32) -> u64 {
    (seconds * HZ as f32) as u64
}

/// 64 bits from a seed and two counters.
fn bits(seed: u64, a: u64, b: u64) -> u64 {
    mix64(seed ^ mix64(a.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ b.rotate_left(29)))
}

/// Uniform in `[0, 1)` from a seed and two counters.
fn unit(seed: u64, a: u64, b: u64) -> f32 {
    (bits(seed, a, b) >> 40) as f32 * (1.0 / 16_777_216.0)
}

/// Uniform in `[lo, hi)` from a seed and two counters.
fn between(seed: u64, a: u64, b: u64, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * unit(seed, a, b)
}

/// What an animal is doing with its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Mode {
    /// Out of sight: indoors, in a nest.
    Hidden,
    /// Still: perched, lying, sitting, browsing.
    Rest,
    /// Feeding on foot (a bird's hops).
    Ground,
    /// Flying.
    Air,
    /// Floating or swimming.
    Water,
    /// Walking.
    Walk,
    /// Running, flying or swimming from a threat.
    Flee,
}

/// The modes, in order, for counts.
pub const MODES: [Mode; 7] = [
    Mode::Hidden,
    Mode::Rest,
    Mode::Ground,
    Mode::Air,
    Mode::Water,
    Mode::Walk,
    Mode::Flee,
];

/// What a group is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Activity {
    /// At its night's place: a roost, nests, home, the fold, the sea.
    Night,
    /// Resting at a site.
    Perch,
    /// Feeding on the ground about its goal.
    Forage,
    /// Going to its members' spots, then `then`.
    Travel,
    /// Circling aloft about a centre.
    Circle,
    /// A swifts' screaming party along the streets.
    Party,
    /// Floating on the sea.
    Float,
    /// A cat out of doors: legs and rests.
    Out,
    /// A dog walked by its owner.
    Walk,
    /// A herd browsing about its goal.
    Graze,
    /// A herd in a tree's shade.
    Shade,
    /// A school swimming to its goal.
    School,
    /// Fleeing to its members' spots, then `then`.
    Flee,
}

/// A cat's yard: where it goes in its block's gardens and on its street.
#[derive(Clone, Debug, PartialEq)]
pub struct Yard {
    /// Places in the gardens it reaches from its own without passing a building; wall tops at
    /// their height.
    pub garden: Vec<Vec3>,
    /// Places along its house's front.
    pub street: Vec<Vec3>,
    /// Its own garden's middle: legs in the gardens pass it.
    pub hub: Vec2,
    /// Its way in from the garden (a cat flap), just inside the garden.
    pub flap: Vec2,
    /// Its house's door.
    pub door: Vec2,
    /// Whether it is on the garden's side.
    pub in_garden: bool,
}

/// A dog's owner and their walk.
#[derive(Clone, Debug, PartialEq)]
pub struct Leash {
    /// The owner's position.
    pub owner: Vec3,
    /// The owner's velocity.
    pub vel: Vec3,
    /// The walk's points: the door, the streets' nodes out and back, the door.
    pub route: Vec<Vec2>,
    /// The point the owner walks to.
    pub leg: u32,
    /// The owner waits until this step (the dog sniffing).
    pub pause: u64,
    /// The owner's pace, m/s.
    pub pace: f32,
}

/// A flock, a herd, a school, a cat or a dog with its owner.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    /// Its species' index.
    pub species: u32,
    /// Its first member.
    pub first: u32,
    /// Its members.
    pub len: u32,
    /// Its seed.
    pub seed: u64,
    /// Its home: a site (sparrows, pigeons, gulls), a building (swifts, cats, dogs), none.
    pub home: u32,
    /// What it is doing.
    pub activity: Activity,
    /// What it does when all have arrived (travel, flight).
    pub then: Activity,
    /// The step its activity ends.
    pub until: u64,
    /// How long `then` lasts, steps.
    pub stay: u64,
    /// Its goal: where it heads, the middle of where it feeds, a circle's point.
    pub goal: Vec3,
    /// A circle's centre.
    pub centre: Vec3,
    /// A circle's or a feeding place's radius, metres.
    pub radius: f32,
    /// A circle's angle, radians.
    pub phase: f32,
    /// The street node a party heads for, the last one before it.
    pub node: [u32; 2],
    /// Members arrived.
    pub landed: u32,
    /// Decisions taken: the hash's counter.
    pub decisions: u64,
    /// A cat's yard.
    pub yard: Option<Yard>,
    /// A dog's owner.
    pub leash: Option<Leash>,
}

/// An animal that fled: who, from what, at what distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flush {
    /// The step.
    pub tick: u64,
    /// The animal.
    pub animal: u32,
    /// Its species.
    pub species: u32,
    /// What it fled.
    pub threat: Threat,
    /// The threat's distance on the ground, metres.
    pub distance: f32,
    /// The distance it lets that threat come to, metres (after [`PASSING`]).
    pub threshold: f32,
}

/// How many groups of each species to make.
#[derive(Clone, Debug, PartialEq)]
pub struct Roster {
    /// Groups per species' index.
    pub groups: Vec<(u32, u32)>,
}

impl Roster {
    /// The example village's: six flocks of sparrows, two of pigeons, three of gulls, three
    /// parties of swifts, ten cats, eight dogs, a herd of goats and three schools of mullet.
    pub fn village(species: &[Species]) -> Self {
        let n = |b: Behaviour, k: u32| crate::species::index_of(species, b).map(|s| (s as u32, k));
        Self {
            groups: [
                n(Behaviour::Sparrow, 6),
                n(Behaviour::Pigeon, 2),
                n(Behaviour::Gull, 3),
                n(Behaviour::Swift, 3),
                n(Behaviour::Cat, 10),
                n(Behaviour::Dog, 8),
                n(Behaviour::Goat, 1),
                n(Behaviour::Fish, 3),
            ]
            .into_iter()
            .flatten()
            .collect(),
        }
    }

    /// Only `k` groups of species `s`.
    pub fn only(s: u32, k: u32) -> Self {
        Self {
            groups: vec![(s, k)],
        }
    }
}

/// Threats in cells, rebuilt every step.
#[derive(Clone, Debug, Default)]
struct ThreatGrid {
    side: usize,
    start: Vec<u32>,
    items: Vec<u32>,
}

/// The cell side of the threats' grid, metres: wider than the largest flight distance.
const THREAT_CELL: f32 = 24.0;

/// The animals of a place.
pub struct Fauna<'p> {
    /// The place.
    pub plan: &'p Plan,
    /// The species.
    pub species: &'p [Species],
    /// The groups.
    pub groups: Vec<Group>,
    /// Each animal's group.
    pub group: Vec<u32>,
    /// Each animal's rank in its group.
    pub member: Vec<u32>,
    /// Each animal's seed.
    pub seed: Vec<u64>,
    /// Where each is.
    pub pos: Vec<Vec3>,
    /// How each moves, m/s.
    pub vel: Vec<Vec3>,
    /// What each is doing with its body.
    pub mode: Vec<Mode>,
    /// Where each heads: its spot at a site, the next shrub, the next hop.
    pub spot: Vec<Vec3>,
    /// Points each passes before its spot, in order.
    pub path: Vec<Vec<Vec3>>,
    /// Each one's own timer: a hop's, a browse's, a rest's end, steps.
    pub until: Vec<u64>,
    /// Metres each has gone.
    pub travelled: Vec<f32>,
    /// Steps since the start of day 0.
    pub tick: u64,
    /// Every animal that fled because it saw a threat itself (its group went with it).
    pub flushes: Vec<Flush>,
    /// Times a flying animal ended a step inside a building, a wall or the ground (it is lifted
    /// out); the tests hold it at zero.
    pub contacts: u64,
    /// The contacts by species.
    pub contacts_by: Vec<u64>,
    threats: Vec<Disturber>,
    source: Vec<u32>,
    grid: ThreatGrid,
    turn: Vec<(f32, f32)>,
}

impl<'p> Fauna<'p> {
    /// The animals of `roster` on `plan` from `seed`, at `seconds` past midnight of day 0, each
    /// group already where its routine puts it then.
    pub fn new(
        plan: &'p Plan,
        species: &'p [Species],
        roster: &Roster,
        seed: u64,
        seconds: f32,
    ) -> Self {
        let mut f = Self {
            plan,
            species,
            groups: Vec::new(),
            group: Vec::new(),
            member: Vec::new(),
            seed: Vec::new(),
            pos: Vec::new(),
            vel: Vec::new(),
            mode: Vec::new(),
            spot: Vec::new(),
            path: Vec::new(),
            until: Vec::new(),
            travelled: Vec::new(),
            tick: ticks(seconds),
            flushes: Vec::new(),
            contacts: 0,
            contacts_by: vec![0; species.len()],
            threats: Vec::new(),
            source: Vec::new(),
            grid: ThreatGrid::default(),
            turn: species
                .iter()
                .map(|s| {
                    let a = s.turn.to_radians() * STEP;
                    (dmath::cos(a), dmath::sin(a))
                })
                .collect(),
        };
        let mut taken = vec![false; plan.parts.buildings.len()];
        for &(s, k) in &roster.groups {
            for n in 0..k {
                let g = f.groups.len() as u64;
                let gseed = bits(seed, g, u64::from(s));
                let sp = &species[s as usize];
                let [lo, hi] = sp.group;
                let len = lo + (bits(gseed, 1, 0) % u64::from(hi - lo + 1)) as u32;
                let home = f.home_for(sp.behaviour, gseed, n, &mut taken);
                let first = f.pos.len() as u32;
                for m in 0..len {
                    f.group.push(g as u32);
                    f.member.push(m);
                    f.seed.push(bits(gseed, 2, u64::from(m)));
                    f.pos.push(Vec3::ZERO);
                    f.vel.push(Vec3::ZERO);
                    f.mode.push(Mode::Rest);
                    f.spot.push(Vec3::ZERO);
                    f.path.push(Vec::new());
                    f.until.push(0);
                    f.travelled.push(0.0);
                }
                f.groups.push(Group {
                    species: s,
                    first,
                    len,
                    seed: gseed,
                    home,
                    activity: Activity::Night,
                    then: Activity::Night,
                    until: 0,
                    stay: 0,
                    goal: Vec3::ZERO,
                    centre: Vec3::ZERO,
                    radius: 0.0,
                    phase: 0.0,
                    node: [NONE; 2],
                    landed: 0,
                    decisions: 0,
                    yard: None,
                    leash: None,
                });
                let g = g as usize;
                match sp.behaviour {
                    Behaviour::Cat => {
                        // Indoors at its door to begin with.
                        let yard = f.yard(home);
                        f.pos[first as usize] = yard.door.extend(0.0);
                        f.mode[first as usize] = Mode::Hidden;
                        f.groups[g].yard = Some(yard);
                    }
                    Behaviour::Dog => {
                        let door = plan.parts.buildings[home as usize].door;
                        f.groups[g].leash = Some(Leash {
                            owner: door.extend(1.0),
                            vel: Vec3::ZERO,
                            route: Vec::new(),
                            leg: 0,
                            pause: 0,
                            pace: between(gseed, 3, 0, 1.0, 1.4),
                        });
                    }
                    _ => {}
                }
                f.decide(g, true);
            }
        }
        f
    }

    /// How many animals there are.
    pub fn len(&self) -> usize {
        self.pos.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.pos.is_empty()
    }

    /// Seconds since midnight now.
    pub fn seconds(&self) -> f32 {
        (self.tick % DAY) as f32 / HZ as f32
    }

    /// Animal `i`'s species.
    pub fn species_of(&self, i: usize) -> &Species {
        &self.species[self.groups[self.group[i] as usize].species as usize]
    }

    /// A home for a group of `b`: a colony's tree near the houses, a roof by the square, the
    /// sea off the quay, a house (each cat and dog its own).
    fn home_for(&self, b: Behaviour, seed: u64, n: u32, taken: &mut [bool]) -> u32 {
        let plan = self.plan;
        let pick = |list: &[u32]| {
            if list.is_empty() {
                NONE
            } else {
                list[(bits(seed, 9, u64::from(n)) % list.len() as u64) as usize]
            }
        };
        let sites_of = |kind: SiteKind| -> Vec<u32> {
            (0..plan.sites.len() as u32)
                .filter(|&s| plan.sites[s as usize].kind == kind)
                .collect()
        };
        match b {
            Behaviour::Sparrow => {
                let near_houses: Vec<u32> = sites_of(SiteKind::Tree)
                    .into_iter()
                    .filter(|&s| {
                        let p = plan.sites[s as usize].pos.truncate();
                        plan.parts.buildings.iter().any(|b| {
                            (plan::nearest_on_outline(p, &b.footprint) - p).length() < 25.0
                        })
                    })
                    .collect();
                pick(&near_houses)
            }
            Behaviour::Pigeon => {
                let centre = plan
                    .parts
                    .squares
                    .first()
                    .map_or(Vec2::splat(plan.parts.size * 0.5), |s| plan::centroid(s));
                let mut roofs = sites_of(SiteKind::Roof);
                roofs.sort_by(|&a, &b| {
                    let d = |s: u32| (plan.sites[s as usize].pos.truncate() - centre).length();
                    d(a).total_cmp(&d(b)).then(a.cmp(&b))
                });
                roofs.truncate(6);
                pick(&roofs)
            }
            Behaviour::Gull => pick(&sites_of(SiteKind::Water)),
            Behaviour::Swift => {
                let with_nests: Vec<u32> = (0..plan.nests.len() as u32)
                    .filter(|&b| plan.nests[b as usize][0] != NONE)
                    .collect();
                pick(&with_nests)
            }
            Behaviour::Cat | Behaviour::Dog => {
                let nb = taken.len() as u64;
                if nb == 0 {
                    return NONE;
                }
                let start = bits(seed, 10, u64::from(n)) % nb;
                for k in 0..nb {
                    let b = ((start + k) % nb) as usize;
                    if !taken[b] {
                        taken[b] = true;
                        return b as u32;
                    }
                }
                start as u32
            }
            Behaviour::Goat | Behaviour::Fish => 0,
        }
    }

    /// A cat's yard at house `b`: its garden's and its neighbours' (those seen from its own
    /// garden's middle within 45 m without passing a building), the wall tops seen from there,
    /// and places along its front within 15 m of its door.
    fn yard(&self, b: u32) -> Yard {
        let plan = self.plan;
        let house = &plan.parts.buildings[b as usize];
        let door = house.door;
        let own = plan.parts.gardens.iter().find(|g| g.house == b as usize);
        let clear = |a: Vec2, c: Vec2| {
            let n = ((c - a).length() / 1.0).ceil().max(1.0) as i32;
            (0..=n).all(|k| {
                plan.building_at(a + (c - a) * (k as f32 / n as f32))
                    .is_none()
            })
        };
        let mut garden = Vec::new();
        let (hub, flap) = match own {
            Some(g) => {
                let hub = plan::centroid(&g.polygon);
                let hc = plan::centroid(&house.footprint);
                let edge = plan::nearest_on_outline(hc, &g.polygon);
                let flap = edge + (hub - edge).normalize_or_zero() * 0.5;
                for o in &plan.parts.gardens {
                    let c = plan::centroid(&o.polygon);
                    if (c - hub).length() < 45.0 && clear(hub, c) {
                        for k in 0..3u64 {
                            let (lo, hi) = o.polygon.iter().fold(
                                (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
                                |(lo, hi), &p| (lo.min(p), hi.max(p)),
                            );
                            let u = Vec2::new(unit(u64::from(b), k, 1), unit(u64::from(b), k, 2));
                            let p = lo + (hi - lo) * (Vec2::splat(0.15) + u * 0.7);
                            if plan::inside(p, &o.polygon) {
                                garden.push(p.extend(0.0));
                            }
                        }
                    }
                }
                for s in &plan.sites {
                    let p = s.pos.truncate();
                    if s.kind == SiteKind::Wall && (p - hub).length() < 45.0 && clear(hub, p) {
                        garden.push(s.pos);
                    }
                }
                (hub, flap)
            }
            None => (door, door),
        };
        let edge = house
            .footprint
            .iter()
            .enumerate()
            .map(|(k, &a)| (a, house.footprint[(k + 1) % house.footprint.len()]))
            .min_by(|x, y| {
                let d =
                    |e: &(Vec2, Vec2)| (plan::nearest_on_segment(door, e.0, e.1) - door).length();
                d(x).total_cmp(&d(y))
            })
            .unwrap_or((door, door));
        let along = (edge.1 - edge.0).normalize_or_zero();
        let outward = (door - plan::nearest_on_segment(door, edge.0, edge.1)).normalize_or_zero();
        let mut street = vec![door.extend(0.0)];
        for s in [-12.0, -6.0, 5.0, 11.0] {
            let p = door + along * s + outward * 1.5;
            if clear(door + outward * 1.5, p) && !plan.in_water(p) {
                street.push(p.extend(0.0));
            }
        }
        Yard {
            garden,
            street,
            hub,
            flap,
            door,
            in_garden: false,
        }
    }

    /// Steps to the next `seconds` past midnight (at least one step).
    fn to(&self, seconds: f32) -> u64 {
        let now = self.tick % DAY;
        let at = ((seconds * HZ as f32).ceil() as u64) % DAY;
        if at > now { at - now } else { at + DAY - now }
    }

    /// A draw in `[lo, hi)` for group `g`'s next decision.
    fn draw(&mut self, g: usize, salt: u64, lo: f32, hi: f32) -> f32 {
        let gr = &self.groups[g];
        between(gr.seed, gr.decisions, salt, lo, hi)
    }

    /// One step: the threats gathered (`outside`: people and vehicles, with the fauna's own dogs,
    /// owners and cats), each group's routine, each animal's move.
    pub fn step(&mut self, outside: &[Disturber]) {
        self.gather(outside);
        for g in 0..self.groups.len() {
            self.routine(g);
        }
        for i in 0..self.len() {
            let before = self.pos[i];
            self.update(i);
            self.travelled[i] += (self.pos[i] - before).length();
        }
        self.tick += 1;
    }

    /// Runs `steps` steps with `outside` from `passers` (none if `None`).
    pub fn run(&mut self, steps: u64, passers: Option<&crate::passers::Passers>) {
        let mut out = Vec::new();
        for _ in 0..steps {
            out.clear();
            if let Some(p) = passers {
                p.at(self.seconds(), self.plan, &mut out);
            }
            self.step(&out);
        }
    }

    /// The threats this step, in a grid.
    fn gather(&mut self, outside: &[Disturber]) {
        self.threats.clear();
        self.source.clear();
        for d in outside {
            self.threats.push(*d);
            self.source.push(NONE);
        }
        for g in &self.groups {
            let sp = &self.species[g.species as usize];
            match sp.behaviour {
                Behaviour::Dog => {
                    if let Some(l) = &g.leash
                        && g.activity == Activity::Walk
                    {
                        self.threats.push(Disturber {
                            pos: l.owner,
                            vel: l.vel,
                            threat: Threat::Person,
                        });
                        self.source.push(NONE);
                        let i = g.first as usize;
                        self.threats.push(Disturber {
                            pos: self.pos[i],
                            vel: self.vel[i],
                            threat: Threat::Dog,
                        });
                        self.source.push(i as u32);
                    }
                }
                Behaviour::Cat => {
                    let i = g.first as usize;
                    if self.mode[i] != Mode::Hidden {
                        self.threats.push(Disturber {
                            pos: self.pos[i],
                            vel: self.vel[i],
                            threat: Threat::Cat,
                        });
                        self.source.push(i as u32);
                    }
                }
                _ => {}
            }
        }
        let side = (self.plan.parts.size / THREAT_CELL) as usize + 1;
        let cell = |p: Vec3| {
            let c = |v: f32| ((v / THREAT_CELL).floor() as i64).clamp(0, side as i64 - 1) as usize;
            c(p.y) * side + c(p.x)
        };
        let grid = &mut self.grid;
        grid.side = side;
        grid.start.clear();
        grid.start.resize(side * side + 1, 0);
        for t in &self.threats {
            grid.start[cell(t.pos) + 1] += 1;
        }
        for k in 0..side * side {
            grid.start[k + 1] += grid.start[k];
        }
        grid.items.clear();
        grid.items.resize(self.threats.len(), 0);
        let mut fill = grid.start.clone();
        for (k, t) in self.threats.iter().enumerate() {
            let c = cell(t.pos);
            grid.items[fill[c] as usize] = k as u32;
            fill[c] += 1;
        }
    }

    /// The distance at which animal `i` flees `threat` coming at it (`approaching`).
    pub fn threshold(&self, i: usize, threat: Threat, approaching: bool) -> f32 {
        let s = self.groups[self.group[i] as usize].species as usize;
        self.threshold_of(s, threat, approaching)
    }

    /// The distance at which species `s` flees `threat` (coming at it if `approaching`).
    pub fn threshold_of(&self, s: usize, threat: Threat, approaching: bool) -> f32 {
        let sp = &self.species[s];
        let scale = if self.plan.parts.urban && sp.behaviour.flies() {
            URBAN
        } else {
            1.0
        };
        let base = match threat {
            Threat::Person => sp.flee_person,
            Threat::Vehicle { limit } => sp.flee_vehicle * limit / 50.0,
            Threat::Dog => sp.flee_dog,
            Threat::Cat => sp.flee_cat,
        } * scale;
        if approaching { base } else { base * PASSING }
    }

    /// Whether a place at `at` is quiet for species `s` now: no threat within the distance it
    /// would flee it at, and a margin of 3 m.
    fn quiet(&self, s: usize, at: Vec2) -> bool {
        self.threats.iter().all(|t| {
            let thr = self.threshold_of(s, t.threat, true);
            thr <= 0.0 || (t.pos.truncate() - at).length() > thr + 3.0
        })
    }

    /// One of the five places of `list` (at `pos_of`) nearest `from`, leaving out the one it is
    /// at (within 3 m) and, if `calm`, those not [`Fauna::quiet`] for species `s`; by `r`.
    fn pick_near(
        &self,
        s: usize,
        list: &[u32],
        pos_of: &dyn Fn(u32) -> Vec2,
        from: Vec2,
        calm: bool,
        r: f32,
    ) -> Option<u32> {
        let mut near: Vec<(f32, u32)> = list
            .iter()
            .map(|&k| ((pos_of(k) - from).length(), k))
            .filter(|&(d, k)| d > 3.0 && (!calm || self.quiet(s, pos_of(k))))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        near.truncate(5);
        near.get((r * near.len() as f32) as usize).map(|x| x.1)
    }

    /// The velocity `v` from `p` if its next 0.3 s is clear of buildings and walls; else the
    /// nearest turned from `heading` by up to twice species `s`'s turn either way, level or
    /// climbing; else a third of `v`, or a climb on the spot.
    fn reachable(&self, p: Vec3, v: Vec3, heading: Vec3, s: usize) -> Vec3 {
        let ok =
            |u: Vec3| !self.plan.solid(p + u * STEP, 0.05) && !self.plan.solid(p + u * 0.3, 0.05);
        if ok(v) {
            return v;
        }
        let sp = &self.species[s];
        let speed = v.length().max(1.0);
        let max = (sp.turn * 2.0 * STEP).min(180.0).to_radians();
        let h = heading.truncate().normalize_or(Vec2::X);
        for f in [0.25f32, 0.5, 0.75, 1.0] {
            let (sn, c) = dmath::sin_cos(max * f);
            for sign in [1.0f32, -1.0] {
                let r = Vec2::new(h.x * c - h.y * sn * sign, h.x * sn * sign + h.y * c);
                for pitch in [0.0f32, 0.5] {
                    let mut u = r.extend(pitch).normalize_or(Vec3::Z) * speed;
                    u.z = u.z.clamp(-sp.climb * 1.5, sp.climb.max(1.0));
                    if ok(u) {
                        return u;
                    }
                }
            }
        }
        if ok(v * 0.3) {
            return v * 0.3;
        }
        Vec3::new(0.0, 0.0, sp.climb.max(1.0))
    }

    /// Whether animal `i` reacts to threats now: on or near the ground, not hidden, not already
    /// fleeing.
    fn alert(&self, i: usize) -> bool {
        let g = &self.groups[self.group[i] as usize];
        if g.activity == Activity::Flee {
            return false;
        }
        let z = self.pos[i].z;
        match self.species_of(i).behaviour {
            Behaviour::Sparrow | Behaviour::Pigeon | Behaviour::Gull => {
                matches!(self.mode[i], Mode::Ground | Mode::Rest | Mode::Water) && z < SAFE_BIRD
            }
            Behaviour::Cat => matches!(self.mode[i], Mode::Rest | Mode::Walk) && z < SAFE_CAT,
            Behaviour::Goat => matches!(self.mode[i], Mode::Rest | Mode::Walk),
            Behaviour::Fish => self.mode[i] == Mode::Water,
            Behaviour::Swift | Behaviour::Dog => false,
        }
    }

    /// The most pressing threat to animal `i`: its distance over its threshold (under 1: flee),
    /// where it is, what it is, the distance and the threshold.
    fn danger(&self, i: usize) -> Option<(f32, Vec3, Threat, f32, f32)> {
        let p = self.pos[i];
        let side = self.grid.side as i64;
        let cx = (p.x / THREAT_CELL).floor() as i64;
        let cy = (p.y / THREAT_CELL).floor() as i64;
        let mut best: Option<(f32, Vec3, Threat, f32, f32)> = None;
        for y in (cy - 1).max(0)..=(cy + 1).min(side - 1) {
            for x in (cx - 1).max(0)..=(cx + 1).min(side - 1) {
                let c = (y * side + x) as usize;
                for k in self.grid.start[c]..self.grid.start[c + 1] {
                    let t = self.grid.items[k as usize] as usize;
                    if self.source[t] == i as u32 {
                        continue;
                    }
                    let d = &self.threats[t];
                    let off = (p - d.pos).truncate();
                    let dist = off.length();
                    let toward = off / dist.max(1e-3);
                    let approaching = d.vel.truncate().dot(toward) > 0.2;
                    let thr = self.threshold(i, d.threat, approaching);
                    if thr <= 0.0 {
                        continue;
                    }
                    let r = dist / thr;
                    if best.is_none_or(|b| r < b.0) {
                        best = Some((r, d.pos, d.threat, dist, thr));
                    }
                }
            }
        }
        best
    }

    /// Group `g`'s routine this step: a decision when its activity ends, a circle's or a
    /// party's goal moved on, a dog's owner walked.
    fn routine(&mut self, g: usize) {
        let sp = &self.species[self.groups[g].species as usize];
        let b = sp.behaviour;
        let now = self.tick;
        let act = self.groups[g].activity;
        match act {
            Activity::Circle => {
                let gr = &mut self.groups[g];
                let w = sp.cruise / gr.radius.max(5.0);
                gr.phase += w * STEP;
                if gr.phase > std::f32::consts::TAU {
                    gr.phase -= std::f32::consts::TAU;
                }
                let (s, c) = dmath::sin_cos(gr.phase);
                gr.goal = gr.centre + Vec3::new(c, s, 0.0) * gr.radius;
            }
            Activity::Party => {
                let c = self.centroid(g, Some(Mode::Air));
                let gr = &self.groups[g];
                if (gr.goal - c).truncate().length() < 10.0 {
                    self.next_waypoint(g);
                }
            }
            _ => {}
        }
        if b == Behaviour::Dog {
            self.walk_owner(g);
        }
        if now >= self.groups[g].until {
            match b {
                Behaviour::Cat => {
                    let i = self.groups[g].first as usize;
                    if matches!(self.mode[i], Mode::Rest | Mode::Hidden) {
                        self.decide(g, false);
                    }
                }
                Behaviour::Dog => {
                    if act == Activity::Night {
                        self.decide(g, false);
                    }
                }
                _ => self.decide(g, false),
            }
        }
    }

    /// The middle of group `g`'s members (in `mode`, or all).
    pub fn centroid(&self, g: usize, mode: Option<Mode>) -> Vec3 {
        let gr = &self.groups[g];
        let (mut s, mut n) = (Vec3::ZERO, 0u32);
        for i in gr.first..gr.first + gr.len {
            if mode.is_none_or(|m| self.mode[i as usize] == m) {
                s += self.pos[i as usize];
                n += 1;
            }
        }
        if n == 0 {
            self.pos[gr.first as usize]
        } else {
            s / n as f32
        }
    }

    /// The next decision for group `g`; `snap` puts the members there at once (the start).
    fn decide(&mut self, g: usize, snap: bool) {
        let b = self.species[self.groups[g].species as usize].behaviour;
        match b {
            Behaviour::Sparrow | Behaviour::Pigeon | Behaviour::Gull | Behaviour::Swift => {
                self.decide_bird(g, snap)
            }
            Behaviour::Cat => self.decide_cat(g, snap),
            Behaviour::Dog => self.decide_dog(g, snap),
            Behaviour::Goat => self.decide_goat(g, snap),
            Behaviour::Fish => self.decide_fish(g, snap),
        }
        self.groups[g].decisions += 1;
    }

    /// Sites of `kinds` within `range` of `from`.
    fn sites_near(&self, kinds: &[SiteKind], from: Vec2, range: f32) -> Vec<u32> {
        (0..self.plan.sites.len() as u32)
            .filter(|&s| {
                let site = &self.plan.sites[s as usize];
                kinds.contains(&site.kind) && (site.pos.truncate() - from).length() <= range
            })
            .collect()
    }

    /// Feeding places of `kinds` within `range` of `from`.
    fn patches_near(&self, kinds: &[PatchKind], from: Vec2, range: f32) -> Vec<u32> {
        (0..self.plan.patches.len() as u32)
            .filter(|&p| {
                let patch = &self.plan.patches[p as usize];
                kinds.contains(&patch.kind) && (patch.pos - from).length() <= range
            })
            .collect()
    }

    /// Where a bird group lives from: its home site's place, or its house's.
    fn bird_home(&self, g: usize) -> Vec2 {
        let gr = &self.groups[g];
        let b = self.species[gr.species as usize].behaviour;
        if gr.home == NONE {
            return Vec2::splat(self.plan.parts.size * 0.5);
        }
        if b == Behaviour::Swift {
            plan::centroid(&self.plan.parts.buildings[gr.home as usize].footprint)
        } else {
            self.plan.sites[gr.home as usize].pos.truncate()
        }
    }

    /// A bird group's next activity: by the hour, then by turns of feeding and resting, each
    /// at one of the five nearest places (a feeding place with no threat about it).
    fn decide_bird(&mut self, g: usize, snap: bool) {
        let s = self.groups[g].species as usize;
        let sp = &self.species[s];
        let (b, range) = (sp.behaviour, sp.range);
        let sun = self.plan.parts.sun;
        let sec = self.seconds();
        let home = self.bird_home(g);
        let here = self.centroid(g, None).truncate();
        let prev = self.groups[g].activity;
        let then_prev = self.groups[g].then;
        let r = self.draw(g, 1, 0.0, 1.0);
        let minutes = |f: &mut Self, lo: f32, hi: f32| ticks(f.draw(g, 2, lo, hi) * 60.0);
        let plan = self.plan;
        let site_at = |k: u32| plan.sites[k as usize].pos.truncate();
        let patch_at = |k: u32| plan.patches[k as usize].pos;
        let fed =
            prev == Activity::Forage || (prev == Activity::Travel && then_prev == Activity::Forage);
        match b {
            Behaviour::Sparrow => {
                let (wake, sleep) = (sun.rise - 900.0, sun.set + 600.0);
                if sec < wake || sec >= sleep {
                    let until = self.to(wake);
                    let home = self.groups[g].home;
                    return self.go_site(g, Activity::Night, home, until, snap);
                }
                let left = self.to(sleep);
                let midday = (13.0 * 3600.0..16.0 * 3600.0).contains(&sec);
                if fed {
                    let kinds = [SiteKind::Tree, SiteKind::Wall, SiteKind::Roof];
                    let sites = self.sites_near(&kinds, home, range);
                    let dur = if midday {
                        minutes(self, 10.0, 25.0)
                    } else {
                        minutes(self, 2.0, 8.0)
                    };
                    if let Some(k) = self.pick_near(s, &sites, &site_at, here, false, r) {
                        return self.go_site(g, Activity::Perch, k, dur.min(left), snap);
                    }
                }
                let kinds = [PatchKind::Garden, PatchKind::Square, PatchKind::Street];
                let patches = self.patches_near(&kinds, home, range);
                let dur = minutes(self, 2.0, 8.0);
                if let Some(k) = self.pick_near(s, &patches, &patch_at, here, true, r) {
                    return self.go_patch(g, k, dur.min(left), snap);
                }
                let home = self.groups[g].home;
                self.go_site(g, Activity::Perch, home, dur.min(left), snap)
            }
            Behaviour::Pigeon => {
                let (wake, sleep) = (sun.rise + 1800.0, sun.set - 1800.0);
                if sec < wake || sec >= sleep {
                    let until = self.to(wake);
                    let home = self.groups[g].home;
                    return self.go_site(g, Activity::Night, home, until, snap);
                }
                let left = self.to(sleep);
                let square = self
                    .plan
                    .parts
                    .squares
                    .first()
                    .map_or(home, |s| plan::centroid(s));
                if !snap && r < 0.12 && prev != Activity::Circle {
                    let dur = ticks(self.draw(g, 3, 90.0, 180.0));
                    let radius = self.draw(g, 4, 35.0, 60.0);
                    let alt = self.draw(g, 5, 16.0, 24.0);
                    return self.circle(g, square.extend(alt), radius, dur.min(left));
                }
                if fed {
                    let roofs = self.sites_near(&[SiteKind::Roof], square, 60.0);
                    if let Some(k) = self.pick_near(s, &roofs, &site_at, here, false, r) {
                        let dur = minutes(self, 5.0, 15.0);
                        return self.go_site(g, Activity::Perch, k, dur.min(left), snap);
                    }
                }
                let kinds = [PatchKind::Square, PatchKind::Street];
                let patches = self.patches_near(&kinds, square, range * 0.4);
                let dur = minutes(self, 8.0, 20.0);
                if let Some(k) = self.pick_near(s, &patches, &patch_at, here, true, r) {
                    return self.go_patch(g, k, dur.min(left), snap);
                }
                let home = self.groups[g].home;
                self.go_site(g, Activity::Perch, home, dur.min(left), snap)
            }
            Behaviour::Gull => {
                let (wake, sleep) = (sun.rise - 1800.0, sun.set + 1800.0);
                if sec < wake || sec >= sleep {
                    let until = self.to(wake);
                    let home = self.groups[g].home;
                    return self.go_site(g, Activity::Night, home, until, snap);
                }
                let left = self.to(sleep);
                let quay = &self.plan.parts.quay;
                let coast = if quay.is_empty() {
                    home
                } else {
                    (quay[0] + quay[quay.len() - 1]) * 0.5
                };
                let morning = sec < 11.0 * 3600.0;
                let forage = if morning { 0.45 } else { 0.2 };
                let r2 = self.draw(g, 8, 0.0, 1.0);
                if !snap && r < 0.3 {
                    let dur = minutes(self, 3.0, 10.0);
                    let radius = self.draw(g, 4, 40.0, 70.0);
                    let alt = self.draw(g, 5, 25.0, 55.0);
                    let off = Vec2::new(self.draw(g, 6, -80.0, 80.0), self.draw(g, 7, -10.0, 40.0));
                    return self.circle(g, (coast + off).extend(alt), radius, dur.min(left));
                }
                if r < 0.3 + forage {
                    let patches = self.patches_near(&[PatchKind::Quay], coast, range);
                    if let Some(k) = self.pick_near(s, &patches, &patch_at, here, true, r2) {
                        let dur = minutes(self, 5.0, 15.0);
                        return self.go_patch(g, k, dur.min(left), snap);
                    }
                }
                if r < 0.75 {
                    let kinds = [SiteKind::Quay, SiteKind::Roof];
                    let perches = self.sites_near(&kinds, coast, 150.0);
                    if let Some(k) = self.pick_near(s, &perches, &site_at, here, false, r2) {
                        let dur = minutes(self, 5.0, 20.0);
                        return self.go_site(g, Activity::Perch, k, dur.min(left), snap);
                    }
                }
                let seas = self.sites_near(&[SiteKind::Water], coast, range);
                let dur = minutes(self, 10.0, 30.0);
                let k = self
                    .pick_near(s, &seas, &site_at, here, false, r2)
                    .unwrap_or(self.groups[g].home);
                self.go_site(g, Activity::Float, k, dur.min(left), snap)
            }
            Behaviour::Swift => {
                let (wake, sleep) = (sun.rise - 1800.0, sun.set + 1200.0);
                let party = sun.set - 4500.0;
                if sec < wake || sec >= sleep {
                    let until = self.to(wake);
                    return self.go_nests(g, until, snap);
                }
                if sec >= party {
                    let until = self.to(sleep);
                    return self.party(g, until, snap);
                }
                let dur = minutes(self, 5.0, 10.0).min(self.to(party));
                let half = self.plan.parts.size * 0.5;
                let c = Vec2::new(
                    self.draw(g, 6, half - 120.0, half + 120.0),
                    self.draw(g, 7, half - 140.0, half + 60.0),
                );
                let radius = self.draw(g, 4, 60.0, 120.0);
                let alt = self.draw(g, 5, 40.0, 90.0);
                if snap {
                    self.circle(g, c.extend(alt), radius, dur);
                    self.snap_air(g);
                } else {
                    self.circle(g, c.extend(alt), radius, dur);
                }
            }
            _ => {}
        }
    }

    /// Group `g` circles aloft about `centre` for `dur` steps.
    fn circle(&mut self, g: usize, centre: Vec3, radius: f32, dur: u64) {
        let now = self.tick;
        let phase = self.draw(g, 9, 0.0, std::f32::consts::TAU);
        let gr = &mut self.groups[g];
        gr.activity = Activity::Circle;
        gr.centre = centre;
        gr.radius = radius;
        gr.phase = phase;
        let (s, c) = dmath::sin_cos(phase);
        gr.goal = centre + Vec3::new(c, s, 0.0) * radius;
        gr.until = now + dur.max(1);
    }

    /// Places group `g`'s members aloft about its goal at once.
    fn snap_air(&mut self, g: usize) {
        let gr = self.groups[g].clone();
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            let s = self.seed[i];
            let off = Vec3::new(
                between(s, 1, 0, -6.0, 6.0),
                between(s, 2, 0, -6.0, 6.0),
                between(s, 3, 0, -2.0, 2.0),
            );
            // Never inside a building or a wall: drawn by its goal among the houses, a swift
            // started inside one and climbed out through its roof (#308's new lots).
            let mut p = gr.goal + off;
            for _ in 0..80 {
                if !self.plan.solid(p, 0.3) {
                    break;
                }
                p.z += 0.5;
            }
            self.pos[i] = p;
            self.vel[i] = Vec3::X * self.species[gr.species as usize].cruise;
            self.mode[i] = Mode::Air;
        }
    }

    /// Group `g` goes to site `s` for `activity`, `dur` steps once there.
    fn go_site(&mut self, g: usize, activity: Activity, s: u32, dur: u64, snap: bool) {
        if s == NONE {
            return self.circle(g, self.centroid(g, None).with_z(12.0), 30.0, dur);
        }
        let site = self.plan.sites[s as usize];
        let gr = self.groups[g].clone();
        let mode = match (activity, site.kind) {
            (_, SiteKind::Nest) => Mode::Hidden,
            (_, SiteKind::Water) => Mode::Water,
            _ => Mode::Rest,
        };
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            let k = self.member[i];
            let mut spot = site.spot(k, gr.len, bits(self.seed[i], u64::from(s), 7));
            // A crown over a roof: the spot moves to the site's middle.
            if self.plan.solid(spot, 0.0) {
                spot = site.pos;
            }
            if site.kind == SiteKind::Water {
                spot.z = 0.0;
            }
            self.spot[i] = spot;
            self.path[i].clear();
        }
        self.arrive_or_go(g, activity, mode, dur, snap);
        self.groups[g].goal = site.pos;
    }

    /// Group `g` goes to feed at patch `p`, `dur` steps once there.
    fn go_patch(&mut self, g: usize, p: u32, dur: u64, snap: bool) {
        let patch = self.plan.patches[p as usize];
        let gr = self.groups[g].clone();
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            let s = bits(self.seed[i], u64::from(p), gr.decisions);
            let a = (s & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
            let d = ((s >> 16) & 0xFFFF) as f32 / 65535.0;
            let (sn, cs) = dmath::sin_cos(a);
            let q = self.plan.open_ground(
                patch.pos + Vec2::new(cs, sn) * d.sqrt() * patch.radius,
                patch.pos,
            );
            self.spot[i] = q.extend(0.0);
            self.path[i].clear();
        }
        self.arrive_or_go(g, Activity::Forage, Mode::Ground, dur, snap);
        let gr = &mut self.groups[g];
        gr.goal = patch.pos.extend(0.0);
        gr.radius = patch.radius;
    }

    /// Group `g`'s swifts to their nests under the eaves of their house and its neighbours'.
    fn go_nests(&mut self, g: usize, dur: u64, snap: bool) {
        let gr = self.groups[g].clone();
        let nb = self.plan.nests.len() as u32;
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            let k = self.member[i];
            // A pair a cavity, two cavities a house, the next houses along.
            let mut house = (gr.home + k / 4) % nb.max(1);
            let mut tries = 0;
            while self.plan.nests[house as usize][0] == NONE && tries < nb {
                house = (house + 1) % nb;
                tries += 1;
            }
            let cav = self.plan.nests[house as usize];
            let s = if cav[1] != NONE && (k / 2) % 2 == 1 {
                cav[1]
            } else {
                cav[0]
            };
            let site = self.plan.sites[s as usize];
            self.spot[i] = site.spot(k % 2, 2, 0);
            self.path[i].clear();
        }
        self.arrive_or_go(g, Activity::Night, Mode::Hidden, dur, snap);
    }

    /// Group `g`'s screaming party along the streets until `until` steps from now.
    fn party(&mut self, g: usize, dur: u64, snap: bool) {
        let now = self.tick;
        let home = self.bird_home(g);
        let n0 = self.plan.nearest_node(home);
        {
            let gr = &mut self.groups[g];
            gr.activity = Activity::Party;
            gr.until = now + dur.max(1);
            gr.node = [n0, NONE];
        }
        self.next_waypoint(g);
        if snap {
            self.snap_air(g);
        }
    }

    /// A party's next street node, never straight back, at 7–13 m.
    fn next_waypoint(&mut self, g: usize) {
        let [at, came] = self.groups[g].node;
        if at == NONE {
            return;
        }
        let out: Vec<u32> = self.plan.adjacent[at as usize]
            .iter()
            .map(|&e| self.plan.other(e, at))
            .filter(|&n| n != came)
            .collect();
        let r = self.draw(g, 11, 0.0, 1.0);
        let alt = self.draw(g, 12, 7.0, 13.0);
        self.groups[g].decisions += 1;
        let next = out
            .get((r * out.len() as f32) as usize)
            .copied()
            .unwrap_or(came);
        if next == NONE {
            return;
        }
        let p = self.plan.parts.nodes[next as usize];
        let gr = &mut self.groups[g];
        gr.node = [next, at];
        gr.goal = p.extend(alt);
    }

    /// Group `g`'s members, their spots set, arrive at once (`snap`) or set out for them.
    fn arrive_or_go(&mut self, g: usize, activity: Activity, mode: Mode, dur: u64, snap: bool) {
        let now = self.tick;
        let gr = self.groups[g].clone();
        if snap {
            for i in gr.first..gr.first + gr.len {
                let i = i as usize;
                self.pos[i] = self.spot[i];
                self.vel[i] = Vec3::ZERO;
                self.mode[i] = mode;
                self.until[i] = now;
            }
            let gr = &mut self.groups[g];
            gr.activity = activity;
            gr.until = now + dur.max(1);
            return;
        }
        // Those already at their spots have arrived; a herd walks.
        let walker = self.species[gr.species as usize].behaviour == Behaviour::Goat;
        let mut landed = 0;
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            if self.path[i].is_empty() && (self.pos[i] - self.spot[i]).length() < 0.5 {
                self.mode[i] = mode;
                landed += 1;
            } else if walker {
                self.mode[i] = Mode::Walk;
            }
        }
        let gr = &mut self.groups[g];
        gr.activity = Activity::Travel;
        gr.then = activity;
        gr.stay = dur.max(1);
        gr.landed = landed;
        // A journey that takes too long is decided again.
        gr.until = now + ticks(240.0);
        if landed >= gr.len {
            gr.activity = activity;
            gr.until = now + gr.stay;
        }
    }

    /// A cat's next move: out to the garden or the street by the hour, rests and legs, home.
    fn decide_cat(&mut self, g: usize, snap: bool) {
        let i = self.groups[g].first as usize;
        let sun = self.plan.parts.sun;
        let sec = self.seconds();
        let wrap = |s: f32| s.rem_euclid(86_400.0);
        let dawn = wrap(sec - (sun.rise - 3600.0)) < 3.0 * 3600.0;
        let dusk = wrap(sec - (sun.set - 3600.0)) < 4.0 * 3600.0;
        let night = !dawn && !dusk && (sec > sun.set || sec < sun.rise);
        let p_out = if dawn || dusk {
            0.8
        } else if night {
            0.35
        } else {
            0.3
        };
        let r = self.draw(g, 1, 0.0, 1.0);
        let now = self.tick;
        let Some(mut yard) = self.groups[g].yard.clone() else {
            return;
        };
        let hidden = self.mode[i] == Mode::Hidden;
        if hidden {
            if snap || r >= p_out {
                // Stay in.
                let stay = ticks(self.draw(g, 2, 20.0, 60.0) * 60.0);
                let gr = &mut self.groups[g];
                gr.activity = Activity::Night;
                gr.until = now + stay;
                self.mode[i] = Mode::Hidden;
                self.pos[i] = yard.door.extend(0.0);
                return;
            }
            // Out by the flap or the door.
            yard.in_garden = !yard.garden.is_empty() && self.draw(g, 3, 0.0, 1.0) < 0.65;
            self.pos[i] = if yard.in_garden { yard.flap } else { yard.door }.extend(0.0);
            self.mode[i] = Mode::Rest;
        } else if r >= p_out {
            // Home.
            let gate = if yard.in_garden { yard.flap } else { yard.door };
            self.path[i] = vec![if yard.in_garden { yard.hub } else { yard.door }.extend(0.0)];
            self.spot[i] = gate.extend(0.0);
            self.mode[i] = Mode::Walk;
            let gr = &mut self.groups[g];
            gr.activity = Activity::Night;
            gr.until = u64::MAX;
            return;
        } else if self.draw(g, 4, 0.0, 1.0) < 0.25 && !yard.garden.is_empty() {
            // To the other side through the house.
            let gate = if yard.in_garden { yard.flap } else { yard.door };
            self.path[i] = vec![if yard.in_garden { yard.hub } else { yard.door }.extend(0.0)];
            self.spot[i] = gate.extend(0.0);
            self.mode[i] = Mode::Walk;
            let gr = &mut self.groups[g];
            gr.activity = Activity::Night;
            gr.until = u64::MAX;
            return;
        }
        let list = if yard.in_garden {
            &yard.garden
        } else {
            &yard.street
        };
        let t = list[(self.draw(g, 5, 0.0, 1.0) * list.len() as f32) as usize];
        let via = if yard.in_garden { yard.hub } else { yard.door };
        self.path[i] = vec![via.extend(0.0)];
        self.spot[i] = t;
        self.mode[i] = Mode::Walk;
        let midday = (11.0 * 3600.0..17.0 * 3600.0).contains(&sec);
        let rest = if midday {
            self.draw(g, 6, 10.0, 45.0)
        } else {
            self.draw(g, 6, 2.0, 30.0)
        };
        let gr = &mut self.groups[g];
        gr.activity = Activity::Out;
        gr.stay = ticks(rest * 60.0);
        gr.until = u64::MAX;
        gr.yard = Some(yard);
    }

    /// A dog's next walk: morning, sometimes noon, evening; or its walk begins.
    fn decide_dog(&mut self, g: usize, snap: bool) {
        let now = self.tick;
        let i = self.groups[g].first as usize;
        let seed = self.groups[g].seed;
        let day = now / DAY;
        let door = self.plan.parts.buildings[self.groups[g].home as usize].door;
        let sec = self.seconds();
        // Today's walks: (start, minutes), Forge's estimates (a UK survey: 34 minutes a walk).
        let walks = |d: u64| -> [(f32, f32); 3] {
            [
                (
                    7.0 * 3600.0 + between(seed, d, 1, -2700.0, 2700.0),
                    between(seed, d, 2, 20.0, 40.0),
                ),
                (
                    if unit(seed, d, 3) < 0.5 {
                        12.5 * 3600.0 + between(seed, d, 4, -1800.0, 1800.0)
                    } else {
                        -1.0
                    },
                    between(seed, d, 5, 10.0, 15.0),
                ),
                (
                    19.5 * 3600.0 + between(seed, d, 6, -3600.0, 3600.0),
                    between(seed, d, 7, 25.0, 50.0),
                ),
            ]
        };
        let today = walks(day);
        let due = !snap
            && self.groups[g].activity == Activity::Night
            && self.groups[g].then == Activity::Walk
            && self.groups[g].until <= now;
        if due {
            // Set out: the minutes are in `stay`.
            let minutes = self.groups[g].stay as f32;
            let pace = self.groups[g].leash.as_ref().map_or(1.2, |l| l.pace);
            let route = self.dog_route(g, door, pace * minutes * 60.0 * 0.5);
            let gr = &mut self.groups[g];
            gr.activity = Activity::Walk;
            gr.then = Activity::Night;
            gr.until = u64::MAX;
            if let Some(l) = &mut gr.leash {
                l.route = route;
                l.leg = 1;
                l.owner = door.extend(1.0);
                l.pause = now;
            }
            self.mode[i] = Mode::Walk;
            self.pos[i] = door.extend(0.0);
            return;
        }
        // Home until the next walk, today's or tomorrow's.
        let mut next = (f32::MAX, 30.0);
        for (k, list) in [today, walks(day + 1)].iter().enumerate() {
            for &(start, minutes) in list {
                let at = start + k as f32 * 86_400.0;
                if start >= 0.0 && at > sec && at < next.0 {
                    next = (at, minutes);
                }
            }
        }
        let gr = &mut self.groups[g];
        gr.activity = Activity::Night;
        gr.then = Activity::Walk;
        gr.until = now + ticks(next.0 - sec).max(1);
        gr.stay = next.1 as u64;
        self.mode[i] = Mode::Hidden;
        self.pos[i] = door.extend(0.0);
        if let Some(l) = &mut gr.leash {
            l.owner = door.extend(1.0);
            l.vel = Vec3::ZERO;
        }
    }

    /// A walk from `door`: the nearest street node, then a random walk on the streets that are
    /// not roads (limit under 50) until `half` metres, never straight back; then back the same
    /// way.
    fn dog_route(&mut self, g: usize, door: Vec2, half: f32) -> Vec<Vec2> {
        let plan = self.plan;
        let mut n = plan.nearest_node(door);
        let mut out = vec![door, plan.parts.nodes[n as usize]];
        let mut came = NONE;
        let mut length = (plan.parts.nodes[n as usize] - door).length();
        let mut k = 0;
        while length < half && k < 40 {
            let choices: Vec<u32> = plan.adjacent[n as usize]
                .iter()
                .copied()
                .filter(|&e| e != came && plan.parts.streets[e as usize].limit < 50.0)
                .collect();
            if choices.is_empty() {
                break;
            }
            let e = choices[(self.draw(g, 20 + k, 0.0, 1.0) * choices.len() as f32) as usize];
            let m = plan.other(e, n);
            length += (plan.parts.nodes[m as usize] - plan.parts.nodes[n as usize]).length();
            out.push(plan.parts.nodes[m as usize]);
            came = e;
            n = m;
            k += 1;
        }
        let back: Vec<Vec2> = out.iter().rev().skip(1).copied().collect();
        out.extend(back);
        out
    }

    /// A dog's owner walks their route at their pace, pausing at some corners for the dog.
    fn walk_owner(&mut self, g: usize) {
        let now = self.tick;
        if self.groups[g].activity != Activity::Walk {
            return;
        }
        let pause_draw = self.draw(g, 30, 0.0, 1.0);
        let pause_len = ticks(self.draw(g, 31, 4.0, 15.0));
        let i = self.groups[g].first as usize;
        let gr = &mut self.groups[g];
        let Some(l) = &mut gr.leash else { return };
        if now < l.pause {
            l.vel = Vec3::ZERO;
            return;
        }
        let target = l.route[(l.leg as usize).min(l.route.len().saturating_sub(1))];
        let p = l.owner.truncate();
        let to = target - p;
        let d = to.length();
        let step = l.pace * STEP;
        if d <= step {
            l.owner = target.extend(1.0);
            l.leg += 1;
            l.vel = Vec3::ZERO;
            if pause_draw < 0.4 {
                l.pause = now + pause_len;
            }
            gr.decisions += 1;
        } else {
            let v = to / d * l.pace;
            l.owner += (v * STEP).extend(0.0);
            l.vel = v.extend(0.0);
        }
        if l.leg as usize >= l.route.len() {
            // Home: the next walk is planned.
            let door = l.route.last().copied().unwrap_or(p);
            gr.activity = Activity::Night;
            gr.then = Activity::Night;
            self.mode[i] = Mode::Hidden;
            self.pos[i] = door.extend(0.0);
            self.decide_dog(g, false);
        }
    }

    /// Group `g`'s members still fleeing go back to `mode`: its flight is over.
    fn calm(&mut self, g: usize, mode: Mode) {
        let gr = &self.groups[g];
        for i in gr.first..gr.first + gr.len {
            if self.mode[i as usize] == Mode::Flee {
                self.mode[i as usize] = mode;
            }
        }
    }

    /// The herd's day: out of the fold mid-morning, browsing, the shade at midday, browsing,
    /// back at evening (Forge's estimates after the ranges' time budgets).
    fn decide_goat(&mut self, g: usize, snap: bool) {
        let now = self.tick;
        self.calm(g, Mode::Walk);
        let seed = self.groups[g].seed;
        let day = now / DAY;
        let sec = self.seconds();
        // On whole steps, as `to` counts them.
        let q = |s: f32| (s * HZ as f32).ceil() / HZ as f32;
        let out = q(8.5 * 3600.0 + between(seed, day, 1, -1200.0, 1200.0));
        let shade = q(12.5 * 3600.0 + between(seed, day, 2, -900.0, 900.0));
        let after = q(15.5 * 3600.0 + between(seed, day, 3, -900.0, 900.0));
        let back = q(19.25 * 3600.0 + between(seed, day, 4, -900.0, 900.0));
        let plan = self.plan;
        let (gate_in, gate_out) = gate(plan);
        let gr = self.groups[g].clone();
        let prev = gr.activity;
        let night = sec < out || sec >= back;
        if night {
            let until = self.to(out);
            if !snap
                && (prev == Activity::Night
                    || prev == Activity::Travel && gr.then == Activity::Night)
            {
                self.groups[g].until = now + until;
                return;
            }
            for i in gr.first..gr.first + gr.len {
                let i = i as usize;
                self.spot[i] = fold_spot(plan, self.seed[i]);
                self.path[i] = if plan::inside(self.pos[i].truncate(), &plan.parts.fold) || snap {
                    Vec::new()
                } else {
                    vec![gate_out, gate_in]
                };
            }
            return self.arrive_or_go(g, Activity::Night, Mode::Rest, until, snap);
        }
        let in_fold = plan::inside(self.centroid(g, None).truncate(), &plan.parts.fold);
        let via = |f: &Self, i: usize| {
            if plan::inside(f.pos[i].truncate(), &plan.parts.fold) && !snap {
                vec![gate_in, gate_out]
            } else {
                Vec::new()
            }
        };
        if (shade..after).contains(&sec) {
            if !snap
                && (prev == Activity::Shade
                    || prev == Activity::Travel && gr.then == Activity::Shade)
            {
                self.groups[g].until = now + self.to(after);
                return;
            }
            // The widest crown in the pasture near the herd.
            let c = if in_fold {
                gate_out.truncate()
            } else {
                self.centroid(g, None).truncate()
            };
            let tree = plan
                .parts
                .trees
                .iter()
                .filter(|t| plan.in_pasture(t.pos) && (t.pos - c).length() < 150.0)
                .fold(None, |best: Option<plan::Tree>, t| match best {
                    Some(b) if b.crown >= t.crown => Some(b),
                    _ => Some(*t),
                });
            if let Some(t) = tree {
                for i in gr.first..gr.first + gr.len {
                    let i = i as usize;
                    let s = self.seed[i];
                    let a = between(s, day, 8, 0.0, std::f32::consts::TAU);
                    let d = between(s, day, 9, 1.0, t.crown.max(2.0) * 0.8);
                    let (sn, cs) = dmath::sin_cos(a);
                    let mut q = t.pos + Vec2::new(cs, sn) * d;
                    if !plan.in_pasture(q) {
                        q = t.pos;
                    }
                    self.spot[i] = q.extend(0.0);
                    self.path[i] = via(self, i);
                }
                let until = self.to(after);
                self.groups[g].goal = t.pos.extend(0.0);
                return self.arrive_or_go(g, Activity::Shade, Mode::Rest, until, snap);
            }
        }
        // Browse: a new goal 25–60 m on, among the shrubs.
        let left = if sec < shade {
            self.to(shade)
        } else {
            self.to(back)
        };
        let from = if in_fold {
            gate_out.truncate()
        } else {
            self.centroid(g, None).truncate()
        };
        let goal = self.browse_goal(g, from);
        let dur = ticks(self.draw(g, 5, 8.0, 15.0) * 60.0).min(left);
        let gr = &mut self.groups[g];
        gr.goal = goal.extend(0.0);
        gr.activity = Activity::Graze;
        gr.until = now + dur.max(1);
        let gr = gr.clone();
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            if snap {
                let q = self.shrub_near(goal, self.seed[i], now);
                self.pos[i] = q.extend(0.0);
                self.spot[i] = self.pos[i];
                self.mode[i] = Mode::Rest;
                self.until[i] = now;
            } else if prev != Activity::Graze && prev != Activity::Flee {
                self.path[i] = via(self, i);
                self.spot[i] = self.shrub_near(goal, self.seed[i], now).extend(0.0);
                self.mode[i] = Mode::Walk;
            }
        }
    }

    /// A shrub-rich place 25–60 m from `from` in the pasture.
    fn browse_goal(&mut self, g: usize, from: Vec2) -> Vec2 {
        let shrubs = &self.plan.parts.shrubs;
        for k in 0..24 {
            let a = self.draw(g, 40 + k, 0.0, std::f32::consts::TAU);
            let d = self.draw(g, 70 + k, 25.0, 60.0);
            let (s, c) = dmath::sin_cos(a);
            let q = from + Vec2::new(c, s) * d;
            if self.plan.in_pasture(q) && !plan::inside(q, &self.plan.parts.fold) {
                // Move it onto the nearest shrub.
                let near = shrubs
                    .iter()
                    .copied()
                    .fold((f32::MAX, q), |b, p| {
                        let e = (p - q).length_squared();
                        if e < b.0 { (e, p) } else { b }
                    })
                    .1;
                return near;
            }
        }
        from
    }

    /// A shrub within 14 m of `goal`, or a place near it.
    fn shrub_near(&self, goal: Vec2, seed: u64, now: u64) -> Vec2 {
        let shrubs = &self.plan.parts.shrubs;
        let mut best = None;
        let mut count = 0u64;
        for &p in shrubs {
            if (p - goal).length_squared() < 196.0 {
                count += 1;
                // Reservoir sampling: each near shrub equally likely.
                if bits(seed, now, count).is_multiple_of(count) {
                    best = Some(p);
                }
            }
        }
        best.unwrap_or_else(|| {
            let a = between(seed, now, 1, 0.0, std::f32::consts::TAU);
            let (s, c) = dmath::sin_cos(a);
            let q = goal + Vec2::new(c, s) * between(seed, now, 2, 1.0, 8.0);
            if self.plan.in_pasture(q) { q } else { goal }
        })
    }

    /// A school's next goal along the quay: near the surface by day (sometimes grazing the
    /// wall), deeper and slower at night.
    fn decide_fish(&mut self, g: usize, snap: bool) {
        let now = self.tick;
        self.calm(g, Mode::Water);
        let sun = self.plan.parts.sun;
        let sec = self.seconds();
        let night = sec < sun.rise - 3600.0 || sec > sun.set + 3600.0;
        let quay = self.plan.parts.quay.clone();
        if quay.len() < 2 {
            return;
        }
        let range = self.species[self.groups[g].species as usize].range;
        let seg = (self.draw(g, 1, 0.0, 1.0) * (quay.len() - 1) as f32) as usize;
        let (a, b) = (quay[seg], quay[seg + 1]);
        let along = a + (b - a) * self.draw(g, 2, 0.05, 0.95);
        let n = Vec2::new((b - a).y, -(b - a).x).normalize_or_zero();
        let sea = if self.plan.in_water(along + n) { n } else { -n };
        let wall = !night && self.draw(g, 3, 0.0, 1.0) < 0.25;
        let off = if wall {
            0.8
        } else {
            self.draw(g, 4, 2.0, (range * 0.5).min(20.0))
        };
        let depth = if night {
            self.draw(g, 5, -3.0, -2.0)
        } else {
            self.draw(g, 5, -1.5, -0.4)
        };
        let goal = (along + sea * off).extend(depth);
        let dur = ticks(self.draw(g, 6, 40.0, 120.0));
        let gr = &mut self.groups[g];
        gr.goal = goal;
        gr.activity = Activity::School;
        gr.until = now + dur;
        if snap {
            let gr = gr.clone();
            for i in gr.first..gr.first + gr.len {
                let i = i as usize;
                let s = self.seed[i];
                let mut p = goal
                    + Vec3::new(
                        between(s, 1, 1, -3.0, 3.0),
                        between(s, 2, 1, -3.0, 3.0),
                        between(s, 3, 1, -0.3, 0.3),
                    );
                if !self.plan.in_water(p.truncate()) {
                    p = goal;
                }
                p.z = p.z.clamp(-4.0, -0.15);
                self.pos[i] = p;
                self.vel[i] = (b - a).normalize_or_zero().extend(0.0) * 0.3;
                self.mode[i] = Mode::Water;
            }
        }
    }

    /// Group `g` flees a threat at `from`.
    fn flee(&mut self, g: usize, from: Vec3) {
        let now = self.tick;
        let gr = self.groups[g].clone();
        let sp = &self.species[gr.species as usize];
        let c = self.centroid(g, None);
        let away = (c - from).truncate().normalize_or(Vec2::X);
        match sp.behaviour {
            Behaviour::Sparrow | Behaviour::Pigeon | Behaviour::Gull => {
                let kinds: &[SiteKind] = match sp.behaviour {
                    Behaviour::Sparrow => &[SiteKind::Tree, SiteKind::Wall, SiteKind::Roof],
                    Behaviour::Pigeon => &[SiteKind::Roof],
                    _ => &[SiteKind::Roof, SiteKind::Water],
                };
                let here = (c - from).truncate().length();
                let home = self.bird_home(g);
                let refuge = self
                    .sites_near(kinds, c.truncate(), sp.range)
                    .into_iter()
                    .filter(|&s| {
                        let p = self.plan.sites[s as usize].pos.truncate();
                        let d = (p - from.truncate()).length();
                        d >= 15.0 && d > here && (p - home).length() <= sp.range * 1.5
                    })
                    .min_by(|&a, &b| {
                        let d = |s: u32| {
                            (self.plan.sites[s as usize].pos.truncate() - c.truncate()).length()
                        };
                        d(a).total_cmp(&d(b)).then(a.cmp(&b))
                    });
                let stay = ticks(self.draw(g, 50, 40.0, 120.0));
                match refuge {
                    Some(s) => {
                        let then = if self.plan.sites[s as usize].kind == SiteKind::Water {
                            Activity::Float
                        } else {
                            Activity::Perch
                        };
                        self.go_site(g, then, s, stay, false);
                    }
                    None => {
                        let centre = (c.truncate() + away * 25.0).extend(12.0);
                        self.circle(g, centre, 15.0, ticks(30.0));
                    }
                }
                if self.groups[g].activity == Activity::Travel {
                    self.groups[g].activity = Activity::Flee;
                }
            }
            Behaviour::Cat => {
                let i = gr.first as usize;
                let Some(yard) = gr.yard.clone() else { return };
                let p = self.pos[i];
                let here = (p - from).truncate().length();
                let wall = if yard.in_garden {
                    yard.garden
                        .iter()
                        .copied()
                        .filter(|q| {
                            q.z >= SAFE_CAT && (q.truncate() - from.truncate()).length() >= here
                        })
                        .min_by(|a, b| (*a - p).length().total_cmp(&(*b - p).length()))
                } else {
                    None
                };
                match wall {
                    Some(w) => {
                        self.path[i].clear();
                        self.spot[i] = w;
                        let stay = ticks(self.draw(g, 51, 60.0, 180.0));
                        let gr = &mut self.groups[g];
                        gr.activity = Activity::Flee;
                        gr.then = Activity::Out;
                        gr.stay = stay;
                        gr.until = u64::MAX;
                    }
                    None => {
                        let gate = if yard.in_garden { yard.flap } else { yard.door };
                        self.path[i].clear();
                        self.spot[i] = gate.extend(0.0);
                        let gr = &mut self.groups[g];
                        gr.activity = Activity::Flee;
                        gr.then = Activity::Night;
                        gr.until = u64::MAX;
                    }
                }
                self.mode[i] = Mode::Flee;
            }
            Behaviour::Goat => {
                for i in gr.first..gr.first + gr.len {
                    let i = i as usize;
                    let p = self.pos[i].truncate();
                    let dir = (p - from.truncate()).normalize_or(away);
                    // As far as the fence lets it, up to 20 m.
                    let to = [20.0f32, 12.0, 6.0, 3.0]
                        .iter()
                        .map(|&d| p + dir * d)
                        .find(|&q| self.plan.in_pasture(q))
                        .unwrap_or(p);
                    self.spot[i] = to.extend(0.0);
                    self.path[i].clear();
                    self.mode[i] = Mode::Flee;
                }
                let gr = &mut self.groups[g];
                gr.activity = Activity::Flee;
                gr.until = now + ticks(8.0);
            }
            Behaviour::Fish => {
                let gr = &mut self.groups[g];
                gr.goal = (c.truncate() + away * 8.0).extend(-2.5);
                gr.activity = Activity::Flee;
                gr.until = now + ticks(3.0);
                for i in gr.first..gr.first + gr.len {
                    self.mode[i as usize] = Mode::Flee;
                }
            }
            Behaviour::Swift | Behaviour::Dog => {}
        }
    }

    /// Animal `i`'s step.
    fn update(&mut self, i: usize) {
        let g = self.group[i] as usize;
        if self.alert(i)
            && let Some((ratio, from, threat, distance, threshold)) = self.danger(i)
        {
            if ratio < 1.0 {
                self.flushes.push(Flush {
                    tick: self.tick,
                    animal: i as u32,
                    species: self.groups[g].species,
                    threat,
                    distance,
                    threshold,
                });
                self.flee(g, from);
            } else if ratio < self.species_of(i).walk_off && self.mode[i] == Mode::Ground {
                // Step aside, away from it.
                let p = self.pos[i].truncate();
                let away = (p - from.truncate()).normalize_or(Vec2::X);
                self.spot[i] = self.plan.open_ground(p + away * 1.5, p).extend(0.0);
                self.until[i] = self.tick;
            }
        }
        match self.species[self.groups[g].species as usize].behaviour {
            Behaviour::Sparrow | Behaviour::Pigeon | Behaviour::Gull | Behaviour::Swift => {
                self.bird(i, g)
            }
            Behaviour::Cat => self.cat(i, g),
            Behaviour::Dog => self.dog(i, g),
            Behaviour::Goat => self.goat(i, g),
            Behaviour::Fish => self.fish(i, g),
        }
    }

    /// A bird's step.
    fn bird(&mut self, i: usize, g: usize) {
        let act = self.groups[g].activity;
        let flying = matches!(
            act,
            Activity::Travel | Activity::Flee | Activity::Circle | Activity::Party
        );
        match self.mode[i] {
            Mode::Air | Mode::Flee => self.fly(i, g),
            Mode::Ground => {
                if flying {
                    self.take_off(i, g);
                } else {
                    self.hop(i, g);
                }
            }
            Mode::Rest | Mode::Water | Mode::Hidden | Mode::Walk => {
                let here = (self.pos[i] - self.spot[i]).length() < 0.5;
                if flying && !(matches!(act, Activity::Travel | Activity::Flee) && here) {
                    self.take_off(i, g);
                }
            }
        }
    }

    /// Bird `i` takes off toward its group's goal, outward from the wall it sits by.
    fn take_off(&mut self, i: usize, g: usize) {
        let gr = &self.groups[g];
        let target = match gr.activity {
            Activity::Travel | Activity::Flee => self.spot[i],
            _ => gr.goal,
        };
        let want = (target - self.pos[i]).truncate().normalize_or(Vec2::X);
        let climb = self.species[gr.species as usize].climb;
        self.pos[i].z = self.pos[i].z.max(0.3);
        // Off a garden wall it was hopping over: onto its top first.
        while self.plan.solid(self.pos[i], 0.0) && self.pos[i].z < 3.0 {
            self.pos[i].z += 0.25;
        }
        let dir = self.clear_dir(self.pos[i], want);
        self.vel[i] = (dir * 3.0).extend(climb);
        self.mode[i] = Mode::Air;
    }

    /// The horizontal direction nearest `want` in which a metre and a half is clear of buildings
    /// and walls from `p` (a nest's or an eave's bird leaves outward).
    fn clear_dir(&self, p: Vec3, want: Vec2) -> Vec2 {
        if !self.plan.solid(p + (want * 1.5).extend(0.0), 0.3) {
            return want;
        }
        for (c, s) in TURNS {
            let r = Vec2::new(want.x * c - want.y * s, want.x * s + want.y * c);
            if !self.plan.solid(p + (r * 1.5).extend(0.0), 0.3) {
                return r;
            }
        }
        want
    }

    /// A ground bird's hop about its feeding place.
    fn hop(&mut self, i: usize, g: usize) {
        let now = self.tick;
        let gr = &self.groups[g];
        let sp = &self.species[gr.species as usize];
        let p = self.pos[i].truncate();
        let to = self.spot[i].truncate() - p;
        let d = to.length();
        if d < 0.02 {
            if now >= self.until[i] {
                let s = bits(self.seed[i], now, 3);
                let a = (s & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
                let r = ((s >> 16) & 0xFFFF) as f32 / 65535.0;
                let (sn, cs) = dmath::sin_cos(a);
                let q = gr.goal.truncate() + Vec2::new(cs, sn) * r.sqrt() * gr.radius;
                self.spot[i] = self.plan.open_ground(q, gr.goal.truncate()).extend(0.0);
                self.until[i] = now + ticks(between(s, 4, 0, 1.0, 6.0));
            }
            self.vel[i] = Vec3::ZERO;
            return;
        }
        let v = sp.walk;
        let step = (v * STEP).min(d);
        let q = self.plan.push_out(p + to / d * step);
        self.vel[i] = ((q - p) / STEP).extend(0.0);
        self.pos[i] = q.extend(0.0);
    }

    /// A flying bird's step: its group's goal, its neighbours, the walls ahead, its turn. It
    /// comes down onto a perch from above; a step that would still end in a wall is replaced by
    /// a climb in place, or by staying put, and counts as a contact.
    fn fly(&mut self, i: usize, g: usize) {
        let gr = &self.groups[g];
        let s = gr.species as usize;
        let sp = &self.species[s];
        let b = sp.behaviour;
        let climb = sp.climb;
        let act = gr.activity;
        let travel = matches!(act, Activity::Travel | Activity::Flee);
        let aloft = matches!(act, Activity::Circle | Activity::Party);
        let settled = !travel && !aloft;
        let p = self.pos[i];
        let speed = match act {
            Activity::Flee | Activity::Party => sp.flee_speed,
            _ => sp.cruise,
        };
        let cruise_alt = match b {
            Behaviour::Sparrow => 5.0,
            Behaviour::Pigeon => 14.0,
            Behaviour::Gull => 18.0,
            _ => 30.0,
        };
        let spot = self.spot[i];
        let to_spot = spot - p;
        let dxy = to_spot.truncate().length();
        let homing = travel || settled;
        // The last of the way: straight down onto the spot from above it, when nothing is
        // between.
        let above = p.z >= spot.z - 0.3;
        let landing = homing
            && dxy < 2.0
            && above
            && (1..=16).all(|k| !self.plan.solid(p + to_spot * (k as f32 / 16.0), 0.0));
        let target = if aloft {
            gr.goal
        } else if dxy > 25.0 {
            spot.with_z(spot.z.max(cruise_alt))
        } else if dxy > 1.5 || !above {
            spot + Vec3::Z * 2.0
        } else {
            spot
        };
        let to = target - p;
        let dist = to.length();
        let goal_dir = to / dist.max(1e-3);
        let (sep, align, cohere) = self.flock(i, g);
        let (w_align, w_cohere) = match b {
            Behaviour::Gull => (0.3, 0.3),
            Behaviour::Swift => (0.8, 0.5),
            _ => (0.8, 0.7),
        };
        let mut desire = goal_dir * if travel && dxy < 10.0 { 3.0 } else { 1.0 };
        match sep {
            Some(away) => desire += away * 1.5,
            None => desire += align * w_align + cohere * w_cohere,
        }
        if p.z < 1.0 && !landing {
            desire.z += 1.0;
        }
        let mut dir = if landing {
            to_spot.normalize_or(goal_dir)
        } else {
            desire.normalize_or(goal_dir)
        };
        let mut brake = false;
        let mut avoiding = false;
        if !landing {
            // Close to its perch it may pass nearer the walls.
            let margin = if homing && to_spot.length() < 4.0 {
                0.15
            } else {
                0.6
            };
            let s0 = self.vel[i].length().max(1.0);
            if let Some((a, slow)) = self.avoid(p, dir, speed.min(s0 * 1.5), climb, margin) {
                dir = a;
                brake = slow;
                avoiding = true;
            }
        }
        let heading = self.vel[i].normalize_or(dir);
        let (cmax, smax) = self.turn[s];
        let new_dir = if landing {
            dir
        } else if avoiding {
            // Twice its usual turn to get out of the way.
            let half = turn_toward(heading, dir, cmax, smax);
            turn_toward(half, dir, cmax, smax)
        } else {
            turn_toward(heading, dir, cmax, smax)
        };
        let near = homing && dxy < 12.0;
        let target_speed = if landing || near {
            speed.min(1.0 + to_spot.length() * 1.2)
        } else if brake {
            speed * 0.5
        } else {
            speed
        };
        let s0 = self.vel[i].length();
        let rate = if brake { STEP * 6.0 } else { STEP * 2.0 };
        let sp_now = s0 + (target_speed - s0) * rate.min(1.0);
        let mut v = new_dir * sp_now;
        v.z = v.z.clamp(-climb * 1.5, climb.max(1.0));
        if !landing {
            v = self.reachable(p, v, heading, s);
        }
        let mut q = p + v * STEP;
        // The ground and a perch below are not walls: never under the ground, never below the
        // spot while coming down onto it.
        q.z = q.z.max(0.0);
        if landing {
            q.z = q.z.max(spot.z);
        }
        // Arrived.
        if homing && (spot - q).length() < 0.4 {
            self.pos[i] = spot;
            self.vel[i] = Vec3::ZERO;
            let then = if travel { self.groups[g].then } else { act };
            self.mode[i] = match then {
                Activity::Forage => Mode::Ground,
                Activity::Float => Mode::Water,
                Activity::Night if b == Behaviour::Gull => Mode::Water,
                _ if b == Behaviour::Swift => Mode::Hidden,
                _ => Mode::Rest,
            };
            self.until[i] = self.tick;
            if travel {
                self.land(g);
            }
            return;
        }
        if self.plan.solid(q, 0.0) {
            self.contacts += 1;
            self.contacts_by[s] += 1;
            // One already in a wall (a spot on a wall's line) climbs out of it.
            let up = Vec3::new(p.x, p.y, p.z + climb * STEP);
            q = if self.plan.solid(up, 0.0) && !self.plan.solid(p, 0.0) {
                p
            } else {
                up
            };
        }
        if q.z < 0.0 {
            q.z = 0.0;
        }
        self.vel[i] = (q - p) / STEP;
        self.pos[i] = q;
    }

    /// One more of group `g` has arrived; when all have, its `then` begins.
    fn land(&mut self, g: usize) {
        let now = self.tick;
        let gr = &mut self.groups[g];
        gr.landed += 1;
        if gr.landed >= gr.len {
            gr.activity = gr.then;
            gr.until = now + gr.stay;
        }
    }

    /// Bird `i`'s neighbours: whether one is too close (and the way out), the nearest's mean
    /// heading and the way to their middle.
    fn flock(&self, i: usize, g: usize) -> (Option<Vec3>, Vec3, Vec3) {
        let gr = &self.groups[g];
        let sp = &self.species[gr.species as usize];
        let k = (sp.neighbours as usize).min(8);
        if k == 0 {
            return (None, Vec3::ZERO, Vec3::ZERO);
        }
        let p = self.pos[i];
        let mut near = [(f32::MAX, 0u32); 8];
        let mut n = 0;
        for j in gr.first..gr.first + gr.len {
            let ju = j as usize;
            if ju == i || !matches!(self.mode[ju], Mode::Air | Mode::Flee | Mode::Water) {
                continue;
            }
            let d2 = (self.pos[ju] - p).length_squared();
            if n < k {
                near[n] = (d2, j);
                n += 1;
                let mut m = n - 1;
                while m > 0 && near[m].0 < near[m - 1].0 {
                    near.swap(m, m - 1);
                    m -= 1;
                }
            } else if d2 < near[k - 1].0 {
                near[k - 1] = (d2, j);
                let mut m = k - 1;
                while m > 0 && near[m].0 < near[m - 1].0 {
                    near.swap(m, m - 1);
                    m -= 1;
                }
            }
        }
        if n == 0 {
            return (None, Vec3::ZERO, Vec3::ZERO);
        }
        let (mut sep, mut crowded) = (Vec3::ZERO, false);
        let (mut heading, mut middle) = (Vec3::ZERO, Vec3::ZERO);
        for &(d2, j) in &near[..n] {
            let q = self.pos[j as usize];
            let d = d2.sqrt();
            if d < sp.separation {
                crowded = true;
                sep += (p - q) / d.max(1e-3);
            }
            heading += self.vel[j as usize].normalize_or_zero();
            middle += q;
        }
        middle /= n as f32;
        let to_mid = middle - p;
        let spread = (sp.separation * 4.0).max(0.5);
        let cohere = to_mid.normalize_or_zero() * (to_mid.length() / spread).min(1.0);
        (
            crowded.then(|| sep.normalize_or(Vec3::Z)),
            heading.normalize_or_zero(),
            cohere,
        )
    }

    /// A way past what is ahead of a flyer at `p` heading `dir` at `speed`, climbing at most
    /// `climb`: the first flyable direction (up, level or down, turned 0° to 135° either way)
    /// whose next 0.8 s is clear of buildings and walls by `margin`, at full speed or else at
    /// half (`true`: brake); `None` when the way ahead is clear (Reynolds 1999's obstacle
    /// avoidance, within the bird's own limits).
    fn avoid(
        &self,
        p: Vec3,
        dir: Vec3,
        speed: f32,
        climb: f32,
        margin: f32,
    ) -> Option<(Vec3, bool)> {
        let clear = |d: Vec3, s: f32| {
            let mut v = d * s;
            v.z = v.z.clamp(-climb * 1.5, climb.max(1.0));
            [STEP, 0.25, 0.5, 0.8]
                .iter()
                .all(|&t| !self.plan.solid(p + v * t, margin))
        };
        if clear(dir, speed) {
            return None;
        }
        let h = dir.truncate().normalize_or(Vec2::X);
        for (s, brake) in [(speed, false), (speed * 0.5, true)] {
            for (c, sn) in std::iter::once((1.0, 0.0)).chain(TURNS) {
                let r = Vec2::new(h.x * c - h.y * sn, h.x * sn + h.y * c);
                for pitch in [0.6f32, 0.0, -0.3] {
                    let d = r.extend(pitch).normalize_or(Vec3::Z);
                    if clear(d, s) {
                        return Some((d, brake));
                    }
                }
            }
        }
        Some((Vec3::Z, true))
    }

    /// A cat's step: along its legs, resting, home.
    fn cat(&mut self, i: usize, g: usize) {
        let now = self.tick;
        let sp = &self.species[self.groups[g].species as usize];
        let (walk, run) = (sp.walk, sp.flee_speed);
        match self.mode[i] {
            Mode::Hidden | Mode::Rest => {
                self.vel[i] = Vec3::ZERO;
            }
            Mode::Walk | Mode::Flee => {
                let speed = if self.mode[i] == Mode::Flee {
                    run
                } else {
                    walk
                };
                // A pause now and then on a leg.
                if self.mode[i] == Mode::Walk && now < self.until[i] {
                    self.vel[i] = Vec3::ZERO;
                    return;
                }
                if self.follow(i, speed, false) {
                    return;
                }
                // Arrived.
                let act = self.groups[g].activity;
                let then = self.groups[g].then;
                if act == Activity::Night || (act == Activity::Flee && then == Activity::Night) {
                    // In at the flap or the door: the other side next, or stay.
                    let stay = if act == Activity::Flee {
                        ticks(self.draw(g, 61, 300.0, 1200.0))
                    } else {
                        ticks(self.draw(g, 61, 10.0, 40.0))
                    };
                    self.mode[i] = Mode::Hidden;
                    let gr = &mut self.groups[g];
                    gr.activity = Activity::Night;
                    gr.until = now + stay;
                    gr.decisions += 1;
                    return;
                }
                if act == Activity::Flee {
                    let gr = &mut self.groups[g];
                    gr.activity = Activity::Out;
                    gr.until = now + gr.stay;
                    self.mode[i] = Mode::Rest;
                    return;
                }
                self.mode[i] = Mode::Rest;
                let gr = &mut self.groups[g];
                gr.until = now + gr.stay;
            }
            _ => {}
        }
    }

    /// Animal `i` walks along its path to its spot at `speed` (sliding along buildings, or held
    /// in the pasture when `fenced`); `true` while under way.
    fn follow(&mut self, i: usize, speed: f32, fenced: bool) -> bool {
        let p = self.pos[i];
        let target = self.path[i].first().copied().unwrap_or(self.spot[i]);
        let to = (target - p).truncate();
        let d = to.length();
        let step = speed * STEP;
        if d <= step {
            let mut q = target;
            if !self.path[i].is_empty() {
                self.path[i].remove(0);
                // A pause at a corner for a cat.
                if self.species_of(i).behaviour == Behaviour::Cat && self.mode[i] == Mode::Walk {
                    let s = bits(self.seed[i], self.tick, 9);
                    if s.is_multiple_of(5) {
                        self.until[i] = self.tick + ticks(between(s, 1, 9, 2.0, 8.0));
                    }
                }
                q.z = 0.0;
                self.vel[i] = (q - p) / STEP;
                self.pos[i] = q;
                return true;
            }
            self.vel[i] = (q - p) / STEP;
            self.pos[i] = q;
            return false;
        }
        let mut q = p.truncate() + to / d * step;
        if fenced {
            q = self.fence(p.truncate(), q);
            if (q - p.truncate()).length() < step * 0.1 {
                // Held by the fence: as far as it goes.
                self.path[i].clear();
                self.spot[i] = p;
                self.vel[i] = Vec3::ZERO;
                return false;
            }
        } else {
            q = self.plan.push_out(q);
        }
        let z = if self.species_of(i).behaviour == Behaviour::Cat && d < 0.6 {
            target.z
        } else {
            0.0
        };
        let nq = q.extend(z);
        self.vel[i] = (nq - p) / STEP;
        self.pos[i] = nq;
        true
    }

    /// The move from `p` to `q` held inside the pasture and fold: slid along the fence.
    fn fence(&self, p: Vec2, q: Vec2) -> Vec2 {
        if self.plan.in_pasture(q) {
            return q;
        }
        let x = Vec2::new(q.x, p.y);
        if self.plan.in_pasture(x) {
            return x;
        }
        let y = Vec2::new(p.x, q.y);
        if self.plan.in_pasture(y) {
            return y;
        }
        p
    }

    /// A dog's step: about its owner on the leash, sniffing to the sides.
    fn dog(&mut self, i: usize, g: usize) {
        let gr = &self.groups[g];
        let sp = &self.species[gr.species as usize];
        let Some(l) = &gr.leash else { return };
        if gr.activity != Activity::Walk {
            self.mode[i] = Mode::Hidden;
            self.vel[i] = Vec3::ZERO;
            return;
        }
        let owner = l.owner.truncate();
        let t = self.tick as f32 * STEP;
        let phase = (self.seed[i] & 0xFFFF) as f32 / 65535.0 * std::f32::consts::TAU;
        let ahead = l.vel.truncate().normalize_or(Vec2::X);
        let swing = 1.2 * dmath::sin(t * 0.25 + phase);
        let (s, c) = dmath::sin_cos(swing);
        let side = Vec2::new(ahead.x * c - ahead.y * s, ahead.x * s + ahead.y * c);
        let reach = if l.vel.length() < 0.05 { 0.9 } else { 0.7 } * sp.leash;
        let target = owner + side * reach;
        let p = self.pos[i].truncate();
        let to = target - p;
        let d = to.length();
        let v = (d * 2.0).min(sp.cruise);
        let mut q = if d > 1e-4 {
            p + to / d * (v * STEP).min(d)
        } else {
            p
        };
        let off = q - owner;
        if off.length() > sp.leash {
            q = owner + off.normalize_or_zero() * sp.leash;
        }
        if self.plan.in_water(q) {
            q = owner;
        }
        q = self.plan.push_out(q);
        self.vel[i] = ((q - p) / STEP).extend(0.0);
        self.pos[i] = q.extend(0.0);
        self.mode[i] = Mode::Walk;
    }

    /// A goat's step: to its spot along its path, browsing, kept by its herd and its fence.
    fn goat(&mut self, i: usize, g: usize) {
        let now = self.tick;
        let gr = self.groups[g].clone();
        let sp = &self.species[gr.species as usize];
        let act = gr.activity;
        match self.mode[i] {
            Mode::Rest => {
                self.vel[i] = Vec3::ZERO;
                if act == Activity::Graze && now >= self.until[i] {
                    // The next shrub near the herd's goal, or back toward the herd.
                    let c = self.centroid(g, None).truncate();
                    let p = self.pos[i].truncate();
                    let next = if (c - p).length() > 22.0 {
                        c
                    } else {
                        self.shrub_near(gr.goal.truncate(), self.seed[i], now)
                    };
                    self.spot[i] = next.extend(0.0);
                    self.mode[i] = Mode::Walk;
                }
            }
            Mode::Walk | Mode::Flee => {
                if self.mode[i] == Mode::Flee && act != Activity::Flee {
                    self.mode[i] = Mode::Walk;
                }
                let p = self.pos[i].truncate();
                let far = (self.centroid(g, None).truncate() - p).length() > 22.0;
                let speed = match (self.mode[i], act) {
                    (Mode::Flee, _) => sp.flee_speed,
                    (_, Activity::Graze) if !far => {
                        sp.walk * between(self.seed[i], now / 600, 5, 1.0, 2.0)
                    }
                    _ => sp.cruise,
                };
                // Keep a body's length from the nearest herdmate.
                let mut push = Vec2::ZERO;
                for j in gr.first..gr.first + gr.len {
                    let ju = j as usize;
                    if ju != i {
                        let off = p - self.pos[ju].truncate();
                        let d = off.length();
                        if d < sp.separation && d > 1e-4 {
                            push += off / d * (sp.separation - d);
                        }
                    }
                }
                if self.follow(i, speed, true) {
                    let q = self.fence(self.pos[i].truncate(), self.pos[i].truncate() + push * 0.5);
                    self.pos[i] = q.extend(0.0);
                    return;
                }
                // Arrived.
                self.mode[i] = Mode::Rest;
                self.vel[i] = Vec3::ZERO;
                match act {
                    Activity::Graze => {
                        self.until[i] = now + ticks(between(self.seed[i], now, 6, 25.0, 90.0));
                    }
                    Activity::Travel => self.land(g),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// A fish's step: its school's goal in Couzin's zones, held in the harbour's water.
    fn fish(&mut self, i: usize, g: usize) {
        let gr = &self.groups[g];
        let s = gr.species as usize;
        let sp = &self.species[s];
        let sun = self.plan.parts.sun;
        let sec = self.seconds();
        let night = sec < sun.rise - 3600.0 || sec > sun.set + 3600.0;
        let speed = if gr.activity == Activity::Flee {
            sp.flee_speed
        } else if night {
            sp.cruise * 0.6
        } else {
            sp.cruise
        };
        if gr.activity != Activity::Flee && self.mode[i] == Mode::Flee {
            self.mode[i] = Mode::Water;
        }
        let p = self.pos[i];
        let to = gr.goal - p;
        let goal_dir = to.normalize_or_zero();
        let (sep, align, cohere) = self.flock(i, g);
        let mut desire = goal_dir * 0.6;
        match sep {
            Some(away) => desire += away * 1.5,
            None => desire += align + cohere * 0.8,
        }
        let dir = desire.normalize_or(goal_dir.normalize_or(Vec3::X));
        let heading = self.vel[i].normalize_or(dir);
        let (cmax, smax) = self.turn[s];
        let nd = turn_toward(heading, dir, cmax, smax);
        let s0 = self.vel[i].length();
        let v = nd * (s0 + (speed - s0) * (STEP * 2.0).min(1.0));
        let mut q = p + v * STEP;
        q.z = q.z.clamp(-4.0, -0.15);
        let range = sp.range;
        let ok = |f: &Self, x: Vec2| f.plan.in_water(x) && f.plan.from_quay(x) <= range;
        let mut vel = (q - p) / STEP;
        if !ok(self, q.truncate()) {
            // The quay's wall or the harbour's edge: turn back along it.
            let x = Vec2::new(q.x, p.y);
            let y = Vec2::new(p.x, q.y);
            let keep = if ok(self, x) {
                x
            } else if ok(self, y) {
                y
            } else {
                p.truncate()
            };
            q = keep.extend(q.z);
            vel = (q - p) / STEP;
            if vel.length() < 0.05 {
                vel = -self.vel[i] * 0.5;
            }
        }
        self.vel[i] = vel;
        self.pos[i] = q;
    }

    /// Puts group `g` about `at` at once for `seconds`, on the ground (birds feeding, goats
    /// browsing, a cat sitting) or in the water (a school): a scripted scene.
    pub fn hold(&mut self, g: usize, at: Vec2, seconds: f32) {
        let now = self.tick;
        let gr = self.groups[g].clone();
        let b = self.species[gr.species as usize].behaviour;
        for i in gr.first..gr.first + gr.len {
            let i = i as usize;
            let s = self.seed[i];
            let off = Vec2::new(between(s, 7, 1, -2.0, 2.0), between(s, 7, 2, -2.0, 2.0));
            let p = if b == Behaviour::Cat || b == Behaviour::Dog {
                at
            } else {
                at + off
            };
            let z = if b == Behaviour::Fish { -0.5 } else { 0.0 };
            self.pos[i] = p.extend(z);
            self.spot[i] = self.pos[i];
            self.vel[i] = Vec3::ZERO;
            self.path[i].clear();
            self.until[i] = now + ticks(seconds);
            self.mode[i] = match b {
                Behaviour::Fish => Mode::Water,
                Behaviour::Sparrow | Behaviour::Pigeon | Behaviour::Gull => Mode::Ground,
                _ => Mode::Rest,
            };
        }
        let gr = &mut self.groups[g];
        gr.activity = match b {
            Behaviour::Sparrow | Behaviour::Pigeon | Behaviour::Gull => Activity::Forage,
            Behaviour::Goat => Activity::Graze,
            Behaviour::Fish => Activity::School,
            Behaviour::Cat => Activity::Out,
            _ => gr.activity,
        };
        gr.goal = at.extend(if b == Behaviour::Fish { -0.5 } else { 0.0 });
        gr.radius = 2.0;
        gr.until = now + ticks(seconds);
        gr.stay = ticks(seconds);
        if let Some(y) = &mut gr.yard {
            y.in_garden = false;
        }
    }

    /// Animals by species and mode: `counts[species][mode]`, the modes in [`MODES`]' order.
    pub fn counts(&self) -> Vec<[u32; 7]> {
        let mut c = vec![[0u32; 7]; self.species.len()];
        for i in 0..self.len() {
            let s = self.groups[self.group[i] as usize].species as usize;
            c[s][self.mode[i] as usize] += 1;
        }
        c
    }

    /// A 64-bit FNV-1a digest of where everyone is and what they do (D-016).
    pub fn digest(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        for i in 0..self.len() {
            for v in [self.pos[i].x, self.pos[i].y, self.pos[i].z] {
                eat(&v.to_bits().to_le_bytes());
            }
            eat(&[self.mode[i] as u8]);
        }
        for g in &self.groups {
            eat(&g.until.to_le_bytes());
        }
        h
    }
}

/// `u` turned toward `d` by at most the angle whose cosine and sine are `c` and `s`.
fn turn_toward(u: Vec3, d: Vec3, c: f32, s: f32) -> Vec3 {
    let k = u.dot(d);
    if k >= c {
        return d;
    }
    let w = d - u * k;
    let wl = w.length();
    let w = if wl < 1e-5 {
        Vec3::new(-u.y, u.x, 0.0).normalize_or(Vec3::Y)
    } else {
        w / wl
    };
    (u * c + w * s).normalize_or(d)
}

/// The fold's gate: a point just inside the fold and one just inside the pasture.
fn gate(plan: &Plan) -> (Vec3, Vec3) {
    let fold = &plan.parts.fold;
    if fold.is_empty() || plan.parts.pasture.is_empty() {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    let pc = plan::centroid(&plan.parts.pasture);
    let edge = plan::nearest_on_outline(pc, fold);
    let fc = plan::centroid(fold);
    let into_fold = (fc - edge).normalize_or_zero();
    (
        (edge + into_fold * 1.5).extend(0.0),
        (edge - into_fold * 1.5).extend(0.0),
    )
}

/// A goat's place in the fold.
fn fold_spot(plan: &Plan, seed: u64) -> Vec3 {
    let fold = &plan.parts.fold;
    let (lo, hi) = fold.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    for k in 0..8 {
        let u = Vec2::new(unit(seed, 11, k), unit(seed, 12, k));
        let p = lo + (hi - lo) * (Vec2::splat(0.1) + u * 0.8);
        if plan::inside(p, fold) {
            return p.extend(0.0);
        }
    }
    plan::centroid(fold).extend(0.0)
}
