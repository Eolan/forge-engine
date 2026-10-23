//! A test mission on the village (#287's proof on the first map, D-062), written by the generator
//! from the first house with a stair: go up to a bedroom; a parcel then lies in the kitchen: take
//! it; then back out to the street before the front door. `--mission` plays it from that street,
//! `--mission-autopilot` walks it on the house's walk field (D-076). Its places are where a
//! walker stands nearest each room's middle, so the record holds wherever the house's furniture
//! falls.

use forge_mission::{
    Action, Condition, End, Item, ItemState, Mission, MissionEvent, Objective, Outcome, Place,
    Runtime, SCHEMA_VERSION, Shape, Status, Step,
};
use forge_nav::{Field, Follow};
use forge_procgen::city::geom;
use forge_procgen::city::interior::InRoom;
use forge_procgen::city::rooms::RoomKind;
use forge_procgen::city::{CityPlan, ground, interior};
use glam::{DVec2, DVec3, Vec3};

use super::{Mass, Structure, floor_level, house_field, structure, village_passages, world};

/// The mission's ticks a second: the walker's.
pub const TICK_RATE: u32 = 60;
/// The most ticks a frame runs; a longer frame's rest is dropped (the walker's own catch-up).
const CATCH_UP: u32 = 4;
/// The autopilot's pace, m/s: a walk.
const AUTOPILOT_SPEED: f32 = 1.4;
/// How near a room's place the feet count as in it, metres.
const PLACE_RADIUS: f64 = 1.0;
/// How far out on the street before the front door the mission starts and ends, metres.
const STREET_OUT: f64 = 3.0;
/// The A* search's budget of nodes (the house's field, 0.1 m cells).
const SEARCH_BUDGET: u32 = 400_000;
/// How fast the autopilot turns the view towards where it walks, radians a second.
const TURN_RATE: f32 = 3.0;
/// The autopilot's view below the horizon, radians: a parcel on the floor stays in sight.
const AUTOPILOT_PITCH: f32 = -0.25;

/// The mission written for a village and what plays it.
pub struct VillageMission {
    /// The record.
    pub mission: Mission,
    /// The house's walk field, for the autopilot.
    pub field: Field,
    /// Where it starts and ends: on the street before the front door, the world's metres.
    pub start: Vec3,
    /// From there towards the door, along the ground.
    pub facing: Vec3,
    /// The house: its index among the masses.
    pub house: usize,
    /// The house's lot.
    pub lot: u32,
}

/// The test mission for the village of `plan` and `masses` (a side of `size` metres): the first
/// house with a stair, a front door, a bedroom upstairs and a kitchen (or a living room)
/// downstairs, all reached from its street on its walk field.
pub fn village(size: f64, plan: &CityPlan, masses: &[Mass]) -> Option<VillageMission> {
    let structures = structure::structures(plan, masses);
    let passages = forge_nav::Passages::new(village_passages(size, plan, masses, &structures));
    for (i, (m, s)) in masses.iter().zip(&structures).enumerate() {
        let Some(inner) = interior::house(m, s).filter(|h| h.stair.is_some()) else {
            continue;
        };
        // Its front door, the core's (as the map check's views take it).
        let Some(door) = s
            .cores
            .iter()
            .find(|c| c.house)
            .and_then(|c| s.openings.get(c.door))
        else {
            continue;
        };
        let bedroom = inner
            .rooms
            .iter()
            .find(|r| r.storey > 0 && r.kind == RoomKind::Bedroom);
        let kitchen = [RoomKind::Kitchen, RoomKind::Living]
            .iter()
            .find_map(|&k| inner.rooms.iter().find(|r| r.storey == 0 && r.kind == k));
        let (Some(bedroom), Some(kitchen)) = (bedroom, kitchen) else {
            continue;
        };
        let w = &s.walls[door.wall];
        let t = (w.b - w.a).normalize_or_zero();
        let inward = DVec2::new(-t.y, t.x);
        let at = w.a + t * door.along;
        let field = house_field(size, masses, &structures, &passages, i, false);
        let Some(from) = field.place_near(
            world(size, at - inward * STREET_OUT, ground::WALK_LEVEL),
            1.0,
        ) else {
            continue;
        };
        let seen = field.reach(from);
        let (Some(up), Some(parcel)) = (
            stand(&field, &seen, size, s, bedroom),
            stand(&field, &seen, size, s, kitchen),
        ) else {
            continue;
        };
        let start = field.point(from);
        let facing = world(size, at, 0.0) - world(size, at - inward, 0.0);
        let sphere = |p: Vec3| Shape::Sphere {
            centre: p.as_dvec3().to_array(),
            radius: PLACE_RADIUS,
        };
        let objective = |id: &str, text: &str| Objective {
            id: id.to_owned(),
            text: text.to_owned(),
        };
        let wait = |id: &str, until: Condition, then: Vec<Action>| Step {
            id: Some(id.to_owned()),
            objective: Some(id.to_owned()),
            until: Some(until),
            then,
            ..Step::default()
        };
        let mission = Mission {
            id: "village:parcel".to_owned(),
            version: SCHEMA_VERSION,
            objectives: vec![
                objective("upstairs", "Go up to the bedroom"),
                objective("parcel", "Take the parcel in the kitchen"),
                objective("out", "Back out to the street"),
            ],
            places: vec![
                Place {
                    id: "bedroom".to_owned(),
                    shape: sphere(up),
                },
                Place {
                    id: "street".to_owned(),
                    shape: sphere(start),
                },
            ],
            items: vec![Item {
                id: "parcel".to_owned(),
                at: parcel.as_dvec3().to_array(),
                reach: 1.0,
            }],
            vars: Default::default(),
            steps: Step {
                id: Some("main".to_owned()),
                seq: Some(vec![
                    wait(
                        "upstairs",
                        Condition::Inside("bedroom".to_owned()),
                        vec![Action::Spawn("parcel".to_owned())],
                    ),
                    wait("parcel", Condition::Holds("parcel".to_owned()), Vec::new()),
                    wait(
                        "out",
                        Condition::Inside("street".to_owned()),
                        vec![Action::End(End::Succeeded)],
                    ),
                ]),
                ..Step::default()
            },
            renamed: Default::default(),
        };
        return Some(VillageMission {
            mission,
            field,
            start,
            facing,
            house: i,
            lot: m.lot,
        });
    }
    None
}

/// Where a walker stands in `room` nearest its middle, among the places `seen` from the street
/// at the room's floor level: a bed or a table may fill the middle itself.
fn stand(field: &Field, seen: &[bool], size: f64, s: &Structure, room: &InRoom) -> Option<Vec3> {
    let level = floor_level(s, room.storey) as f32;
    let middle = world(size, geom::centroid(&room.outline), f64::from(level));
    let ring: Vec<DVec2> = room
        .outline
        .iter()
        .map(|&p| {
            let w = world(size, p, 0.0);
            DVec2::new(f64::from(w.x), f64::from(w.z))
        })
        .collect();
    let (lo, hi) = ring.iter().fold(
        (DVec2::splat(f64::MAX), DVec2::splat(f64::MIN)),
        |(lo, hi), &p| (lo.min(p), hi.max(p)),
    );
    let d = field.desc();
    let (i0, j0) = d.column(lo.as_vec2())?;
    let (i1, j1) = d.column(hi.as_vec2())?;
    (j0..=j1)
        .flat_map(|j| (i0..=i1).map(move |i| (i, j)))
        .filter(|&(i, j)| geom::contains(&ring, d.centre(i, j).as_dvec2()))
        .flat_map(|(i, j)| field.column(i, j))
        .filter(|&q| {
            seen[q as usize]
                && field.fits(q)
                && (field.spans()[q as usize].floor - level).abs() < 0.2
        })
        .map(|q| field.point(q))
        .min_by(|a, b| {
            a.distance(middle)
                .total_cmp(&b.distance(middle))
                .then(a.x.total_cmp(&b.x))
                .then(a.z.total_cmp(&b.z))
        })
}

/// The mission under way: its runtime ticked with the walker's feet, and the autopilot's way.
pub struct MissionRun {
    runtime: Runtime,
    field: Field,
    /// Whether the autopilot walks it (`--mission-autopilot`).
    autopilot: bool,
    /// The autopilot's target and the path it follows there.
    way: Option<(DVec3, Follow)>,
    tick: u64,
    /// Seconds not ticked yet.
    owed: f64,
    /// The autopilot's view: its yaw, turned towards where it walks.
    yaw: f32,
    /// The evaluator's ticks: their count, their time and the longest, microseconds.
    cost: (u64, f64, f64),
    lot: u32,
}

impl MissionRun {
    /// `m` at its start.
    pub fn new(m: VillageMission, autopilot: bool) -> anyhow::Result<Self> {
        let yaw = (-m.facing.x).atan2(-m.facing.z);
        let runtime = Runtime::new(m.mission, TICK_RATE)?;
        tracing::info!(
            lot = m.lot,
            objectives = runtime.mission().objectives.len(),
            autopilot,
            "the village's mission"
        );
        Ok(Self {
            runtime,
            field: m.field,
            autopilot,
            way: None,
            tick: 0,
            owed: 0.0,
            yaw,
            cost: (0, 0.0, 0.0),
            lot: m.lot,
        })
    }

    /// The autopilot's walk from `feet` (m/s along the ground, the world's x and z) towards the
    /// mission's first target on the house's field, or `None` when the player walks.
    pub fn walk(&mut self, feet: Vec3) -> Option<[f32; 2]> {
        if !self.autopilot {
            return None;
        }
        let Some(&target) = self.runtime.targets().first() else {
            return Some([0.0; 2]);
        };
        if self.way.as_ref().is_none_or(|(t, _)| *t != target) {
            let path = self.field.path(feet, target.as_vec3(), &[], SEARCH_BUDGET);
            if path.as_ref().is_none_or(|p| !p.complete) {
                tracing::warn!(?target, "the mission's autopilot: no full path");
            }
            self.way = path.map(|p| (target, Follow::new(&p)));
        }
        let (_, follow) = self.way.as_mut()?;
        let v = follow.preferred(feet, AUTOPILOT_SPEED, Some(&self.field));
        Some(v.to_array())
    }

    /// Turns the autopilot's view towards `wish` (its walk, m/s) for `dt` seconds.
    pub fn steer(&mut self, wish: [f32; 2], dt: f32) {
        if !self.autopilot || wish[0].hypot(wish[1]) < 0.2 {
            return;
        }
        let to = (-wish[0]).atan2(-wish[1]);
        let turn = (to - self.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
            - std::f32::consts::PI;
        self.yaw += turn.clamp(-TURN_RATE * dt, TURN_RATE * dt);
    }

    /// The autopilot's view, its yaw and pitch, while it walks the mission.
    pub fn view(&self) -> Option<(f32, f32)> {
        self.autopilot.then_some((self.yaw, AUTOPILOT_PITCH))
    }

    /// Runs the ticks `dt` seconds owe with the player's feet at `feet` (the world's metres),
    /// and logs what they changed.
    pub fn advance(&mut self, feet: Vec3, dt: f64) {
        let tick = 1.0 / f64::from(TICK_RATE);
        self.owed += dt;
        let mut ran = 0;
        while self.owed >= tick && self.runtime.outcome() == Outcome::Running {
            self.owed -= tick;
            ran += 1;
            if ran > CATCH_UP {
                self.owed = 0.0;
                break;
            }
            self.tick += 1;
            let started = std::time::Instant::now();
            self.runtime.tick(self.tick, feet.as_dvec3());
            let us = started.elapsed().as_secs_f64() * 1e6;
            self.cost = (self.cost.0 + 1, self.cost.1 + us, self.cost.2.max(us));
            for event in self.runtime.events() {
                tracing::info!(tick = self.tick, ?event, "mission");
                if matches!(event, MissionEvent::Succeeded | MissionEvent::Failed) {
                    let (n, sum, most) = self.cost;
                    tracing::info!(
                        lot = self.lot,
                        tick = self.tick(),
                        digest = format!("{:016x}", self.digest()),
                        mean_us = format!("{:.2}", sum / n.max(1) as f64),
                        max_us = format!("{most:.1}"),
                        "the village's mission ended: {:?}",
                        self.runtime.outcome()
                    );
                }
            }
        }
    }

    /// Where the parcel lies (the world's metres), while it lies there.
    pub fn parcel(&self) -> Option<Vec3> {
        self.runtime
            .items()
            .find(|(_, state)| *state == ItemState::Present)
            .map(|(item, _)| DVec3::from(item.at).as_vec3())
    }

    /// What the player is asked now, or how the mission ended.
    pub fn line(&self) -> String {
        match self.outcome() {
            Outcome::Succeeded => "mission complete".to_owned(),
            Outcome::Failed => "mission failed".to_owned(),
            Outcome::Running => self
                .runtime
                .objectives()
                .find(|(_, status)| *status == Status::Active)
                .map_or_else(String::new, |(o, _)| o.text.clone()),
        }
    }

    /// Whether it ended, and how.
    pub fn outcome(&self) -> Outcome {
        self.runtime.outcome()
    }

    /// The ticks it ran.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Its state's digest (`Runtime::digest`).
    pub fn digest(&self) -> u64 {
        self.runtime.digest()
    }
}

/// The parcel: a cardboard box 40 × 25 × 30 cm on the origin, its bottom on the ground.
pub fn parcel_mesh() -> forge_geom::procedural::TriMesh {
    let mut m = forge_geom::procedural::TriMesh::default();
    let (hx, hy, hz) = (0.2_f32, 0.25_f32, 0.15_f32);
    // Each face: its normal and its two in-plane axes, so its corners wind counter-clockwise.
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (Vec3::NEG_X, Vec3::Z, Vec3::Y),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (Vec3::NEG_Y, Vec3::X, Vec3::Z),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (Vec3::NEG_Z, Vec3::Y, Vec3::X),
    ];
    let half = Vec3::new(hx, hy * 0.5, hz);
    let centre = Vec3::new(0.0, hy * 0.5, 0.0);
    for (n, u, v) in faces {
        let base = m.positions.len() as u32;
        for (a, b) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = centre + (n + u * a + v * b) * half;
            m.positions.push(p.to_array());
            m.normals.push(n.to_array());
        }
        m.indices
            .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    m
}
