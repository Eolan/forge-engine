//! The smoke yard (D-064, #271): the particles' first effects in the lab, from the effects of
//! `crates/forge-render/data/particles.toml`.
//! - Three chimneys of stacked blocks, smoking.
//! - A ring of rocks round a fire: its flames (the baked fire), its embers rising, its smoke, and
//!   its light, a lamp (D-065) of the flames' size whose rocks cast wide penumbrae on the floor
//!   (the lamps' penumbra filter's test).
//! - A walker's footsteps along the near side: a puff of dust a step, the walk starting over
//!   every eight seconds.
//! - A grinder's sparks, streaking down onto the floor and bouncing off it and a chimney's foot
//!   (the particles that meet the scene).

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
/// Where a grinder's sparks fly from.
const GRINDER: Vec3 = Vec3::new(3.2, 1.1, -2.6);

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

/// The fire's light at `time` (the scene's seconds), in the world frame (D-065): a lamp in the
/// flames, 0.8 m over the floor, as large as they are (12 cm), at 1 900 K (a wood fire) and about
/// 150 cd, rising and falling by up to an eighth a few times a second. Its rocks' shadows on the
/// floor have penumbrae half a metre wide.
pub(super) fn lamps(time: f32) -> Vec<forge_render::lamps::Lamp> {
    use std::f32::consts::TAU;
    let flicker = 0.6 * (TAU * 0.7 * time).sin()
        + 0.3 * (TAU * 1.7 * time + 1.3).sin()
        + 0.1 * (TAU * 3.1 * time + 2.1).sin();
    vec![forge_render::lamps::Lamp {
        position: FIRE + Vec3::Y * 0.8,
        radius: 0.12,
        candela: 150.0 * (1.0 + 0.12 * flicker),
        kelvin: 1900.0,
        tint: Vec3::ONE,
        regions: forge_render::lamps::EVERY_REGION,
        emission: forge_render::lamps::Emission::Every,
    }]
}

/// The yard's emitters and the bursts alive at `time` (the scene's seconds).
pub(super) fn particles(
    time: f32,
    particles: &Particles,
    emitters: &mut Vec<Emitter>,
    bursts: &mut Vec<Burst>,
) {
    let (Some(smoke), Some(dust), Some(embers), Some(sparks), Some(fire)) = (
        particles.effect("chimney-smoke"),
        particles.effect("footstep-dust"),
        particles.effect("embers"),
        particles.effect("sparks"),
        particles.effect("fire"),
    ) else {
        return;
    };
    // The fire's flames in its ring, under its smoke and embers.
    emitters.push(Emitter {
        effect: fire,
        position: FIRE + Vec3::Y * 0.1,
        direction: Vec3::Y,
        seed: 202,
        start: 0.0,
    });
    // A grinder's sparks, down onto the floor before the right chimney: they bounce off the
    // floor and the chimney's foot.
    emitters.push(Emitter {
        effect: sparks,
        position: GRINDER,
        direction: Vec3::new(1.0, -1.1, -1.6).normalize(),
        seed: 300,
        start: 0.0,
    });
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
