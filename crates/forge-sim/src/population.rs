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
}

/// A building of a city, as the population sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Building {
    /// Where it is, metres.
    pub pos: [f32; 2],
    /// Its floors' area, m².
    pub floor_area: f32,
    /// Whether its ground floor is a shop, a café or a bar.
    pub active_ground: bool,
}

/// How a city's floor area is used.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Uses {
    /// The share of floors that are homes.
    pub homes: f32,
    /// Home floor area a person, m².
    pub home_area: f32,
    /// Work floor area a job, m².
    pub job_area: f32,
    /// One building in this many is a school.
    pub school_every: u32,
}

impl Default for Uses {
    fn default() -> Self {
        Self {
            homes: 0.66,
            home_area: 40.0,
            job_area: 30.0,
            school_every: 120,
        }
    }
}

/// Places from a city's buildings: homes and jobs by floor area, every `school_every`-th building
/// a school, active ground floors as shops, cafés and bars in turn, and the parks.
pub fn places_from_buildings(
    buildings: &[Building],
    parks: &[[f32; 2]],
    uses: &Uses,
) -> Vec<Place> {
    let mut places = Vec::new();
    for (i, b) in buildings.iter().enumerate() {
        let residents = (b.floor_area * uses.homes / uses.home_area) as u32;
        if residents > 0 {
            places.push(Place {
                kind: PlaceKind::Home,
                pos: b.pos,
                capacity: residents,
            });
        }
        let jobs = (b.floor_area * (1.0 - uses.homes) / uses.job_area) as u32;
        let school = uses.school_every > 0 && i as u32 % uses.school_every == uses.school_every / 2;
        if school {
            places.push(Place {
                kind: PlaceKind::School,
                pos: b.pos,
                capacity: 600,
            });
        } else if jobs > 0 {
            places.push(Place {
                kind: PlaceKind::Work,
                pos: b.pos,
                capacity: jobs,
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
            });
        }
    }
    places.extend(parks.iter().map(|&pos| Place {
        kind: PlaceKind::Park,
        pos,
        capacity: 500,
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
}

impl World {
    /// Indexes the places in cells of `cell` metres over a square frame of `size` metres.
    pub fn new(places: Vec<Place>, size: f32, cell: f32) -> Self {
        let side = (size / cell).ceil().max(1.0) as usize + 1;
        let mut grids = vec![vec![Vec::new(); side * side]; KINDS];
        for (i, p) in places.iter().enumerate() {
            let x = ((p.pos[0] / cell).floor().max(0.0) as usize).min(side - 1);
            let y = ((p.pos[1] / cell).floor().max(0.0) as usize).min(side - 1);
            grids[p.kind as usize][y * side + x].push(i as u32);
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
        }
    }

    /// One of the few places of `kind` nearest `pos`, chosen by `pick`, or `None` if the city has
    /// none.
    pub fn near(&self, kind: PlaceKind, pos: [f32; 2], pick: u64) -> Option<u32> {
        let grid = &self.grids[kind as usize];
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
            // A ring past the first that found any is enough for "a few nearest".
            if found.len() >= 4 && ring >= 1 {
                break;
            }
        }
        if found.is_empty() {
            return None;
        }
        found.sort_by(|a, b| a.0.partial_cmp(&b.0).expect("finite").then(a.1.cmp(&b.1)));
        let k = found.len().min(4);
        Some(found[(pick % k as u64) as usize].1)
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
/// work at a distance drawn from an exponential of mean 2.5 km in a random direction, their school
/// the nearest.
pub fn generate(world: &World, mix: &Mix, seed: Seed) -> Population {
    let mut pop = Population::default();
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
        for _ in 0..home.capacity {
            let id = pop.len() as u64;
            let own = seed.derive(id).value();
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
                        let distance = -2500.0 * dmath::ln(1.0 - rng.next_f64() * 0.98);
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
    let near = |kind: PlaceKind, pick: u64| world.near(kind, pos, pick).unwrap_or(home);
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
            plan.push(anchor, hm(17, 0) + j(0, 5400));
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
            } else if j(0, 3) == 0 {
                plan.push(near(PlaceKind::Park, pick), hm(18, 30) + j(0, 1800));
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
            plan.push(home, hm(15, 0) + j(0, 3600));
            if j(0, 2) == 0 {
                plan.push(near(PlaceKind::Park, pick ^ 1), hm(17, 0) + j(0, 3600));
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

/// How long a trip takes, game seconds: on foot under 1.2 km (1.3 m/s), by bike under 4 km
/// (4.5 m/s) for the open, else by tram (5.5 m/s with a 5-minute wait); the street distance a
/// third longer than the straight line.
pub fn travel_time(world: &World, from: u32, to: u32, personality: Personality) -> GameSeconds {
    let d = world.distance(from, to) * 1.3;
    let s = if d < 1200.0 {
        d / 1.3
    } else if d < 4000.0 && personality.openness() >= 5 {
        d / 4.5
    } else {
        300.0 + d / 5.5
    };
    s.round().max(60.0) as GameSeconds
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

    /// Person `p`'s trip, if on one: from where, to where, since when and for how long.
    pub fn trip(&self, p: usize) -> Option<Trip> {
        (self.pop.at[p] & TRIP != 0).then(|| {
            let (from, to) = (self.pop.origin[p], self.pop.dest[p]);
            Trip {
                from,
                to,
                since: self.pop.since[p],
                time: travel_time(self.world, from, to, self.pop.personality[p]),
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
