//! The scenes' sound (#318): the ambience's beds by the hour, the player's footsteps on what
//! the scene says is under them, and the scene's own sounds ([`crate::scenario::Running::sounds`]).
//!
//! It plays only where the shell opened the device ([`forge_app::Context::sound`]): an
//! interactive run, never a scripted one. The listener is the camera, in the scene's frame.

use forge_app::FlyCamera;
use forge_app::sound::Sound;
use forge_audio::ambience::{Ambience, AmbienceAssets, AmbienceState, BirdMix, Habitat};
use forge_audio::{Foley, Listener, PlayParams, Step, Surface};
use glam::{DVec3, Vec3};

/// Seconds the place the ambience hears takes to follow the scene's habitat by `e`: a step
/// through a door or out of a wood crossfades the beds (#350).
const HABITAT_EASE: f32 = 1.5;

/// Metres between steps walking, and running (above [`RUN`] m/s).
const STRIDE: f32 = 0.75;
const RUN_STRIDE: f32 = 1.1;
const RUN: f32 = 2.5;

/// The world's sound while a scene runs.
pub struct WorldSound {
    ambience: Option<Ambience>,
    state: AmbienceState,
    /// The place the scene says the listener is in, which `state.habitat` follows.
    habitat: Habitat,
    foley: Foley,
    /// The player's feet last frame and the ground walked since the last step.
    feet: Option<Vec3>,
    walked: f32,
    /// −1 left, 1 right: the next step's side.
    foot: f32,
}

impl WorldSound {
    /// Starts the beds (the recorded birds and cicadas where `tools/fetch-sounds.sh` put them,
    /// under the engine's root whatever the working directory, #350; beds made in code
    /// otherwise) at `hour`, the birds by the scene's `bird_mix` (the default's without one).
    pub fn start(sound: &mut Sound, hour: f32, bird_mix: Option<BirdMix>) -> Self {
        let rate = sound.sample_rate();
        let state = AmbienceState {
            hour,
            ..AmbienceState::default()
        };
        let root = forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR"));
        let mut assets = AmbienceAssets::load(root.join("assets/sounds"), rate).unwrap_or_default();
        if let Some(mix) = bird_mix {
            assets.bird_mix = mix;
        }
        let ambience = Ambience::start_with(
            &mut sound.engine,
            &sound.streams,
            sound.buses.ambience,
            318,
            &state,
            &assets,
        )
        .inspect_err(|e| tracing::warn!("the ambience: {e}"))
        .ok();
        let mut params = PlayParams {
            bus: sound.buses.foley,
            ..PlayParams::default()
        };
        params.sends[0] = Some((sound.buses.reverb, 0.15));
        Self {
            ambience,
            state,
            habitat: Habitat::default(),
            foley: Foley::new(rate, 318, params),
            feet: None,
            walked: 0.0,
            foot: 1.0,
        }
    }

    /// The place round the listener this frame (#350): the scene's measure, open country
    /// without one. The beds follow it over [`HABITAT_EASE`].
    pub fn set_habitat(&mut self, habitat: Option<Habitat>) {
        self.habitat = habitat.unwrap_or_default();
    }

    /// The frame's sound: the listener at the camera, the beds at `hour` in the place
    /// [`Self::set_habitat`] gave, and a step each stride the player's `feet` walk on `surface`.
    pub fn update(
        &mut self,
        sound: &mut Sound,
        camera: &FlyCamera,
        feet: Option<Vec3>,
        surface: Surface,
        hour: f32,
        dt: f32,
    ) {
        let listener = Listener {
            position: camera.position.as_dvec3(),
            forward: camera.forward(),
            up: Vec3::Y,
            velocity: Vec3::ZERO,
        };
        sound.engine.set_listener(listener);
        self.state.hour = hour.rem_euclid(24.0);
        self.state.listener = listener;
        let to = self.habitat;
        let k = 1.0 - (-dt.max(0.0) / HABITAT_EASE).exp();
        let place = &mut self.state.habitat;
        place.green += (to.green - place.green) * k;
        place.indoors += (to.indoors - place.indoors) * k;
        if let Some(ambience) = &mut self.ambience {
            ambience.update(&mut sound.engine, &self.state, dt);
        }
        let Some(feet) = feet else {
            self.feet = None;
            return;
        };
        let moved = self.feet.map_or(0.0, |last| {
            let d = feet - last;
            Vec3::new(d.x, 0.0, d.z).length()
        });
        self.feet = Some(feet);
        // A jump of a metre or more in a frame is a teleport, not a step.
        if moved > 1.0 || dt <= 0.0 {
            self.walked = 0.0;
            return;
        }
        let speed = moved / dt;
        if speed < 0.2 {
            // Standing: the next step starts a stride from here.
            self.walked = self.walked.min(0.5 * STRIDE);
            return;
        }
        self.walked += moved;
        let stride = if speed > RUN { RUN_STRIDE } else { STRIDE };
        if self.walked >= stride {
            self.walked -= stride;
            self.foot = -self.foot;
            let side = camera.forward().cross(Vec3::Y).normalize_or_zero() * 0.12 * self.foot;
            self.foley.own_step(
                &mut sound.engine,
                &Step {
                    surface,
                    position: (feet + side).as_dvec3() + DVec3::new(0.0, 0.05, 0.0),
                    speed,
                    weight: 80.0,
                    footwear: forge_audio::Footwear::Sneakers,
                    start: None,
                },
                self.foot,
            );
        }
    }
}

/// The hour of the gallery's sun cycle at `t` (0 sunrise, 0.5 noon, 1 sunset, 1.5 midnight),
/// with the ambience's summer day (sunrise 6:12, sunset 20:36).
pub fn hour_of_cycle(t: f32) -> f32 {
    let day = AmbienceState::default();
    let t = t.rem_euclid(2.0);
    if t <= 1.0 {
        day.sunrise + t * (day.sunset - day.sunrise)
    } else {
        (day.sunset + (t - 1.0) * (24.0 - day.sunset + day.sunrise)).rem_euclid(24.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cycle_maps_onto_the_summer_day() {
        let day = AmbienceState::default();
        assert!((hour_of_cycle(0.0) - day.sunrise).abs() < 1e-4);
        assert!((hour_of_cycle(1.0) - day.sunset).abs() < 1e-4);
        assert!((hour_of_cycle(0.5) - 0.5 * (day.sunrise + day.sunset)).abs() < 1e-4);
        assert!((hour_of_cycle(2.0) - day.sunrise).abs() < 1e-4);
        let midnight = hour_of_cycle(1.5);
        assert!(!(day.sunrise..day.sunset).contains(&midnight));
    }
}
