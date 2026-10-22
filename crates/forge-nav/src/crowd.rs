//! Following and avoidance (#317, D-076): agents walking their paths among one another, on a
//! walk field.
//!
//! - **Following** ([`Follow`]): an agent heads for its path's next corner and passes it once
//!   within [`CORNER_REACH`] where the way on to the corner after it is walkable from where it
//!   stands, else at the corner or once it stops coming nearer. It slows within [`ARRIVAL`]
//!   metres of where it must stop: the path's end, or a corner where a link starts. There it
//!   waits for the game to cross the link
//!   ([`Follow::at_link`], then [`Follow::crossed`]). Within [`NEAR_WALLS`] of the walkable's
//!   edge it heads [`LOOK_AHEAD`] along its leg instead, back to the line the path was pulled
//!   along clear of the walls (a door's middle); in the open it may drift aside.
//! - **Avoidance** ([`Crowd`]): ORCA, optimal reciprocal collision avoidance (van den Berg, Guy,
//!   Lin and Manocha, "Reciprocal n-body collision avoidance", 2011), written here from the
//!   algorithm of UNC's RVO2 library (Apache-2.0).
//!   - **The constraints:** each neighbour within [`CrowdSettings::neighbour_distance`] gives a
//!     half-plane of velocities that keeps the two apart for [`CrowdSettings::time_horizon`]
//!     seconds, each agent taking half the avoidance.
//!   - **The choice:** the agent takes the velocity nearest the one it prefers, within the
//!     half-planes and its top speed. This is a linear program in the plane, solved
//!     incrementally. When the half-planes leave nothing, it takes the velocity that breaks them
//!     least.
//!   - **Keeping right:** slowed away from walls, it turns the velocity it prefers to its right,
//!     up to 45°, and a crowd jammed face to face turns as one and unwinds.
//!   - **Giving way:** two agents heading opposite ways, blocked face to face for 3 s (a door
//!     too narrow for both), the one with the higher index backs off for 1.5 s.
//!   - **Same result on every machine** (D-016: `+ − × ÷ sqrt` only):
//!     - the neighbours come from a grid: the nearest [`CrowdSettings::max_neighbours`], ordered
//!       by distance and then by index;
//!     - every new velocity is chosen from the step's state before anyone moves;
//!     - a nudge of 1 mm/s from a hash of the agent and the step breaks perfect symmetry.
//! - **The ground:** given a walk field, a move the field does not walk straight is slid along x
//!   or z, or not made.
//!
//! No patent was found on ORCA (`docs/research/navigation-traversal.md`, §4). RVO2's licence
//! carries a patent licence from its contributors.

use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::{Field, Path, Tiles};

/// How near a corner an agent passes it, metres (where the way on is clear: [`Follow`]).
pub const CORNER_REACH: f32 = 0.25;
/// How near a corner an agent passes it whatever lies beyond, metres.
const AT_CORNER: f32 = 0.03;
/// After how many calls without coming a centimetre nearer its corner an agent within reach
/// passes it whatever lies beyond: half a second at 60 steps a second.
const STALLED: u32 = 30;
/// How far before a stop an agent starts slowing, metres.
pub const ARRIVAL: f32 = 1.0;
/// How far along its path's leg, past where it stands across from it, an agent near walls
/// heads, metres.
pub const LOOK_AHEAD: f32 = 0.7;
/// How near the walkable's edge an agent keeps to its path's line, metres.
pub const NEAR_WALLS: f32 = 1.0;
/// How far apart in height two agents are on different floors and ignore each other, metres.
const OTHER_FLOOR: f32 = 1.5;
/// Below this, two lines of the linear program are parallel.
const PARALLEL: f32 = 1e-5;

fn plan(p: Vec3) -> Vec2 {
    Vec2::new(p.x, p.z)
}

/// The determinant of `a` and `b`: positive when `b` turns left of `a`.
fn det(a: Vec2, b: Vec2) -> f32 {
    a.x * b.y - a.y * b.x
}

/// Whether `at` lies on `field` within [`NEAR_WALLS`] of the walkable's edge.
fn near_walls(field: Option<&Field>, at: Vec3) -> bool {
    field.is_some_and(|f| {
        f.place(at).is_some_and(|s| {
            let clearance = f32::from(f.spans()[s as usize].clearance) * f.desc().cell * 0.5;
            clearance < NEAR_WALLS
        })
    })
}

/// A path being followed.
#[derive(Clone, Debug, PartialEq)]
pub struct Follow {
    points: Vec<Vec3>,
    /// Where each link starts (the corner's index) and the link's index, as the path gives them.
    links: Vec<(usize, usize)>,
    /// The corner it heads for.
    next: usize,
    /// The link it waits at.
    waiting: Option<usize>,
    /// The nearest it has come to its next corner, metres, and the calls since it last came
    /// a centimetre nearer.
    closest: (f32, u32),
}

impl Follow {
    /// `path` followed from its first corner.
    pub fn new(path: &Path) -> Self {
        Self {
            points: path.points.clone(),
            links: path.links.clone(),
            next: path.points.len().min(1),
            waiting: None,
            closest: (f32::MAX, 0),
        }
    }

    /// The corner it heads for, if any is left.
    pub fn next(&self) -> Option<Vec3> {
        self.points.get(self.next).copied()
    }

    /// Whether it reached the path's end.
    pub fn arrived(&self) -> bool {
        self.next >= self.points.len()
    }

    /// The link it waits at, at the corner where the link starts, for the game to cross.
    pub fn at_link(&self) -> Option<usize> {
        self.waiting
    }

    /// The far end of the link it waits at: where its path goes on once the link is crossed, a
    /// place on its field (a crossing lands there: `CrossingMotion::landing`).
    pub fn link_end(&self) -> Option<Vec3> {
        self.waiting?;
        self.points.get(self.next + 1).copied()
    }

    /// The game crossed the link it waited at: the agent is at the link's other end and goes on
    /// from there.
    pub fn crossed(&mut self) {
        if self.waiting.take().is_some() {
            self.next += 2;
        }
    }

    /// The velocity it prefers at `at` (x and z), `speed` m/s at most: towards its next corner,
    /// slowing to stop at the path's end or at a link's start, zero there. It passes the corners
    /// it reaches. On `field`, within [`NEAR_WALLS`] of the walkable's edge, it heads back to its
    /// path's line instead.
    pub fn preferred(&mut self, at: Vec3, speed: f32, field: Option<&Field>) -> Vec2 {
        let near_walls = near_walls(field, at);
        while self.waiting.is_none() {
            let Some(corner) = self.next() else {
                break;
            };
            let d = plan(corner) - plan(at);
            let distance = d.length();
            if distance < self.closest.0 - 0.01 {
                self.closest = (distance, 0);
            } else {
                self.closest.1 += 1;
            }
            // Within reach it passes the corner where the way on to the corner after it is a
            // straight walkable line from where it stands, else once at it or once it stops
            // coming nearer (others in the way: a queue at a door). Passing two corners round a
            // wall's end within reach, one cut the end and stood wedged against it (#317).
            let way_on = || {
                let (Some(f), Some(&after)) = (field, self.points.get(self.next + 1)) else {
                    return true;
                };
                match (f.place(at), f.place(after)) {
                    (Some(a), Some(b)) => f.walkable_line(a, b),
                    _ => true,
                }
            };
            let pass = distance <= CORNER_REACH
                && (distance < AT_CORNER || self.closest.1 >= STALLED || way_on());
            if !pass {
                let stop = distance + self.beyond();
                // Near walls, towards the point [`LOOK_AHEAD`] along its leg past where it
                // stands across from it: pushed off the leg by others, it comes back to the line
                // the path was pulled along clear of them, through a door's middle. In the open
                // it heads for the corner, and a crowd that pushed it aside can turn.
                let from = plan(self.points[self.next - 1]);
                let leg = plan(corner) - from;
                let length = leg.length();
                let ahead = if near_walls && length > 0.0 {
                    let along = ((plan(at) - from).dot(leg) / length).max(0.0);
                    from + leg * ((along + LOOK_AHEAD).min(length) / length) - plan(at)
                } else {
                    d
                };
                let way = if ahead.length_squared() > 1e-8 {
                    ahead.normalize()
                } else {
                    d / distance
                };
                return way * (speed * (stop / ARRIVAL).min(1.0));
            }
            match self.links.iter().find(|&&(k, _)| k == self.next) {
                Some(&(_, link)) => self.waiting = Some(link),
                None => self.next += 1,
            }
            self.closest = (f32::MAX, 0);
        }
        Vec2::ZERO
    }

    /// The way from the next corner to where it must stop, metres in plan.
    fn beyond(&self) -> f32 {
        let last = self.points.len() - 1;
        let stop = self
            .links
            .iter()
            .map(|&(k, _)| k)
            .filter(|&k| k >= self.next)
            .min()
            .unwrap_or(last);
        (self.next..stop)
            .map(|k| plan(self.points[k + 1]).distance(plan(self.points[k])))
            .sum()
    }
}

/// An agent of a crowd.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Agent {
    /// Its feet, metres.
    pub position: Vec3,
    /// Its velocity over the ground (x and z), m/s: the one it took last step.
    pub velocity: Vec2,
    /// The velocity it would take alone this step (x and z): its path's, or the game's.
    pub preferred: Vec2,
    /// Its radius, metres.
    pub radius: f32,
    /// Its top speed, m/s.
    pub max_speed: f32,
    /// How long it has gone at under a fifth of the speed it prefers, seconds.
    pub blocked_for: f32,
    /// How long it still gives way, backing off from one it met face to face, seconds.
    pub giving_way: f32,
}

impl Agent {
    /// A still agent at `position`, `radius` metres round, walking at most `max_speed` m/s.
    pub fn new(position: Vec3, radius: f32, max_speed: f32) -> Self {
        Self {
            position,
            velocity: Vec2::ZERO,
            preferred: Vec2::ZERO,
            radius,
            max_speed,
            blocked_for: 0.0,
            giving_way: 0.0,
        }
    }

    /// The velocity it heads for this step: the one it prefers, or back from it at half speed
    /// while it gives way.
    fn wanted(&self) -> Vec2 {
        if self.giving_way > 0.0 {
            -self.preferred * 0.5
        } else {
            self.preferred
        }
    }
}

/// How long two agents stand blocked face to face before one gives way, seconds.
const GIVE_WAY_AFTER: f32 = 3.0;
/// How long the one that gives way backs off, seconds.
const GIVE_WAY_FOR: f32 = 1.5;
/// How near two agents' edges are when they meet face to face, metres.
const FACE_TO_FACE: f32 = 0.6;

/// How a crowd's agents avoid one another.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CrowdSettings {
    /// How far off a neighbour counts, metres (also the grid's cell).
    pub neighbour_distance: f32,
    /// How many of the nearest neighbours count.
    pub max_neighbours: usize,
    /// How far ahead the agents keep clear of one another, seconds.
    pub time_horizon: f32,
}

impl Default for CrowdSettings {
    /// People: neighbours within 5 m, the nearest 10, kept clear 2 s ahead.
    fn default() -> Self {
        Self {
            neighbour_distance: 5.0,
            max_neighbours: 10,
            time_horizon: 2.0,
        }
    }
}

/// A half-plane of velocities: those left of the line through `point` along `direction`.
#[derive(Clone, Copy, Debug)]
struct Line {
    point: Vec2,
    direction: Vec2,
}

/// Agents walking among one another.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Crowd {
    /// The agents, by index.
    pub agents: Vec<Agent>,
    /// How they avoid one another.
    pub settings: CrowdSettings,
    /// The steps taken: with an agent's index, the seed of its nudge.
    steps: u64,
}

/// How far each preferred velocity is nudged, m/s: enough to break a perfectly symmetric
/// meeting, which ORCA alone can hold still (RVO2's examples nudge so too), too little to see.
const NUDGE: f32 = 1e-3;

/// How far a blocked agent turns the velocity it prefers to its right, at most, as a share of
/// that velocity set sideways (1.0: 45°), in proportion to how much slower it goes. People
/// keep to one side, and a crowd jammed face to face turns as one and unwinds; ORCA alone holds
/// such a jam still, the velocities that would turn it lying square to those preferred.
const KEEP_RIGHT: f32 = 1.0;

/// A number in [−1, 1) from `seed` (SplitMix64's finaliser).
fn unit(seed: u64) -> f32 {
    let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 23) as f32 - 1.0
}

impl Crowd {
    /// A crowd of none.
    pub fn new(settings: CrowdSettings) -> Self {
        Self {
            agents: Vec::new(),
            settings,
            steps: 0,
        }
    }

    /// Adds an agent; its index.
    pub fn add(&mut self, agent: Agent) -> usize {
        self.agents.push(agent);
        self.agents.len() - 1
    }

    /// One step of `dt` seconds: every agent's velocity chosen from the step's state, then the
    /// moves, kept to `field`'s walkable ground when it is given.
    pub fn step(&mut self, dt: f32, field: Option<&Field>) {
        self.step_on(dt, |_| field);
    }

    /// [`Self::step`] on field tiles: each agent on the field of the tile under its feet
    /// ([`Tiles::field_at`]); where no tile is built, moves are not kept to a field.
    pub fn step_tiled(&mut self, dt: f32, tiles: &Tiles) {
        self.step_on(dt, |p| tiles.field_at(p));
    }

    /// [`Self::step`], each agent on the field `field_at` gives at its feet.
    fn step_on<'f>(&mut self, dt: f32, field_at: impl Fn(Vec3) -> Option<&'f Field>) {
        if dt <= 0.0 {
            return;
        }
        let grid = self.grid();
        let chosen: Vec<Vec2> = (0..self.agents.len())
            .map(|i| {
                let at = self.agents[i].position;
                self.avoid(i, dt, &grid, near_walls(field_at(at), at))
            })
            .collect();
        for (agent, v) in self.agents.iter_mut().zip(chosen) {
            let to = walk_on(field_at(agent.position), agent.position, v * dt);
            agent.velocity = (plan(to) - plan(agent.position)) / dt;
            agent.position = to;
            agent.giving_way = (agent.giving_way - dt).max(0.0);
            let speed = agent.preferred.length();
            agent.blocked_for = if speed > 0.0
                && agent.giving_way == 0.0
                && agent.velocity.length() < 0.2 * speed
            {
                agent.blocked_for + dt
            } else {
                0.0
            };
        }
        self.give_way(&grid);
        self.steps += 1;
    }

    /// Two agents blocked face to face for [`GIVE_WAY_AFTER`] (a doorway too narrow for both):
    /// the one with the higher index backs off for [`GIVE_WAY_FOR`] and the other goes through.
    fn give_way(&mut self, grid: &HashMap<(i32, i32), Vec<u32>>) {
        let mut yielding = Vec::new();
        for i in 0..self.agents.len() {
            let a = self.agents[i];
            if a.blocked_for < GIVE_WAY_AFTER {
                continue;
            }
            let met = self.neighbours(i, grid).into_iter().any(|j| {
                let b = self.agents[j as usize];
                let towards = plan(b.position) - plan(a.position);
                (j as usize) < i
                    && b.blocked_for >= GIVE_WAY_AFTER
                    && towards.length() - a.radius - b.radius < FACE_TO_FACE
                    && a.preferred.dot(towards) > 0.0
                    && b.preferred.dot(-towards) > 0.0
                    && a.preferred.dot(b.preferred) < 0.0
            });
            if met {
                yielding.push(i);
            }
        }
        for i in yielding {
            let a = &mut self.agents[i];
            a.giving_way = GIVE_WAY_FOR;
            a.blocked_for = 0.0;
        }
    }

    /// The agents by the grid's cells.
    fn grid(&self) -> HashMap<(i32, i32), Vec<u32>> {
        let mut grid: HashMap<(i32, i32), Vec<u32>> = HashMap::new();
        for (i, a) in self.agents.iter().enumerate() {
            grid.entry(self.cell(a.position))
                .or_default()
                .push(i as u32);
        }
        grid
    }

    fn cell(&self, p: Vec3) -> (i32, i32) {
        let c = self.settings.neighbour_distance.max(0.1);
        ((p.x / c).floor() as i32, (p.z / c).floor() as i32)
    }

    /// Agent `i`'s neighbours: within the neighbour distance on its floor, the nearest first
    /// (ties by index), at most the settings' number.
    fn neighbours(&self, i: usize, grid: &HashMap<(i32, i32), Vec<u32>>) -> Vec<u32> {
        let me = self.agents[i].position;
        let (cx, cz) = self.cell(me);
        let reach = self.settings.neighbour_distance;
        let mut near: Vec<(f32, u32)> = Vec::new();
        for x in cx - 1..=cx + 1 {
            for z in cz - 1..=cz + 1 {
                for &j in grid.get(&(x, z)).map_or(&[][..], |v| &v[..]) {
                    let other = self.agents[j as usize].position;
                    if j as usize == i || (other.y - me.y).abs() > OTHER_FLOOR {
                        continue;
                    }
                    let d = plan(other).distance_squared(plan(me));
                    if d <= reach * reach {
                        near.push((d, j));
                    }
                }
            }
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        near.truncate(self.settings.max_neighbours);
        near.into_iter().map(|(_, j)| j).collect()
    }

    /// Agent `i`'s new velocity: ORCA's, from the step's state (`walled` near walls, where it
    /// does not turn).
    fn avoid(&self, i: usize, dt: f32, grid: &HashMap<(i32, i32), Vec<u32>>, walled: bool) -> Vec2 {
        let me = self.agents[i];
        let (p, v) = (plan(me.position), me.velocity);
        let inv_horizon = 1.0 / self.settings.time_horizon;
        let mut lines = Vec::new();
        for j in self.neighbours(i, grid) {
            let other = self.agents[j as usize];
            let rel_p = plan(other.position) - p;
            let rel_v = v - other.velocity;
            let dist_sq = rel_p.length_squared();
            let r = me.radius + other.radius;
            let r_sq = r * r;
            let (direction, u);
            if dist_sq > r_sq {
                // Apart: the velocity obstacle truncated at the time horizon.
                let w = rel_v - rel_p * inv_horizon;
                let w_len_sq = w.length_squared();
                let dot = w.dot(rel_p);
                if dot < 0.0 && dot * dot > r_sq * w_len_sq {
                    // Nearest its cut-off circle.
                    let w_len = w_len_sq.sqrt();
                    let unit = w / w_len;
                    direction = Vec2::new(unit.y, -unit.x);
                    u = unit * (r * inv_horizon - w_len);
                } else {
                    // Nearest one of its legs.
                    let leg = (dist_sq - r_sq).sqrt();
                    direction = if det(rel_p, w) > 0.0 {
                        Vec2::new(rel_p.x * leg - rel_p.y * r, rel_p.x * r + rel_p.y * leg)
                            / dist_sq
                    } else {
                        -Vec2::new(rel_p.x * leg + rel_p.y * r, -rel_p.x * r + rel_p.y * leg)
                            / dist_sq
                    };
                    u = direction * rel_v.dot(direction) - rel_v;
                }
            } else {
                // Overlapping: apart within this step.
                let inv_step = 1.0 / dt;
                let w = rel_v - rel_p * inv_step;
                let w_len = w.length();
                let unit = if w_len > 0.0 {
                    w / w_len
                } else if i < j as usize {
                    // On the same spot, still: the lower index goes one way, the other the other.
                    Vec2::X
                } else {
                    Vec2::NEG_X
                };
                direction = Vec2::new(unit.y, -unit.x);
                u = unit * (r * inv_step - w_len);
            }
            lines.push(Line {
                point: v + u * 0.5,
                direction,
            });
        }
        // Slowed, it turns to its right as people keep to one side (not by walls: turning there
        // only drives it into them), and a nudge breaks a perfectly symmetric meeting.
        let wanted = me.wanted();
        let speed = wanted.length();
        let preferred = if speed > 0.0 {
            let blocked = if walled {
                0.0
            } else {
                (1.0 - me.velocity.length() / speed).clamp(0.0, 1.0)
            };
            let right = Vec2::new(-wanted.y, wanted.x);
            let seed = (self.steps << 24) ^ i as u64;
            let nudge = Vec2::new(unit(seed * 2), unit(seed * 2 + 1)) * NUDGE;
            wanted + right * (KEEP_RIGHT * blocked) + nudge
        } else {
            wanted
        };
        let mut result = Vec2::ZERO;
        let failed = program2(&lines, me.max_speed, preferred, false, &mut result);
        if failed < lines.len() {
            program3(&lines, failed, me.max_speed, &mut result);
        }
        result
    }
}

/// The move of `step` from `from`, on `field`'s walkable ground when given: straight, else along
/// x alone, else along z alone, else none.
fn walk_on(field: Option<&Field>, from: Vec3, step: Vec2) -> Vec3 {
    let to = from + Vec3::new(step.x, 0.0, step.y);
    let Some(field) = field else {
        return to;
    };
    let start = field.place(from);
    for t in [
        to,
        from + Vec3::new(step.x, 0.0, 0.0),
        from + Vec3::new(0.0, 0.0, step.y),
    ] {
        if let Some(b) = field.place(t)
            && start.is_none_or(|a| a == b || field.walkable_line(a, b))
        {
            return Vec3::new(t.x, field.point(b).y, t.z);
        }
    }
    from
}

/// The best velocity on line `k` within speed `radius`, the lines before it kept: nearest `opt`,
/// or furthest along it when `direction` (`opt` then a unit direction). False when they leave
/// none on the line.
fn program1(
    lines: &[Line],
    k: usize,
    radius: f32,
    opt: Vec2,
    direction: bool,
    result: &mut Vec2,
) -> bool {
    let line = lines[k];
    let dot = line.point.dot(line.direction);
    let discriminant = dot * dot + radius * radius - line.point.length_squared();
    if discriminant < 0.0 {
        // The speed circle lies wholly outside the line's half-plane.
        return false;
    }
    let root = discriminant.sqrt();
    let (mut left, mut right) = (-dot - root, -dot + root);
    for other in &lines[..k] {
        let denominator = det(line.direction, other.direction);
        let numerator = det(other.direction, line.point - other.point);
        if denominator.abs() <= PARALLEL {
            if numerator < 0.0 {
                return false;
            }
            continue;
        }
        let t = numerator / denominator;
        if denominator >= 0.0 {
            right = right.min(t);
        } else {
            left = left.max(t);
        }
        if left > right {
            return false;
        }
    }
    let t = if direction {
        if opt.dot(line.direction) > 0.0 {
            right
        } else {
            left
        }
    } else {
        line.direction.dot(opt - line.point).clamp(left, right)
    };
    *result = line.point + line.direction * t;
    true
}

/// The velocity within speed `radius` and every line's half-plane nearest `opt` (or furthest
/// along it when `direction`): the index of the first line that leaves none, or the lines' count.
fn program2(lines: &[Line], radius: f32, opt: Vec2, direction: bool, result: &mut Vec2) -> usize {
    *result = if direction {
        opt * radius
    } else if opt.length_squared() > radius * radius {
        opt.normalize() * radius
    } else {
        opt
    };
    for k in 0..lines.len() {
        if det(lines[k].direction, lines[k].point - *result) > 0.0 {
            let kept = *result;
            if !program1(lines, k, radius, opt, direction, result) {
                *result = kept;
                return k;
            }
        }
    }
    lines.len()
}

/// When the half-planes from `first` on leave nothing: the velocity within speed `radius` that
/// breaks them least (the greatest distance into a broken half-plane, least).
fn program3(lines: &[Line], first: usize, radius: f32, result: &mut Vec2) {
    let mut distance = 0.0;
    for k in first..lines.len() {
        if det(lines[k].direction, lines[k].point - *result) <= distance {
            continue;
        }
        let line = lines[k];
        let mut projected = Vec::with_capacity(k);
        for other in &lines[..k] {
            let determinant = det(line.direction, other.direction);
            let point = if determinant.abs() <= PARALLEL {
                if line.direction.dot(other.direction) > 0.0 {
                    // The same way: the other adds nothing.
                    continue;
                }
                (line.point + other.point) * 0.5
            } else {
                line.point
                    + line.direction
                        * (det(other.direction, line.point - other.point) / determinant)
            };
            projected.push(Line {
                point,
                direction: (other.direction - line.direction).normalize_or_zero(),
            });
        }
        let kept = *result;
        let away = Vec2::new(-line.direction.y, line.direction.x);
        if program2(&projected, radius, away, true, result) < projected.len() {
            // Only rounding gets here: keep the last.
            *result = kept;
        }
        distance = det(line.direction, line.point - *result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{desc, walled};
    use crate::{Field, PERSON};

    /// A straight path from `a` to `b`.
    fn straight(a: Vec3, b: Vec3) -> Path {
        Path {
            points: vec![a, b],
            links: Vec::new(),
            complete: true,
            expanded: 0,
        }
    }

    /// Runs `crowd` with each agent following its path at its top speed for `seconds` at 60 Hz;
    /// the least distance between two agents' edges on the way (negative when they overlapped).
    fn run(crowd: &mut Crowd, follows: &mut [Follow], seconds: f32, field: Option<&Field>) -> f32 {
        let dt = 1.0 / 60.0;
        let mut least = f32::INFINITY;
        for _ in 0..(seconds * 60.0) as u32 {
            for (a, f) in crowd.agents.iter_mut().zip(follows.iter_mut()) {
                a.preferred = f.preferred(a.position, a.max_speed, field);
            }
            crowd.step(dt, field);
            let n = crowd.agents.len();
            for i in 0..n {
                for j in i + 1..n {
                    let (a, b) = (crowd.agents[i], crowd.agents[j]);
                    let gap = plan(a.position).distance(plan(b.position)) - a.radius - b.radius;
                    least = least.min(gap);
                }
            }
        }
        least
    }

    /// `n` people on a circle of `r` metres round the origin, each going to the opposite point.
    /// `n` people on a circle of `r` metres round the origin, each off its place by up to 10 cm
    /// (people never stand perfectly round), each going to the point opposite its place.
    fn circle(n: usize, r: f32) -> (Crowd, Vec<Follow>, Vec<Vec3>) {
        let mut crowd = Crowd::new(CrowdSettings::default());
        let (mut follows, mut goals) = (Vec::new(), Vec::new());
        for k in 0..n {
            let (s, c) = (k as f32 / n as f32 * std::f32::consts::TAU).sin_cos();
            let off = Vec3::new(unit(k as u64 * 2), 0.0, unit(k as u64 * 2 + 1)) * 0.1;
            let start = Vec3::new(c * r, 0.0, s * r) + off;
            let goal = -start;
            crowd.add(Agent::new(start, 0.3, 1.4));
            follows.push(Follow::new(&straight(start, goal)));
            goals.push(goal);
        }
        (crowd, follows, goals)
    }

    #[test]
    fn two_people_meeting_head_on_step_round_each_other() {
        let mut crowd = Crowd::new(CrowdSettings::default());
        let (a, b) = (Vec3::new(-4.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0));
        crowd.add(Agent::new(a, 0.3, 1.4));
        crowd.add(Agent::new(b, 0.3, 1.4));
        let mut follows = vec![Follow::new(&straight(a, b)), Follow::new(&straight(b, a))];
        let least = run(&mut crowd, &mut follows, 12.0, None);
        assert!(least > -0.01, "{least}");
        assert!(follows.iter().all(Follow::arrived));
        assert!(crowd.agents[0].position.distance(b) < CORNER_REACH + 0.01);
        assert!(crowd.agents[1].position.distance(a) < CORNER_REACH + 0.01);
    }

    #[test]
    fn a_circle_of_people_crosses_itself_without_touching_the_same_on_every_run() {
        // Twelve on a circle 16 m across, each to the opposite point: they meet in the middle,
        // turn as one to their right and unwind. (Twice as many pack into a ring touching all
        // round, which ORCA cannot set turning: the limit of D-076's first step.)
        let (mut crowd, mut follows, goals) = circle(12, 8.0);
        let least = run(&mut crowd, &mut follows, 40.0, None);
        assert!(least > -0.02, "{least}");
        for (a, g) in crowd.agents.iter().zip(&goals) {
            assert!(a.position.distance(*g) < CORNER_REACH + 0.05, "{a:?} {g}");
        }
        // The same run again: the same places, to the bit.
        let (mut again, mut follows, _) = circle(12, 8.0);
        run(&mut again, &mut follows, 40.0, None);
        for (a, b) in crowd.agents.iter().zip(&again.agents) {
            assert_eq!(a.position.to_array(), b.position.to_array());
        }
    }

    #[test]
    fn two_groups_crossing_pass_through_each_other() {
        // Ten people at each end of a 16 m stretch, 0.7 m apart across it, walking to the other.
        let mut crowd = Crowd::new(CrowdSettings::default());
        let mut follows = Vec::new();
        for k in 0..20 {
            let z = (k % 10) as f32 * 0.7 - 3.15;
            let (from, to) = if k < 10 { (-8.0, 8.0) } else { (8.0, -8.0) };
            let (a, b) = (Vec3::new(from, 0.0, z), Vec3::new(to, 0.0, z));
            crowd.add(Agent::new(a, 0.3, 1.4));
            follows.push(Follow::new(&straight(a, b)));
        }
        let least = run(&mut crowd, &mut follows, 40.0, None);
        assert!(least > -0.02, "{least}");
        let left = follows.iter().filter(|f| !f.arrived()).count();
        assert_eq!(left, 0, "{:?}", crowd.agents);
    }

    #[test]
    fn pursuers_gather_round_their_target_without_overlapping() {
        // Sixteen on a circle 16 m across, all to its middle: they close round it, packed; those
        // on the rim jostle, one giving way to another now and then, within 2.5 m of it.
        let mut crowd = Crowd::new(CrowdSettings::default());
        let mut follows = Vec::new();
        for k in 0..16 {
            let (s, c) = (k as f32 / 16.0 * std::f32::consts::TAU).sin_cos();
            let a = Vec3::new(c * 8.0, 0.0, s * 8.0);
            crowd.add(Agent::new(a, 0.3, 1.4));
            follows.push(Follow::new(&straight(a, Vec3::ZERO)));
        }
        let least = run(&mut crowd, &mut follows, 20.0, None);
        assert!(least > -0.02, "{least}");
        for a in &crowd.agents {
            assert!(plan(a.position).length() < 2.5, "{a:?}");
        }
    }

    #[test]
    fn a_follower_waits_at_a_link_and_goes_on_once_it_is_crossed() {
        let p = |x: f32, z: f32| Vec3::new(x, 0.0, z);
        let path = Path {
            points: vec![p(0.0, 0.0), p(3.0, 0.0), p(4.0, 0.0), p(4.0, 3.0)],
            links: vec![(1, 5)],
            complete: true,
            expanded: 0,
        };
        let mut crowd = Crowd::new(CrowdSettings::default());
        crowd.add(Agent::new(p(0.0, 0.0), 0.3, 1.4));
        let mut follow = Follow::new(&path);
        let dt = 1.0 / 60.0;
        for _ in 0..600 {
            crowd.agents[0].preferred = follow.preferred(crowd.agents[0].position, 1.4, None);
            crowd.step(dt, None);
        }
        assert_eq!(follow.at_link(), Some(5));
        assert_eq!(follow.link_end(), Some(p(4.0, 0.0)));
        assert!(crowd.agents[0].position.distance(p(3.0, 0.0)) <= CORNER_REACH);
        // The game carries it over and says so.
        crowd.agents[0].position = p(4.0, 0.0);
        follow.crossed();
        assert_eq!(follow.at_link(), None);
        for _ in 0..600 {
            crowd.agents[0].preferred = follow.preferred(crowd.agents[0].position, 1.4, None);
            crowd.step(dt, None);
        }
        assert!(follow.arrived());
        assert!(crowd.agents[0].position.distance(p(4.0, 3.0)) <= CORNER_REACH);
    }

    #[test]
    fn the_crowd_keeps_to_the_field_and_its_paths_through_a_door() {
        let field = Field::build(desc(), PERSON, &walled(4.5, 5.4));
        // Pushed at the wall, slanting: it slides along it and never passes it.
        let mut crowd = Crowd::new(CrowdSettings::default());
        crowd.add(Agent::new(Vec3::new(3.0, 0.0, 1.0), 0.3, 1.4));
        for _ in 0..300 {
            crowd.agents[0].preferred = Vec2::new(1.0, 0.4);
            crowd.step(1.0 / 60.0, Some(&field));
            assert!(
                crowd.agents[0].position.x < 4.9,
                "{}",
                crowd.agents[0].position
            );
        }
        assert!(
            crowd.agents[0].position.z > 2.0,
            "{}",
            crowd.agents[0].position
        );
        // Two people through a 2 m opening from either side at once, on the field's paths. (In
        // a doorway too narrow for both, one must wait: ORCA alone does not choose who.)
        let field = Field::build(desc(), PERSON, &walled(4.0, 6.0));
        let (a, b) = (Vec3::new(2.0, 0.0, 2.0), Vec3::new(8.0, 0.0, 8.0));
        let mut crowd = Crowd::new(CrowdSettings::default());
        crowd.add(Agent::new(a, 0.3, 1.4));
        crowd.add(Agent::new(b, 0.3, 1.4));
        let mut follows = vec![
            Follow::new(&field.path(a, b, &[], 1 << 20).unwrap()),
            Follow::new(&field.path(b, a, &[], 1 << 20).unwrap()),
        ];
        let least = run(&mut crowd, &mut follows, 30.0, Some(&field));
        assert!(least > -0.02, "{least}");
        assert!(follows.iter().all(Follow::arrived), "{:?}", crowd.agents);
    }

    #[test]
    fn two_people_meeting_in_a_narrow_door_take_turns() {
        // A 0.9 m door, too narrow for two: they meet in it, stand a second, the second gives
        // way, and both get through.
        let field = Field::build(desc(), PERSON, &walled(4.5, 5.4));
        let (a, b) = (Vec3::new(2.0, 0.0, 4.95), Vec3::new(8.0, 0.0, 4.95));
        let mut crowd = Crowd::new(CrowdSettings::default());
        crowd.add(Agent::new(a, 0.3, 1.4));
        crowd.add(Agent::new(b, 0.3, 1.4));
        let mut follows = vec![
            Follow::new(&field.path(a, b, &[], 1 << 20).unwrap()),
            Follow::new(&field.path(b, a, &[], 1 << 20).unwrap()),
        ];
        let least = run(&mut crowd, &mut follows, 30.0, Some(&field));
        assert!(least > -0.02, "{least}");
        assert!(follows.iter().all(Follow::arrived), "{:?}", crowd.agents);
    }

    #[test]
    fn a_thousand_people_step_in_a_few_milliseconds() {
        // A thousand people on a 1.5 m grid, each walking to the grid's mirror point.
        let mut crowd = Crowd::new(CrowdSettings::default());
        for k in 0..1000 {
            let (i, j) = ((k % 40) as f32, (k / 40) as f32);
            let at = Vec3::new(i * 1.5, 0.0, j * 1.5);
            let mut agent = Agent::new(at, 0.3, 1.4);
            agent.preferred = (Vec2::new(30.0, 18.0) - plan(at)).normalize_or_zero() * 1.4;
            crowd.add(agent);
        }
        let mut best = std::time::Duration::MAX;
        for _ in 0..10 {
            let t = std::time::Instant::now();
            crowd.step(1.0 / 60.0, None);
            best = best.min(t.elapsed());
        }
        eprintln!("1000 people: a step in {best:?}");
        assert!(best.as_secs_f32() < 0.05, "{best:?}");
    }
}
