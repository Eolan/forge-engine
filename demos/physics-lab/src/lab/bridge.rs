//! `physics-lab --lab bridge` (issue #147): one of Phase 3's advanced tests, a bridge collapsing
//! under a convoy. A timber deck spans a gap of 16 m between two banks 4 m high, its panels held
//! to each other and its ends to the banks by joints that break as the wall's mortar does
//! (`bonds`). Four cars wait on the near bank, held until Space (or `--release N`) lets them go;
//! then they drive across on an autopilot that keeps them on the middle line at a steady pace and
//! stops them on the far bank. The deck holds itself, and gives way under the convoy. A recording
//! of the run replays to the same digests.

use anyhow::Result;
use forge_geom::city::{Block, PropKind, PropSpec};
use forge_physics::{BodyDesc, BodyId, JointId, Shape, Transform, VehicleId, World};
use forge_sim::TICK;
use glam::{DVec3, Mat4, Vec3};

use super::bonds::{Bonds, Limits};
use super::drive;

/// The gap's half width along z, metres, and the banks either side: their half sizes, their top
/// at 4 m.
const GAP_HALF: f64 = 8.0;
const BANK_HALF: [f32; 3] = [6.0, 2.0, 20.0];
/// The deck: this many panels across the gap, each the gap's share long, 4.4 m wide and 12 cm
/// thick, of timber (kg/m³); its top level with the banks'.
const PANELS: u32 = 16;
const PANEL_HALF: [f32; 3] = [2.2, 0.06, 0.5];
const TIMBER: f32 = 600.0;
/// What the deck's joints hold and how far they give before they break.
const JOINTS: Limits = Limits {
    force: 120_000.0,
    torque: 78_000.0,
    stretch: 0.012,
    twist: 0.011,
};
/// The solver's velocity and position iterations over the deck: enough for a span of joints to
/// stand rigid.
const DECK_STEPS: (u32, u32) = (40, 10);
/// Ticks the deck settles for before the cars come.
const SETTLE: u32 = 300;
/// The convoy: this many cars on the near bank, this far apart, the first this far from the gap.
const CARS: u32 = 4;
const CAR_SPACING: f64 = 7.0;
const FIRST_CAR: f64 = 6.0;
/// The autopilot: the pace, m/s, and the line along the far bank where the cars stop.
const CRUISE: f32 = 4.0;
const STOP_Z: f32 = -26.0;
/// A car is down when its body is this low, metres: off the deck, in the gap.
const DOWN_Y: f64 = 2.5;

/// The scene's props: a bank and a deck panel.
pub(super) fn props() -> Vec<PropSpec> {
    let block = |name: &str, half: [f32; 3], radius: f32| PropSpec {
        name: name.to_owned(),
        kind: PropKind::Block(Block {
            half,
            radius,
            segments: 4,
        }),
    };
    vec![
        block("lab-bank", BANK_HALF, 0.1),
        block("lab-bridge-panel", PANEL_HALF, 0.01),
    ]
}

/// What the scene puts in the world.
pub(super) struct Site {
    /// The banks, drawn: (prop, transform).
    pub statics: Vec<(usize, Mat4)>,
    pub panels: Vec<BodyId>,
    pub convoy: Convoy,
}

/// The deck's joints, and the cars with what holds them until they go.
#[derive(Clone, Debug)]
pub(super) struct Convoy {
    pub deck: Bonds,
    pub cars: Vec<(BodyId, VehicleId)>,
    holds: Vec<JointId>,
}

/// Builds the scene into `world`: the banks drawn with `bank`, the deck, the cars held.
pub(super) fn build(world: &mut World, bank: usize) -> Result<Site> {
    let bank_shape = Shape::cuboid(Vec3::from_array(BANK_HALF), 0.05, 0.0)?;
    let mut statics = Vec::new();
    for side in [1.0, -1.0] {
        let at = DVec3::new(
            0.0,
            f64::from(BANK_HALF[1]),
            side * (GAP_HALF + f64::from(BANK_HALF[2])),
        );
        world.add_body(&BodyDesc {
            friction: 0.8,
            ..BodyDesc::fixed(&bank_shape, at)
        })?;
        statics.push((bank, Mat4::from_translation(at.as_vec3())));
    }
    // The deck's panels from the near bank across, each joined to the one before it, the first
    // and the last to the banks.
    let top = 2.0 * f64::from(BANK_HALF[1]);
    let length = 2.0 * GAP_HALF / f64::from(PANELS);
    let shape = Shape::cuboid(Vec3::from_array(PANEL_HALF), 0.01, TIMBER)?;
    let (mut panels, mut laid) = (Vec::new(), Vec::new());
    for k in 0..PANELS {
        let at = DVec3::new(
            0.0,
            top - f64::from(PANEL_HALF[1]),
            GAP_HALF - length * (f64::from(k) + 0.5),
        );
        panels.push(world.add_body(&BodyDesc {
            friction: 0.8,
            ..BodyDesc::dynamic(&shape, at)
        })?);
        laid.push(at);
    }
    let mut deck = Bonds::new(panels.clone(), laid, JOINTS);
    deck.join(world, None, 0, DECK_STEPS);
    for k in 1..PANELS {
        deck.join(world, Some(k - 1), k, DECK_STEPS);
    }
    deck.join(world, None, PANELS - 1, DECK_STEPS);
    // The deck settles under its own weight before the scene starts: laid without its load, its
    // joints take a couple of seconds to carry it, swinging past their load at rest on the way.
    for _ in 0..SETTLE {
        world.step(TICK, 1)?;
    }
    // The cars in a line on the near bank, facing the gap, each held where it stands.
    let mut cars = Vec::new();
    let mut holds = Vec::new();
    for n in 0..CARS {
        let z = GAP_HALF + FIRST_CAR + CAR_SPACING * f64::from(n);
        let car = drive::car(world, DVec3::new(0.0, top + 0.15, z))?;
        holds.push(world.join_fixed(None, car.0, (0, 0)));
        cars.push(car);
    }
    Ok(Site {
        statics,
        panels,
        convoy: Convoy { deck, cars, holds },
    })
}

impl Convoy {
    /// Lets the cars go.
    pub(super) fn release(&self, world: &mut World) {
        world.set_holding(&self.holds, false);
    }

    /// Whether the cars are still held.
    pub(super) fn held(&self, world: &World) -> bool {
        let mut holding = Vec::new();
        world.holding(&self.holds, &mut holding);
        holding.iter().any(|&h| h)
    }

    /// Hands each car its controls for the coming step: held, the handbrake; let go, the
    /// autopilot: steer back to the middle line (against its offset and its heading), keep the
    /// pace, and brake to a stop past the line on the far bank, or at once behind a car that
    /// went down.
    pub(super) fn tick(&self, world: &mut World) {
        let held = self.held(world);
        let bodies: Vec<BodyId> = self.cars.iter().map(|c| c.0).collect();
        let (mut t, mut v) = (Vec::new(), Vec::new());
        world.transforms(&bodies, &mut t);
        world.velocities(&bodies, &mut v);
        // Behind a car that went down, a driver stops where they are.
        let mut ahead_down = false;
        for ((&(_, vehicle), t), v) in self.cars.iter().zip(&t).zip(&v) {
            if held {
                world.drive(vehicle, 0.0, 0.0, 0.0, 1.0);
                continue;
            }
            let forward = t.rotation * Vec3::NEG_Z;
            let ahead = v.linear.dot(forward);
            let x = t.position.x as f32;
            let steer = (-0.3 * x - 1.5 * forward.x).clamp(-1.0, 1.0);
            if ahead_down {
                world.drive(vehicle, 0.0, steer, 1.0, 1.0);
            } else if (t.position.z as f32) < STOP_Z {
                world.drive(vehicle, 0.0, steer, 1.0, 0.0);
            } else {
                let throttle = (0.3 * (CRUISE - ahead)).clamp(0.0, 0.6);
                world.drive(vehicle, throttle, steer, 0.0, 0.0);
            }
            ahead_down |= t.position.y < DOWN_Y;
        }
    }

    /// The cars' wheels for the movers, after the bodies, car by car.
    pub(super) fn wheels(&self, world: &World, out: &mut Vec<Transform>) {
        let mut wheels = Vec::new();
        for &(_, vehicle) in &self.cars {
            world.wheels(vehicle, &mut wheels);
            out.extend(&wheels);
        }
    }

    /// The cars down in the gap.
    pub(super) fn down(&self, world: &World) -> usize {
        let bodies: Vec<BodyId> = self.cars.iter().map(|c| c.0).collect();
        let mut t = Vec::new();
        world.transforms(&bodies, &mut t);
        t.iter().filter(|t| t.position.y < DOWN_Y).count()
    }

    /// The cars across, past the far bank's edge.
    pub(super) fn across(&self, world: &World) -> usize {
        let bodies: Vec<BodyId> = self.cars.iter().map(|c| c.0).collect();
        let mut t = Vec::new();
        world.transforms(&bodies, &mut t);
        t.iter()
            .filter(|t| t.position.z < -GAP_HALF - 3.0 && t.position.y > DOWN_Y)
            .count()
    }
}
