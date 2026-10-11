//! The population on the streets round the camera (D-059's step 6; `population-simulation.md`'s
//! T0: the drawn crowd is the real population). The plan's people (`forge_sim::population`) run
//! on the real clock here (see [`TIME_SCALE`]). Those within [`REACH`] of the camera are drawn as
//! placeholder figures (`forge_geom::figures`):
//! - on their way, along the road graph's shortest way (A*) from the street nearest where they set
//!   out to the street nearest where they go, at the pace their trip's time gives: walking on the
//!   pavement, or riding a bike in the carriageway's right lane when quicker than a walk (trips
//!   quicker than a bike ride the trams, and are not drawn yet);
//! - in a park, standing about it.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use forge_core::Seed;
use forge_procgen::city::geom::{self, PointGrid};
use forge_procgen::city::mass::Mass;
use forge_procgen::city::{BlockUse, CityPlan, Profile};
use forge_sim::population::{self, Building, Mix, PlaceKind, Sim, Uses, World};
use glam::{DVec2, Vec2, Vec3};

/// The figures' palettes.
pub const GROUPS: usize = 8;
/// Figures a mesh (a palette walking, standing or riding) shows at most.
pub const PER_MESH: usize = 60;
/// How far round the camera people are drawn, metres.
const REACH: f64 = 220.0;
/// Game seconds a real second for the drawn crowd: one. The game's clock runs four (an hour in
/// 15 real minutes), at which its walkers would cross the screen at 5 m/s; near the player
/// people walk at their true pace, and how the two clocks meet is D-059's open question.
const TIME_SCALE: f64 = 1.0;
/// Routes found a frame at most.
const NEW_ROUTES: usize = 40;
/// The fastest a drawn walker goes, m/s.
const WALK_MAX: f64 = 2.2;
/// The fastest a drawn cyclist goes, m/s; faster trips ride the trams.
const RIDE_MAX: f64 = 6.5;

/// A way along the streets: its points and, per segment, its street's half width and its
/// pavement's level.
struct Route {
    points: Vec<DVec2>,
    along: Vec<f64>,
    half: Vec<f64>,
    level: Vec<f32>,
}

impl Route {
    fn length(&self) -> f64 {
        self.along[self.along.len() - 1]
    }

    /// The point `s` metres along, on the right: the pavement on foot, the carriageway's lane
    /// riding; the way it faces and the ground's level there.
    fn at(&self, s: f64, riding: bool) -> (DVec2, DVec2, f32) {
        let s = s.clamp(0.0, self.length());
        let i = self
            .along
            .partition_point(|&a| a <= s)
            .clamp(1, self.points.len() - 1);
        let (a, b) = (self.points[i - 1], self.points[i]);
        let seg = self.along[i] - self.along[i - 1];
        let t = if seg > 1e-9 {
            (s - self.along[i - 1]) / seg
        } else {
            0.0
        };
        let dir = (b - a).normalize_or_zero();
        // In the plan's frame (y south) the right of the way is (-dy, dx).
        let right = DVec2::new(-dir.y, dir.x);
        let half = self.half[i - 1];
        let (side, level) = if riding {
            ((half - 4.5).max(1.2), 0.16)
        } else {
            ((half - 2.0).max(1.0), self.level[i - 1])
        };
        (a + (b - a) * t + right * side, dir, level)
    }
}

/// The streets as a graph to route on.
struct Graph {
    nodes: Vec<DVec2>,
    /// Per node: (the next node, the edge, forwards along its points).
    adj: Vec<Vec<(u32, u32, bool)>>,
    near: PointGrid,
}

/// The crowd: the population on its clock, and the walkers' routes.
pub struct Crowd {
    sim: Sim<'static>,
    /// Game seconds.
    clock: f64,
    graph: Graph,
    plan_edges: Vec<(Vec<DVec2>, f64, f64)>,
    routes: HashMap<u32, Option<Route>>,
    /// The parks' outlines: their visitors stand inside them.
    parks: Vec<Vec<DVec2>>,
    size: f64,
}

impl Crowd {
    /// The people of `plan`'s `masses`, their day run to `hour` (game hours from midnight).
    pub fn new(plan: &CityPlan, masses: &[Mass], hour: f64) -> Self {
        let buildings: Vec<Building> = masses
            .iter()
            .map(|m| {
                let c = geom::centroid(&m.footprint);
                Building {
                    pos: [c.x as f32, c.y as f32],
                    floor_area: (geom::signed_area(&m.footprint) * f64::from(m.floors)) as f32,
                    active_ground: m.street_width >= 20.0 || m.street_width <= 8.0,
                }
            })
            .collect();
        let parks: Vec<[f32; 2]> = plan
            .blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Park)
            .map(|b| [b.centre.x as f32, b.centre.y as f32])
            .collect();
        let places = population::places_from_buildings(&buildings, &parks, &Uses::default());
        // The demo's lifetime: the places live as long as the app.
        let world: &'static World =
            Box::leak(Box::new(World::new(places, plan.size as f32, 250.0)));
        let pop = population::generate(world, &Mix::default(), Seed::new(2026));
        let mut sim = Sim::new(world, pop);
        let clock = hour * 3600.0;
        sim.run_until(clock as u32);
        // The graph: nodes, and each edge both ways.
        let nodes: Vec<DVec2> = plan.roads.nodes.iter().map(|n| n.pos).collect();
        let mut adj = vec![Vec::new(); nodes.len()];
        let mut near = PointGrid::new(plan.size, 50.0);
        for (i, n) in nodes.iter().enumerate() {
            near.insert(*n, i as u32);
        }
        let mut plan_edges = Vec::new();
        for (i, e) in plan.roads.edges.iter().enumerate() {
            adj[e.a as usize].push((e.b, i as u32, true));
            adj[e.b as usize].push((e.a, i as u32, false));
            // The street's half width, and its pavement's level: the avenues' raised.
            let side = e.profile.width() * 0.5;
            let level = match e.profile {
                Profile::TramAvenue | Profile::Avenue => 0.30,
                _ => 0.16,
            };
            plan_edges.push((e.points.clone(), side, level));
        }
        tracing::info!(
            people = sim.pop.len(),
            places = world.places.len(),
            hour,
            "the crowd: the plan's people on their clock"
        );
        Self {
            sim,
            clock,
            graph: Graph { nodes, adj, near },
            plan_edges,
            routes: HashMap::new(),
            parks: plan
                .blocks
                .iter()
                .filter(|b| b.use_ == BlockUse::Park)
                // Inside the streets round them: a park's outline runs along their centrelines.
                .map(|b| geom::inset(&b.polygon, 9.0).unwrap_or_else(|| b.polygon.clone()))
                .collect(),
            size: plan.size,
        }
    }

    /// Advances the people's day by `real` seconds.
    pub fn advance(&mut self, real: f64) {
        self.clock += real * TIME_SCALE;
        self.sim.run_until(self.clock as u32);
    }

    /// The figures to draw round `camera` (the world's x and z): for each of the
    /// `3 × GROUPS` meshes in order (each palette walking, standing, riding), up to [`PER_MESH`]
    /// places and turns.
    pub fn figures(&mut self, camera: Vec2) -> Vec<Vec<(Vec3, f32)>> {
        let half = self.size * 0.5;
        let c = DVec2::new(f64::from(camera.x) + half, f64::from(camera.y) + half);
        let mut out = vec![Vec::new(); 3 * GROUPS];
        let mut budget = NEW_ROUTES;
        let places = &self.sim.world.places;
        let now = self.clock;
        for p in 0..self.sim.pop.len() {
            let Some(trip) = self.sim.trip(p) else {
                // In a park: standing about it, scattered by their seed as the maps show them.
                let at = self.sim.pop.at[p] as usize;
                if places.get(at).is_some_and(|q| q.kind == PlaceKind::Park) {
                    let s = self.sim.pop.seed[p];
                    // Over a park's extent, about: the place stands at its centre.
                    let jitter = |bits: u32| (f64::from(bits & 0xFF) / 255.0 - 0.5) * 140.0;
                    let q = places[at].pos;
                    let q = DVec2::new(
                        f64::from(q[0]) + jitter(s),
                        f64::from(q[1]) + jitter(s >> 8),
                    );
                    let mesh = (s as usize % GROUPS) * 3 + 1;
                    let inside = self.parks.iter().any(|p| geom::contains(p, q));
                    if inside && q.distance(c) <= REACH && out[mesh].len() < PER_MESH {
                        let at = Vec3::new((q.x - half) as f32, 0.08, (q.y - half) as f32);
                        out[mesh].push((at, ((s >> 20) as f32 / 4096.0) * std::f32::consts::TAU));
                    }
                }
                continue;
            };
            let (a, b) = (places[trip.from as usize].pos, places[trip.to as usize].pos);
            let (a, b) = (
                DVec2::new(f64::from(a[0]), f64::from(a[1])),
                DVec2::new(f64::from(b[0]), f64::from(b[1])),
            );
            let gone =
                ((now - f64::from(trip.since)) / f64::from(trip.time.max(1))).clamp(0.0, 1.0);
            // Near the camera by the straight line first, which the route stays close to.
            if (a + (b - a) * gone).distance(c) > REACH * 1.5 {
                continue;
            }
            let key = p as u32;
            if !self.routes.contains_key(&key) {
                if budget == 0 {
                    continue;
                }
                budget -= 1;
                let route = self
                    .route(a, b)
                    .filter(|r| r.length() / f64::from(trip.time.max(1)) <= RIDE_MAX);
                self.routes.insert(key, route);
            }
            let Some(Some(route)) = self.routes.get(&key) else {
                continue;
            };
            // On foot, or riding a bike: by the pace the trip's time gives.
            let riding = route.length() / f64::from(trip.time.max(1)) > WALK_MAX;
            let (q, dir, level) = route.at(gone * route.length(), riding);
            if q.distance(c) > REACH {
                continue;
            }
            let seed = self.sim.pop.seed[p];
            let mesh = (seed as usize % GROUPS) * 3 + if riding { 2 } else { 0 };
            if out[mesh].len() >= PER_MESH {
                continue;
            }
            let at = Vec3::new((q.x - half) as f32, level, (q.y - half) as f32);
            out[mesh].push((at, (-dir.y).atan2(dir.x) as f32));
        }
        // Forget the routes of trips that have ended.
        if self.routes.len() > 20_000 {
            let sim = &self.sim;
            self.routes.retain(|&p, _| sim.trip(p as usize).is_some());
        }
        out
    }

    /// The shortest way along the streets from the node nearest `from` to the node nearest
    /// `to` (A*, the straight distance as the estimate), as a route on the pavements.
    fn route(&self, from: DVec2, to: DVec2) -> Option<Route> {
        let (_, start) = self.graph.near.nearest(from, 400.0)?;
        let (_, goal) = self.graph.near.nearest(to, 400.0)?;
        if start == goal {
            return None;
        }
        let nodes = &self.graph.nodes;
        let n = nodes.len();
        let mut best = vec![f64::MAX; n];
        let mut came: Vec<Option<(u32, u32, bool)>> = vec![None; n];
        let mut open = BinaryHeap::new();
        let key = |f: f64| (f * 100.0) as u64;
        best[start as usize] = 0.0;
        open.push(Reverse((
            key(nodes[start as usize].distance(nodes[goal as usize])),
            start,
        )));
        let mut steps = 0;
        while let Some(Reverse((_, u))) = open.pop() {
            if u == goal {
                break;
            }
            steps += 1;
            if steps > 20_000 {
                return None;
            }
            for &(v, e, fwd) in &self.graph.adj[u as usize] {
                let length = geom::polyline_length(&self.plan_edges[e as usize].0);
                let g = best[u as usize] + length;
                if g < best[v as usize] {
                    best[v as usize] = g;
                    came[v as usize] = Some((u, e, fwd));
                    let h = nodes[v as usize].distance(nodes[goal as usize]);
                    open.push(Reverse((key(g + h), v)));
                }
            }
        }
        came[goal as usize]?;
        // Walk back to the start, then lay the edges' points out in order.
        let mut legs = Vec::new();
        let mut v = goal;
        while v != start {
            let (u, e, fwd) = came[v as usize]?;
            legs.push((e, fwd));
            v = u;
        }
        legs.reverse();
        let mut points = Vec::new();
        let mut half = Vec::new();
        let mut level = Vec::new();
        for (e, fwd) in legs {
            let (pts, s, l) = &self.plan_edges[e as usize];
            let mut pts = pts.clone();
            if !fwd {
                pts.reverse();
            }
            for (k, p) in pts.into_iter().enumerate() {
                if k == 0 && !points.is_empty() {
                    continue;
                }
                points.push(p);
                half.push(*s);
                level.push(*l as f32);
            }
        }
        if points.len() < 2 {
            return None;
        }
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along[along.len() - 1] + w[0].distance(w[1]));
        }
        Some(Route {
            points,
            along,
            half,
            level,
        })
    }
}
