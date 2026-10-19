//! Pursuers (#317, D-076's proof as a scene, `--pursuers`): four people on the street before the
//! first house with insides go for its hall on the house's walk field (`forge_nav`):
//! - they follow their paths and keep clear of one another (ORCA);
//! - they go in by the door, or over a window's sill as a carried crossing;
//! - a window is taken one at a time, its far side clear, and they land where the path goes on.

use forge_nav::{
    Agent, CrossingMotion, CrossingSpec, Crowd, CrowdSettings, Field, Follow, Link, Passage,
    PassageKind,
};
use glam::{Vec2, Vec3};

/// How many at most.
pub const COUNT: usize = 4;
/// Their simulation's tick, seconds.
const TICK: f64 = 1.0 / 60.0;
/// The most ticks a frame runs; a longer frame's rest is dropped.
const CATCH_UP: u32 = 4;

/// The pursuers and the house they go for.
pub struct Pursuers {
    field: Field,
    /// The house's ground-floor windows, in their links' order.
    windows: Vec<Passage>,
    crowd: Crowd,
    follows: Vec<Follow>,
    /// Each one carried over a sill: its crossing, the seconds into it and the window.
    carried: Vec<Option<(CrossingMotion, f32, usize)>>,
    /// Each one's way it faces (x and z, unit).
    facing: Vec<Vec2>,
    /// Seconds not yet ticked.
    owed: f64,
    /// The windows climbed through so far.
    pub crossings: usize,
}

impl Pursuers {
    /// Up to four on the street 3 m before `door`, `spread` metres apart along it, going for the
    /// point 1.5 m inside it on `field`, the house's ground-floor `windows` their climbers' links
    /// (two seconds each): those that stand on the field and have a way in. None when none has.
    pub fn new(field: Field, door: &Passage, windows: Vec<Passage>, spread: f32) -> Option<Self> {
        let hall = door.middle() + door.across * 1.5 + Vec3::Y * door.drop;
        let links: Vec<Link> = windows
            .iter()
            .map(|p| p.link(2.0, PassageKind::Window as u16, 0.5, true))
            .collect();
        let along = (door.ends[1] - door.ends[0]).normalize_or_zero();
        let mut crowd = Crowd::new(CrowdSettings::default());
        let mut follows = Vec::new();
        for k in 0..COUNT {
            let at = door.middle() - door.across * 3.0 + along * ((k as f32 - 1.5) * spread);
            let Some(path) = field.path(at, hall, &links, 400_000).filter(|p| p.complete) else {
                continue;
            };
            crowd.add(Agent::new(path.points[0], 0.3, 1.4));
            follows.push(Follow::new(&path));
        }
        if follows.is_empty() {
            return None;
        }
        let n = follows.len();
        let towards = Vec2::new(door.across.x, door.across.z);
        Some(Self {
            field,
            windows,
            crowd,
            follows,
            carried: vec![None; n],
            facing: vec![towards; n],
            owed: 0.0,
            crossings: 0,
        })
    }

    /// Runs the ticks `dt` seconds owe, at most four.
    pub fn advance(&mut self, dt: f64) {
        self.owed += dt;
        let due = (self.owed / TICK).floor();
        self.owed -= due * TICK;
        for _ in 0..(due as u32).min(CATCH_UP) {
            self.tick();
        }
    }

    /// One tick: each one's way chosen, the crowd stepped, the carried moved along their sills.
    fn tick(&mut self) {
        let dt = TICK as f32;
        for k in 0..self.follows.len() {
            if self.carried[k].is_none()
                && let Some(link) = self.follows[k].at_link()
            {
                let p = &self.windows[link];
                let spec = CrossingSpec::person(p.crossing());
                let feet = self.crowd.agents[k].position;
                // Landing where its path goes on, a place on the field.
                let motion = CrossingMotion::over(p, feet, 0.3, spec).map(|m| {
                    match self.follows[k].link_end() {
                        Some(end) => m.landing(end),
                        None => m,
                    }
                });
                // A window taken one at a time, its far side clear.
                let carried = &self.carried;
                let agents = &self.crowd.agents;
                let free = |m: &CrossingMotion| {
                    !carried.iter().flatten().any(|c| c.2 == link)
                        && agents
                            .iter()
                            .enumerate()
                            .all(|(j, a)| j == k || a.position.distance(m.exit) > 0.6)
                };
                if let Some(m) = motion.filter(|m| free(m)) {
                    self.facing[k] = Vec2::new(m.ahead.x, m.ahead.z);
                    self.carried[k] = Some((m, 0.0, link));
                }
            }
            let agent = &mut self.crowd.agents[k];
            agent.preferred = if self.carried[k].is_some() || self.follows[k].at_link().is_some() {
                Vec2::ZERO
            } else {
                self.follows[k].preferred(agent.position, agent.max_speed, Some(&self.field))
            };
        }
        self.crowd.step(dt, Some(&self.field));
        for k in 0..self.follows.len() {
            let v = self.crowd.agents[k].velocity;
            if v.length_squared() > 0.01 {
                self.facing[k] = v.normalize();
            }
            // The carried go where their crossing has them, whatever the crowd did.
            let Some((m, t, _)) = &mut self.carried[k] else {
                continue;
            };
            *t += dt;
            self.crowd.agents[k].position = m.at(*t);
            self.crowd.agents[k].velocity = Vec2::ZERO;
            if m.done(*t) {
                // Landed: on the field's place nearest the exit the walker fits.
                let exit = m.exit;
                self.crowd.agents[k].position = self
                    .field
                    .place_near(exit, 1.0)
                    .map_or(exit, |s| self.field.point(s));
                self.carried[k] = None;
                self.follows[k].crossed();
                self.crossings += 1;
            }
        }
    }

    /// Each one's feet, metres.
    pub fn feet(&self) -> Vec<Vec3> {
        self.crowd.agents.iter().map(|a| a.position).collect()
    }

    /// Each one's way it faces (x and z, unit).
    pub fn facing(&self) -> &[Vec2] {
        &self.facing
    }

    /// Whether each is being carried over a sill.
    #[cfg(test)]
    pub fn climbing(&self) -> Vec<bool> {
        self.carried.iter().map(Option::is_some).collect()
    }
}
