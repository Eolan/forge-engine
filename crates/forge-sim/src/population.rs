//! A persistent population as records advanced by events (#250; `docs/research/
//! population-simulation.md`, "Recommendation for Forge", steps 1 and 2; D-059).
//!
//! Everyone is a few columns of a structure of arrays, about 34 bytes a person: a home, an anchor
//! (work or school), a role, a personality, a seed, and where they are (a place, or a trip with its
//! destination) since when. Nobody is updated every tick. A calendar of events, ordered by
//! (game second, person), holds each person's next change: an activity ends or a trip arrives.
//! The day's plan is never stored: it is made again from the person's seed, role and personality
//! and the day's number whenever an event needs it, so the same person does the same day on every
//! machine (D-016).
//!
//! - [`places_from_buildings`]: homes, jobs, schools, shops, food and leisure from a city's
//!   buildings (their floor area and whether their ground floor is a shop), and its parks.
//! - [`generate`]: the population from the places and a [`Mix`] of roles, from a seed.
//! - [`Sim`]: the calendar, run to any game second; [`Sim::census`] counts where everyone is.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use forge_core::{Seed, SplitMix64, dmath};

/// Game seconds since the population's start.
pub type GameSeconds = u32;
/// A game day, seconds.
pub const DAY: GameSeconds = 86_400;
/// Game seconds a real second: a game hour in 20 real minutes, a day in 8 real hours (D-059,
/// the owner's second walk, 2026-10-11; 15 minutes before).
pub const GAME_SPEED: GameSeconds = 3;
/// The places an outing is chosen among ([`World::choose`]): the nearest this many...
pub const CHOICE: usize = 32;
/// ...and this many of the nearest with some appeal.
pub const CHOICE_APPEALING: usize = 8;
/// How much further a place of appeal 1 is worth going on a weekday, km; twice at weekends.
pub const APPEAL_KM: f32 = 1.5;
/// A workplace's distance from home, straight: drawn from a log-normal of this median, metres,
/// and spread. Draws that land outside the city are drawn again, so the commutes come out
/// shorter: in an 8 km city by the sea, a median of 3.3 km, 23 % under 2 km and 78 % under 5 km,
/// near French dense centres' (4.8–5.4 km by road, about 3.5 km straight;
/// docs/research/trip-distances.md).
pub const WORK_MEDIAN: f64 = 6000.0;
/// The spread of [`WORK_MEDIAN`]'s log-normal.
pub const WORK_SIGMA: f64 = 0.8;
/// No place.
pub const NONE: u32 = u32::MAX;
/// The `at` column's flag for a person on a trip.
const TRIP: u32 = 1 << 31;

/// What a place is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PlaceKind {
    /// Homes.
    Home,
    /// Jobs.
    Work,
    /// A school.
    School,
    /// A shop.
    Shop,
    /// A café or restaurant.
    Food,
    /// A bar, a club, a cinema.
    Leisure,
    /// A park.
    Park,
}

const KINDS: usize = 7;

/// A place people go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    /// What it is for.
    pub kind: PlaceKind,
    /// Where it is, metres (the city plan's frame).
    pub pos: [f32; 2],
    /// How many it holds: residents, jobs, pupils or visitors.
    pub capacity: u32,
    /// How far it draws people from: 0 an ordinary place, 1 a shop, café or bar on a square, a
    /// park or the water (worth [`APPEAL_KM`] more of the way), a park 0.5.
    pub appeal: f32,
}

/// A building of a city, as the population sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Building {
    /// Where it is, metres.
    pub pos: [f32; 2],
    /// Its floors' net area (inside the walls, less stairs, lifts and partitions:
    /// `forge_procgen::city::structure`, D-060), m².
    pub floor_area: f32,
    /// Whether its ground floor is a shop, a café or a bar.
    pub active_ground: bool,
    /// How far its shop, café or bar draws people from ([`Place::appeal`]): 1 on a square, a
    /// park or the water, else 0.
    pub appeal: f32,
}

/// How a city's floor area is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uses {
    /// The share of floors that are homes.
    pub homes: f32,
    /// Net home floor area a person, m² (INSEE: 32.4 in apartments).
    pub home_area: f32,
    /// Net work floor area a job, m².
    pub job_area: f32,
    /// One building in this many is a school.
    pub school_every: u32,
}

impl Default for Uses {
    fn default() -> Self {
        Self {
            homes: 0.66,
            home_area: 32.4,
            // 30 m² gross a job, net about 0.82 of gross.
            job_area: 25.0,
            school_every: 120,
        }
    }
}

/// How many people live in building `b`: its floor area's homes by `uses`.
pub fn residents(b: &Building, uses: &Uses) -> u32 {
    (b.floor_area * uses.homes / uses.home_area) as u32
}

/// Places from a city's buildings: homes and jobs by floor area, every `school_every`-th building
/// a school, active ground floors as shops, cafés and bars in turn, and the parks. The homes come
/// in their buildings' order, one for each building with [`residents`].
pub fn places_from_buildings(
    buildings: &[Building],
    parks: &[[f32; 2]],
    uses: &Uses,
) -> Vec<Place> {
    let mut places = Vec::new();
    for (i, b) in buildings.iter().enumerate() {
        let residents = residents(b, uses);
        if residents > 0 {
            places.push(Place {
                kind: PlaceKind::Home,
                pos: b.pos,
                capacity: residents,
                appeal: 0.0,
            });
        }
        let jobs = (b.floor_area * (1.0 - uses.homes) / uses.job_area) as u32;
        let school = uses.school_every > 0 && i as u32 % uses.school_every == uses.school_every / 2;
        if school {
            places.push(Place {
                kind: PlaceKind::School,
                pos: b.pos,
                capacity: 600,
                appeal: 0.0,
            });
        } else if jobs > 0 {
            places.push(Place {
                kind: PlaceKind::Work,
                pos: b.pos,
                capacity: jobs,
                appeal: 0.0,
            });
        }
        if b.active_ground {
            let kind = match i % 3 {
                0 => PlaceKind::Shop,
                1 => PlaceKind::Food,
                _ => PlaceKind::Leisure,
            };
            places.push(Place {
                kind,
                pos: b.pos,
                capacity: 40,
                appeal: b.appeal,
            });
        }
    }
    places.extend(parks.iter().map(|&pos| Place {
        kind: PlaceKind::Park,
        pos,
        capacity: 500,
        appeal: 0.5,
    }));
    places
}

/// A person's main role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Role {
    /// Works by day.
    Worker,
    /// Works at night.
    NightWorker,
    /// At school or university.
    Student,
    /// A child at school.
    Child,
    /// Retired.
    Retiree,
}

/// The shares of each role (they need not sum to one; they are weights).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mix {
    /// Day workers.
    pub worker: f32,
    /// Night workers.
    pub night_worker: f32,
    /// Students.
    pub student: f32,
    /// Children.
    pub child: f32,
    /// Retirees.
    pub retiree: f32,
}

impl Default for Mix {
    /// A European city's mix, roughly: 45 % in work by day, 3 % by night, 9 % students, 15 %
    /// children, 28 % retired or at home.
    fn default() -> Self {
        Self {
            worker: 0.45,
            night_worker: 0.03,
            student: 0.09,
            child: 0.15,
            retiree: 0.28,
        }
    }
}

impl Mix {
    /// The default mix without children, the other shares scaled to fill their place: for a
    /// world that has none (a game world where no child should ever come to harm).
    pub fn adults() -> Self {
        let d = Self::default();
        let k = 1.0 / (1.0 - d.child);
        Self {
            worker: d.worker * k,
            night_worker: d.night_worker * k,
            student: d.student * k,
            child: 0.0,
            retiree: d.retiree * k,
        }
    }
}

/// Five traits of four bits each: openness, conscientiousness, extraversion, agreeableness,
/// neuroticism (the Big Five), 0 to 15.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Personality(pub u32);

impl Personality {
    /// Openness.
    pub fn openness(self) -> u32 {
        self.0 & 15
    }
    /// Conscientiousness.
    pub fn conscientiousness(self) -> u32 {
        (self.0 >> 4) & 15
    }
    /// Extraversion.
    pub fn extraversion(self) -> u32 {
        (self.0 >> 8) & 15
    }
    /// Agreeableness.
    pub fn agreeableness(self) -> u32 {
        (self.0 >> 12) & 15
    }
    /// Neuroticism.
    pub fn neuroticism(self) -> u32 {
        (self.0 >> 16) & 15
    }
}

/// The places of a city with a grid per kind for "a place of this kind near here".
pub struct World {
    /// The places.
    pub places: Vec<Place>,
    /// The places' bounds, metres: the lowest and highest corner.
    pub bounds: [[f32; 2]; 2],
    cell: f32,
    side: usize,
    grids: Vec<Vec<Vec<u32>>>,
    /// The same, for the places with some appeal only.
    appealing: Vec<Vec<Vec<u32>>>,
}

impl World {
    /// Indexes the places in cells of `cell` metres over a square frame of `size` metres.
    pub fn new(places: Vec<Place>, size: f32, cell: f32) -> Self {
        let side = (size / cell).ceil().max(1.0) as usize + 1;
        let mut grids = vec![vec![Vec::new(); side * side]; KINDS];
        let mut appealing = grids.clone();
        for (i, p) in places.iter().enumerate() {
            let x = ((p.pos[0] / cell).floor().max(0.0) as usize).min(side - 1);
            let y = ((p.pos[1] / cell).floor().max(0.0) as usize).min(side - 1);
            grids[p.kind as usize][y * side + x].push(i as u32);
            if p.appeal > 0.0 {
                appealing[p.kind as usize][y * side + x].push(i as u32);
            }
        }
        let bounds = places
            .iter()
            .fold([[f32::MAX; 2], [f32::MIN; 2]], |[lo, hi], p| {
                [
                    [lo[0].min(p.pos[0]), lo[1].min(p.pos[1])],
                    [hi[0].max(p.pos[0]), hi[1].max(p.pos[1])],
                ]
            });
        Self {
            places,
            bounds,
            cell,
            side,
            grids,
            appealing,
        }
    }

    /// One of the few places of `kind` nearest `pos`, chosen by `pick`, or `None` if the city has
    /// none.
    pub fn near(&self, kind: PlaceKind, pos: [f32; 2], pick: u64) -> Option<u32> {
        let found = self.nearest(&self.grids[kind as usize], pos, 4);
        let k = found.len().min(4);
        (k > 0).then(|| found[(pick % k as u64) as usize].1)
    }

    /// A place of `kind` for an outing from `pos`, chosen by `pick` among the [`CHOICE`] nearest
    /// and the [`CHOICE_APPEALING`] nearest with some appeal, each weighed by
    /// `exp(−per_km · (km − appeal · bonus_km))`: near places mostly, now and then one further off
    /// that draws people (docs/research/trip-distances.md: a logit of distance and appeal). `None`
    /// if the city has none.
    pub fn choose(
        &self,
        kind: PlaceKind,
        pos: [f32; 2],
        pick: u64,
        per_km: f32,
        bonus_km: f32,
    ) -> Option<u32> {
        let mut found = self.nearest(&self.grids[kind as usize], pos, CHOICE);
        found.truncate(CHOICE);
        let mut far = self.nearest(&self.appealing[kind as usize], pos, CHOICE_APPEALING);
        far.truncate(CHOICE_APPEALING);
        for f in far {
            if !found.iter().any(|g| g.1 == f.1) {
                found.push(f);
            }
        }
        if found.is_empty() {
            return None;
        }
        let weight = |&(d2, i): &(f32, u32)| {
            let km = d2.sqrt() / 1000.0;
            let appeal = self.places[i as usize].appeal;
            dmath::exp(-per_km * (km - appeal * bonus_km))
        };
        let total: f32 = found.iter().map(weight).sum();
        let h = pick.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (pick >> 29);
        let mut r = (h >> 40) as f32 / (1u64 << 24) as f32 * total;
        for f in &found {
            r -= weight(f);
            if r <= 0.0 {
                return Some(f.1);
            }
        }
        found.last().map(|f| f.1)
    }

    /// At least `k` places of `grid` nearest `pos` when it has them (whole rings of cells, so
    /// more), nearest first: squared distance and index.
    fn nearest(&self, grid: &[Vec<u32>], pos: [f32; 2], k: usize) -> Vec<(f32, u32)> {
        let cx = ((pos[0] / self.cell).floor().max(0.0) as usize).min(self.side - 1) as i64;
        let cy = ((pos[1] / self.cell).floor().max(0.0) as usize).min(self.side - 1) as i64;
        let mut found: Vec<(f32, u32)> = Vec::new();
        for ring in 0..self.side as i64 {
            for y in (cy - ring).max(0)..=(cy + ring).min(self.side as i64 - 1) {
                for x in (cx - ring).max(0)..=(cx + ring).min(self.side as i64 - 1) {
                    if (x - cx).abs() != ring && (y - cy).abs() != ring {
                        continue;
                    }
                    for &i in &grid[y as usize * self.side + x as usize] {
                        let p = self.places[i as usize].pos;
                        let d = (p[0] - pos[0]).powi(2) + (p[1] - pos[1]).powi(2);
                        found.push((d, i));
                    }
                }
            }
            // A ring past the first that found enough.
            if found.len() >= k && ring >= 1 {
                break;
            }
        }
        // The `k` nearest only, then in order: plans are made again on every event.
        let order = |a: &(f32, u32), b: &(f32, u32)| {
            a.0.partial_cmp(&b.0).expect("finite").then(a.1.cmp(&b.1))
        };
        if found.len() > k && k > 0 {
            found.select_nth_unstable_by(k - 1, order);
            found.truncate(k);
        }
        found.sort_by(order);
        found
    }

    /// The straight-line distance between two places, metres.
    fn distance(&self, a: u32, b: u32) -> f32 {
        let (p, q) = (self.places[a as usize].pos, self.places[b as usize].pos);
        ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt()
    }
}

/// The population's columns, one entry a person.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Population {
    /// Their home.
    pub home: Vec<u32>,
    /// Their work or school ([`NONE`] for none).
    pub anchor: Vec<u32>,
    /// Their role.
    pub role: Vec<Role>,
    /// Their personality.
    pub personality: Vec<Personality>,
    /// Their own seed, from which their days are made.
    pub seed: Vec<u32>,
    /// Where they are: a place, or [`TRIP`] with the trip's destination in `dest`.
    pub at: Vec<u32>,
    /// Their trip's destination (meaningful on a trip).
    pub dest: Vec<u32>,
    /// Where their trip set out from (meaningful on a trip).
    pub origin: Vec<u32>,
    /// Since when they are where they are.
    pub since: Vec<GameSeconds>,
    /// Their day.
    pub day: Vec<u16>,
    /// Their activity in the day's plan.
    pub step: Vec<u8>,
}

impl Population {
    /// How many people.
    pub fn len(&self) -> usize {
        self.home.len()
    }

    /// Whether it is empty.
    pub fn is_empty(&self) -> bool {
        self.home.is_empty()
    }

    /// The bytes a person takes in the columns.
    pub fn bytes_per_person() -> usize {
        4 + 4 + 1 + 4 + 4 + 4 + 4 + 4 + 4 + 2 + 1
    }

    /// The people in `parts` contiguous runs of nearly equal length, in order.
    pub fn split(mut self, parts: usize) -> Vec<Population> {
        let parts = parts.max(1);
        let n = self.len();
        let mut out = Vec::with_capacity(parts);
        // From the back, so each run is cut off the end of the columns.
        for k in (0..parts).rev() {
            let from = n * k / parts;
            out.push(Population {
                home: self.home.split_off(from),
                anchor: self.anchor.split_off(from),
                role: self.role.split_off(from),
                personality: self.personality.split_off(from),
                seed: self.seed.split_off(from),
                at: self.at.split_off(from),
                dest: self.dest.split_off(from),
                origin: self.origin.split_off(from),
                since: self.since.split_off(from),
                day: self.day.split_off(from),
                step: self.step.split_off(from),
            });
        }
        out.reverse();
        out
    }
}

/// Makes the population of a world: everyone who fits in its homes, their roles by `mix`, their
/// work at a distance drawn from a log-normal of median [`WORK_MEDIAN`] in a random direction,
/// their school the nearest.
pub fn generate(world: &World, mix: &Mix, seed: Seed) -> Population {
    generate_in(world, mix, seed, &|_| true)
}

/// As [`generate`], only the people of the homes `keep` takes (by place): each the same person
/// as in the whole population, from the same seed, at their own index here. The residents'
/// lights need a few thousand buildings' people, not the city's million (D-060 step 3).
pub fn generate_in(world: &World, mix: &Mix, seed: Seed, keep: &dyn Fn(u32) -> bool) -> Population {
    let mut pop = Population::default();
    // Everyone's index in the whole population.
    let mut id = 0_u64;
    let weights = [
        mix.worker,
        mix.night_worker,
        mix.student,
        mix.child,
        mix.retiree,
    ];
    let total: f32 = weights.iter().sum();
    let roles = [
        Role::Worker,
        Role::NightWorker,
        Role::Student,
        Role::Child,
        Role::Retiree,
    ];
    for (h, home) in world.places.iter().enumerate() {
        if home.kind != PlaceKind::Home {
            continue;
        }
        if !keep(h as u32) {
            id += u64::from(home.capacity);
            continue;
        }
        for _ in 0..home.capacity {
            let own = seed.derive(id).value();
            id += 1;
            let mut rng = SplitMix64::new(own);
            let mut r = rng.next_f32() * total;
            let mut role = Role::Retiree;
            for (w, &candidate) in weights.iter().zip(&roles) {
                if r < *w {
                    role = candidate;
                    break;
                }
                r -= w;
            }
            let personality = Personality((rng.next_u32()) & 0xF_FFFF);
            let anchor = match role {
                Role::Worker | Role::NightWorker => {
                    // A target outside the city's places is drawn again (up to eight times), or
                    // the jobs would pile up on its edges.
                    let [lo, hi] = world.bounds;
                    let mut target = home.pos;
                    for _ in 0..8 {
                        // A log-normal (Box–Muller): most near home, a long tail.
                        let u = 1.0 - rng.next_f64() * 0.999;
                        let z = (-2.0 * dmath::ln(u)).sqrt()
                            * dmath::cos(rng.next_f64() * std::f64::consts::TAU);
                        let distance = WORK_MEDIAN * dmath::exp(WORK_SIGMA * z);
                        let (s, c) = dmath::sin_cos(rng.next_f64() * std::f64::consts::TAU);
                        target = [
                            home.pos[0] + (c * distance) as f32,
                            home.pos[1] + (s * distance) as f32,
                        ];
                        if (lo[0]..=hi[0]).contains(&target[0])
                            && (lo[1]..=hi[1]).contains(&target[1])
                        {
                            break;
                        }
                    }
                    world
                        .near(PlaceKind::Work, target, rng.next_u64())
                        .unwrap_or(NONE)
                }
                Role::Student | Role::Child => world
                    .near(PlaceKind::School, home.pos, rng.next_u64())
                    .unwrap_or(NONE),
                Role::Retiree => NONE,
            };
            pop.home.push(h as u32);
            pop.anchor.push(anchor);
            pop.role.push(role);
            pop.personality.push(personality);
            pop.seed.push(own as u32);
            pop.at.push(h as u32);
            pop.dest.push(NONE);
            pop.origin.push(NONE);
            pop.since.push(0);
            pop.day.push(0);
            pop.step.push(0);
        }
    }
    pop
}

/// One activity of a day's plan: where, and until when (game seconds since the day's start;
/// past [`DAY`] runs into the next).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Activity {
    /// Where.
    pub place: u32,
    /// Until when.
    pub until: GameSeconds,
}

/// A day's plan: at most eight activities, the last at home until the next day's first ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// The activities.
    pub steps: [Activity; 8],
    /// How many.
    pub len: u8,
}

impl Plan {
    fn push(&mut self, place: u32, until: GameSeconds) {
        if (self.len as usize) < self.steps.len() {
            self.steps[self.len as usize] = Activity { place, until };
            self.len += 1;
        }
    }
}

const fn hm(h: u32, m: u32) -> u32 {
    h * 3600 + m * 60
}

/// An outing's distance decay by its kind, per km. With evenly spread places the median trip is
/// about 1.7 over it (docs/research/trip-distances.md): shopping 1.2 (1.4 km), a café or tea
/// room 1.5 (1.1 km), an evening out 0.7 (2.4 km), a park 1.0.
fn per_km(kind: PlaceKind) -> f32 {
    match kind {
        PlaceKind::Shop => 1.2,
        PlaceKind::Food => 1.5,
        PlaceKind::Leisure => 0.7,
        PlaceKind::Park => 1.0,
        _ => 2.0,
    }
}

/// How many households home place `home` holds: its residents by 2.2 (the French mean), at least
/// one.
pub fn households(world: &World, home: u32) -> u32 {
    ((world.places[home as usize].capacity as f32 / 2.2) as u32).max(1)
}

/// Person `p`'s household among their home's [`households`], theirs by their seed.
pub fn household_index(world: &World, pop: &Population, p: usize) -> u32 {
    pop.seed[p] % households(world, pop.home[p])
}

/// Person `p`'s household, as a key: one of their home's households (2.2 residents each, the
/// French mean), theirs by their seed.
pub fn household(world: &World, pop: &Population, p: usize) -> u64 {
    (u64::from(pop.home[p]) << 24) ^ u64::from(household_index(world, pop, p))
}

/// Person `p`'s plan for day `day`, made from their seed, role and personality.
pub fn plan(world: &World, pop: &Population, p: usize, day: u16) -> Plan {
    let mut rng = SplitMix64::new((u64::from(pop.seed[p]) << 16) ^ u64::from(day));
    let mut j = |lo: u32, hi: u32| lo + (rng.next_u64() % u64::from(hi - lo + 1)) as u32;
    let home = pop.home[p];
    let anchor = pop.anchor[p];
    let me = pop.personality[p];
    let pos = world.places[home as usize].pos;
    let mut plan = Plan {
        steps: [Activity {
            place: home,
            until: 0,
        }; 8],
        len: 0,
    };
    let weekend = day % 7 >= 5;
    // Outings by distance and appeal: near home mostly, now and then a place that draws people
    // from further off, the more so at weekends (the owner, 2026-10-11).
    let bonus = if weekend { 2.0 * APPEAL_KM } else { APPEAL_KM };
    let near = |kind: PlaceKind, pick: u64| {
        world
            .choose(kind, pos, pick, per_km(kind), bonus)
            .unwrap_or(home)
    };
    // The conscientious rise earlier, the extraverted go out more and later.
    let early = (15 - me.conscientiousness()) * 120;
    let out = me.extraversion();
    let pick = u64::from(pop.seed[p]) ^ (u64::from(day) * 0x9E37);
    match pop.role[p] {
        Role::Worker if anchor != NONE && !weekend => {
            plan.push(home, hm(7, 0) + early + j(0, 3600));
            plan.push(anchor, hm(12, 0) + j(0, 1800));
            if j(0, 9) < 3 {
                let lunch = world
                    .near(PlaceKind::Food, world.places[anchor as usize].pos, pick)
                    .unwrap_or(anchor);
                plan.push(lunch, hm(13, 0) + j(0, 900));
            }
            // One workday in ten a workplace goes for a drink after work, and its outgoing
            // people join: one time and one bar for all, near work, so they leave and walk
            // together (the owner: people go out with friends, family, sometimes coworkers).
            let crew =
                SplitMix64::new((u64::from(anchor) << 20) ^ u64::from(day) ^ 0xC0FFEE).next_u64();
            let drinks = crew.is_multiple_of(10) && j(0, 15) < out;
            let leave = if drinks {
                hm(18, 0) + ((crew >> 8) % 1800) as u32
            } else {
                hm(17, 0) + j(0, 5400)
            };
            plan.push(anchor, leave);
            if drinks {
                let bar = world
                    .choose(
                        PlaceKind::Leisure,
                        world.places[anchor as usize].pos,
                        crew >> 16,
                        2.0,
                        0.0,
                    )
                    .unwrap_or(home);
                plan.push(bar, leave + 5400 + ((crew >> 40) % 2700) as u32);
                plan.push(home, DAY);
                return plan;
            }
            // On the way home, about one in seven: the shops near work (France: 15 % of
            // home-work journeys stop on the way; docs/research/trip-distances.md).
            if j(0, 99) < 15 {
                let shop = world
                    .choose(
                        PlaceKind::Shop,
                        world.places[anchor as usize].pos,
                        pick ^ 3,
                        2.0,
                        0.0,
                    )
                    .unwrap_or(home);
                plan.push(shop, leave + 900 + j(0, 1800));
            }
            // About one in seven out on a weekday evening, the extraverted more often.
            let evening = j(0, 47);
            if evening < out / 2 {
                plan.push(
                    near(PlaceKind::Leisure, pick),
                    hm(21, 0) + out * 600 + j(0, 3600),
                );
            } else if evening < out {
                plan.push(near(PlaceKind::Food, pick), hm(21, 30) + j(0, 1800));
            } else if evening < out + 8 {
                plan.push(near(PlaceKind::Shop, pick), hm(18, 30) + j(0, 2700));
            }
        }
        Role::NightWorker if anchor != NONE && !weekend => {
            plan.push(home, hm(14, 0) + j(0, 3600));
            if j(0, 2) == 0 {
                plan.push(near(PlaceKind::Shop, pick), hm(16, 0) + j(0, 1800));
            }
            plan.push(home, hm(21, 15) + j(0, 900));
            plan.push(anchor, DAY + hm(6, 0) + j(0, 900));
        }
        Role::Student | Role::Child if anchor != NONE && !weekend => {
            plan.push(home, hm(7, 45) + j(0, 900));
            plan.push(anchor, hm(16, 30) + j(0, 1800));
            if pop.role[p] == Role::Student && j(0, 31) < out {
                plan.push(near(PlaceKind::Leisure, pick), hm(22, 0) + j(0, 7200));
            } else {
                // After school or classes, now and then: a café, a park or the shops.
                let kind = match j(0, 7) {
                    0 => Some(PlaceKind::Food),
                    1 => Some(PlaceKind::Park),
                    2 => Some(PlaceKind::Shop),
                    _ => None,
                };
                if let Some(kind) = kind {
                    plan.push(near(kind, pick), hm(18, 30) + j(0, 1800));
                }
            }
        }
        _ => {
            // Retirees, weekends and anyone without an anchor: errands, a park, maybe dinner out.
            // Errands on two days in five, spread over the morning; a park on one in three.
            let errand = j(0, 4) < 2;
            let leave = hm(9, 0) + early + j(0, 7200);
            plan.push(home, leave);
            if errand {
                plan.push(near(PlaceKind::Shop, pick), leave + 1800 + j(0, 2700));
            }
            // At weekends, two days in seven, a household goes out together: one place and one
            // time for all of it, so they leave and walk together ([`household`]).
            let family =
                SplitMix64::new(household(world, pop, p) ^ (u64::from(day) << 48)).next_u64();
            if weekend && family % 7 < 2 {
                let kind = match (family >> 8) % 4 {
                    0 => PlaceKind::Park,
                    1 | 2 => PlaceKind::Food,
                    _ => PlaceKind::Shop,
                };
                let leave = hm(15, 0) + ((family >> 16) % 3600) as u32;
                plan.push(home, leave);
                let place = world
                    .choose(kind, pos, family >> 24, per_km(kind), bonus)
                    .unwrap_or(home);
                plan.push(place, leave + 5400 + ((family >> 40) % 1800) as u32);
                plan.push(home, DAY);
                return plan;
            }
            plan.push(home, hm(15, 0) + j(0, 3600));
            // The afternoon out on two days in five: a café or a tea room, the shops, a pub, a
            // park. The parks took everyone out before (the owner, 2026-10-11: people would be
            // at the cafés, pubs and tea rooms, or shopping, more than standing in a square).
            let afternoon = match j(0, 14) {
                0 => Some(PlaceKind::Park),
                1 | 2 => Some(PlaceKind::Food),
                3 | 4 => Some(PlaceKind::Shop),
                5 => Some(PlaceKind::Leisure),
                _ => None,
            };
            if let Some(kind) = afternoon {
                plan.push(near(kind, pick ^ 1), hm(17, 0) + j(0, 3600));
            }
            if j(0, 63) < out {
                plan.push(near(PlaceKind::Food, pick ^ 2), hm(22, 0) + j(0, 3600));
            }
        }
    }
    // Home until the next day's first activity ends: the calendar plans that day then.
    plan.push(home, DAY);
    plan
}

/// When person `p` wakes on day `day` and goes to bed that night, from `today`, their [`plan`]
/// for it: game seconds since the day's start, a bedtime past [`DAY`] falling in the next day's
/// small hours. The windows needed it (D-060 step 3: the residents light their own windows).
/// - They wake 45 to 90 minutes before their plan's first step ends: they leave then, or would
///   on a day at home.
/// - They go to bed at their role's hour or half an hour to an hour after coming home, whichever
///   is later. The extraverted and the unconscientious go later, and everyone an hour later
///   before a day off. A night worker on shift sleeps after coming home in the morning.
pub fn sleep(pop: &Population, p: usize, day: u16, today: &Plan) -> (GameSeconds, GameSeconds) {
    let mut rng =
        SplitMix64::new((u64::from(pop.seed[p]) << 16) ^ u64::from(day) ^ (0x51EE9 << 40));
    let mut j = |lo: u32, hi: u32| lo + (rng.next_u64() % u64::from(hi - lo + 1)) as u32;
    let wake = today.steps[0].until.saturating_sub(j(2700, 5400));
    let me = pop.personality[p];
    let hour = match pop.role[p] {
        Role::Child => hm(20, 30),
        Role::Student => hm(23, 30),
        Role::NightWorker => DAY + hm(0, 30),
        Role::Worker | Role::Retiree => hm(22, 30),
    };
    // Friday's and Saturday's nights (the weekend is days 5 and 6 of each week).
    let off = if matches!(day % 7, 4 | 5) { 3600 } else { 0 };
    let usual = hour + off + me.extraversion() * 240 + j(0, 1800) - me.conscientiousness() * 120;
    let home = pop.home[p];
    let unwind = j(1800, 3600);
    let back = today.steps[..today.len as usize]
        .iter()
        .rev()
        .find(|s| s.place != home)
        .map_or(0, |s| s.until + unwind);
    (wake, usual.max(back))
}

/// Whether person `p` is at home and awake at `t`, game seconds into day `day` (under [`DAY`]).
/// The evening before counts from day 1 on: a night out, a night shift or a late bedtime runs
/// past midnight. Day 0, the run's first, starts with everyone at home, as the [`Sim`] does.
pub fn home_awake(world: &World, pop: &Population, p: usize, day: u16, t: GameSeconds) -> bool {
    let home = pop.home[p];
    let today = plan(world, pop, p, day);
    let at = |plan: &Plan, t: GameSeconds| {
        plan.steps[..plan.len as usize]
            .iter()
            .find(|s| s.until > t)
            .map(|s| s.place)
    };
    let before = (day > 0).then(|| plan(world, pop, p, day - 1));
    let place = before
        .as_ref()
        .and_then(|b| at(b, DAY + t))
        .or_else(|| at(&today, t))
        .unwrap_or(home);
    if place != home {
        return false;
    }
    let bed_before = before.map_or(0, |b| sleep(pop, p, day - 1, &b).1);
    let (wake, bed) = sleep(pop, p, day, &today);
    let asleep = (DAY + t >= bed_before && t < wake) || t >= bed;
    !asleep
}

/// How a trip goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// On foot.
    Walk,
    /// By bike.
    Bike,
    /// By tram, after a wait at the stop of [`TRAM_WAIT`] seconds.
    Tram,
}

/// The wait at the stop a tram trip begins with, game seconds.
pub const TRAM_WAIT: GameSeconds = 300;

/// How a trip goes and how long it takes, game seconds: on foot under 1.2 km (1.3 m/s), by bike
/// under 4 km (4.5 m/s) for the open, else by tram (5.5 m/s after a 5-minute wait); the street
/// distance a third longer than the straight line.
pub fn travel(world: &World, from: u32, to: u32, personality: Personality) -> (Mode, GameSeconds) {
    let d = world.distance(from, to) * 1.3;
    let (mode, s) = if d < 1200.0 {
        (Mode::Walk, d / 1.3)
    } else if d < 4000.0 && personality.openness() >= 5 {
        (Mode::Bike, d / 4.5)
    } else {
        (Mode::Tram, TRAM_WAIT as f32 + d / 5.5)
    };
    (mode, s.round().max(60.0) as GameSeconds)
}

/// How long a trip takes, game seconds ([`travel`]'s time).
pub fn travel_time(world: &World, from: u32, to: u32, personality: Personality) -> GameSeconds {
    travel(world, from, to, personality).1
}

/// A trip under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trip {
    /// The place it set out from.
    pub from: u32,
    /// The place it goes to.
    pub to: u32,
    /// When it set out.
    pub since: GameSeconds,
    /// How long it takes.
    pub time: GameSeconds,
    /// How it goes.
    pub mode: Mode,
}

/// What a person is doing, for a map.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Doing {
    /// At home.
    Home,
    /// At work or school.
    Anchor,
    /// Shopping.
    Shop,
    /// In a café, a restaurant or a bar.
    Out,
    /// In a park.
    Park,
    /// On the way.
    Travelling,
}

/// Where everyone is, by kind of place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Census {
    /// At home.
    pub home: u32,
    /// At work or school.
    pub anchor: u32,
    /// In shops.
    pub shop: u32,
    /// In cafés and restaurants.
    pub food: u32,
    /// In bars, clubs and cinemas.
    pub leisure: u32,
    /// In parks.
    pub park: u32,
    /// Travelling.
    pub travelling: u32,
}

/// The calendar and the population it advances.
pub struct Sim<'w> {
    /// The places.
    pub world: &'w World,
    /// The people.
    pub pop: Population,
    queue: BinaryHeap<Reverse<(GameSeconds, u32)>>,
    /// The game second reached.
    pub now: GameSeconds,
    /// Events handled.
    pub events: u64,
}

impl<'w> Sim<'w> {
    /// A calendar with everyone at home at the start of day 0, each with their first event.
    pub fn new(world: &'w World, pop: Population) -> Self {
        let mut sim = Self {
            world,
            queue: BinaryHeap::with_capacity(pop.len()),
            pop,
            now: 0,
            events: 0,
        };
        for p in 0..sim.pop.len() {
            let first = plan(world, &sim.pop, p, 0).steps[0].until;
            sim.queue.push(Reverse((first, p as u32)));
        }
        sim
    }

    /// Handles every event up to game second `t`, in (time, person) order.
    pub fn run_until(&mut self, t: GameSeconds) {
        while let Some(&Reverse((time, p))) = self.queue.peek() {
            if time > t {
                break;
            }
            self.queue.pop();
            self.events += 1;
            let next = self.event(time, p as usize);
            self.queue.push(Reverse((next, p)));
        }
        self.now = t;
    }

    /// A person's event at `now`: a trip arrives, or an activity ends. Returns their next event.
    fn event(&mut self, now: GameSeconds, p: usize) -> GameSeconds {
        let pop = &mut self.pop;
        let day_start = u32::from(pop.day[p]) * DAY;
        if pop.at[p] & TRIP != 0 {
            // Arrived: stay until the activity's end.
            pop.at[p] = pop.dest[p];
            pop.since[p] = now;
            let plan = plan(self.world, pop, p, pop.day[p]);
            let until = day_start + plan.steps[pop.step[p] as usize].until;
            return until.max(now + 60);
        }
        // An activity ended: the next one, on the next day after the last.
        let mut plan_today = plan(self.world, pop, p, pop.day[p]);
        let mut step = pop.step[p] + 1;
        if step >= plan_today.len {
            pop.day[p] += 1;
            step = 0;
            plan_today = plan(self.world, pop, p, pop.day[p]);
        }
        pop.step[p] = step;
        let day_start = u32::from(pop.day[p]) * DAY;
        let next = plan_today.steps[step as usize];
        let here = pop.at[p];
        if next.place == here {
            return (day_start + next.until).max(now + 60);
        }
        let time = travel_time(self.world, here, next.place, pop.personality[p]);
        pop.at[p] = TRIP;
        pop.origin[p] = here;
        pop.dest[p] = next.place;
        pop.since[p] = now;
        now + time
    }

    /// Person `p`'s trip, if on one: from where, to where, since when, for how long and how.
    pub fn trip(&self, p: usize) -> Option<Trip> {
        (self.pop.at[p] & TRIP != 0).then(|| {
            let (from, to) = (self.pop.origin[p], self.pop.dest[p]);
            let (mode, time) = travel(self.world, from, to, self.pop.personality[p]);
            Trip {
                from,
                to,
                since: self.pop.since[p],
                time,
                mode,
            }
        })
    }

    /// Everyone's position now and what they are doing: at a place, scattered a little about it
    /// (by their seed); on a trip, along the straight line from where they set out, by the share
    /// of the trip's time gone.
    pub fn positions(&self) -> Vec<([f32; 2], Doing)> {
        let places = &self.world.places;
        (0..self.pop.len())
            .map(|p| {
                let at = self.pop.at[p];
                if at & TRIP != 0 {
                    let (from, to) = (self.pop.origin[p], self.pop.dest[p]);
                    let time = travel_time(self.world, from, to, self.pop.personality[p]);
                    let gone = (self.now.saturating_sub(self.pop.since[p])) as f32 / time as f32;
                    let (a, b) = (places[from as usize].pos, places[to as usize].pos);
                    let t = gone.clamp(0.0, 1.0);
                    return (
                        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
                        Doing::Travelling,
                    );
                }
                let s = self.pop.seed[p];
                let jitter = |bits: u32| ((bits & 0xFF) as f32 / 255.0 - 0.5) * 24.0;
                let q = places[at as usize].pos;
                let pos = [q[0] + jitter(s), q[1] + jitter(s >> 8)];
                let doing = if at == self.pop.anchor[p] {
                    Doing::Anchor
                } else {
                    match places[at as usize].kind {
                        PlaceKind::Home => Doing::Home,
                        PlaceKind::Work | PlaceKind::School => Doing::Anchor,
                        PlaceKind::Shop => Doing::Shop,
                        PlaceKind::Food | PlaceKind::Leisure => Doing::Out,
                        PlaceKind::Park => Doing::Park,
                    }
                };
                (pos, doing)
            })
            .collect()
    }

    /// Where everyone is now.
    pub fn census(&self) -> Census {
        let mut c = Census::default();
        for p in 0..self.pop.len() {
            let at = self.pop.at[p];
            if at & TRIP != 0 {
                c.travelling += 1;
                continue;
            }
            if at == self.pop.anchor[p] {
                c.anchor += 1;
                continue;
            }
            match self.world.places[at as usize].kind {
                PlaceKind::Home => c.home += 1,
                PlaceKind::Work | PlaceKind::School => c.anchor += 1,
                PlaceKind::Shop => c.shop += 1,
                PlaceKind::Food => c.food += 1,
                PlaceKind::Leisure => c.leisure += 1,
                PlaceKind::Park => c.park += 1,
            }
        }
        c
    }

    /// A 64-bit FNV-1a digest of where everyone is and since when (D-016).
    pub fn digest(&self) -> u64 {
        self.fold_digest(FNV_BASIS)
    }

    /// The digest carried on from `h` over these people: shards fold in order to the digest of
    /// the whole population.
    fn fold_digest(&self, mut h: u64) -> u64 {
        for p in 0..self.pop.len() {
            for v in [self.pop.at[p], self.pop.dest[p], self.pop.since[p]] {
                for b in v.to_le_bytes() {
                    h = (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
        h
    }

    /// A snapshot of what changes as the people live (#250's persistence): where each is, where
    /// bound and from where, since when, their day and their step in it, and the time of their
    /// next event; with the game second and the events handled. Column by column, little-endian,
    /// 23 bytes a person. Homes, work, roles, personalities and seeds are not in it: they are the
    /// generated base, made again from the seed (D-053).
    pub fn snapshot(&self) -> Vec<u8> {
        let n = self.pop.len();
        let mut next = vec![0 as GameSeconds; n];
        for &Reverse((time, p)) in &self.queue {
            next[p as usize] = time;
        }
        let mut out = Vec::with_capacity(SNAPSHOT_HEADER + n * SNAPSHOT_BYTES);
        out.extend_from_slice(SNAPSHOT_MAGIC);
        out.extend_from_slice(&SNAPSHOT_VERSION.to_le_bytes());
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&self.now.to_le_bytes());
        out.extend_from_slice(&self.events.to_le_bytes());
        let pop = &self.pop;
        for column in [&pop.at, &pop.dest, &pop.origin, &pop.since, &next] {
            for v in column {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        for v in &pop.day {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out.extend_from_slice(&pop.step);
        out
    }

    /// A population restored from a [`Sim::snapshot`] over its generated `base`.
    pub fn restore(
        world: &'w World,
        base: Population,
        bytes: &[u8],
    ) -> Result<Self, SnapshotError> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != SNAPSHOT_MAGIC {
            return Err(SnapshotError::NotASnapshot);
        }
        let version = r.u32()?;
        if version != SNAPSHOT_VERSION {
            return Err(SnapshotError::Version(version));
        }
        let n = r.u32()? as usize;
        if n != base.len() {
            return Err(SnapshotError::People {
                snapshot: n,
                base: base.len(),
            });
        }
        let now = r.u32()?;
        let events = r.u64()?;
        let mut pop = base;
        let column = |r: &mut Reader| (0..n).map(|_| r.u32()).collect::<Result<Vec<_>, _>>();
        pop.at = column(&mut r)?;
        pop.dest = column(&mut r)?;
        pop.origin = column(&mut r)?;
        pop.since = column(&mut r)?;
        let next = column(&mut r)?;
        pop.day = (0..n)
            .map(|_| r.take(2).map(|b| u16::from_le_bytes([b[0], b[1]])))
            .collect::<Result<_, _>>()?;
        pop.step = r.take(n)?.to_vec();
        if r.at != bytes.len() {
            return Err(SnapshotError::Length);
        }
        let queue = next
            .iter()
            .enumerate()
            .map(|(p, &time)| Reverse((time, p as u32)))
            .collect();
        Ok(Self {
            world,
            pop,
            queue,
            now,
            events,
        })
    }
}

/// The start of a population snapshot.
const SNAPSHOT_MAGIC: &[u8; 4] = b"FPOP";
/// Its layout's version.
const SNAPSHOT_VERSION: u32 = 1;
/// Its header's bytes: the magic, the version, the people, the game second, the events.
pub const SNAPSHOT_HEADER: usize = 4 + 4 + 4 + 4 + 8;
/// Its bytes a person: four `u32` columns, the next event's time, the day and the step.
pub const SNAPSHOT_BYTES: usize = 4 * 4 + 4 + 2 + 1;

/// Why a snapshot does not restore.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    /// It does not start as a snapshot does.
    NotASnapshot,
    /// It is of another layout's version.
    Version(u32),
    /// It holds another number of people than the base it is restored over.
    People {
        /// Its people.
        snapshot: usize,
        /// The base's.
        base: usize,
    },
    /// It ends early, or goes on past its end.
    Length,
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotASnapshot => write!(f, "not a population snapshot"),
            Self::Version(v) => write!(f, "a snapshot of layout version {v}"),
            Self::People { snapshot, base } => {
                write!(f, "a snapshot of {snapshot} people over a base of {base}")
            }
            Self::Length => write!(f, "a snapshot of the wrong length"),
        }
    }
}

impl std::error::Error for SnapshotError {}

/// Reads a snapshot's bytes in order.
struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], SnapshotError> {
        let end = self.at.checked_add(n).ok_or(SnapshotError::Length)?;
        let out = self.bytes.get(self.at..end).ok_or(SnapshotError::Length)?;
        self.at = end;
        Ok(out)
    }

    fn u32(&mut self) -> Result<u32, SnapshotError> {
        self.take(4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, SnapshotError> {
        let b = self.take(8)?;
        let mut a = [0; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }
}

/// FNV-1a's offset basis.
const FNV_BASIS: u64 = 0xcbf2_9ce4_8422_2325;

/// The population in shards advanced on worker threads (#250): contiguous runs of people, each
/// with its own calendar. People do not act on each other yet, so a shard's events depend on
/// its people alone, and any number of workers reaches the same state as one: the digests are
/// equal to the bit.
pub struct Shards<'w> {
    /// The shards, in the people's order.
    pub shards: Vec<Sim<'w>>,
}

impl<'w> Shards<'w> {
    /// `pop` in `workers` shards, each with its calendar.
    pub fn new(world: &'w World, pop: Population, workers: usize) -> Self {
        Self {
            shards: pop
                .split(workers)
                .into_iter()
                .map(|p| Sim::new(world, p))
                .collect(),
        }
    }

    /// Handles every event up to game second `t`, each shard on its own thread.
    pub fn run_until(&mut self, t: GameSeconds) {
        std::thread::scope(|scope| {
            for shard in &mut self.shards {
                scope.spawn(move || shard.run_until(t));
            }
        });
    }

    /// Events handled.
    pub fn events(&self) -> u64 {
        self.shards.iter().map(|s| s.events).sum()
    }

    /// The digest of the whole population, as one [`Sim`] would give it.
    pub fn digest(&self) -> u64 {
        self.shards.iter().fold(FNV_BASIS, |h, s| s.fold_digest(h))
    }

    /// Where everyone is now.
    pub fn census(&self) -> Census {
        let mut c = Census::default();
        for s in &self.shards {
            let d = s.census();
            c.home += d.home;
            c.anchor += d.anchor;
            c.shop += d.shop;
            c.food += d.food;
            c.leisure += d.leisure;
            c.park += d.park;
            c.travelling += d.travelling;
        }
        c
    }

    /// Everyone's position and what they are doing, in the people's order.
    pub fn positions(&self) -> Vec<([f32; 2], Doing)> {
        self.shards.iter().flat_map(Sim::positions).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2 km square town: a grid of buildings, shops on every third, two parks.
    fn town() -> World {
        let mut buildings = Vec::new();
        for y in 0..20 {
            for x in 0..20 {
                buildings.push(Building {
                    pos: [x as f32 * 100.0 + 50.0, y as f32 * 100.0 + 50.0],
                    floor_area: 2400.0,
                    active_ground: (x + y) % 3 == 0,
                    // The town's middle row looks onto its square.
                    appeal: if y == 10 { 1.0 } else { 0.0 },
                });
            }
        }
        let places = places_from_buildings(
            &buildings,
            &[[500.0, 500.0], [1500.0, 1500.0]],
            &Uses::default(),
        );
        World::new(places, 2000.0, 250.0)
    }

    #[test]
    fn outings_weigh_distance_and_appeal() {
        let world = town();
        let from = [150.0_f32, 150.0];
        let dist = |i: u32| {
            let p = world.places[i as usize].pos;
            ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt()
        };
        let picks: Vec<u32> = (0..2000_u64)
            .filter_map(|k| world.choose(PlaceKind::Food, from, k * 7919, 1.5, APPEAL_KM))
            .collect();
        let mut d: Vec<f32> = picks.iter().map(|&i| dist(i)).collect();
        d.sort_by(f32::total_cmp);
        let median = d[d.len() / 2];
        // Further than "one of the four nearest", still mostly near.
        let fourth = world.nearest(&world.grids[PlaceKind::Food as usize], from, 4)[3]
            .0
            .sqrt();
        assert!(
            median > fourth && median < 1500.0,
            "median {median} m, fourth {fourth} m"
        );
        // The square's row, 900 m off, draws some.
        let drawn = picks
            .iter()
            .filter(|&&i| world.places[i as usize].appeal > 0.0)
            .count();
        assert!(drawn > 20, "{drawn} of {}", picks.len());
        // The same pick, the same place.
        assert_eq!(
            world.choose(PlaceKind::Food, from, 42, 1.5, APPEAL_KM),
            world.choose(PlaceKind::Food, from, 42, 1.5, APPEAL_KM)
        );
    }

    #[test]
    fn households_and_coworkers_go_out_together() {
        let world = town();
        let pop = generate(&world, &Mix::default(), Seed::new(4));
        // The steps out of home and work that two people share, place and time.
        let shared = |day: u16, key: &dyn Fn(usize) -> Option<u64>| {
            let mut groups: std::collections::HashMap<u64, Vec<usize>> = Default::default();
            for p in 0..pop.len() {
                if let Some(k) = key(p) {
                    groups.entry(k).or_default().push(p);
                }
            }
            let mut together = 0;
            for members in groups.values().filter(|m| m.len() >= 2) {
                let steps = |p: usize| {
                    let plan = plan(&world, &pop, p, day);
                    plan.steps[..plan.len as usize]
                        .iter()
                        .filter(|s| s.place != pop.home[p] && s.place != pop.anchor[p])
                        .map(|s| (s.place, s.until))
                        .collect::<Vec<_>>()
                };
                let first = steps(members[0]);
                if members[1..]
                    .iter()
                    .any(|&q| steps(q).iter().any(|s| first.contains(s)))
                {
                    together += 1;
                }
            }
            together
        };
        // A Saturday: households out together.
        let families = shared(5, &|p| Some(household(&world, &pop, p)));
        assert!(families > 20, "{families} households out together");
        // A Wednesday: coworkers off for a drink.
        let workplace = |p: usize| {
            (pop.role[p] == Role::Worker && pop.anchor[p] != NONE).then(|| u64::from(pop.anchor[p]))
        };
        let crews = shared(2, &workplace);
        assert!(crews > 3, "{crews} workplaces out together");
    }

    #[test]
    fn homes_are_awake_in_the_evening_and_asleep_in_the_small_hours() {
        let world = town();
        let pop = generate(&world, &Mix::adults(), Seed::new(9));
        // A Tuesday, its Monday night before: the share of people at home and awake.
        let share = |h: u32, m: u32, role: Option<Role>| {
            let people: Vec<usize> = (0..pop.len())
                .filter(|&p| role.is_none_or(|r| pop.role[p] == r))
                .collect();
            let awake = people
                .iter()
                .filter(|&&p| home_awake(&world, &pop, p, 1, hm(h, m)))
                .count();
            awake as f64 / people.len() as f64
        };
        let (night, early, noon, evening, late) = (
            share(3, 0, None),
            share(6, 30, None),
            share(12, 0, None),
            share(20, 30, None),
            share(23, 30, None),
        );
        assert!(night < 0.05, "3:00 {night}");
        assert!(early > 0.10, "6:30 {early}");
        assert!(noon < evening, "noon {noon}, 20:30 {evening}");
        assert!(evening > 0.6, "20:30 {evening}");
        assert!(late < evening && late > night, "23:30 {late}");
        // The night workers sleep in the morning, home from their shift, and are out at night.
        let shift = |h: u32| share(h, 0, Some(Role::NightWorker));
        assert!(shift(9) < 0.2, "9:00 {}", shift(9));
        assert!(shift(18) > 0.5, "18:00 {}", shift(18));
        assert!(shift(2) < 0.2, "2:00 {}", shift(2));
    }

    #[test]
    fn some_homes_people_are_the_whole_populations() {
        let world = town();
        let whole = generate(&world, &Mix::default(), Seed::new(3));
        let some = generate_in(&world, &Mix::default(), Seed::new(3), &|h| h % 5 == 2);
        let kept: Vec<usize> = (0..whole.len())
            .filter(|&p| whole.home[p] % 5 == 2)
            .collect();
        assert_eq!(kept.len(), some.len());
        for (q, &p) in kept.iter().enumerate() {
            assert_eq!(
                (whole.home[p], whole.anchor[p], whole.role[p], whole.seed[p]),
                (some.home[q], some.anchor[q], some.role[q], some.seed[q])
            );
            assert_eq!(whole.personality[p], some.personality[q]);
        }
    }

    #[test]
    fn a_town_sleeps_at_night_and_works_by_day() {
        let world = town();
        let pop = generate(&world, &Mix::default(), Seed::new(4));
        let n = pop.len() as u32;
        assert!(n > 10_000, "{n} people");
        let mut sim = Sim::new(&world, pop);
        sim.run_until(hm(3, 0));
        let night = sim.census();
        assert_eq!(
            night.home,
            n - night.anchor
                - night.travelling
                - night.leisure
                - night.food
                - night.shop
                - night.park
        );
        assert!(night.home as f32 > 0.9 * n as f32, "{night:?}");
        sim.run_until(hm(10, 30));
        let morning = sim.census();
        assert!(morning.anchor as f32 > 0.5 * n as f32, "{morning:?}");
        sim.run_until(hm(21, 30));
        let evening = sim.census();
        assert!(evening.leisure + evening.food > 0, "{evening:?}");
        sim.run_until(DAY + hm(3, 0));
        let next = sim.census();
        assert!(next.home as f32 > 0.85 * n as f32, "{next:?}");
    }

    #[test]
    fn the_same_seed_gives_the_same_day() {
        let world = town();
        let run = || {
            let mut sim = Sim::new(&world, generate(&world, &Mix::default(), Seed::new(9)));
            sim.run_until(DAY);
            (sim.digest(), sim.events)
        };
        assert_eq!(run(), run());
    }

    #[test]
    fn an_adults_mix_has_no_children() {
        let world = town();
        let pop = generate(&world, &Mix::adults(), Seed::new(5));
        assert!(pop.len() > 100);
        assert!(pop.role.iter().all(|&r| r != Role::Child));
        // The other roles take the children's place, in their usual proportions.
        let workers = pop.role.iter().filter(|&&r| r == Role::Worker).count();
        assert!(workers * 2 > pop.len() / 2, "{workers} of {}", pop.len());
    }

    #[test]
    fn a_snapshot_restored_runs_on_to_the_same_day() {
        let world = town();
        let pop = || generate(&world, &Mix::default(), Seed::new(9));
        let mut straight = Sim::new(&world, pop());
        straight.run_until(hm(13, 0));
        let bytes = straight.snapshot();
        assert_eq!(
            bytes.len(),
            SNAPSHOT_HEADER + straight.pop.len() * SNAPSHOT_BYTES
        );
        let mut restored = Sim::restore(&world, pop(), &bytes).expect("restores");
        assert_eq!(restored.digest(), straight.digest());
        for sim in [&mut straight, &mut restored] {
            sim.run_until(DAY + hm(9, 0));
        }
        assert_eq!(restored.digest(), straight.digest());
        assert_eq!(restored.events, straight.events);
        // Damaged snapshots say why.
        let mut short = bytes.clone();
        short.pop();
        assert_eq!(
            Sim::restore(&world, pop(), &short).err(),
            Some(SnapshotError::Length)
        );
        assert_eq!(
            Sim::restore(&world, pop(), b"nope").err(),
            Some(SnapshotError::NotASnapshot)
        );
    }

    #[test]
    fn any_number_of_workers_gives_the_same_day() {
        let world = town();
        let pop = || generate(&world, &Mix::default(), Seed::new(9));
        let mut one = Sim::new(&world, pop());
        let n = one.pop.len();
        for t in [hm(8, 0), hm(13, 0), DAY + hm(2, 0)] {
            one.run_until(t);
            for workers in [1, 3, 6] {
                let mut shards = Shards::new(&world, pop(), workers);
                shards.run_until(t);
                assert_eq!(shards.shards.len(), workers);
                assert_eq!(shards.shards.iter().map(|s| s.pop.len()).sum::<usize>(), n);
                assert_eq!(shards.digest(), one.digest(), "{workers} workers at {t}");
                assert_eq!(shards.events(), one.events);
                assert_eq!(shards.census(), one.census());
                assert_eq!(shards.positions(), one.positions());
            }
        }
    }
}
