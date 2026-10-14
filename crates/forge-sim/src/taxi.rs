//! An automated taxi fleet on a road network (the city's D-061; the owner's model): people book
//! a taxi from their smart pad or communicator, the nearest free one drives to them and carries
//! them; idle taxis wait at their stands, and at busy hours they circulate round the busy places,
//! where the next booking is most likely.
//!
//! Deterministic: the same network, stands, demand and seed, stepped the same way, gives the same
//! fleet (D-016). Taxis drive at a city pace with no traffic of their own to meet (they pass
//! through one another for now).

use std::cmp::Reverse;
use std::collections::{BinaryHeap, VecDeque};

use forge_core::SplitMix64;
use glam::DVec2;

/// A road network: nodes and the two-way links between them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    /// The nodes' positions, metres.
    pub nodes: Vec<DVec2>,
    /// The links.
    pub links: Vec<Link>,
}

/// A two-way road between two nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    /// Its first node.
    pub a: u32,
    /// Its last node.
    pub b: u32,
    /// Its centreline from `a` to `b`.
    pub points: Vec<DVec2>,
    /// Its length, metres.
    pub length: f64,
}

impl Link {
    /// A link along `points` from node `a` to node `b`.
    pub fn new(a: u32, b: u32, points: Vec<DVec2>) -> Self {
        let length = points.windows(2).map(|w| w[0].distance(w[1])).sum();
        Self {
            a,
            b,
            points,
            length,
        }
    }

    /// The point `s` metres from `a` and the way towards `b`.
    pub fn at(&self, s: f64) -> (DVec2, DVec2) {
        let mut left = s.clamp(0.0, self.length);
        for w in self.points.windows(2) {
            let len = w[0].distance(w[1]);
            if left <= len && len > 0.0 {
                let dir = (w[1] - w[0]) / len;
                return (w[0] + dir * left, dir);
            }
            left -= len;
        }
        let n = self.points.len();
        let dir = (self.points[n - 1] - self.points[n - 2]).normalize_or_zero();
        (self.points[n - 1], dir)
    }
}

/// A place on the network: a link and how far along it from its `a`, metres.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// The link.
    pub link: u32,
    /// Metres from its `a`.
    pub s: f64,
}

impl Network {
    /// The spot nearest `p`.
    pub fn nearest(&self, p: DVec2) -> Option<Spot> {
        let mut best: Option<(f64, Spot)> = None;
        for (k, l) in self.links.iter().enumerate() {
            let mut walked = 0.0;
            for w in l.points.windows(2) {
                let d = w[1] - w[0];
                let len = d.length();
                let t = if len > 0.0 {
                    ((p - w[0]).dot(d) / (len * len)).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let q = w[0] + d * t;
                let dist = q.distance(p);
                if best.is_none_or(|(b, _)| dist < b) {
                    best = Some((
                        dist,
                        Spot {
                            link: k as u32,
                            s: walked + len * t,
                        },
                    ));
                }
                walked += len;
            }
        }
        best.map(|(_, s)| s)
    }

    /// A spot's position and the way along its link.
    pub fn at(&self, spot: Spot) -> (DVec2, DVec2) {
        self.links[spot.link as usize].at(spot.s)
    }
}

/// A stretch of a route: a link from `from` to `to` metres along it (decreasing when driven
/// from `b` to `a`).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Leg {
    link: u32,
    from: f64,
    to: f64,
}

impl Leg {
    fn forward(&self) -> bool {
        self.to >= self.from
    }

    fn length(&self) -> f64 {
        (self.to - self.from).abs()
    }

    /// Metres from the end of `link` it enters by, `into` metres into the leg.
    fn along(&self, link: &Link, into: f64) -> f64 {
        if self.forward() {
            self.from + into
        } else {
            link.length - self.from + into
        }
    }

    /// The node it ends at (a leg before a route's last ends at its link's end).
    fn end(&self, link: &Link) -> u32 {
        if self.forward() { link.b } else { link.a }
    }
}

/// A taxi in a lane: its link, its way along it, how far from the end it entered by, and its
/// index. Sorted, the fleet's lanes front to back.
type Lane = (u32, bool, f64, u32);

/// Nobody holds the junction.
const NOBODY: u32 = u32::MAX;

/// The first taxi in the lane `link`, `forward` past `along` metres from its entry (or at it, by a
/// higher index than `me`), other than `me`: how far along and which.
fn first_past(lanes: &[Lane], link: u32, forward: bool, along: f64, me: u32) -> Option<(f64, u32)> {
    let start = lanes.partition_point(|o| {
        (o.0, o.1) < (link, forward)
            || ((o.0, o.1) == (link, forward) && (o.2 < along || (o.2 == along && o.3 <= me)))
    });
    lanes[start..]
        .iter()
        .take_while(|o| o.0 == link && o.1 == forward)
        .find(|o| o.3 != me)
        .map(|o| (o.2, o.3))
}

/// A route over the network.
#[derive(Clone, Debug, Default, PartialEq)]
struct Route {
    legs: Vec<Leg>,
    length: f64,
}

impl Route {
    /// The position and heading `s` metres along.
    fn at(&self, net: &Network, s: f64) -> (DVec2, DVec2) {
        let mut left = s.clamp(0.0, self.length);
        for leg in &self.legs {
            let len = (leg.to - leg.from).abs();
            if left <= len || std::ptr::eq(leg, self.legs.last().expect("a leg")) {
                let forward = leg.to >= leg.from;
                let along = if forward {
                    leg.from + left.min(len)
                } else {
                    leg.from - left.min(len)
                };
                let (p, dir) = net.links[leg.link as usize].at(along);
                return (p, if forward { dir } else { -dir });
            }
            left -= len;
        }
        (DVec2::ZERO, DVec2::X)
    }
}

/// What a taxi is doing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Task {
    /// Parked at a stand in a bay, waiting.
    Waiting {
        /// The stand.
        stand: u32,
        /// The bay.
        bay: u32,
    },
    /// Driving to pick a booking's passenger up.
    Fetching {
        /// The booking's index among the fleet's bookings.
        booking: u32,
    },
    /// Carrying a booking's passenger.
    Carrying {
        /// The booking.
        booking: u32,
    },
    /// Stopped at the kerb, someone getting in or out, for the seconds left.
    Stopped {
        /// Seconds left.
        left: f64,
        /// What follows: carrying that booking (`Some`), or free (`None`).
        then: Option<u32>,
    },
    /// Driving back to a stand.
    Returning {
        /// The stand.
        stand: u32,
    },
    /// Circulating round the busy places at a busy hour.
    Cruising,
}

/// A booking from someone's communicator.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Booking {
    /// When it was made, seconds.
    pub time: f64,
    /// Where the passenger waits.
    pub from: Spot,
    /// Where they go.
    pub to: Spot,
    /// When a taxi picked them up, if one has.
    pub picked: Option<f64>,
}

/// A taxi.
#[derive(Clone, Debug, PartialEq)]
pub struct Taxi {
    /// What it does.
    pub task: Task,
    route: Route,
    s: f64,
    speed: f64,
    /// Seconds it has waited at a stop line.
    waited: f64,
}

/// Where the demand comes from and goes to, and how much of it there is by the hour.
#[derive(Clone, Debug, PartialEq)]
pub struct Demand {
    /// Places people book from or go to, with their weight (homes and shops).
    pub places: Vec<(Spot, f64)>,
    /// The trips an hour across the city that could take a taxi (by tram or by taxi), hour by
    /// hour from midnight.
    pub per_hour: [f64; 24],
    /// The hours the fleet circulates rather than waits: bookings an hour over this.
    pub busy_over: f64,
    /// Each tram line's stops: a trip with both ends within [`Demand::walk`] of one line's stops
    /// takes the tram (the owner: a taxi has to be useful to the one who takes it).
    pub tram_stops: Vec<Vec<DVec2>>,
    /// How far people walk to a tram stop, metres.
    pub walk: f64,
    /// Which lines meet, by line: a trip takes the tram with one change where a line near its
    /// start meets one near its end ([`Demand::meets`]).
    pub tram_meets: Vec<Vec<bool>>,
    /// The share of the trips the trams do not serve that book a taxi; the rest change trams,
    /// walk or cycle.
    pub taxi_share: f64,
}

impl Demand {
    /// Which of the lines `stops` meet: two lines with stops within `change` metres of each
    /// other, where people change trams.
    pub fn meets(stops: &[Vec<DVec2>], change: f64) -> Vec<Vec<bool>> {
        let n = stops.len();
        let mut meets = vec![vec![false; n]; n];
        for i in 0..n {
            meets[i][i] = true;
            for j in i + 1..n {
                let m = stops[i]
                    .iter()
                    .any(|p| stops[j].iter().any(|q| p.distance(*q) <= change));
                meets[i][j] = m;
                meets[j][i] = m;
            }
        }
        meets
    }

    /// Whether a trip from `a` to `b` takes the tram: a line stops within a walk of both, or a
    /// line near `a` meets one near `b` (one change).
    pub fn by_tram(&self, a: DVec2, b: DVec2) -> bool {
        let near = |p: DVec2| -> Vec<usize> {
            (0..self.tram_stops.len())
                .filter(|&l| {
                    self.tram_stops[l]
                        .iter()
                        .any(|s| s.distance(p) <= self.walk)
                })
                .collect()
        };
        let (from, to) = (near(a), near(b));
        from.iter().any(|&i| {
            to.iter().any(|&j| {
                i == j
                    || self
                        .tram_meets
                        .get(i)
                        .and_then(|m| m.get(j))
                        .copied()
                        .unwrap_or(false)
            })
        })
    }
}

/// A stand: its spot on the network and how many bays it has.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stand {
    /// Where it is.
    pub at: Spot,
    /// Its bays.
    pub bays: u32,
}

/// The pace taxis keep in town, metres a second (30 km/h).
pub const CRUISE: f64 = 8.3;
/// How hard they speed up and slow down, metres a second squared.
pub const ACCEL: f64 = 1.5;
/// How long a passenger takes to get in or out, seconds.
pub const DWELL: f64 = 25.0;
/// The shortest trip people book a taxi for, metres (shorter, they walk or take the tram).
pub const SHORTEST_TRIP: f64 = 700.0;
/// A cab's length, metres (`forge_geom::vehicles::taxi_cab`).
pub const LENGTH: f64 = 3.6;
/// The gap a taxi keeps to the one ahead when both stand, metres.
pub const STANDSTILL: f64 = 2.0;
/// The time gap it keeps to the one ahead at speed, seconds.
const HEADWAY: f64 = 1.2;
/// How hard it brakes in comfort, metres a second squared.
const BRAKE: f64 = 2.0;
/// How far along its route it looks for the taxi ahead, metres.
const LOOKAHEAD: f64 = 60.0;
/// Where a taxi waits for a junction another holds: this far before its node, metres (behind
/// the crossings round it).
pub const STOP_LINE: f64 = 10.0;
/// How far before a junction a taxi asks for it, metres.
const APPROACH: f64 = 25.0;
/// How far past a junction's node a taxi holds it, metres.
const CLEAR: f64 = 8.0;
/// How long a taxi waits at a stop line before it goes anyway, seconds: no two queues wait on
/// each other for ever.
const PATIENCE: f64 = 20.0;
/// How far from a junction pick-ups and stands keep, metres (a taxi stopped there would hold
/// the junction's traffic).
pub const JUNCTION_CLEAR: f64 = 15.0;

/// The fleet.
#[derive(Clone, Debug)]
pub struct Fleet {
    net: Network,
    /// Each node's links out: (link, the other node, whether the link runs from this node).
    adjacency: Vec<Vec<(u32, u32, bool)>>,
    /// The taxi crossing each junction (three links or more), or `NOBODY`.
    holder: Vec<u32>,
    /// The stands.
    pub stands: Vec<Stand>,
    /// The taxis.
    pub taxis: Vec<Taxi>,
    /// Every booking made so far.
    pub bookings: Vec<Booking>,
    waiting: VecDeque<u32>,
    demand: Demand,
    cumulative: Vec<f64>,
    rng: SplitMix64,
    /// The fleet's clock, seconds from midnight of day 0.
    pub time: f64,
    next_booking: f64,
    /// The trips taken by tram so far: one line served both ends.
    pub by_tram: u64,
    /// The trips the trams did not serve that went another way than by taxi so far.
    pub otherwise: u64,
}

impl Fleet {
    /// A fleet of `taxis` waiting at `stands` (filled bay by bay in turn) on `net`, its clock at
    /// `time` seconds from midnight.
    pub fn new(
        net: Network,
        mut stands: Vec<Stand>,
        taxis: usize,
        mut demand: Demand,
        seed: u64,
        time: f64,
    ) -> Self {
        let mut adjacency = vec![Vec::new(); net.nodes.len()];
        for (k, l) in net.links.iter().enumerate() {
            adjacency[l.a as usize].push((k as u32, l.b, true));
            adjacency[l.b as usize].push((k as u32, l.a, false));
        }
        // Pick-ups and stands clear of the junctions.
        let clear = |spot: &mut Spot| {
            let length = net.links[spot.link as usize].length;
            let margin = JUNCTION_CLEAR.min(length * 0.5);
            spot.s = spot.s.clamp(margin, length - margin);
        };
        for (spot, _) in &mut demand.places {
            clear(spot);
        }
        for stand in &mut stands {
            clear(&mut stand.at);
        }
        let mut fleet_taxis = Vec::with_capacity(taxis);
        for k in 0..taxis {
            let n = stands.len().max(1);
            let (stand, bay) = ((k % n) as u32, (k / n) as u32);
            fleet_taxis.push(Taxi {
                task: Task::Waiting { stand, bay },
                route: Route::default(),
                s: 0.0,
                speed: 0.0,
                waited: 0.0,
            });
        }
        let mut cumulative = Vec::with_capacity(demand.places.len());
        let mut total = 0.0;
        for &(_, w) in &demand.places {
            total += w.max(0.0);
            cumulative.push(total);
        }
        let holder = vec![NOBODY; net.nodes.len()];
        let mut fleet = Self {
            net,
            holder,
            adjacency,
            stands,
            taxis: fleet_taxis,
            bookings: Vec::new(),
            waiting: VecDeque::new(),
            demand,
            cumulative,
            rng: SplitMix64::new(seed),
            time,
            next_booking: time,
            by_tram: 0,
            otherwise: 0,
        };
        fleet.next_booking = time + fleet.gap();
        fleet
    }

    /// The network.
    pub fn network(&self) -> &Network {
        &self.net
    }

    /// The hour of the day at the fleet's clock.
    pub fn hour(&self) -> usize {
        ((self.time / 3600.0).floor() as i64).rem_euclid(24) as usize
    }

    /// Whether the fleet circulates now rather than waits.
    pub fn busy(&self) -> bool {
        self.demand.per_hour[self.hour()] > self.demand.busy_over
    }

    /// The seconds to the next booking, drawn from the hour's rate.
    fn gap(&mut self) -> f64 {
        let rate = self.demand.per_hour[self.hour()].max(1e-3) / 3600.0;
        let u = (self.rng.next_f64()).clamp(1e-12, 1.0);
        -u.ln() / rate
    }

    /// A place drawn by its weight.
    fn place(&mut self) -> Option<Spot> {
        let total = *self.cumulative.last()?;
        let x = self.rng.next_f64() * total;
        let i = self.cumulative.partition_point(|&c| c < x);
        self.demand
            .places
            .get(i.min(self.demand.places.len() - 1))
            .map(|p| p.0)
    }

    /// Advances the fleet to `time`, in steps of at most `dt` seconds.
    pub fn advance_to(&mut self, time: f64, dt: f64) {
        while self.time + 1e-9 < time {
            let step = (time - self.time).min(dt);
            self.step(step);
        }
    }

    /// One step of `dt` seconds.
    pub fn step(&mut self, dt: f64) {
        self.time += dt;
        // The bookings made in this step.
        while self.next_booking <= self.time {
            let made = self.next_booking;
            self.next_booking += self.gap();
            let (Some(from), Some(to)) = (self.place(), self.place()) else {
                break;
            };
            let (a, _) = self.net.at(from);
            let (b, _) = self.net.at(to);
            if a.distance(b) < SHORTEST_TRIP {
                continue;
            }
            // By tram where one line serves both ends; else a share of them books a taxi.
            if self.demand.by_tram(a, b) {
                self.by_tram += 1;
                continue;
            }
            if self.rng.next_f64() >= self.demand.taxi_share {
                self.otherwise += 1;
                continue;
            }
            self.bookings.push(Booking {
                time: made,
                from,
                to,
                picked: None,
            });
            self.waiting.push_back((self.bookings.len() - 1) as u32);
        }
        // Each waiting booking to the nearest free taxi.
        while let Some(&b) = self.waiting.front() {
            let Some(k) = self.nearest_free(self.bookings[b as usize].from) else {
                break;
            };
            self.waiting.pop_front();
            let from = self.taxi_spot(k);
            let to = self.bookings[b as usize].from;
            self.send(k, from, to, Task::Fetching { booking: b });
        }
        // At a busy hour the taxis waiting behind a stand's first go out to circulate round the
        // busy places, one at a time as the road at the stand clears (they left together, one
        // on another); one stays at each stand.
        if self.busy() {
            let mut left = vec![false; self.stands.len()];
            for k in 0..self.taxis.len() {
                if let Task::Waiting { stand, bay } = self.taxis[k].task
                    && bay > 0
                    && !left[stand as usize]
                    && !self.crowded(self.stands[stand as usize].at)
                {
                    left[stand as usize] = true;
                    let at = self.stands[stand as usize].at;
                    self.free(k, at);
                    // Its bay is freed: the ones behind move up.
                    for t in &mut self.taxis {
                        if let Task::Waiting { stand: s, bay: b } = &mut t.task
                            && *s == stand
                            && *b > bay
                        {
                            *b -= 1;
                        }
                    }
                }
            }
        }
        // The junctions their holders have crossed are free again.
        for node in 0..self.holder.len() {
            let h = self.holder[node];
            if h != NOBODY && !self.holds(h as usize, node as u32) {
                self.holder[node] = NOBODY;
            }
        }
        // The lanes as they stand at the step's start: each taxi follows the one ahead of it in
        // its lane along its route, and waits at a junction another taxi crosses.
        let mut lanes: Vec<Lane> = (0..self.taxis.len())
            .filter_map(|k| {
                let (i, into) = self.on_leg(k)?;
                let leg = self.taxis[k].route.legs[i];
                let link = &self.net.links[leg.link as usize];
                Some((leg.link, leg.forward(), leg.along(link, into), k as u32))
            })
            .collect();
        lanes.sort_by(|a, b| {
            (a.0, a.1)
                .cmp(&(b.0, b.1))
                .then(a.2.total_cmp(&b.2))
                .then(a.3.cmp(&b.3))
        });
        for k in 0..self.taxis.len() {
            let (mut gap, mut lead, junction) = self.ahead(k, &lanes);
            if let Some((node, room, to_node)) = junction {
                let h = self.holder[node as usize];
                let patient = self.taxis[k].waited >= PATIENCE;
                if h != k as u32 && ((h == NOBODY && room) || patient) {
                    self.holder[node as usize] = k as u32;
                }
                if self.holder[node as usize] == k as u32 {
                    self.taxis[k].waited = 0.0;
                } else if to_node > STOP_LINE - 1.0 {
                    // At its stop line, behind the crossings round the junction.
                    let line = (to_node - STOP_LINE).max(0.0);
                    if line < gap {
                        gap = line;
                        lead = 0.0;
                    }
                    if self.taxis[k].speed < 0.5 {
                        self.taxis[k].waited += dt;
                    }
                }
            }
            self.drive(k, dt, gap, lead);
        }
    }

    /// Where taxi `k` is along its route: its leg and how far into it; `None` off the roads
    /// (waiting in its lay-by, or with no route).
    fn on_leg(&self, k: usize) -> Option<(usize, f64)> {
        let t = &self.taxis[k];
        if matches!(t.task, Task::Waiting { .. }) || t.route.legs.is_empty() {
            return None;
        }
        let mut left = t.s.clamp(0.0, t.route.length);
        let last = t.route.legs.len() - 1;
        for (i, leg) in t.route.legs.iter().enumerate() {
            if left <= leg.length() || i == last {
                return Some((i, left.min(leg.length())));
            }
            left -= leg.length();
        }
        None
    }

    /// Whether a taxi on the road stands within a length and a gap of `spot`, either way.
    fn crowded(&self, spot: Spot) -> bool {
        (0..self.taxis.len()).any(|k| {
            self.on_leg(k).is_some_and(|(i, into)| {
                let leg = self.taxis[k].route.legs[i];
                let s = if leg.forward() {
                    leg.from + into
                } else {
                    leg.from - into
                };
                leg.link == spot.link && (s - spot.s).abs() < LENGTH + STANDSTILL
            })
        })
    }

    /// Whether taxi `k` is still crossing junction `node`: driving up to it or just past it.
    fn holds(&self, k: usize, node: u32) -> bool {
        let t = &self.taxis[k];
        if matches!(t.task, Task::Waiting { .. } | Task::Stopped { .. }) {
            return false;
        }
        let Some((i, into)) = self.on_leg(k) else {
            return false;
        };
        let legs = &t.route.legs;
        let link = |leg: &Leg| &self.net.links[leg.link as usize];
        let coming = i + 1 < legs.len()
            && legs[i].end(link(&legs[i])) == node
            && legs[i].length() - into <= APPROACH;
        let leaving = i > 0 && legs[i - 1].end(link(&legs[i - 1])) == node && into < CLEAR;
        coming || leaving
    }

    /// What lies ahead of taxi `k` in `lanes`: the gap to the taxi in front along its route,
    /// bumper to bumper (metres), and its speed; and the junction it nears, if any: its node,
    /// whether there is room for the taxi past it, and how far the node is.
    fn ahead(&self, k: usize, lanes: &[Lane]) -> (f64, f64, Option<(u32, bool, f64)>) {
        let none = (f64::INFINITY, 0.0, None);
        if matches!(self.taxis[k].task, Task::Stopped { .. }) {
            return none;
        }
        let Some((i, into)) = self.on_leg(k) else {
            return none;
        };
        let me = k as u32;
        let legs = &self.taxis[k].route.legs;
        let link = |leg: &Leg| &self.net.links[leg.link as usize];
        let (mut gap, mut lead) = (f64::INFINITY, 0.0);
        // In its own leg, then along the next ones as far as it looks.
        let mut to_leg = -into;
        for (j, leg) in legs.iter().enumerate().skip(i) {
            if to_leg > LOOKAHEAD {
                break;
            }
            let entry = if j == i { into } else { 0.0 };
            let mut from = leg.along(link(leg), entry);
            // Pulling out, it goes before a taxi level with it (one stopped at the kerb where it
            // turned, which it waited behind for the whole stop).
            if self.taxis[k].s < 0.01 && j == i {
                from += 0.01;
            }
            if let Some((at, other)) = first_past(lanes, leg.link, leg.forward(), from, me)
                && at <= leg.along(link(leg), leg.length()) + LENGTH
            {
                gap = to_leg + (at - leg.along(link(leg), 0.0)) - LENGTH;
                lead = self.taxis[other as usize].speed;
                break;
            }
            to_leg += leg.length();
        }
        // The junction at its leg's end.
        let mut junction = None;
        if i + 1 < legs.len() {
            let node = legs[i].end(link(&legs[i]));
            let to_node = legs[i].length() - into;
            if self.adjacency[node as usize].len() >= 3 && to_node <= APPROACH {
                let exit = &legs[i + 1];
                let entry = exit.along(link(exit), 0.0);
                let room = first_past(lanes, exit.link, exit.forward(), entry, me)
                    .is_none_or(|(at, _)| at - entry >= 2.0 * LENGTH + STANDSTILL);
                junction = Some((node, room, to_node));
            }
        }
        (gap, lead, junction)
    }

    /// The free taxi nearest `spot` (waiting, returning or cruising), as the crow flies.
    fn nearest_free(&self, spot: Spot) -> Option<usize> {
        let (p, _) = self.net.at(spot);
        self.taxis
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                matches!(
                    t.task,
                    Task::Waiting { .. } | Task::Returning { .. } | Task::Cruising
                )
            })
            .map(|(k, _)| (self.pose(k).0.distance(p), k))
            .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
            .map(|(_, k)| k)
    }

    /// Where taxi `k` is on the network.
    fn taxi_spot(&self, k: usize) -> Spot {
        let t = &self.taxis[k];
        match t.task {
            Task::Waiting { stand, .. } => self.stands[stand as usize].at,
            _ => {
                let mut left = t.s.clamp(0.0, t.route.length);
                for leg in &t.route.legs {
                    let len = (leg.to - leg.from).abs();
                    if left <= len {
                        let s = if leg.to >= leg.from {
                            leg.from + left
                        } else {
                            leg.from - left
                        };
                        return Spot { link: leg.link, s };
                    }
                    left -= len;
                }
                let last = t.route.legs.last().copied().unwrap_or(Leg {
                    link: 0,
                    from: 0.0,
                    to: 0.0,
                });
                Spot {
                    link: last.link,
                    s: last.to,
                }
            }
        }
    }

    /// Sends taxi `k` from `from` to `to` on `task`.
    fn send(&mut self, k: usize, from: Spot, to: Spot, task: Task) {
        let route = self.route(from, to);
        let t = &mut self.taxis[k];
        t.route = route;
        t.s = 0.0;
        t.task = task;
    }

    /// The shortest route from `from` to `to` (Dijkstra in whole millimetres, ties broken by
    /// node, so the same everywhere).
    fn route(&self, from: Spot, to: Spot) -> Route {
        let (lf, lt) = (
            &self.net.links[from.link as usize],
            &self.net.links[to.link as usize],
        );
        // On one link, driving along it.
        let direct = (from.link == to.link).then(|| Route {
            legs: vec![Leg {
                link: from.link,
                from: from.s,
                to: to.s,
            }],
            length: (to.s - from.s).abs(),
        });
        let mm = |m: f64| (m * 1000.0).round() as u64;
        let n = self.net.nodes.len();
        let mut dist = vec![u64::MAX; n];
        let mut via: Vec<Option<(u32, u32, bool)>> = vec![None; n];
        let mut heap = BinaryHeap::new();
        for (node, cost) in [(lf.a, from.s), (lf.b, lf.length - from.s)] {
            let c = mm(cost);
            if c < dist[node as usize] {
                dist[node as usize] = c;
                heap.push(Reverse((c, node)));
            }
        }
        while let Some(Reverse((d, v))) = heap.pop() {
            if d > dist[v as usize] {
                continue;
            }
            for &(link, w, forward) in &self.adjacency[v as usize] {
                let nd = d + mm(self.net.links[link as usize].length);
                if nd < dist[w as usize] {
                    dist[w as usize] = nd;
                    via[w as usize] = Some((v, link, forward));
                    heap.push(Reverse((nd, w)));
                }
            }
        }
        // The best way onto the last link: by its `a` or its `b`.
        let ends = [(lt.a, to.s), (lt.b, lt.length - to.s)];
        let best = ends
            .iter()
            .filter(|(node, _)| dist[*node as usize] != u64::MAX)
            .map(|&(node, cost)| (dist[node as usize] + mm(cost), node))
            .min();
        let Some((total, last)) = best else {
            return direct.unwrap_or_default();
        };
        if let Some(d) = &direct
            && mm(d.length) <= total
        {
            return d.clone();
        }
        // Back from the last node to one of the first link's ends.
        let mut chain = Vec::new();
        let mut v = last;
        while let Some((u, link, forward)) = via[v as usize] {
            let l = &self.net.links[link as usize];
            chain.push(if forward {
                Leg {
                    link,
                    from: 0.0,
                    to: l.length,
                }
            } else {
                Leg {
                    link,
                    from: l.length,
                    to: 0.0,
                }
            });
            v = u;
        }
        chain.reverse();
        let mut legs = vec![Leg {
            link: from.link,
            from: from.s,
            to: if v == lf.a { 0.0 } else { lf.length },
        }];
        legs.extend(chain);
        legs.push(Leg {
            link: to.link,
            from: if last == lt.a { 0.0 } else { lt.length },
            to: to.s,
        });
        legs.retain(|l| (l.to - l.from).abs() > 1e-6);
        let length = legs.iter().map(|l| (l.to - l.from).abs()).sum();
        Route { legs, length }
    }

    /// The stand nearest `p` with a bay free, and that bay; the nearest stand if every one is
    /// full (a taxi waits behind).
    fn stand_for(&self, p: DVec2) -> Option<(u32, u32)> {
        let mut used = vec![0_u32; self.stands.len()];
        for t in &self.taxis {
            match t.task {
                Task::Waiting { stand, .. } | Task::Returning { stand } => {
                    used[stand as usize] += 1;
                }
                _ => {}
            }
        }
        let order = |free: bool| {
            self.stands
                .iter()
                .enumerate()
                .filter(|(i, s)| !free || used[*i] < s.bays)
                .map(|(i, s)| (self.net.at(s.at).0.distance(p), i))
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
                .map(|(_, i)| (i as u32, used[i]))
        };
        order(true).or_else(|| order(false))
    }

    fn drive(&mut self, k: usize, dt: f64, gap: f64, lead: f64) {
        let task = self.taxis[k].task;
        match task {
            Task::Waiting { .. } => return,
            Task::Stopped { left, then } => {
                let left = left - dt;
                if left > 0.0 {
                    self.taxis[k].task = Task::Stopped { left, then };
                    return;
                }
                let here = self.taxi_spot(k);
                match then {
                    Some(b) => {
                        let to = self.bookings[b as usize].to;
                        self.send(k, here, to, Task::Carrying { booking: b });
                    }
                    None => self.free(k, here),
                }
                return;
            }
            _ => {}
        }
        let t = &mut self.taxis[k];
        // Speeding up to the cruise, slowing to stop at the route's end; and behind the taxi or
        // the stop line `gap` metres ahead, which goes at `lead`, the Intelligent Driver Model
        // (Treiber, Hennecke and Helbing 2000): a gap of STANDSTILL standing, HEADWAY at speed.
        let remaining = (t.route.length - t.s).max(0.0);
        let can_stop = (2.0 * ACCEL * remaining).sqrt();
        let target = CRUISE.min(can_stop);
        let v = t.speed;
        let free = if v < target {
            (v + ACCEL * dt).min(target)
        } else {
            target
        };
        let wanted = STANDSTILL + v * HEADWAY + v * (v - lead) / (2.0 * (ACCEL * BRAKE).sqrt());
        let accel = ACCEL * (1.0 - (v / CRUISE).powi(4) - (wanted.max(0.0) / gap.max(0.1)).powi(2));
        t.speed = free.min((v + accel * dt).max(0.0));
        // Never into what is ahead.
        t.s += (t.speed * dt).min((gap - 0.2).max(0.0));
        if t.s + 0.05 < t.route.length {
            return;
        }
        t.s = t.route.length;
        t.speed = 0.0;
        match task {
            Task::Fetching { booking } => {
                self.bookings[booking as usize].picked = Some(self.time);
                self.taxis[k].task = Task::Stopped {
                    left: DWELL,
                    then: Some(booking),
                };
            }
            Task::Carrying { .. } => {
                self.taxis[k].task = Task::Stopped {
                    left: DWELL,
                    then: None,
                };
            }
            Task::Returning { stand } => {
                let bay = self
                    .taxis
                    .iter()
                    .filter(|o| matches!(o.task, Task::Waiting { stand: s, .. } if s == stand))
                    .count() as u32;
                self.taxis[k].task = Task::Waiting { stand, bay };
            }
            Task::Cruising => {
                let here = self.taxi_spot(k);
                self.free(k, here);
            }
            _ => {}
        }
    }

    /// Taxi `k`, free at `here`: at a busy hour it circulates to a busy place within 2 km (a
    /// place weighted by its demand), else it goes back to the nearest stand with a bay free.
    fn free(&mut self, k: usize, here: Spot) {
        let (p, _) = self.net.at(here);
        if self.busy() {
            for _ in 0..8 {
                let Some(to) = self.place() else { break };
                let (q, _) = self.net.at(to);
                if q.distance(p) <= 2000.0 && q.distance(p) > 200.0 {
                    self.send(k, here, to, Task::Cruising);
                    return;
                }
            }
        }
        match self.stand_for(p) {
            Some((stand, _)) => {
                let to = self.stands[stand as usize].at;
                self.send(k, here, to, Task::Returning { stand });
            }
            None => {
                self.taxis[k].task = Task::Cruising;
                self.taxis[k].route = Route::default();
            }
        }
    }

    /// Taxi `k`'s position and heading on the network (a waiting one's at its stand).
    pub fn pose(&self, k: usize) -> (DVec2, DVec2) {
        let t = &self.taxis[k];
        match t.task {
            Task::Waiting { stand, .. } => self.net.at(self.stands[stand as usize].at),
            _ => t.route.at(&self.net, t.s),
        }
    }

    /// How many bookings wait for a taxi, and the mean wait of those picked up so far, seconds.
    pub fn service(&self) -> (usize, f64) {
        let picked: Vec<f64> = self
            .bookings
            .iter()
            .filter_map(|b| b.picked.map(|p| p - b.time))
            .collect();
        let mean = if picked.is_empty() {
            0.0
        } else {
            picked.iter().sum::<f64>() / picked.len() as f64
        };
        (self.waiting.len(), mean)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A grid of roads 200 m apart, `n` × `n` nodes.
    fn grid(n: u32) -> Network {
        let mut net = Network::default();
        for y in 0..n {
            for x in 0..n {
                net.nodes
                    .push(DVec2::new(f64::from(x) * 200.0, f64::from(y) * 200.0));
            }
        }
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                if x + 1 < n {
                    let (a, b) = (net.nodes[i as usize], net.nodes[(i + 1) as usize]);
                    net.links.push(Link::new(i, i + 1, vec![a, b]));
                }
                if y + 1 < n {
                    let (a, b) = (net.nodes[i as usize], net.nodes[(i + n) as usize]);
                    net.links.push(Link::new(i, i + n, vec![a, b]));
                }
            }
        }
        net
    }

    fn fleet(per_hour: f64, taxis: usize) -> Fleet {
        let net = grid(8);
        let stands = [0_u32, 20, 40, 60]
            .iter()
            .map(|&l| Stand {
                at: Spot { link: l, s: 100.0 },
                bays: 2,
            })
            .collect();
        let places = (0..net.links.len() as u32)
            .map(|l| (Spot { link: l, s: 100.0 }, 1.0))
            .collect();
        let demand = Demand {
            places,
            per_hour: [per_hour; 24],
            busy_over: per_hour * 2.0,
            tram_stops: Vec::new(),
            walk: 400.0,
            tram_meets: Vec::new(),
            taxi_share: 1.0,
        };
        Fleet::new(net, stands, taxis, demand, 7, 8.0 * 3600.0)
    }

    #[test]
    fn a_booked_taxi_fetches_carries_and_comes_back() {
        let mut f = fleet(60.0, 8);
        f.advance_to(f.time + 2.0 * 3600.0, 0.5);
        let made = f.bookings.len();
        let picked = f.bookings.iter().filter(|b| b.picked.is_some()).count();
        let (left, wait) = f.service();
        assert!(made > 60, "{made} bookings in two hours");
        assert!(
            picked + left + 10 >= made,
            "{picked} of {made} picked, {left} waiting"
        );
        assert!(wait > 0.0 && wait < 600.0, "mean wait {wait:.0} s");
        // Every taxi on the network: never off it.
        for k in 0..f.taxis.len() {
            let (p, _) = f.pose(k);
            assert!(
                p.x >= -1.0 && p.y >= -1.0 && p.x <= 1401.0 && p.y <= 1401.0,
                "{p}"
            );
        }
    }

    #[test]
    fn the_same_fleet_steps_the_same_way() {
        let (mut a, mut b) = (fleet(80.0, 6), fleet(80.0, 6));
        a.advance_to(a.time + 1800.0, 0.5);
        b.advance_to(b.time + 1800.0, 0.5);
        assert_eq!(a.bookings, b.bookings);
        assert_eq!(a.taxis, b.taxis);
    }

    #[test]
    fn at_busy_hours_free_taxis_circulate() {
        let mut f = fleet(200.0, 12);
        f.demand.busy_over = 100.0;
        f.advance_to(f.time + 1800.0, 0.5);
        let cruising = f.taxis.iter().filter(|t| t.task == Task::Cruising).count();
        assert!(cruising > 0, "no taxi cruising at a busy hour");
    }

    #[test]
    fn routes_follow_the_roads_end_to_end() {
        let f = fleet(1.0, 1);
        let from = Spot { link: 0, s: 50.0 };
        let to = Spot {
            link: 100,
            s: 150.0,
        };
        let r = f.route(from, to);
        let (p0, _) = r.at(&f.net, 0.0);
        let (p1, _) = r.at(&f.net, r.length);
        assert!(p0.distance(f.net.at(from).0) < 1e-6);
        assert!(p1.distance(f.net.at(to).0) < 1e-6);
        // Along the grid, the Manhattan distance at least.
        let manhattan = (p1 - p0).abs().element_sum();
        assert!(r.length + 1e-6 >= manhattan, "{} < {manhattan}", r.length);
    }

    #[test]
    fn a_taxi_stops_behind_one_standing_in_its_lane() {
        let mut f = fleet(0.001, 2);
        // Taxi 1 stands at the kerb 100 m along link 0; taxi 0 drives up the same lane.
        let ahead = f.route(Spot { link: 0, s: 20.0 }, Spot { link: 0, s: 100.0 });
        f.taxis[1].route = ahead;
        f.taxis[1].s = f.taxis[1].route.length;
        f.taxis[1].task = Task::Stopped {
            left: 1e6,
            then: None,
        };
        let behind = f.route(Spot { link: 0, s: 10.0 }, Spot { link: 0, s: 190.0 });
        f.taxis[0].route = behind;
        f.taxis[0].task = Task::Cruising;
        f.advance_to(f.time + 60.0, 0.25);
        let (p0, _) = f.pose(0);
        let (p1, _) = f.pose(1);
        let gap = p0.distance(p1) - LENGTH;
        assert!(
            (STANDSTILL * 0.5..STANDSTILL + 1.0).contains(&gap),
            "a gap of {gap:.2} m behind the standing taxi"
        );
        assert!(
            f.taxis[0].speed < 0.01,
            "still moving at {}",
            f.taxis[0].speed
        );
    }

    #[test]
    fn no_two_taxis_overlap_in_a_lane() {
        let mut f = fleet(400.0, 40);
        f.demand.busy_over = 100.0;
        let (mut worst, mut samples, mut queued, mut pulling) = (f64::INFINITY, 0, 0, 0);
        for _ in 0..(2 * 3600 / 5) {
            f.advance_to(f.time + 5.0, 0.25);
            let mut lanes: Vec<(u32, bool, f64, usize)> = (0..f.taxis.len())
                .filter_map(|k| {
                    let (i, into) = f.on_leg(k)?;
                    let leg = f.taxis[k].route.legs[i];
                    let link = &f.net.links[leg.link as usize];
                    Some((leg.link, leg.forward(), leg.along(link, into), k))
                })
                .collect();
            lanes.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)).then(a.2.total_cmp(&b.2)));
            for w in lanes.windows(2) {
                if (w[0].0, w[0].1) != (w[1].0, w[1].1) {
                    continue;
                }
                samples += 1;
                let gap = w[1].2 - w[0].2 - LENGTH;
                if gap < STANDSTILL + 1.0 {
                    queued += 1;
                }
                // Pulling out from the kerb where it stopped, a taxi may pass by one standing
                // level with it or cut in ahead of one coming: for a few seconds.
                let starting = |k: usize| {
                    !matches!(f.taxis[k].task, Task::Stopped { .. })
                        && f.taxis[k].s < LENGTH + STANDSTILL
                };
                if starting(w[0].3) || starting(w[1].3) {
                    pulling += usize::from(gap < -0.5);
                    continue;
                }
                worst = worst.min(gap);
            }
        }
        println!(
            "{samples} pairs in one lane, {queued} queued, {pulling} pulling out across another, \
             the closest others {worst:.2} m apart"
        );
        assert!(samples > 100, "only {samples} pairs met in a lane");
        assert!(worst > -0.5, "two taxis {:.2} m into each other", -worst);
        assert!(
            pulling * 100 <= samples,
            "{pulling} of {samples} pulling out across another"
        );
    }

    #[test]
    fn one_taxi_at_a_time_crosses_a_junction() {
        let mut f = fleet(400.0, 40);
        f.demand.busy_over = 100.0;
        let junctions: Vec<DVec2> = (0..f.net.nodes.len())
            .filter(|&n| f.adjacency[n].len() >= 3)
            .map(|n| f.net.nodes[n])
            .collect();
        let (mut crossings, mut shared) = (0, 0);
        for _ in 0..(2 * 3600 * 2) {
            f.advance_to(f.time + 0.5, 0.25);
            for &node in &junctions {
                let inside = (0..f.taxis.len())
                    .filter(|&k| f.on_leg(k).is_some() && f.pose(k).0.distance(node) < 4.0)
                    .count();
                crossings += usize::from(inside > 0);
                shared += usize::from(inside > 1);
            }
        }
        println!("{crossings} junction samples with a taxi in, {shared} with two or more");
        assert!(
            crossings > 100,
            "only {crossings} junction samples with a taxi in"
        );
        assert!(
            shared * 50 <= crossings,
            "{shared} of {crossings} with two taxis in"
        );
    }

    #[test]
    fn a_trip_one_tram_line_serves_takes_the_tram() {
        // A line along the grid's first row, a stop every 200 m: trips between places on that
        // row go by tram; those to the far rows book taxis.
        let mut f = fleet(400.0, 8);
        f.demand.tram_stops = vec![
            (0..8)
                .map(|x| DVec2::new(f64::from(x) * 200.0, 0.0))
                .collect(),
        ];
        let row = |spot: &Spot| f.net.at(*spot).0.y < 1.0;
        let near: Vec<(Spot, f64)> = f
            .demand
            .places
            .iter()
            .copied()
            .filter(|(s, _)| row(s))
            .collect();
        f.demand.places = near;
        f.cumulative = (1..=f.demand.places.len()).map(|n| n as f64).collect();
        f.advance_to(f.time + 3600.0, 0.5);
        assert!(f.by_tram > 50, "{} trips by tram", f.by_tram);
        assert!(
            f.bookings.is_empty(),
            "{} taxis booked along the line",
            f.bookings.len()
        );
        // A share of the rest books.
        let mut g = fleet(400.0, 8);
        g.demand.taxi_share = 0.25;
        g.advance_to(g.time + 3600.0, 0.5);
        let (booked, other) = (g.bookings.len() as f64, g.otherwise as f64);
        let share = booked / (booked + other);
        assert!(
            (0.15..0.35).contains(&share),
            "{share:.2} of the trips booked a taxi"
        );
    }
}
