//! The population on the streets round the camera (D-059's step 6; `population-simulation.md`'s
//! T0: the drawn crowd is the real population). The plan's people (`forge_sim::population`) run
//! on the real clock here (see [`TIME_SCALE`]). Those within [`REACH`] of the camera are drawn as
//! placeholder figures (`forge_geom::figures`):
//! - on their way, along the road graph's shortest way (A*) from the street nearest where they set
//!   out to the street nearest where they go, at the pace their trip's time gives: walking on the
//!   pavement, or riding a bike in the carriageway's right lane, as the simulation's trip goes;
//!   by tram, waiting on the platform nearest their start (aboard, not drawn yet);
//! - in a park, standing about it;
//! - out at a café: seated at the terraces round it, in the share the people out there make of
//!   the seats.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use forge_core::Seed;
use forge_procgen::city::geom::{self, PointGrid};
use forge_procgen::city::mass::Mass;
use forge_procgen::city::{BlockUse, CityPlan, Profile};
use forge_sim::population::{self, Building, Mix, Mode, PlaceKind, Sim, TRAM_WAIT, Uses, World};
use forge_sim::transit::Timetable;
use glam::{DVec2, Vec2, Vec3};

/// The figures' palettes.
pub const GROUPS: usize = 8;
/// The meshes of each palette: left stride, standing, riding, seated, right stride.
pub const POSES: usize = 5;
/// Figures a mesh (a palette in one pose) shows at most.
pub const PER_MESH: usize = 60;
/// How far round the camera people are drawn, metres.
const REACH: f64 = 220.0;
/// Game seconds a real second for the drawn crowd: one. The game's clock runs four (an hour in
/// 15 real minutes), at which its walkers would cross the screen at 5 m/s; near the player
/// people walk at their true pace, and how the two clocks meet is D-059's open question.
const TIME_SCALE: f64 = 1.0;
/// A terrace seat's neighbourhood, metres: the cafés whose people may sit there.
const NEIGHBOURHOOD: f64 = 300.0;
/// Frames a scan of the whole population takes: who may be drawn round the camera, and who is
/// at each place (the cafés' people). Each frame scans its share, so no frame walks the million.
const SCAN_FRAMES: usize = 30;
/// How much further than [`REACH`] the scan looks, metres: the camera may move, and a route
/// strays from its straight line, before the next scan ends.
const SCAN_MARGIN: f64 = 300.0;
/// Routes found a frame at most.
const NEW_ROUTES: usize = 40;
/// The fastest a drawn trip goes along its street route, m/s: a route much longer than the
/// simulation's straight-line estimate is not drawn.
const RIDE_MAX: f64 = 6.5;

/// A way along the streets: its points and, per segment, its street's half width and its
/// pavement's level.
struct Route {
    points: Vec<DVec2>,
    along: Vec<f64>,
    half: Vec<f64>,
    level: Vec<f32>,
    /// Per point (as `half`), its street's raised pavement's width; 0 on a shared surface.
    pave: Vec<f64>,
}

impl Route {
    fn length(&self) -> f64 {
        self.along[self.along.len() - 1]
    }

    /// How far right of segment `i`'s centreline its walkers or riders go. On foot, 3 m from
    /// the building line: past the café terraces (2.4 m deep), short of the trees at the kerb
    /// or down a shared street's middle. Riding, in the bike strip a metre from the kerb, or
    /// on a shared surface 3.4 m in from the building line.
    fn side(&self, i: usize, riding: bool) -> f64 {
        let (half, pave) = (self.half[i], self.pave[i]);
        if !riding {
            (half - 3.0).max(1.0)
        } else if pave > 0.0 {
            (half - pave - 1.0).max(1.2)
        } else {
            (half - 3.4).max(1.2)
        }
    }

    /// Where the offset lines of segments `i - 1` and `i` meet at point `i` (the corner of the
    /// pavements or lanes), or the offset point itself at the route's ends.
    fn corner(&self, i: usize, riding: bool) -> DVec2 {
        let n = self.points.len();
        // In the plan's frame (y south) the right of the way is (-dy, dx).
        let right = |d: DVec2| DVec2::new(-d.y, d.x);
        let dir = |k: usize| (self.points[k + 1] - self.points[k]).normalize_or_zero();
        let v = self.points[i];
        if i == 0 {
            return v + right(dir(0)) * self.side(0, riding);
        }
        if i == n - 1 {
            return v + right(dir(n - 2)) * self.side(n - 2, riding);
        }
        let (d0, d1) = (dir(i - 1), dir(i));
        let (s0, s1) = (self.side(i - 1, riding), self.side(i, riding));
        let (p, q) = (v + right(d0) * s0, v + right(d1) * s1);
        let denom = geom::cross(d0, d1);
        let mean = (p + q) * 0.5;
        if denom.abs() < 1e-6 {
            return mean;
        }
        let mitre = p + d0 * (geom::cross(q - p, d1) / denom);
        // A sharp turn's mitre runs far out: the two offsets' mean instead.
        if mitre.distance(v) > 3.0 * s0.max(s1) {
            mean
        } else {
            mitre
        }
    }

    /// The point `s` metres along, on the right: the pavement on foot, the carriageway's lane
    /// riding; the way it faces and the ground's level there. Between corners it runs along
    /// the offset line, so at a junction a walker goes on to the corner of the pavements and
    /// crosses straight over, as at a crossing, rather than jumping from one side to the other.
    fn at(&self, s: f64, riding: bool) -> (DVec2, DVec2, f32) {
        let s = s.clamp(0.0, self.length());
        let i = self
            .along
            .partition_point(|&a| a <= s)
            .clamp(1, self.points.len() - 1);
        let seg = self.along[i] - self.along[i - 1];
        let t = if seg > 1e-9 {
            (s - self.along[i - 1]) / seg
        } else {
            0.0
        };
        let (a, b) = (self.corner(i - 1, riding), self.corner(i, riding));
        let dir = (b - a)
            .try_normalize()
            .unwrap_or_else(|| (self.points[i] - self.points[i - 1]).normalize_or_zero());
        let level = if riding { 0.16 } else { self.level[i - 1] };
        (a + (b - a) * t, dir, level)
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
    plan_edges: Vec<(Vec<DVec2>, f64, f64, f64)>,
    routes: HashMap<u32, Option<Route>>,
    /// The parks' outlines: their visitors stand inside them.
    parks: Vec<Vec<DVec2>>,
    size: f64,
    /// The terraces' seats (the world's place, the sitter's turn), and each seat's
    /// neighbourhood: the cafés within [`NEIGHBOURHOOD`] and how many seats.
    seats: Vec<(Vec3, f32)>,
    neighbourhoods: Vec<(Vec<u32>, u32)>,
    /// The tram stops' platforms, their middles on the plan found by a grid, the lines'
    /// timetables, and who has boarded from a platform (until their trip ends).
    platforms: Vec<Platform>,
    stop_grid: PointGrid,
    timetables: Vec<Timetable>,
    boarded: HashSet<u32>,
    /// People at each place, from the last whole scan.
    counts: Vec<u32>,
    /// The people who may be drawn round the camera, from the last whole scan.
    near: Vec<u32>,
    /// The scan under way: the next person, and what it has found so far.
    scan: (usize, Vec<u32>, Vec<u32>),
}

/// A tram stop's platform: where it stands (its corner, `x` along it, `z` away from its track),
/// its line, its stop on the line, and whether it serves the outbound trams.
#[derive(Clone, Copy, Debug)]
pub struct Platform {
    /// Its transform.
    pub at: glam::Mat4,
    /// Its line, an index into the timetables.
    pub line: usize,
    /// Its stop on the line.
    pub stop: usize,
    /// Whether the outbound trams stop at it.
    pub outbound: bool,
}

impl Crowd {
    /// The people of `plan`'s `masses`, their day run to `hour` (game hours from midnight).
    pub fn new(
        plan: &CityPlan,
        masses: &[Mass],
        hour: f64,
        seats: Vec<(Vec3, f32)>,
        platforms: Vec<Platform>,
        timetables: Vec<Timetable>,
    ) -> Self {
        let mut stop_grid = PointGrid::new(plan.size, 100.0);
        for (k, p) in platforms.iter().enumerate() {
            let (corner, along) = (p.at.w_axis.truncate(), p.at.x_axis.truncate());
            let mid = corner + along * 18.0;
            let half = plan.size * 0.5;
            stop_grid.insert(
                DVec2::new(f64::from(mid.x) + half, f64::from(mid.z) + half),
                k as u32,
            );
        }
        let buildings: Vec<Building> = masses
            .iter()
            .map(|m| {
                let c = geom::centroid(&m.footprint);
                Building {
                    pos: [c.x as f32, c.y as f32],
                    floor_area: (geom::signed_area(&m.footprint) * f64::from(m.floors)) as f32,
                    active_ground: m.street_width >= 18.0 || m.street_width <= 8.0,
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
            plan_edges.push((e.points.clone(), side, level, e.profile.pavement()));
        }
        // Each terrace seat's neighbourhood: the cafés and the seats within 300 m of it. A seat
        // is taken in the share the people out at those cafés make of those seats.
        let half = plan.size * 0.5;
        let mut cafe_grid = PointGrid::new(plan.size, 100.0);
        for (i, q) in world.places.iter().enumerate() {
            if q.kind == PlaceKind::Food {
                cafe_grid.insert(
                    DVec2::new(f64::from(q.pos[0]), f64::from(q.pos[1])),
                    i as u32,
                );
            }
        }
        let mut seat_grid = PointGrid::new(plan.size, 100.0);
        let plan_at = |p: Vec3| DVec2::new(f64::from(p.x) + half, f64::from(p.z) + half);
        for (k, (p, _)) in seats.iter().enumerate() {
            seat_grid.insert(plan_at(*p), k as u32);
        }
        let neighbourhoods: Vec<(Vec<u32>, u32)> = seats
            .iter()
            .map(|(p, _)| {
                let at = plan_at(*p);
                let mut cafes = Vec::new();
                cafe_grid.for_each_within(at, NEIGHBOURHOOD, |_, cafe| cafes.push(cafe));
                let mut around = 0;
                seat_grid.for_each_within(at, NEIGHBOURHOOD, |_, _| around += 1);
                (cafes, around)
            })
            .collect();
        tracing::info!(
            people = sim.pop.len(),
            places = world.places.len(),
            seats = seats.len(),
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
            seats,
            neighbourhoods,
            stop_grid,
            timetables,
            boarded: HashSet::new(),
            platforms,
            counts: Vec::new(),
            near: Vec::new(),
            scan: (0, Vec::new(), Vec::new()),
        }
    }

    /// Scans `n` more people for who may be drawn round `c` (the plan's frame) and who is at
    /// each place; at the population's end the scan's findings replace the last ones.
    fn scan(&mut self, c: DVec2, n: usize) {
        let places = &self.sim.world.places;
        let total = self.sim.pop.len();
        let (next, found, counts) = &mut self.scan;
        if counts.len() != places.len() {
            *counts = vec![0; places.len()];
        }
        let end = (*next + n).min(total);
        let now = self.clock;
        let pos = |i: u32| {
            let q = places[i as usize].pos;
            DVec2::new(f64::from(q[0]), f64::from(q[1]))
        };
        for p in *next..end {
            match self.sim.trip(p) {
                None => {
                    let at = self.sim.pop.at[p];
                    if let Some(k) = counts.get_mut(at as usize) {
                        *k += 1;
                    }
                    // In a park: standing up to 70 m from its place.
                    if places
                        .get(at as usize)
                        .is_some_and(|q| q.kind == PlaceKind::Park)
                        && pos(at).distance(c) <= REACH + 100.0 + SCAN_MARGIN
                    {
                        found.push(p as u32);
                    }
                }
                Some(trip) => {
                    let (a, b) = (pos(trip.from), pos(trip.to));
                    let near = if trip.mode == Mode::Tram {
                        // Waiting at the platform nearest where they set out.
                        a.distance(c) <= REACH + 500.0 + SCAN_MARGIN
                    } else {
                        let gone = ((now - f64::from(trip.since)) / f64::from(trip.time.max(1)))
                            .clamp(0.0, 1.0);
                        (a + (b - a) * gone).distance(c) <= REACH * 1.5 + SCAN_MARGIN
                    };
                    if near {
                        found.push(p as u32);
                    }
                }
            }
        }
        *next = end;
        if end == total {
            self.near = std::mem::take(found);
            self.counts = std::mem::replace(counts, vec![0; places.len()]);
            *next = 0;
        }
    }

    /// The people out at cafés, seated at the terraces' seats within [`REACH`] of `c` (the plan's
    /// frame): each seat taken in the share the people out round it make of the seats round it,
    /// a palette's seated mesh each.
    fn seat_people(&mut self, c: DVec2, out: &mut [Vec<(Vec3, f32)>]) {
        let half = self.size * 0.5;
        for (k, &(p, turn)) in self.seats.iter().enumerate() {
            let at = DVec2::new(f64::from(p.x) + half, f64::from(p.z) + half);
            if at.distance(c) > REACH {
                continue;
            }
            let (cafes, around) = &self.neighbourhoods[k];
            let people: u32 = cafes
                .iter()
                .filter_map(|&q| self.counts.get(q as usize))
                .sum();
            let share = (f64::from(people) / f64::from((*around).max(1))).min(0.9);
            // The same seats stay taken while the share holds (a hash of the seat, not a draw).
            let pick = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 54;
            if (pick as f64) / 1024.0 >= share {
                continue;
            }
            let mesh = (k * 7 + 3) % GROUPS * POSES + 3;
            if out[mesh].len() < PER_MESH {
                out[mesh].push((p, turn));
            }
        }
    }

    /// Advances the people's day by `real` seconds.
    pub fn advance(&mut self, real: f64) {
        self.clock += real * TIME_SCALE;
        self.sim.run_until(self.clock as u32);
    }

    /// The figures to draw round `camera` (the world's x and z): for each of the
    /// `POSES × GROUPS` meshes in order (each palette's left stride, standing, riding, seated,
    /// right stride), up to [`PER_MESH`] places and turns; `tram_clock` is the trams' service
    /// clock, whose standing trams the waiting people board.
    pub fn figures(&mut self, camera: Vec2, tram_clock: f64) -> Vec<Vec<(Vec3, f32)>> {
        let half = self.size * 0.5;
        let c = DVec2::new(f64::from(camera.x) + half, f64::from(camera.y) + half);
        let mut out = vec![Vec::new(); POSES * GROUPS];
        let mut budget = NEW_ROUTES;
        // The trams standing at their stops now: (line, stop, outbound).
        let mut standing = HashSet::new();
        for (line, t) in self.timetables.iter().enumerate() {
            for k in 0..t.vehicles() {
                let run = t.at(k, tram_clock);
                if let Some(stop) = run.at_stop {
                    standing.insert((line, stop, run.outbound));
                }
            }
        }
        // This frame's share of the scan; the whole of it before the first frame.
        let total = self.sim.pop.len();
        let share = if self.counts.is_empty() {
            total
        } else {
            total.div_ceil(SCAN_FRAMES)
        };
        self.scan(c, share);
        let near = std::mem::take(&mut self.near);
        let places = &self.sim.world.places;
        let now = self.clock;
        for &p in &near {
            let p = p as usize;
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
                    let mesh = (s as usize % GROUPS) * POSES + 1;
                    if q.distance(c) <= REACH
                        && out[mesh].len() < PER_MESH
                        && self.parks.iter().any(|p| geom::contains(p, q))
                    {
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
            // By tram: waiting at the platform nearest where they set out until a tram of its
            // line stands there going its way, then aboard (not drawn yet).
            if trip.mode == Mode::Tram {
                if now - f64::from(trip.since) < f64::from(TRAM_WAIT)
                    && !self.boarded.contains(&(p as u32))
                    && let Some((_, k)) = self.stop_grid.nearest(a, 500.0)
                {
                    let platform = self.platforms[k as usize];
                    if standing.contains(&(platform.line, platform.stop, platform.outbound)) {
                        self.boarded.insert(p as u32);
                        continue;
                    }
                    let corner = platform.at.w_axis.truncate();
                    let (along, away) =
                        (platform.at.x_axis.truncate(), platform.at.z_axis.truncate());
                    let s = self.sim.pop.seed[p];
                    let spot = corner
                        + along * (4.0 + (s % 28) as f32)
                        + away * (0.8 + ((s >> 8) % 14) as f32 / 10.0)
                        + Vec3::Y * 0.28;
                    let at = DVec2::new(f64::from(spot.x) + half, f64::from(spot.z) + half);
                    let mesh = (s as usize % GROUPS) * POSES + 1;
                    if at.distance(c) <= REACH && out[mesh].len() < PER_MESH {
                        out[mesh].push((spot, away.z.atan2(-away.x)));
                    }
                }
                continue;
            }
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
            // On foot, or riding a bike.
            let riding = trip.mode == Mode::Bike;
            let (q, dir, level) = route.at(gone * route.length(), riding);
            if q.distance(c) > REACH {
                continue;
            }
            let seed = self.sim.pop.seed[p];
            // Riding, or walking: the left stride, passing, the right stride, passing, a pose
            // every 0.7 m along the way.
            let pose = if riding {
                2
            } else {
                [0, 1, 4, 1][((gone * route.length() / 0.7) as usize) % 4]
            };
            let mesh = (seed as usize % GROUPS) * POSES + pose;
            if out[mesh].len() >= PER_MESH {
                continue;
            }
            let at = Vec3::new((q.x - half) as f32, level, (q.y - half) as f32);
            out[mesh].push((at, (-dir.y).atan2(dir.x) as f32));
        }
        self.near = near;
        self.seat_people(c, &mut out);
        // Forget who boarded once their trip has ended, and the routes of trips that have ended.
        if self.boarded.len() > 5000 {
            let sim = &self.sim;
            self.boarded.retain(|&p| sim.trip(p as usize).is_some());
        }
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
        let mut pave = Vec::new();
        for (e, fwd) in legs {
            let (pts, s, l, pv) = &self.plan_edges[e as usize];
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
                pave.push(*pv);
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
            pave,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walkers_turn_corners_without_jumping() {
        // East 100 m, then a turn either way onto a narrower street, then on.
        for turn in [1.0, -1.0] {
            let points = vec![
                DVec2::new(0.0, 0.0),
                DVec2::new(100.0, 0.0),
                DVec2::new(100.0, 100.0 * turn),
                DVec2::new(160.0, 100.0 * turn),
            ];
            let mut along = vec![0.0];
            for w in points.windows(2) {
                along.push(along[along.len() - 1] + w[0].distance(w[1]));
            }
            let route = Route {
                points,
                along,
                half: vec![14.0, 8.0, 8.0],
                level: vec![0.3, 0.16, 0.16],
                pave: vec![6.0, 0.0, 0.0],
            };
            for riding in [false, true] {
                let mut last = route.at(0.0, riding).0;
                let mut s = 0.5;
                while s <= route.length() {
                    let p = route.at(s, riding).0;
                    assert!(
                        p.distance(last) < 1.5,
                        "a jump of {} m at {s}",
                        p.distance(last)
                    );
                    last = p;
                    s += 0.5;
                }
            }
        }
    }
}
