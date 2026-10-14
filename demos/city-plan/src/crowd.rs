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
use forge_procgen::city::{BlockUse, CityPlan};
use forge_sim::population::{self, Building, Mix, Mode, PlaceKind, Sim, TRAM_WAIT, Uses, World};
use forge_sim::transit::Timetable;
use glam::{DVec2, Vec2, Vec3};

/// The figures' palettes.
pub const GROUPS: usize = 8;
/// The meshes of each palette: left stride, standing, riding, seated, right stride, and the half
/// strides left and right between them.
pub const POSES: usize = 7;
/// Figures a mesh (a palette in one pose) shows at most.
pub const PER_MESH: usize = 60;
/// How far round the camera people are drawn, metres.
const REACH: f64 = 220.0;
/// Game seconds a real second for the drawn crowd: one. The game's clock runs three (an hour in
/// 20 real minutes, `population::GAME_SPEED`), at which its walkers would cross the screen at
/// 4 m/s; near the player people walk at their true pace (the owner: the pace looks right).
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
/// A terrace seat's figure is told apart from the people by this plus the seat's number.
const SEATED: u32 = 1 << 31;
/// A chess game's watcher, told apart by this plus eight a game and their number.
const WATCHING: u32 = 3 << 30;
/// An empty slot.
const EMPTY: u32 = u32::MAX;

/// A park people stand about in.
struct Park {
    /// Where they stand: inside the streets round it.
    inside: Vec<DVec2>,
    /// How far they spread from its middle, metres.
    spread: f64,
}

/// A figure in a mesh's slot.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Figure {
    /// Where it stands, in the scene.
    pub at: Vec3,
    /// Its turn about +y, radians.
    pub turn: f32,
    /// Whether the slot showed the same person last frame: its motion is then theirs. A slot
    /// that changed hands has no motion of its own this frame.
    pub kept: bool,
}

/// `found`'s figures (who, where, turn; at most [`PER_MESH`]) in a mesh's slots: who `slots`
/// showed last frame keeps their slot, the others take the free ones in turn. `slots` then
/// holds who each slot shows now.
fn keep_slots(slots: &mut [u32], found: &[(u32, Vec3, f32)]) -> Vec<Option<Figure>> {
    let was: HashMap<u32, usize> = slots
        .iter()
        .enumerate()
        .filter(|&(_, &who)| who != EMPTY)
        .map(|(k, &who)| (who, k))
        .collect();
    let mut shown = vec![None; slots.len()];
    let mut now = vec![EMPTY; slots.len()];
    let mut newcomers = Vec::new();
    for &(who, at, turn) in found {
        match was.get(&who) {
            Some(&k) => {
                shown[k] = Some(Figure {
                    at,
                    turn,
                    kept: true,
                });
                now[k] = who;
            }
            None => newcomers.push((who, at, turn)),
        }
    }
    let mut free = (0..slots.len())
        .filter(|&k| now[k] == EMPTY)
        .collect::<Vec<_>>()
        .into_iter();
    for (who, at, turn) in newcomers {
        let Some(k) = free.next() else { break };
        shown[k] = Some(Figure {
            at,
            turn,
            kept: false,
        });
        now[k] = who;
    }
    slots.copy_from_slice(&now);
    shown
}

/// A zebra crossing at a junction, as a walker takes it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Zebra {
    /// The way its street leaves the junction, unit.
    arm: DVec2,
    /// How far from the junction's node it lies, metres.
    from: f64,
    /// Its ends on the pavements either side, past the kerbs.
    ends: [DVec2; 2],
    /// Where it meets each kerb, on the same sides as `ends`.
    kerbs: [DVec2; 2],
    /// Its level between the kerbs: the walking level on a raised table, else the road's.
    level: f32,
}

/// A way along the streets: its points and, per segment, its street's half width and its
/// pavement's level; and the walkers' way along it, over the zebras.
struct Route {
    points: Vec<DVec2>,
    along: Vec<f64>,
    half: Vec<f64>,
    level: Vec<f32>,
    /// Per point (as `half`), its street's raised pavement's width; 0 on a shared surface.
    pave: Vec<f64>,
    /// On foot: the pavements' corners, and round the junctions where the way crosses a street
    /// with a zebra, over it (they crossed in the junction's middle, the taxis' way).
    walk: Vec<(DVec2, f32)>,
    /// How far along `walk` each of its points lies.
    walk_along: Vec<f64>,
}

impl Route {
    /// The route along `points` (its segments' `half` widths, `level`s and `pave`ments), its
    /// junctions `junctions` (a point's index and its node) taking the `zebras` there.
    fn new(
        points: Vec<DVec2>,
        half: Vec<f64>,
        level: Vec<f32>,
        pave: Vec<f64>,
        junctions: &[(usize, u32)],
        zebras: &HashMap<u32, Vec<Zebra>>,
    ) -> Self {
        let mut along = vec![0.0];
        for w in points.windows(2) {
            along.push(along[along.len() - 1] + w[0].distance(w[1]));
        }
        let mut route = Self {
            points,
            along,
            half,
            level,
            pave,
            walk: Vec::new(),
            walk_along: Vec::new(),
        };
        route.walk = route.footway(junctions, zebras);
        let mut walked = vec![0.0];
        for w in route.walk.windows(2) {
            walked.push(walked[walked.len() - 1] + w[0].0.distance(w[1].0));
        }
        route.walk_along = walked;
        route
    }

    /// The walkers' way: each point's corner of the pavements, or at a junction whose streets
    /// on the right of the way have zebras, along the pavement to each zebra, over it, and on.
    fn footway(
        &self,
        junctions: &[(usize, u32)],
        zebras: &HashMap<u32, Vec<Zebra>>,
    ) -> Vec<(DVec2, f32)> {
        let n = self.points.len();
        let right = |d: DVec2| DVec2::new(-d.y, d.x);
        let dir = |k: usize| (self.points[k + 1] - self.points[k]).normalize_or_zero();
        let angle = |d: DVec2| d.y.atan2(d.x);
        let tau = std::f64::consts::TAU;
        let mut walk: Vec<(DVec2, f32)> = Vec::with_capacity(n);
        for i in 0..n {
            let level = self.level[i.min(n - 2)];
            let node = junctions.iter().find(|j| j.0 == i).map(|j| j.1);
            let list = node
                .and_then(|v| zebras.get(&v))
                .filter(|_| i > 0 && i + 1 < n);
            if let Some(list) = list {
                let (d0, d1) = (dir(i - 1), dir(i));
                // The streets on the way's right between the one it leaves and the one it
                // takes: turning from -d0 towards d1 the way the right turns.
                let (start, sweep) = (angle(d1), (angle(-d0) - angle(d1)).rem_euclid(tau));
                let mut crossed: Vec<(f64, &Zebra)> = list
                    .iter()
                    .filter_map(|z| {
                        let a = (angle(z.arm) - start).rem_euclid(tau);
                        (a > 0.05 && a < sweep - 0.05).then_some((sweep - a, z))
                    })
                    .collect();
                if !crossed.is_empty() {
                    // In the order the walker meets them, from the street it leaves.
                    crossed.sort_by(|a, b| a.0.total_cmp(&b.0));
                    let v = self.points[i];
                    let (s0, s1) = (self.side(i - 1, false), self.side(i, false));
                    let back = crossed[0]
                        .1
                        .from
                        .min((self.along[i] - self.along[i - 1]) * 0.5);
                    let after = crossed[crossed.len() - 1].1.from;
                    walk.push((v + right(d0) * s0 - d0 * back, level));
                    for (_, z) in crossed {
                        let last = walk[walk.len() - 1].0;
                        let near = usize::from(z.ends[1].distance(last) < z.ends[0].distance(last));
                        let far = 1 - near;
                        walk.push((z.ends[near], level));
                        walk.push((z.kerbs[near], z.level));
                        walk.push((z.kerbs[far], z.level));
                        walk.push((z.ends[far], level));
                    }
                    let on = after.min((self.along[i + 1] - self.along[i]) * 0.5);
                    walk.push((v + right(d1) * s1 + d1 * on, level));
                    continue;
                }
            }
            walk.push((self.corner(i, false), level));
        }
        walk
    }
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
            // By the kerb, clear of the trams; at a stop, over its boarding area behind the
            // shelter, at its height (`Crowd::boarding_level`).
            (half - pave - 0.35).max(1.2)
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
        // On foot, the same share of the walkers' way.
        if !riding && self.walk.len() >= 2 && self.length() > 0.0 {
            let total = self.walk_along[self.walk_along.len() - 1];
            let w = s / self.length() * total;
            let i = self
                .walk_along
                .partition_point(|&a| a <= w)
                .clamp(1, self.walk.len() - 1);
            let seg = self.walk_along[i] - self.walk_along[i - 1];
            let t = if seg > 1e-9 {
                (w - self.walk_along[i - 1]) / seg
            } else {
                0.0
            };
            let ((a, la), (b, lb)) = (self.walk[i - 1], self.walk[i]);
            let dir = (b - a).normalize_or_zero();
            return (a + (b - a) * t, dir, la + (lb - la) * t as f32);
        }
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
        let level = match (riding, self.pave[i - 1] > 0.0) {
            (true, true) => crate::ROAD_LEVEL as f32,
            (true, false) => crate::WALK_LEVEL as f32,
            (false, _) => self.level[i - 1],
        };
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
    /// The zebras at each junction, by its node.
    zebras: HashMap<u32, Vec<Zebra>>,
    routes: HashMap<u32, Option<Route>>,
    /// The parks visitors stand about in, in the order of their places from
    /// `first_park` on.
    parks: Vec<Park>,
    first_park: usize,
    size: f64,
    /// The terraces' seats (the world's place, the sitter's turn), and each seat's
    /// neighbourhood: the cafés within [`NEIGHBOURHOOD`] and how many seats.
    seats: Vec<(Vec3, f32)>,
    neighbourhoods: Vec<(Vec<u32>, u32)>,
    /// The chess tables: each one's middle and its two seats among `seats`.
    games: Vec<(Vec3, [usize; 2])>,
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
    /// Who each mesh's slots showed last frame ([`EMPTY`] where none).
    slots: Vec<Vec<u32>>,
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
    /// The people of `plan`'s `masses` (each its net floor area in `nets`), their day run to
    /// `hour` (game hours from midnight).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        plan: &CityPlan,
        masses: &[Mass],
        nets: &[f64],
        hour: f64,
        seats: Vec<(Vec3, f32)>,
        platforms: Vec<Platform>,
        timetables: Vec<Timetable>,
        mix: &Mix,
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
        // Homes and jobs from each building's net floor area (D-060), `nets` in mass order.
        let buildings: Vec<Building> = masses
            .iter()
            .zip(nets)
            .map(|(m, &net)| {
                let c = geom::centroid(&m.footprint);
                Building {
                    pos: [c.x as f32, c.y as f32],
                    floor_area: net as f32,
                    active_ground: m.lively(),
                    appeal: if m.open { 1.0 } else { 0.0 },
                }
            })
            .collect();
        // The parks people go out to and stand about in: inside the streets round each, spread
        // over its extent from its middle.
        let open: Vec<Park> = plan
            .blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Park)
            .map(|b| {
                let inside = geom::inset(&b.polygon, 9.0).unwrap_or_else(|| b.polygon.clone());
                let spread = (geom::signed_area(&inside).abs().sqrt() * 0.5).min(70.0);
                Park { inside, spread }
            })
            .collect();
        let parks: Vec<[f32; 2]> = plan
            .blocks
            .iter()
            .filter(|b| b.use_ == BlockUse::Park)
            .map(|b| [b.centre.x as f32, b.centre.y as f32])
            .collect();
        let places = population::places_from_buildings(&buildings, &parks, &Uses::default());
        // The parks' places come last, in that order.
        let first_park = places.len() - parks.len();
        // The demo's lifetime: the places live as long as the app.
        let world: &'static World =
            Box::leak(Box::new(World::new(places, plan.size as f32, 250.0)));
        let pop = population::generate(world, mix, Seed::new(2026));
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
            // The street's half width, and its walking level: one level wherever people walk
            // (D-061).
            let side = e.profile.width() * 0.5;
            let level = crate::WALK_LEVEL;
            plan_edges.push((e.points.clone(), side, level, e.profile.pavement()));
        }
        // The zebras, by the junction they serve: their ends a metre onto the pavements.
        let mut zebras: HashMap<u32, Vec<Zebra>> = HashMap::new();
        for c in &plan.ground.crossings {
            let across = DVec2::new(-c.along.y, c.along.x);
            let half = c.span * 0.5;
            zebras.entry(c.node).or_default().push(Zebra {
                arm: c.along,
                from: c.centre.distance(nodes[c.node as usize]),
                ends: [
                    c.centre + across * (half + 1.0),
                    c.centre - across * (half + 1.0),
                ],
                kerbs: [c.centre + across * half, c.centre - across * half],
                level: if c.raised {
                    crate::WALK_LEVEL as f32
                } else {
                    crate::ROAD_LEVEL as f32
                },
            });
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
            zebras,
            routes: HashMap::new(),
            parks: open,
            first_park,
            size: plan.size,
            seats,
            neighbourhoods,
            games: Vec::new(),
            stop_grid,
            timetables,
            boarded: HashSet::new(),
            platforms,
            counts: Vec::new(),
            near: Vec::new(),
            scan: (0, Vec::new(), Vec::new()),
            slots: Vec::new(),
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
    fn seat_people(&mut self, c: DVec2, out: &mut [Vec<(u32, Vec3, f32)>]) {
        let half = self.size * 0.5;
        // Whether seat `k` is taken: in the share the people out round it make of the seats
        // there, the same seats while the share holds (a hash of the seat, not a draw).
        let taken = |k: usize| {
            let (cafes, around) = &self.neighbourhoods[k];
            let people: u32 = cafes
                .iter()
                .filter_map(|&q| self.counts.get(q as usize))
                .sum();
            let share = (f64::from(people) / f64::from((*around).max(1))).min(0.9);
            let pick = (k as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 54;
            (pick as f64) / 1024.0 < share
        };
        for (k, &(p, turn)) in self.seats.iter().enumerate() {
            let at = DVec2::new(f64::from(p.x) + half, f64::from(p.z) + half);
            if at.distance(c) > REACH || !taken(k) {
                continue;
            }
            let mesh = (k * 7 + 3) % GROUPS * POSES + 3;
            out[mesh].push((SEATED + k as u32, p, turn));
        }
        // Round a chess game being played, more people stand and watch than play: two to five,
        // a step back from the table on the players' sides, facing it (the owner, 2026-10-11).
        for (g, &(middle, [a, b])) in self.games.iter().enumerate() {
            let at = DVec2::new(f64::from(middle.x) + half, f64::from(middle.z) + half);
            if at.distance(c) > REACH || !(taken(a) && taken(b)) {
                continue;
            }
            let h = (g as u64 + 1).wrapping_mul(0xD1B5_4A32_D192_ED03);
            let watchers = 2 + (h % 4) as usize;
            // The players sit along the table's local z; the watchers stand round its x sides.
            let (pa, pb) = (self.seats[a].0, self.seats[b].0);
            let across = (pa - pb).with_y(0.0).normalize_or_zero();
            let side = Vec3::new(-across.z, 0.0, across.x);
            for w in 0..watchers {
                let bits = h >> (8 * w);
                let lean = (bits & 0xFF) as f32 / 255.0 - 0.5;
                let s = if w % 2 == 0 { 1.0 } else { -1.0 };
                let dir = (side * s + across * lean * 1.4).normalize_or_zero();
                let spot = middle + dir * (1.3 + ((bits >> 8) & 0x3F) as f32 / 120.0);
                let face = middle - spot;
                let mesh = (((bits >> 16) as usize) % GROUPS) * POSES + 1;
                out[mesh].push((WATCHING + (g * 8 + w) as u32, spot, (-face.z).atan2(face.x)));
            }
        }
    }

    /// The chess tables, each its middle and its two seats among the seats: their games draw
    /// watchers ([`Self::seat_people`]).
    pub fn with_games(mut self, games: Vec<(Vec3, [usize; 2])>) -> Self {
        self.games = games;
        self
    }

    /// The level of a tram stop's boarding area at `q` on the plan, its ramps' slope at its ends,
    /// or `None` off every boarding area: the bikes' strip by the kerb runs over them.
    fn boarding_level(&self, q: DVec2) -> Option<f32> {
        let half = self.size * 0.5;
        let w = Vec3::new((q.x - half) as f32, 0.0, (q.y - half) as f32);
        let (deep, high) = (crate::boarding_deep() as f32, crate::PLATFORM_HIGH as f32);
        let (length, run) = (crate::PLATFORM, crate::PLATFORM_HIGH as f32 * 12.0);
        let mut level = None;
        self.stop_grid.for_each_within(q, 40.0, |_, k| {
            let at = self.platforms[k as usize].at;
            let corner = at.w_axis.truncate();
            let d = w - Vec3::new(corner.x, 0.0, corner.z);
            let (x, z) = (d.dot(at.x_axis.truncate()), d.dot(at.z_axis.truncate()));
            if !(0.0..=deep).contains(&z) || !(-run..=length + run).contains(&x) {
                return;
            }
            let h = if x < 0.0 {
                high * (1.0 + x / run)
            } else if x > length {
                high * (1.0 - (x - length) / run)
            } else {
                high
            };
            let y = corner.y + h;
            level = Some(level.map_or(y, |l: f32| l.max(y)));
        });
        level
    }

    /// Advances the people's day by `real` seconds.
    pub fn advance(&mut self, real: f64) {
        self.clock += real * TIME_SCALE;
        self.sim.run_until(self.clock as u32);
    }

    /// The figures to draw round `camera` (the world's x and z): for each of the
    /// `POSES × GROUPS` meshes in order (each palette's left stride, standing, riding, seated,
    /// right stride, the half strides), its [`PER_MESH`] slots, each empty or a figure;
    /// `tram_clock` is the trams' service clock, whose standing trams the waiting people board.
    ///
    /// A mesh shows the nearest of those it could show, so a full mesh drops the farthest, not
    /// whoever the scan met last (the owner saw a walker ahead blink in and out). Someone drawn
    /// in a mesh keeps their slot while they stay in it, so their motion is their own.
    pub fn figures(&mut self, camera: Vec2, tram_clock: f64) -> Vec<Vec<Option<Figure>>> {
        let out = self.candidates(camera, tram_clock);
        let c = Vec3::new(camera.x, 0.0, camera.y);
        if self.slots.len() != out.len() {
            self.slots = vec![vec![EMPTY; PER_MESH]; out.len()];
        }
        out.into_iter()
            .zip(&mut self.slots)
            .map(|(mut found, slots)| {
                if found.len() > PER_MESH {
                    let far =
                        |(_, at, _): &(u32, Vec3, f32)| (*at - c).with_y(0.0).length_squared();
                    found.select_nth_unstable_by(PER_MESH - 1, |a, b| far(a).total_cmp(&far(b)));
                    found.truncate(PER_MESH);
                }
                keep_slots(slots, &found)
            })
            .collect()
    }

    /// Everyone [`Self::figures`] could draw round `camera`, by mesh: (who, where, turn).
    fn candidates(&mut self, camera: Vec2, tram_clock: f64) -> Vec<Vec<(u32, Vec3, f32)>> {
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
                // In a park: standing about it, scattered by their seed over its extent (the
                // place stands at its middle).
                let at = self.sim.pop.at[p] as usize;
                let park = at
                    .checked_sub(self.first_park)
                    .and_then(|k| self.parks.get(k));
                if let Some(park) =
                    park.filter(|_| places.get(at).is_some_and(|q| q.kind == PlaceKind::Park))
                {
                    let s = self.sim.pop.seed[p];
                    let jitter =
                        |bits: u32| (f64::from(bits & 0xFF) / 255.0 - 0.5) * 2.0 * park.spread;
                    let q = places[at].pos;
                    let q = DVec2::new(
                        f64::from(q[0]) + jitter(s),
                        f64::from(q[1]) + jitter(s >> 8),
                    );
                    let mesh = (s as usize % GROUPS) * POSES + 1;
                    if q.distance(c) <= REACH && geom::contains(&park.inside, q) {
                        let at = Vec3::new(
                            (q.x - half) as f32,
                            crate::WALK_LEVEL as f32,
                            (q.y - half) as f32,
                        );
                        out[mesh].push((
                            p as u32,
                            at,
                            ((s >> 20) as f32 / 4096.0) * std::f32::consts::TAU,
                        ));
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
                        + away * (0.8 + ((s >> 8) % 12) as f32 / 10.0)
                        + Vec3::Y * 0.28;
                    let at = DVec2::new(f64::from(spot.x) + half, f64::from(spot.z) + half);
                    let mesh = (s as usize % GROUPS) * POSES + 1;
                    if at.distance(c) <= REACH {
                        out[mesh].push((p as u32, spot, away.z.atan2(-away.x)));
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
            let (q, dir, mut level) = route.at(gone * route.length(), riding);
            if q.distance(c) > REACH {
                continue;
            }
            if riding && let Some(boarding) = self.boarding_level(q) {
                level = level.max(boarding);
            }
            let seed = self.sim.pop.seed[p];
            // Each a little to one side of the line, by their seed (±0.6 m on foot, ±0.3 m
            // riding): a household or a workplace going out together walks side by side
            // rather than in one another.
            let aside = (((seed >> 8) % 9) as f64 - 4.0) * if riding { 0.075 } else { 0.15 };
            let q = q + DVec2::new(-dir.y, dir.x) * aside;
            // Riding, or walking a gait cycle of 1.4 m in eight poses: the left stride, half,
            // passing, half, the right stride, half, passing, half. Four poses held 0.7 m each
            // read as jerky (the owner); each person starts at their own phase.
            let pose = if riding {
                2
            } else {
                let phase = (gone * route.length() / 0.175) as usize + seed as usize;
                [0, 5, 1, 6, 4, 6, 1, 5][phase % 8]
            };
            let mesh = (seed as usize % GROUPS) * POSES + pose;
            let at = Vec3::new((q.x - half) as f32, level, (q.y - half) as f32);
            out[mesh].push((p as u32, at, (-dir.y).atan2(dir.x) as f32));
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
            legs.push((u, e, fwd));
            v = u;
        }
        legs.reverse();
        let mut points = Vec::new();
        let mut half = Vec::new();
        let mut level = Vec::new();
        let mut pave = Vec::new();
        let mut junctions = Vec::new();
        for (u, e, fwd) in legs {
            // Where the last leg ended: the junction this one leaves.
            if !points.is_empty() {
                junctions.push((points.len() - 1, u));
            }
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
        Some(Route::new(
            points,
            half,
            level,
            pave,
            &junctions,
            &self.zebras,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_keep_their_slots() {
        let mut slots = vec![EMPTY; 4];
        let at = |x: f32| Vec3::new(x, 0.0, 0.0);
        let first = keep_slots(&mut slots, &[(7, at(1.0), 0.0), (9, at(2.0), 0.0)]);
        assert_eq!(slots, [7, 9, EMPTY, EMPTY]);
        assert!(first.iter().flatten().all(|f| !f.kept));
        // 7 goes, 5 comes, 9 walks on: 9 stays in its slot and moves, 5 takes the free one.
        let next = keep_slots(&mut slots, &[(5, at(3.0), 0.0), (9, at(2.1), 0.0)]);
        assert_eq!(slots, [5, 9, EMPTY, EMPTY]);
        assert_eq!(
            next[1],
            Some(Figure {
                at: at(2.1),
                turn: 0.0,
                kept: true
            })
        );
        assert!(next[0].is_some_and(|f| !f.kept && f.at == at(3.0)));
        assert_eq!(next[2], None);
    }

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
            let route = Route::new(
                points,
                vec![14.0, 8.0, 8.0],
                vec![0.3, 0.16, 0.16],
                vec![6.0, 0.0, 0.0],
                &[(1, 0), (2, 1)],
                &HashMap::new(),
            );
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

    #[test]
    fn walkers_cross_the_side_street_on_its_zebra() {
        // East through a junction at (100, 0) with a street south off it, on the walker's right;
        // its zebra 12 m down it, 6 m of carriageway.
        let zebra = Zebra {
            arm: DVec2::Y,
            from: 12.0,
            ends: [DVec2::new(96.0, 12.0), DVec2::new(104.0, 12.0)],
            kerbs: [DVec2::new(97.0, 12.0), DVec2::new(103.0, 12.0)],
            level: 0.16,
        };
        let zebras = HashMap::from([(7, vec![zebra])]);
        let route = Route::new(
            vec![DVec2::ZERO, DVec2::new(100.0, 0.0), DVec2::new(200.0, 0.0)],
            vec![9.0, 9.0],
            vec![0.3, 0.3],
            vec![6.0, 6.0],
            &[(1, 7)],
            &zebras,
        );
        // Over the zebra, near end first, at the road's level between the kerbs.
        let walk: Vec<DVec2> = route.walk.iter().map(|w| w.0).collect();
        let near = walk.iter().position(|p| p.distance(zebra.ends[0]) < 1e-9);
        let far = walk.iter().position(|p| p.distance(zebra.ends[1]) < 1e-9);
        assert!(near.is_some() && far.is_some() && near < far, "{walk:?}");
        let (_, _, level) = route.at(route.length() * 0.5, false);
        assert!(level >= 0.16, "{level}");
        // Never across the junction's middle: nowhere within 5 m of the node.
        let mut s = 0.0;
        while s <= route.length() {
            let (p, _, _) = route.at(s, false);
            assert!(
                p.distance(DVec2::new(100.0, 0.0)) > 5.0,
                "at {p} through the junction"
            );
            s += 0.25;
        }
        // A walker turning right there crosses nothing.
        let right = Route::new(
            vec![
                DVec2::ZERO,
                DVec2::new(100.0, 0.0),
                DVec2::new(100.0, 100.0),
            ],
            vec![9.0, 9.0],
            vec![0.3, 0.3],
            vec![6.0, 6.0],
            &[(1, 7)],
            &zebras,
        );
        assert_eq!(right.walk.len(), 3, "{:?}", right.walk);
    }
}
