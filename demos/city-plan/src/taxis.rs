//! The city's automated taxis (`forge_sim::taxi`, D-061) in the scene: the fleet built from the
//! plan's taxi roads, stands and blocks, stepped with the scene's clock, and each cab placed on
//! its lane, at a stand's lay-by, or at the kerb picking someone up.

use forge_procgen::city::blocks::BlockUse;
use forge_procgen::city::mass::Mass;
use forge_procgen::city::{CityPlan, taxi};
use forge_sim::taxi::{Demand, Fleet, Link, Network, Stand, Task};
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
/// How far a moving taxi keeps right of its road's centreline, metres: in its lane.
const LANE: f64 = 1.5;
/// How far right of the centreline a taxi stops to pick up, metres: at the kerb.
const KERB: f64 = 2.3;
/// The fleet's step, seconds.
const STEP: f64 = 0.25;
/// The cabs a stand's lay-by holds, one each end.
const BAYS: u32 = 2;

/// The scene's taxis.
pub struct Taxis {
    fleet: Fleet,
    stands: Vec<taxi::Stand>,
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
                        at: forge_sim::taxi::Spot { link, s: spot },
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
        let fleet = Fleet::new(net, stands, taxis, demand, 0x7A_C1, start);
        Some(Self {
            fleet,
            stands: plan.taxi_stands.clone(),
            offset: start - clock,
            unserved,
            taxi_share,
        })
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
    }

    /// Each taxi's position on the plan, its yaw, and the level it stands on.
    pub fn poses(&self) -> Vec<(DVec2, f32)> {
        (0..self.fleet.taxis.len())
            .map(|k| {
                let (p, dir) = self.fleet.pose(k);
                let right = DVec2::new(-dir.y, dir.x);
                let (at, heading) = match self.fleet.taxis[k].task {
                    // In its stand's lay-by, bay after bay along it.
                    Task::Waiting { stand, bay } => {
                        let s = &self.stands[stand as usize];
                        let along = (f64::from(bay % 2) - 0.5) * 6.0;
                        (
                            s.kerb + s.out * (taxi::DEPTH * 0.5) + s.along * along,
                            s.along,
                        )
                    }
                    Task::Stopped { .. } => (p + right * KERB, dir),
                    _ => (p + right * LANE, dir),
                };
                (at, (-heading.y).atan2(heading.x) as f32)
            })
            .collect()
    }

    /// Whether taxi `k` waits behind its stand's bays: in a depot, not drawn (the depots, where
    /// the cabs would charge, are not built yet; drawn, they stood one on another in the lay-by).
    pub fn in_depot(&self, k: usize) -> bool {
        matches!(self.fleet.taxis[k].task, Task::Waiting { bay, .. } if bay >= BAYS)
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

/// The world position of a taxi standing at `p` on the plan, on the carriageway's level.
pub fn world(size: f64, p: DVec2) -> Vec3 {
    Vec3::new(
        (p.x - size * 0.5) as f32,
        forge_procgen::city::ground::ROAD_LEVEL as f32,
        (p.y - size * 0.5) as f32,
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
        let poses = taxis.poses();
        println!(
            "{} taxis; first {:?}",
            poses.len(),
            &poses[..3.min(poses.len())]
        );
        let off = poses
            .iter()
            .filter(|(p, _)| !city.ground.on_carriageway(*p))
            .count();
        assert_eq!(
            off,
            0,
            "{off} of {} waiting taxis off the carriageway",
            poses.len()
        );
        // None drawn one on another: the cabs behind a stand's bays wait in a depot.
        let drawn: Vec<DVec2> = (0..poses.len())
            .filter(|&k| !taxis.in_depot(k))
            .map(|k| poses[k].0)
            .collect();
        let stacked = (0..drawn.len())
            .flat_map(|i| (i + 1..drawn.len()).map(move |j| (i, j)))
            .filter(|&(i, j)| drawn[i].distance(drawn[j]) < 3.0)
            .count();
        assert_eq!(stacked, 0, "{stacked} pairs of cabs one on another");
        // An hour on: every taxi still on a carriageway (its lane, a lay-by, the kerb).
        taxis.advance(600.0);
        for _ in 0..6 {
            let t = taxis.fleet.time + 590.0;
            taxis.fleet.advance_to(t, STEP);
        }
        let off = taxis
            .poses()
            .iter()
            .filter(|(p, _)| !city.ground.on_carriageway(*p))
            .count();
        let (waiting, wait) = taxis.service();
        println!(
            "after an hour: {off} off the carriageway, {waiting} bookings waiting, mean wait {wait:.0} s"
        );
        assert!(
            off * 20 <= taxis.len(),
            "{off} of {} taxis off the carriageway",
            taxis.len()
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
        let before = taxis.poses();
        let t = taxis.fleet.time + 120.0;
        taxis.fleet.advance_to(t, STEP);
        let after = taxis.poses();
        let stuck = (0..taxis.len())
            .filter(|&k| {
                matches!(
                    taxis.fleet.taxis[k].task,
                    Task::Fetching { .. }
                        | Task::Carrying { .. }
                        | Task::Returning { .. }
                        | Task::Cruising
                ) && before[k].0.distance(after[k].0) < 0.5
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
}
