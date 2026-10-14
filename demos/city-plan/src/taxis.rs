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
/// Taxi trips a person makes a day: a city with trams, few taxis of its own.
const TRIPS_PER_PERSON: f64 = 0.08;
/// Trips one taxi serves a day: about one an eight minutes over its busy hours.
const TRIPS_PER_TAXI: f64 = 90.0;
/// How far a moving taxi keeps right of its road's centreline, metres: in its lane.
const LANE: f64 = 1.5;
/// How far right of the centreline a taxi stops to pick up, metres: at the kerb.
const KERB: f64 = 2.3;
/// The fleet's step, seconds.
const STEP: f64 = 0.25;

/// The scene's taxis.
pub struct Taxis {
    fleet: Fleet,
    stands: Vec<taxi::Stand>,
    /// The fleet's clock less the scene's.
    offset: f64,
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
                        bays: 2,
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
        let daily = people * TRIPS_PER_PERSON;
        let total: f64 = HOURLY.iter().sum();
        let per_hour = HOURLY.map(|h| daily * h / total);
        let mean = daily / 24.0;
        let demand = Demand {
            places,
            per_hour,
            busy_over: mean * 1.3,
        };
        let taxis = ((daily / TRIPS_PER_TAXI).round() as usize).clamp(10, 4000);
        let start = hour * 3600.0;
        let fleet = Fleet::new(net, stands, taxis, demand, 0x7A_C1, start);
        Some(Self {
            fleet,
            stands: plan.taxi_stands.clone(),
            offset: start - clock,
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

    /// Bookings waiting and the mean wait so far, seconds.
    pub fn service(&self) -> (usize, f64) {
        self.fleet.service()
    }
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
}
