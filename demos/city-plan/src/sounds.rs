//! The village's own sounds (#318): the pursuers' footsteps (`--pursuers`), muffled by the
//! walls and floors between each step and the listener, and at most [`STEPS_CAP`] of them
//! mixed at once (a family of voices, the mixer's cap per family).

use forge_app::sound::Sound;
use forge_audio::{AcousticMaterial, Control, Foley, Occlusion, PlayParams, Step, Surface};
use glam::Vec3;

/// The pursuers' footsteps' family of voices.
pub const FAMILY_STEPS: u8 = 1;

/// The most of their steps mixed at once: the nearest win.
const STEPS_CAP: u16 = 6;

/// Metres between two steps of one pursuer.
const STRIDE: f32 = 0.75;

/// The walls a sound passes through that still count: past three it is gone anyway.
const MOST_LAYERS: usize = 3;

/// The pursuers' footsteps.
pub struct Steps {
    foley: Foley,
    last: Vec<Vec3>,
    walked: Vec<f32>,
}

impl Steps {
    /// Their foley on the app's foley bus, and their family's cap.
    pub fn new(sound: &mut Sound) -> Self {
        sound.engine.set_family_cap(FAMILY_STEPS, STEPS_CAP);
        let mut params = PlayParams {
            bus: sound.buses.foley,
            family: FAMILY_STEPS,
            ..PlayParams::default()
        };
        params.sends[0] = Some((sound.buses.reverb, 0.2));
        Self {
            foley: Foley::new(sound.sample_rate(), 3181, params),
            last: Vec::new(),
            walked: Vec::new(),
        }
    }

    /// A step for each pursuer that walked a stride since its last, on `surface_at` its feet,
    /// muffled by the `between` layers (walls, floors) from it to `listener`.
    pub fn update(
        &mut self,
        sound: &mut Sound,
        feet: &[Vec3],
        listener: Vec3,
        surface_at: impl Fn(Vec3) -> Surface,
        between: impl Fn(Vec3, Vec3) -> usize,
    ) {
        if self.last.len() != feet.len() {
            self.last = feet.to_vec();
            self.walked = vec![0.0; feet.len()];
            return;
        }
        for (k, &at) in feet.iter().enumerate() {
            let d = at - self.last[k];
            self.last[k] = at;
            let moved = Vec3::new(d.x, 0.0, d.z).length();
            // A pursuer carried over a sill or set down elsewhere: no step.
            if moved > 1.0 {
                self.walked[k] = 0.0;
                continue;
            }
            self.walked[k] += moved;
            if self.walked[k] < STRIDE {
                continue;
            }
            self.walked[k] -= STRIDE;
            let mut step = Step::walk(surface_at(at), at.as_dvec3());
            step.footwear = forge_audio::Footwear::Boots;
            let voice = self.foley.step(&mut sound.engine, &step);
            let layers = between(at + Vec3::Y * 0.3, listener).min(MOST_LAYERS);
            if layers > 0 {
                let occlusion =
                    Occlusion::through(1.0, &[AcousticMaterial::BRICK; MOST_LAYERS][..layers]);
                sound
                    .engine
                    .control(Control::Occlusion { voice, occlusion });
            }
        }
    }
}
