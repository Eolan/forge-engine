//! The smoke yard (D-064, #271): the particles' first effects in the lab, from the effects of
//! `crates/forge-render/data/particles.toml`.
//! - Three chimneys of stacked blocks, smoking.
//! - A ring of rocks round a fire: its embers rising, and its smoke.
//! - A walker's footsteps along the near side: a puff of dust a step, the walk starting over
//!   every eight seconds.

use forge_render::{Burst, Emitter, Particles};
use glam::{Mat4, Vec3};

use super::{BLOCK, BLOCK_HALF, ROCK_RADII, ROCKS};

/// The chimneys: where each stands (x, z) and how many blocks high.
const CHIMNEYS: [(f32, f32, u32); 3] = [(-4.0, -6.0, 4), (0.5, -8.5, 6), (4.5, -5.0, 3)];
/// The fire's middle on the floor, and its ring of rocks.
const FIRE: Vec3 = Vec3::new(-1.0, 0.0, -1.5);
const FIRE_RING: f32 = 0.95;
const FIRE_ROCKS: u32 = 9;
/// The walk: a step every `STEP` seconds from `WALK_START`, `STRIDE` metres apart along x from
/// `WALK_FROM`, the feet `FOOT` metres either side of z = `WALK_Z`, starting over after
/// `STEPS` steps.
const STEP: f32 = 0.5;
const WALK_START: f32 = 0.5;
const STRIDE: f32 = 0.7;
const WALK_FROM: f32 = -4.5;
const WALK_Z: f32 = 2.0;
const FOOT: f32 = 0.12;
const STEPS: u32 = 16;
/// The dust's longest life, seconds: a step's burst is passed on that long.
const DUST_LIFE: f32 = 2.0;

/// The yard's statics: the chimneys' blocks and the fire's rocks.
pub(super) fn build() -> Vec<(usize, Mat4)> {
    let mut statics = Vec::new();
    for (x, z, high) in CHIMNEYS {
        for level in 0..high {
            let y = BLOCK_HALF * (2 * level + 1) as f32;
            statics.push((BLOCK, Mat4::from_translation(Vec3::new(x, y, z))));
        }
    }
    let radius = ROCK_RADII[0];
    for k in 0..FIRE_ROCKS {
        let angle = std::f32::consts::TAU * k as f32 / FIRE_ROCKS as f32;
        let at = FIRE + Vec3::new(angle.cos(), 0.0, angle.sin()) * FIRE_RING;
        statics.push((
            ROCKS[0],
            Mat4::from_rotation_translation(
                glam::Quat::from_rotation_y(angle * 1.7),
                at + Vec3::Y * (0.5 * radius),
            ),
        ));
    }
    statics
}

/// The yard's emitters and the bursts alive at `time` (the scene's seconds).
pub(super) fn particles(
    time: f32,
    particles: &Particles,
    emitters: &mut Vec<Emitter>,
    bursts: &mut Vec<Burst>,
) {
    let (Some(smoke), Some(dust), Some(embers)) = (
        particles.effect("chimney-smoke"),
        particles.effect("footstep-dust"),
        particles.effect("embers"),
    ) else {
        return;
    };
    for (i, (x, z, high)) in CHIMNEYS.into_iter().enumerate() {
        emitters.push(Emitter {
            effect: smoke,
            position: Vec3::new(x, 2.0 * BLOCK_HALF * high as f32 + 0.05, z),
            direction: Vec3::Y,
            seed: 100 + i as u32,
            start: 0.0,
        });
    }
    emitters.push(Emitter {
        effect: embers,
        position: FIRE + Vec3::Y * 0.15,
        direction: Vec3::Y,
        seed: 200,
        start: 0.0,
    });
    emitters.push(Emitter {
        effect: smoke,
        position: FIRE + Vec3::Y * 0.3,
        direction: Vec3::Y,
        seed: 201,
        start: 0.0,
    });
    // The steps whose dust is still alive: from the one DUST_LIFE ago to now.
    let first = ((time - DUST_LIFE - WALK_START) / STEP).ceil().max(0.0) as u32;
    let last = ((time - WALK_START) / STEP).floor();
    if last < 0.0 {
        return;
    }
    for k in first..=last as u32 {
        let along = k % STEPS;
        let side = if k % 2 == 0 { -FOOT } else { FOOT };
        bursts.push(Burst {
            effect: dust,
            position: Vec3::new(WALK_FROM + STRIDE * along as f32, 0.02, WALK_Z + side),
            direction: Vec3::Y,
            // The walker's pace, a little of it carried into the dust.
            carry: Vec3::new(0.3 * STRIDE / STEP, 0.0, 0.0),
            seed: 1000 + k,
            time: WALK_START + STEP * k as f32,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_yard_stands_on_the_floor() {
        let statics = build();
        let blocks: u32 = CHIMNEYS.iter().map(|c| c.2).sum();
        assert_eq!(statics.len() as u32, blocks + FIRE_ROCKS);
        // The blocks stand on the floor and on one another: whole levels, the lowest on y = 0.
        let mut lowest = f32::MAX;
        for (prop, at) in &statics {
            let y = at.w_axis.y;
            if *prop == BLOCK {
                let level = (y - BLOCK_HALF) / (2.0 * BLOCK_HALF);
                assert!((level - level.round()).abs() < 1e-4, "{y}");
                lowest = lowest.min(y);
            }
        }
        assert!((lowest - BLOCK_HALF).abs() < 1e-6);
    }
}
