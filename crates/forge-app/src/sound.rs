//! Sound in a running app (#318): the shell owns the audio output, the game plays into it.
//!
//! A run whose demo asks for sound ([`crate::AppConfig::sound`]) opens the system's default
//! output device (on Windows, through `forge-audio`'s `device` feature), with the mixer at the
//! device's rate on a thread of its own and a decode thread for streamed sounds. Scripted runs
//! never open it: a frame limit, a capture, `--no-sound` or `FORGE_SOUND=0` keep the run silent
//! (sound during scripted runs disturbs the owner). `snd.volume` is the master's level.
//!
//! The game reaches it through [`crate::Context::sound`]: it plays voices on
//! [`Sound::engine`] into [`Sound::buses`], moves the listener with [`Sound::listen`] (the
//! camera, every frame), and reads the voices that ended in [`Sound::events`].

use std::time::Duration;

use forge_audio::ambience::AmbienceBuses;
use forge_audio::dsp::ReverbParams;
use forge_audio::stream::{PumpThread, Streams, streams};
use forge_audio::{
    AudioEngine, BusId, BusLayout, Event, Listener, Mixer, MixerConfig, PlayParams, create,
};
use glam::{DVec3, Vec3};

/// Frames the mix thread runs ahead of the device: 21 ms at 48 kHz.
#[cfg_attr(not(windows), allow(dead_code))]
const RING_FRAMES: usize = 1024;

/// Seconds of a streamed sound decoded ahead.
#[cfg_attr(not(windows), allow(dead_code))]
const STREAM_AHEAD: f32 = 0.25;

/// The buses the shell lays out.
#[derive(Clone, Copy, Debug)]
pub struct Buses {
    /// The ambience's: `world` holds everything heard in the world, with the medium's filter.
    pub ambience: AmbienceBuses,
    /// Sounds placed in the world (under `world`): what the game plays.
    pub effects: BusId,
    /// Footsteps and impacts (under `world`).
    pub foley: BusId,
    /// The world's reverb, an aux bus the effects and the foley send to (under `world`).
    pub reverb: BusId,
    /// The interface: not placed, not filtered under water (under the master).
    pub ui: BusId,
}

/// The game's end of the sound output.
pub struct Sound {
    /// Plays voices, moves them, stops them; the mixer applies them on the sample asked for.
    pub engine: AudioEngine,
    /// Where they play.
    pub buses: Buses,
    /// Opens streamed sounds (the ambience's beds, music); they decode on the shell's thread.
    pub streams: Streams,
    /// The voices that ended since the previous frame.
    pub events: Vec<Event>,
    _decode: PumpThread,
}

impl Sound {
    /// Frames a second.
    pub fn sample_rate(&self) -> u32 {
        self.engine.sample_rate()
    }

    /// Moves the listener: the camera's place, where it looks, its up and its velocity (m/s,
    /// for Doppler).
    pub fn listen(&mut self, position: DVec3, forward: Vec3, up: Vec3, velocity: Vec3) {
        self.engine.set_listener(Listener {
            position,
            forward,
            up,
            velocity,
        });
    }

    /// A voice placed at `position` in the effects bus, sending a fifth of itself to the
    /// world's reverb.
    pub fn at(&self, position: DVec3) -> PlayParams {
        let mut params = PlayParams::at(position);
        params.bus = self.buses.effects;
        params.sends[0] = Some((self.buses.reverb, 0.2));
        params
    }

    /// Hands the voices that ended to the game and drops what the mixer is done with (the
    /// shell calls it at the start of each frame).
    pub(crate) fn update(&mut self) {
        self.events.clear();
        self.events.extend_from_slice(self.engine.update());
    }
}

/// Whether this run may make sound: the demo asks for it, nothing scripts the run, and
/// `FORGE_SOUND` does not say 0 or off.
pub(crate) fn wanted(config: &crate::AppConfig) -> bool {
    let scripted =
        config.frame_limit.is_some() || config.capture.is_some() || config.capture_every.is_some();
    let silenced = std::env::var("FORGE_SOUND")
        .is_ok_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "0" | "off" | "no"));
    config.sound && !scripted && !silenced
}

/// The mix at `rate`: the buses and the engine.
#[cfg_attr(not(windows), allow(dead_code))]
fn mix(rate: u32) -> (Sound, Mixer) {
    let mut layout = BusLayout::new();
    let ambience = AmbienceBuses::add(&mut layout);
    let reverb = layout.add_reverb("reverb", ambience.world, ReverbParams::OPEN);
    let effects = layout.add("effects", ambience.world);
    let foley = layout.add("foley", ambience.world);
    let ui = layout.add("ui", BusId::MASTER);
    let config = MixerConfig {
        sample_rate: rate,
        ..MixerConfig::default()
    };
    let (engine, mixer) = create(config, &layout);
    let (streams, pump) = streams(STREAM_AHEAD);
    let sound = Sound {
        engine,
        buses: Buses {
            ambience,
            effects,
            foley,
            reverb,
            ui,
        },
        streams,
        events: Vec::new(),
        _decode: pump.spawn(Duration::from_millis(5)),
    };
    (sound, mixer)
}

/// The playing device, held by the shell for as long as the run lasts.
pub(crate) struct Output {
    #[cfg(windows)]
    _device: forge_audio::device::Output,
}

/// Opens the default output device and the mix for it; `None` (logged) where there is no
/// device, or off Windows for now.
pub(crate) fn open() -> Option<(Sound, Output)> {
    #[cfg(windows)]
    {
        let opened = forge_audio::device::Device::default_output().and_then(|device| {
            let rate = device.sample_rate();
            let name = device.name().to_owned();
            let (sound, mixer) = mix(rate);
            let output = device.play(mixer, RING_FRAMES)?;
            tracing::info!(device = %name, rate, "sound");
            Ok((sound, Output { _device: output }))
        });
        match opened {
            Ok(opened) => Some(opened),
            Err(e) => {
                tracing::warn!("no sound: {e}");
                None
            }
        }
    }
    #[cfg(not(windows))]
    {
        tracing::info!("no sound: the audio device is opened on Windows only for now");
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripted_runs_stay_silent() {
        let interactive = crate::AppConfig {
            sound: true,
            ..crate::AppConfig::default()
        };
        // (FORGE_SOUND may be set where the tests run: the scripted cases hold either way.)
        for scripted in [
            crate::AppConfig {
                frame_limit: Some(60),
                ..interactive.clone()
            },
            crate::AppConfig {
                capture: Some(("x.png".into(), 10)),
                ..interactive.clone()
            },
            crate::AppConfig {
                capture_every: Some(2),
                ..interactive.clone()
            },
            crate::AppConfig {
                sound: false,
                ..interactive.clone()
            },
        ] {
            assert!(!wanted(&scripted));
        }
    }

    #[test]
    fn the_mix_lays_out_its_buses_and_renders_silence() {
        let (sound, mut mixer) = mix(48_000);
        let b = sound.buses;
        let ids = [b.effects, b.foley, b.reverb, b.ui, b.ambience.world];
        for (i, a) in ids.iter().enumerate() {
            for c in &ids[i + 1..] {
                assert_ne!(a, c);
            }
        }
        let mut out = vec![1.0f32; forge_audio::BLOCK * forge_audio::CHANNELS];
        mixer.render(&mut out);
        assert!(out.iter().all(|s| *s == 0.0));
    }
}
