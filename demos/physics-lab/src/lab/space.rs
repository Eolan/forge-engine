//! `physics-lab --lab space` (issue #150, Phase 3's step 5): a spaceship in zero g, no gravity and
//! no air, in orbit over a planet under the stars (the owner's ask of 2026-10-03: the space scene
//! looking like space, the rocket a sci-fi ship; [`super::ship`]). Its engines push it along its
//! nose, its jets turn it on the stick, and ahead of it float 27 crates in a block. Nothing damps
//! its flight, so what it tests is that the solver keeps momentum: the ship gains what its engines
//! give it and no more, and the crates it scatters carry away what it loses.

use anyhow::Result;
use forge_physics::{BodyDesc, BodyId, Shape, World};
use glam::{DVec3, Quat, Vec3};

use super::ship;

/// Where everything floats (the scene's frame: no floor in space), metres, where the ship's
/// middle starts along z, and the crates' block: its middle, the crates a side and how far apart;
/// each crate's mass, kg.
pub(super) const HEIGHT: f64 = 30.0;
const SHIP_Z: f64 = 4.0;
const BLOCK_AT: DVec3 = DVec3::new(0.0, HEIGHT, -30.0);
const SIDE: i32 = 3;
const PITCH: f64 = 1.2;
const CRATE_MASS: f32 = 100.0;
/// The crates, after the ship among the scene's bodies.
pub(super) const CRATES: usize = (SIDE * SIDE * SIDE) as usize;

/// What the scene puts in the world.
pub(super) struct Site {
    pub ship: BodyId,
    pub crates: Vec<BodyId>,
}

/// Builds the scene into `world` (its gravity none): the ship nose first along −z, and the crates
/// (of `crate_shape`) in their block ahead of it, nothing damped.
pub(super) fn build(world: &mut World, crate_shape: &Shape) -> Result<Site> {
    let ship = ship::add(world, DVec3::new(0.0, HEIGHT, SHIP_Z), Quat::IDENTITY)?;
    let mut crates = Vec::new();
    for i in -1..SIDE - 1 {
        for j in -1..SIDE - 1 {
            for k in -1..SIDE - 1 {
                let at = BLOCK_AT + DVec3::new(f64::from(i), f64::from(j), f64::from(k)) * PITCH;
                crates.push(world.add_body(&BodyDesc {
                    mass: Some(CRATE_MASS),
                    friction: 0.5,
                    restitution: 0.3,
                    linear_damping: 0.0,
                    angular_damping: 0.0,
                    ..BodyDesc::dynamic(crate_shape, at)
                })?);
            }
        }
    }
    Ok(Site { ship, crates })
}

/// The momentum of the ship and the crates, kg·m/s.
pub(super) fn momentum(world: &World, ship: BodyId, crates: &[BodyId]) -> Vec3 {
    let mut v = Vec::new();
    world.velocities(&[ship], &mut v);
    let flying = v[0].linear * ship::MASS;
    world.velocities(crates, &mut v);
    v.iter().fold(flying, |p, v| p + v.linear * CRATE_MASS)
}
