//! The lab's impacts (Phase 7's contact events to the material layer): each contact a tick
//! begins sounds as its two bodies' rows (D-007), through the foley's rule for two bodies
//! meeting ([`Impact::between`]): each that has a surface rings, the softer one setting how long
//! the contact lasts.
//!
//! Each sound takes 0.1 to 1 ms to make (10 to 40 ns a sample, and a heavy body rings for up to a
//! second), on the audio engine's maker thread, which the next frame waits for; and a pile coming
//! down begins some 700 meetings a second: more than any ear tells apart, or [`IMPACTS_CAP`]
//! voices hold. So the hardest meeting waiting is made each frame ([`MOST_A_FRAME`]), the others
//! wait up to [`LATE`] frames (balls landing in the same tick are heard one after the other, as
//! from 20 m away), and a moving body sounds again only [`COOLDOWN`] ticks after it last did.

use std::collections::HashMap;

use forge_app::sound::Sound;
use forge_audio::{Foley, Impact, PlayParams};
use forge_physics::BodyId;

use super::Heard;

/// The impacts' family of voices.
pub(crate) const FAMILY_IMPACTS: u8 = 2;

/// The most impacts mixed at once: the loudest win.
const IMPACTS_CAP: u16 = 8;

/// The slowest meeting that sounds, m/s: a body settling or woken against another is silent.
const QUIETEST: f32 = 0.3;

/// The meetings made into sounds a frame (each one or two sounds), the hardest waiting first.
const MOST_A_FRAME: usize = 1;

/// The frames a meeting waits to be made before it is let go: 67 ms at 60 Hz, the time sound
/// takes over 23 m.
const LATE: u32 = 4;

/// Ticks before a moving body sounds again (a sixth of a second): its rattle as it settles is one
/// sound, not one a tick.
const COOLDOWN: u64 = 10;

/// A meeting waiting to be made: the contact, its impacts, the frames it has waited.
struct Waiting {
    heard: Heard,
    impacts: Vec<Impact>,
    waited: u32,
}

impl Waiting {
    /// The force's strength: the pair's reduced mass and the speed, as the level follows
    /// (`√m × v`).
    fn strength(&self) -> f32 {
        self.impacts[0].mass.sqrt() * self.impacts[0].speed
    }
}

/// The lab's impacts.
pub(crate) struct Impacts {
    foley: Foley,
    waiting: Vec<Waiting>,
    /// The tick each moving body last sounded at.
    last: HashMap<BodyId, u64>,
    made: u64,
    /// Meetings let go after waiting too long.
    let_go: u64,
    /// Milliseconds the game thread spent on them over the run (choosing and asking; the making
    /// is the maker thread's).
    ms: f64,
}

impl Impacts {
    /// Their foley on the app's foley bus with a little of the reverb, and their family's cap.
    pub(crate) fn new(sound: &mut Sound) -> Self {
        sound.engine.set_family_cap(FAMILY_IMPACTS, IMPACTS_CAP);
        let mut params = PlayParams {
            bus: sound.buses.foley,
            family: FAMILY_IMPACTS,
            ..PlayParams::default()
        };
        params.sends[0] = Some((sound.buses.reverb, 0.2));
        Self {
            foley: Foley::new(sound.sample_rate(), 1490, params),
            waiting: Vec::new(),
            last: HashMap::new(),
            made: 0,
            let_go: 0,
            ms: 0.0,
        }
    }

    /// Takes `heard` (the contacts of the ticks since the last frame) and plays the hardest
    /// meeting waiting.
    pub(crate) fn play(&mut self, sound: &mut Sound, heard: &[Heard]) {
        let start = std::time::Instant::now();
        for w in &mut self.waiting {
            w.waited += 1;
        }
        let before = self.waiting.len();
        self.waiting.retain(|w| w.waited <= LATE);
        self.let_go += (before - self.waiting.len()) as u64;
        self.waiting.extend(
            heard
                .iter()
                .filter(|h| h.speed >= QUIETEST)
                .map(|&h| Waiting {
                    heard: h,
                    impacts: Impact::between(h.bodies[0], h.bodies[1], h.point, h.speed, None)
                        .collect(),
                    waited: 0,
                })
                .filter(|w| !w.impacts.is_empty()),
        );
        // The hardest first; a stable sort keeps the ticks' order between equals.
        self.waiting
            .sort_by(|a, b| b.strength().total_cmp(&a.strength()));
        let mut played = 0;
        while played < MOST_A_FRAME && !self.waiting.is_empty() {
            let h = self.waiting[0].heard;
            // The moving bodies (a fixed one, the floor, is met by everything).
            let moving = || {
                (0..2)
                    .filter(|&i| h.bodies[i].mass.is_some())
                    .map(|i| h.ids[i])
            };
            if moving().all(|id| self.last.get(&id).is_some_and(|&t| h.tick < t + COOLDOWN)) {
                // Its bodies just sounded: this one is part of that.
                self.waiting.remove(0);
                continue;
            }
            for id in moving() {
                self.last.insert(id, h.tick);
            }
            let w = self.waiting.remove(0);
            if self.made == 0 {
                tracing::info!(
                    tick = h.tick,
                    surfaces = ?w.impacts.iter().map(|i| i.surface.name()).collect::<Vec<_>>(),
                    speed = h.speed,
                    mass = w.impacts[0].mass,
                    "the first impact heard"
                );
            }
            for impact in &w.impacts {
                self.foley.impact(&mut sound.engine, impact);
                self.made += 1;
            }
            played += 1;
        }
        self.ms += start.elapsed().as_secs_f64() * 1e3;
    }
}

impl Drop for Impacts {
    fn drop(&mut self) {
        tracing::info!(
            made = self.made,
            let_go = self.let_go,
            ms = self.ms,
            "impacts"
        );
    }
}
