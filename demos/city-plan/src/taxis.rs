//! The city's automated taxis (`forge_sim::taxi`, D-061) in the scene: the fleet built from the
//! plan's taxi roads, stands and blocks, stepped with the scene's clock, and each cab placed on
//! its lane, at a stand's lay-by, or at the kerb picking someone up.

use forge_procgen::city::blocks::BlockUse;
use forge_procgen::city::mass::Mass;
use forge_procgen::city::{CityPlan, taxi};
use forge_sim::taxi::{Demand, Depot, Fleet, Link, Network, Spot, Stand, Task};
use glam::{DVec2, Vec3};

/// The share of a day's bookings in each hour from midnight: the night's few, the morning's and
/// the evening's peaks, a busier late evening out.
const HOURLY: [f64; 24] = [
    0.8, 0.5, 0.3, 0.2, 0.2, 0.5, 1.5, 4.0, 7.0, 6.0, 4.0, 4.0, 5.0, 5.0, 4.0, 4.0, 5.0, 7.0, 8.0,
    7.0, 6.0, 5.0, 4.0, 2.0,
];
/// The trips a person makes a day by tram or by taxi: of some 2.8 trips, 22 % by tram and 2 % by
/// taxi (D-059's expected shares; walking and cycling take most of the rest).
const TRANSIT_TRIPS_PER_PERSON: f64 = 0.68;
/// The trips a person makes a day by taxi: not everybody takes one (the owner), they take the
/// trips the trams serve badly.
const TAXI_TRIPS_PER_PERSON: f64 = 0.056;
/// How far people walk to a tram stop rather than take a taxi, metres.
const WALK_TO_STOP: f64 = 400.0;
/// How close two lines' stops stand where people change trams, metres.
const CHANGE: f64 = 150.0;
/// Trips one taxi serves in the busiest hour: a fetch, a ride, two stops and the way to the next
/// take twenty minutes or so in town, and a booking should wait a minute or so (one taxi for 90
/// trips a day left 527 bookings waiting at 9:00, 402 s on average; 4 an hour, 223 waiting).
const PEAK_TRIPS_PER_TAXI: f64 = 3.0;
/// The fleet's share of 8-seat shuttles: groups of five or more are about 3 % of the bookings
/// (an estimate), and ride longer.
const SHUTTLE_SHARE: f64 = 0.05;
/// How far a moving taxi keeps right of its road's centreline, metres: in its lane.
const LANE: f64 = 1.5;
/// How far right of the centreline a taxi stops to pick up, metres: at the kerb.
const KERB: f64 = 2.3;
/// The fleet's step, seconds.
const STEP: f64 = 0.25;
/// The cabs a stand's lay-by holds, one each end.
const BAYS: u32 = 2;

/// Where a taxi is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Its place on the plan.
    pub at: DVec2,
    /// Its yaw.
    pub yaw: f32,
    /// The level it stands on, metres.
    pub level: f64,
    /// Whether it is drawn.
    pub shown: bool,
}

/// A point of depot `d`'s deck's frame (`forge_geom::depot`) on the plan, and its level.
fn deck_point(d: &taxi::Depot, p: Vec3) -> (DVec2, f64) {
    let walk = forge_procgen::city::ground::WALK_LEVEL;
    let at = d.entrance() + d.across() * f64::from(p.x) - d.out * f64::from(p.z);
    (at, walk + f64::from(p.y))
}

/// A direction of depot `d`'s deck's frame on the plan.
fn deck_direction(d: &taxi::Depot, v: Vec3) -> DVec2 {
    (d.across() * f64::from(v.x) - d.out * f64::from(v.z)).normalize_or_zero()
}

/// Where along `way` (a path in a deck's frame) a cab is at `t` of the way (0 to 1): the point
/// and the way it heads, nose first, or backing out along the first stretch (`backing`).
fn along_way(way: &[Vec3], t: f64, backing: bool) -> (Vec3, Vec3) {
    let lengths: Vec<f32> = way.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    let mut left = t.clamp(0.0, 1.0) as f32 * total;
    for (i, w) in way.windows(2).enumerate() {
        if left <= lengths[i] || i + 1 == lengths.len() {
            let f = if lengths[i] > 0.0 {
                (left / lengths[i]).min(1.0)
            } else {
                1.0
            };
            let mut dir = Vec3::new(w[1].x - w[0].x, 0.0, w[1].z - w[0].z).normalize_or_zero();
            if backing && i == 0 {
                dir = -dir;
            }
            return (w[0].lerp(w[1], f), dir);
        }
        left -= lengths[i];
    }
    (way[0], Vec3::Z)
}

/// The scene's taxis.
pub struct Taxis {
    fleet: Fleet,
    stands: Vec<taxi::Stand>,
    depots: Vec<taxi::Depot>,
    /// The bays of a depot's deck (`forge_geom::depot::bays`).
    bays: Vec<forge_geom::depot::Bay>,
    /// Each taxi's bay in its depot while it is parked or driving out: its depot and the bay.
    bay_of: Vec<Option<(u32, usize)>>,
    /// Each depot's bays taken.
    taken: Vec<Vec<bool>>,
    /// The fleet's clock less the scene's.
    offset: f64,
    /// The share of the trips by tram or taxi that the trams do not serve (one change at most).
    pub unserved: f64,
    /// The share of those that book a taxi.
    pub taxi_share: f64,
}

impl Taxis {
    /// The fleet for `plan` and its `masses` (`nets`: each mass's net floor area), its day at
    /// `hour` when the scene's clock reads `clock`; `None` without taxi roads or stands.
    pub fn new(
        plan: &CityPlan,
        masses: &[Mass],
        nets: &[f64],
        hour: f64,
        clock: f64,
    ) -> Option<Self> {
        // The driven roads as the fleet's network.
        let mut index = vec![u32::MAX; plan.roads.nodes.len()];
        let mut net = Network::default();
        let mut link_of = vec![u32::MAX; plan.roads.edges.len()];
        for (k, e) in plan.roads.edges.iter().enumerate() {
            if !e.profile.driven() {
                continue;
            }
            for n in [e.a, e.b] {
                if index[n as usize] == u32::MAX {
                    index[n as usize] = net.nodes.len() as u32;
                    net.nodes.push(plan.roads.nodes[n as usize].pos);
                }
            }
            link_of[k] = net.links.len() as u32;
            net.links.push(Link::new(
                index[e.a as usize],
                index[e.b as usize],
                e.points.clone(),
            ));
        }
        if net.links.is_empty() || plan.taxi_stands.is_empty() {
            return None;
        }
        let stands: Vec<Stand> = plan
            .taxi_stands
            .iter()
            .filter_map(|s| {
                let link = link_of[s.edge as usize];
                (link != u32::MAX).then(|| {
                    let l = &net.links[link as usize];
                    // The stand's place along its road: its kerb point's projection.
                    let spot = nearest_on(l, s.kerb);
                    Stand {
                        at: Spot { link, s: spot },
                        bays: BAYS,
                    }
                })
            })
            .collect();
        // Where people book from and go to: the built blocks by their floor area, the squares.
        let mut area = vec![0.0_f64; plan.blocks.len()];
        for (m, &n) in masses.iter().zip(nets) {
            area[plan.lots[m.lot as usize].block as usize] += n;
        }
        let places = plan
            .blocks
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                let weight = match b.use_ {
                    BlockUse::Built => area[i],
                    BlockUse::Square => 5_000.0,
                    _ => 0.0,
                };
                (weight > 0.0)
                    .then(|| net.nearest(b.centre).map(|s| (s, weight)))
                    .flatten()
            })
            .collect();
        let people: f64 = nets.iter().sum::<f64>() * (2.0 / 3.0) / 32.4;
        let daily = people * TRANSIT_TRIPS_PER_PERSON;
        let total: f64 = HOURLY.iter().sum();
        let per_hour = HOURLY.map(|h| daily * h / total);
        let mean = daily / 24.0;
        let tram_stops: Vec<Vec<DVec2>> = plan.tram_lines.iter().map(|l| l.stops.clone()).collect();
        let tram_meets = Demand::meets(&tram_stops, CHANGE);
        let mut demand = Demand {
            places,
            per_hour,
            busy_over: mean * 1.3,
            tram_stops,
            walk: WALK_TO_STOP,
            tram_meets,
            taxi_share: 1.0,
        };
        // Of the trips long enough to ride, those the trams do not serve with one change at most
        // (drawn as the fleet draws them): a share of them books a taxi, so that taxis carry
        // TAXI_TRIPS_PER_PERSON.
        let unserved = unserved_share(&net, &demand);
        demand.taxi_share =
            (TAXI_TRIPS_PER_PERSON / (TRANSIT_TRIPS_PER_PERSON * unserved).max(1e-9)).min(1.0);
        let peak = per_hour.iter().copied().fold(0.0, f64::max) * unserved * demand.taxi_share;
        let taxis = ((peak / PEAK_TRIPS_PER_TAXI).round() as usize).clamp(10, 6000);
        let start = hour * 3600.0;
        let taxi_share = demand.taxi_share;
        // The depots, where the taxis no stand holds park and charge.
        let depots: Vec<Depot> = plan
            .depots
            .iter()
            .filter_map(|d| {
                let link = link_of[d.edge as usize];
                (link != u32::MAX).then(|| {
                    let l = &net.links[link as usize];
                    Depot {
                        at: Spot {
                            link,
                            s: nearest_on(l, d.kerb),
                        },
                        exit: Spot {
                            link,
                            s: nearest_on(l, d.exit_kerb()),
                        },
                        capacity: d.capacity() as u32,
                    }
                })
            })
            .collect();
        // A share of shuttles for the groups, one a depot at least.
        let shuttles = ((taxis as f64 * SHUTTLE_SHARE).round() as usize).max(depots.len());
        let fleet = Fleet::new(net, stands, taxis, demand, 0x7A_C1, start)
            .with_shuttles(shuttles)
            .with_depots(depots);
        let bays = forge_geom::depot::bays();
        let mut taxis = Self {
            bay_of: vec![None; fleet.taxis.len()],
            taken: vec![vec![false; bays.len()]; plan.depots.len()],
            bays,
            fleet,
            stands: plan.taxi_stands.clone(),
            depots: plan.depots.clone(),
            offset: start - clock,
            unserved,
            taxi_share,
        };
        taxis.keep_bays();
        Some(taxis)
    }

    /// Each taxi parked in a depot to a bay of its own, the first free, until it has driven out
    /// (taking each one's place in the order parked made the cabs jump bays as others left).
    fn keep_bays(&mut self) {
        for k in 0..self.fleet.taxis.len() {
            let parked = match self.fleet.taxis[k].task {
                Task::Parked { depot } => Some(depot),
                _ => None,
            };
            match (parked, self.bay_of[k]) {
                (Some(depot), None) => {
                    let taken = &mut self.taken[depot as usize];
                    if let Some(b) = taken.iter().position(|t| !t) {
                        taken[b] = true;
                        self.bay_of[k] = Some((depot, b));
                    }
                }
                (None, Some((depot, b))) if self.fleet.hold(k) <= 0.0 => {
                    self.taken[depot as usize][b] = false;
                    self.bay_of[k] = None;
                }
                _ => {}
            }
        }
    }

    /// How many of them are shuttles: the last ones.
    pub fn shuttles(&self) -> usize {
        self.fleet
            .taxis
            .iter()
            .filter(|t| t.seats > forge_sim::taxi::CAB_SEATS)
            .count()
    }

    /// How many taxis there are.
    pub fn len(&self) -> usize {
        self.fleet.taxis.len()
    }

    /// Steps the fleet to the scene's `clock`.
    pub fn advance(&mut self, clock: f64) {
        let to = clock + self.offset;
        // A jump of more than ten minutes (the clock set by hand): the fleet starts again there.
        if to - self.fleet.time > 600.0 || to < self.fleet.time {
            self.offset = self.fleet.time - clock;
            return;
        }
        self.fleet.advance_to(to, STEP);
        self.keep_bays();
    }

    /// Each taxi's place: in its lane, at the kerb, in its stand's lay-by, or in a bay of its
    /// depot's deck, seen through its fins (the taxis behind a stand's bays with no depot to go
    /// to are not shown: drawn, they stood one on another in the lay-by).
    pub fn poses(&self) -> Vec<Pose> {
        use forge_geom::depot::{way_in, way_out};
        use forge_sim::taxi::DEPOT_DRIVE;
        let road = forge_procgen::city::ground::ROAD_LEVEL;
        let walk = forge_procgen::city::ground::WALK_LEVEL;
        (0..self.fleet.taxis.len())
            .map(|k| {
                let (p, dir) = self.fleet.pose(k);
                let right = DVec2::new(-dir.y, dir.x);
                let pose = |at: DVec2, heading: DVec2, level: f64, shown: bool| Pose {
                    at,
                    yaw: (-heading.y).atan2(heading.x) as f32,
                    level,
                    shown,
                };
                // Along a way through a depot's deck: in from the lane before its way in, out
                // to the lane before its way out.
                let on_way = |depot: u32, way: Vec<Vec3>, t: f64, backing: bool| {
                    let d = &self.depots[depot as usize];
                    let (q, heading) = along_way(&way, t, backing);
                    let (at, level) = deck_point(d, q);
                    pose(at, deck_direction(d, heading), level, true)
                };
                let in_deck = matches!(self.fleet.taxis[k].task, Task::Parked { .. })
                    || self.fleet.hold(k) > 0.0;
                if let Some((depot, b)) = self.bay_of[k].filter(|_| in_deck) {
                    let d = &self.depots[depot as usize];
                    let bay = &self.bays[b];
                    // The lane 1.5 m past the kerb, from the deck's front.
                    let lane = (d.front + 1.5) as f32;
                    let dip = (road - walk) as f32;
                    let hold = self.fleet.hold(k);
                    if hold > 0.0 {
                        let t = 1.0 - hold / DEPOT_DRIVE;
                        return on_way(depot, way_out(bay, lane, dip), t, true);
                    }
                    let since = self.fleet.time - self.fleet.since(k);
                    if since < DEPOT_DRIVE {
                        return on_way(depot, way_in(bay, lane, dip), since / DEPOT_DRIVE, false);
                    }
                    let (at, level) = deck_point(d, bay.at());
                    return pose(at, deck_direction(d, bay.facing()), level, true);
                }
                match self.fleet.taxis[k].task {
                    // In its stand's lay-by, bay after bay along it.
                    Task::Waiting { stand, bay } => {
                        let s = &self.stands[stand as usize];
                        let along = (f64::from(bay % 2) - 0.5) * 6.0;
                        let at = s.kerb + s.out * (taxi::DEPTH * 0.5) + s.along * along;
                        pose(at, s.along, road, bay < BAYS)
                    }
                    // Parked with every bay taken: out of sight.
                    Task::Parked { depot } => {
                        let d = &self.depots[depot as usize];
                        pose(d.entrance(), d.out, road, false)
                    }
                    Task::Stopped { .. } => pose(p + right * KERB, dir, road, true),
                    _ => pose(p + right * LANE, dir, road, true),
                }
            })
            .collect()
    }

    /// How many taxis are parked in the depots.
    pub fn parked(&self) -> usize {
        self.fleet
            .taxis
            .iter()
            .filter(|t| matches!(t.task, Task::Parked { .. }))
            .count()
    }

    /// Bookings waiting and the mean wait so far, seconds.
    pub fn service(&self) -> (usize, f64) {
        self.fleet.service()
    }
}

/// The share of the trips drawn between `demand`'s places by their weights that are long enough
/// to ride and served end to end by no tram line: those a taxi is useful for.
fn unserved_share(net: &Network, demand: &Demand) -> f64 {
    let total: f64 = demand.places.iter().map(|p| p.1).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let mut cumulative = Vec::with_capacity(demand.places.len());
    let mut sum = 0.0;
    for p in &demand.places {
        sum += p.1;
        cumulative.push(sum);
    }
    let mut rng = forge_core::SplitMix64::new(0x7A_C2);
    let mut draw = || {
        let x = rng.next_f64() * total;
        let i = cumulative
            .partition_point(|&c| c < x)
            .min(demand.places.len() - 1);
        net.at(demand.places[i].0).0
    };
    let unserved = (0..4000)
        .filter(|_| {
            let (a, b) = (draw(), draw());
            a.distance(b) >= forge_sim::taxi::SHORTEST_TRIP && !demand.by_tram(a, b)
        })
        .count();
    unserved as f64 / 4000.0
}

/// How far along `l` the point nearest `p` lies, metres.
fn nearest_on(l: &Link, p: DVec2) -> f64 {
    let (mut best, mut at, mut walked) = (f64::MAX, 0.0, 0.0);
    for w in l.points.windows(2) {
        let d = w[1] - w[0];
        let len = d.length();
        let t = if len > 0.0 {
            ((p - w[0]).dot(d) / (len * len)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let dist = (w[0] + d * t).distance(p);
        if dist < best {
            best = dist;
            at = walked + len * t;
        }
        walked += len;
    }
    at
}

/// The world position of a taxi drawn at `pose`.
pub fn world(size: f64, pose: &Pose) -> Vec3 {
    Vec3::new(
        (pose.at.x - size * 0.5) as f32,
        pose.level as f32,
        (pose.at.y - size * 0.5) as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_procgen::city::{example, plan};

    #[test]
    fn waiting_taxis_stand_in_their_lay_bys_and_moving_ones_on_the_roads() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = forge_procgen::city::mass::masses(
            &city,
            &forge_procgen::city::bible::StyleBible::default(),
            7,
        );
        let structures = forge_procgen::city::structure::structures(&city, &masses);
        let nets: Vec<f64> = structures.iter().map(|s| s.net).collect();
        let mut taxis = Taxis::new(&city, &masses, &nets, 4.0, 0.0).expect("a fleet");
        // Each where it should be: on a carriageway (its lane, a lay-by, the kerb), or on its
        // depot's deck; none drawn on another.
        let misplaced = |taxis: &Taxis| {
            let poses = taxis.poses();
            let mut off = 0;
            for p in &poses {
                // On a carriageway, or in or on the way through its depot's deck.
                let placed = city.ground.on_carriageway(p.at)
                    || city
                        .depots
                        .iter()
                        .any(|d| d.on_deck(p.at, 0.0) || d.on_driveway(p.at, 0.5));
                off += usize::from(p.shown && !placed);
            }
            let shown: Vec<&Pose> = poses.iter().filter(|p| p.shown).collect();
            let stacked = (0..shown.len())
                .flat_map(|i| (i + 1..shown.len()).map(move |j| (i, j)))
                .filter(|&(i, j)| {
                    shown[i].at.distance(shown[j].at) < 2.0
                        && (shown[i].level - shown[j].level).abs() < 1.0
                })
                .count();
            (off, stacked, taxis.parked())
        };
        let (off, stacked, parked) = misplaced(&taxis);
        println!(
            "{} taxis, {parked} parked in {} depots",
            taxis.len(),
            city.depots.len()
        );
        assert_eq!(off, 0, "{off} of {} taxis out of place", taxis.len());
        assert_eq!(stacked, 0, "{stacked} pairs of cabs one on another");
        assert!(parked > 0, "none in the depots");
        // An hour on: still in place, give or take one in twenty on a turn or a ramp.
        taxis.advance(600.0);
        for _ in 0..6 {
            let t = taxis.fleet.time + 590.0;
            taxis.fleet.advance_to(t, STEP);
            taxis.keep_bays();
        }
        let (off, stacked, parked) = misplaced(&taxis);
        let (waiting, wait) = taxis.service();
        println!(
            "after an hour: {off} out of place, {stacked} stacked, {parked} parked, {waiting} bookings waiting, mean wait {wait:.0} s"
        );
        assert!(
            off * 20 <= taxis.len(),
            "{off} of {} taxis out of place",
            taxis.len()
        );
        assert!(
            stacked * 50 <= taxis.len(),
            "{stacked} pairs one on another"
        );
    }

    #[test]
    fn at_the_morning_peak_the_taxis_keep_moving() {
        let size = 4000.0;
        let city = plan(&example::site(size, 10.0), &example::params(size));
        let masses = forge_procgen::city::mass::masses(
            &city,
            &forge_procgen::city::bible::StyleBible::default(),
            7,
        );
        let structures = forge_procgen::city::structure::structures(&city, &masses);
        let nets: Vec<f64> = structures.iter().map(|s| s.net).collect();
        let mut taxis = Taxis::new(&city, &masses, &nets, 8.0, 0.0).expect("a fleet");
        // An hour of the peak, then two minutes: a taxi on its way that has not moved in them
        // is stuck in a queue that does not clear.
        let t = taxis.fleet.time + 3600.0;
        taxis.fleet.advance_to(t, STEP);
        taxis.keep_bays();
        let before = taxis.poses();
        let t = taxis.fleet.time + 120.0;
        taxis.fleet.advance_to(t, STEP);
        taxis.keep_bays();
        let after = taxis.poses();
        let stuck = (0..taxis.len())
            .filter(|&k| {
                matches!(
                    taxis.fleet.taxis[k].task,
                    Task::Fetching { .. }
                        | Task::Carrying { .. }
                        | Task::Returning { .. }
                        | Task::Cruising
                        | Task::ToDepot { .. }
                ) && before[k].at.distance(after[k].at) < 0.5
            })
            .count();
        let (waiting, wait) = taxis.service();
        println!(
            "{} taxis at 9:02: {stuck} stuck on their way, {waiting} bookings waiting, mean wait {wait:.0} s",
            taxis.len()
        );
        // The trams take the trips one line serves; a share of the rest books a taxi.
        let f = &taxis.fleet;
        println!(
            "since 8:00: {} trips by tram, {} another way, {} by taxi",
            f.by_tram,
            f.otherwise,
            f.bookings.len()
        );
        assert!(
            f.by_tram > f.bookings.len() as u64,
            "the taxis took more trips than the trams"
        );
        assert!(
            stuck * 50 <= taxis.len(),
            "{stuck} of {} taxis stuck",
            taxis.len()
        );
        assert!(wait < 120.0, "a mean wait of {wait:.0} s");
        assert!(waiting * 20 <= taxis.len(), "{waiting} bookings waiting");
    }

    #[test]
    fn the_plans_depots_are_the_decks_drawn() {
        use forge_geom::depot;
        assert_eq!(taxi::DEPOT_WIDTH, f64::from(depot::WIDTH));
        assert_eq!(taxi::DEPOT_DEPTH, f64::from(depot::DEPTH));
        assert_eq!(taxi::DEPOT_CAPACITY, depot::bays().len());
        assert!((taxi::DEPOT_EXIT - f64::from(depot::EXIT_X)).abs() < 1e-4);
    }
}
