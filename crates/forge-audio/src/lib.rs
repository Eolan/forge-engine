//! `forge-audio` — the sound system (Phase 6, issue #267; D-011, D-062).
//!
//! The game thread talks to the mixer through an [`AudioEngine`]; the mixer, a [`Mixer`], runs
//! wherever the output wants it: inside a device's callback, on its own thread
//! ([`MixThread`]), or in a test that renders seconds of sound into a buffer
//! ([`OfflineRenderer`]). Nothing in this crate opens an audio device.
//!
//! - **Commands** cross from the game to the mixer through a wait-free single-producer,
//!   single-consumer ring (`rtrb`), each stamped with the sample frame it acts on, so a sound
//!   starts or stops on the exact sample the game asked for ([`clock`]). What the mixer is
//!   done with (a voice's sound, a stream's reader) goes back through a second ring and is
//!   dropped on the game thread: the mixer never allocates, frees, locks or reads a file.
//! - **Voices** play sounds held in memory ([`SampleBuffer`]) or streamed from a decoder
//!   ([`stream`]), resampled on the fly. A budget bounds the voices that are mixed; past it
//!   the weakest voice by priority, then by how loud it is at the listener, is stolen, faded
//!   out, and kept as a virtual voice whose time runs on until it wins its place back.
//! - **Buses** form a tree with gains and post-fader sends ([`BusLayout`]); the master bus
//!   ends in a look-ahead limiter that holds a ceiling ([`dsp::Limiter`]).
//! - **Space** ([`spatial`]): equal-power panning by the listener, distance curves, air
//!   absorption and occlusion as three-band gains, Doppler. Steam Audio's binaural effect is
//!   the slot D-011 names for later ([`spatial::Spatializer`]).
//! - **Decoders** ([`decode`]): WAV (integer and float PCM) in our own code, Ogg Vorbis
//!   through `lewton`, Ogg Opus through `opus-rs`, all pure Rust; each streams from a reader
//!   on a decode thread or inline.
//!
//! The output is interleaved stereo `f32` at the mixer's rate (48 kHz by default) in blocks
//! of [`BLOCK`] frames. Rendering is deterministic: the same commands give the same samples,
//! to the bit (D-016's rule for one binary).

#![forbid(unsafe_code)]

pub mod bus;
pub mod clock;
pub mod decode;
pub mod dsp;
pub mod engine;
pub mod mixer;
pub mod render;
pub mod sound;
pub mod spatial;
pub mod stream;
pub mod thread;
mod voice;

pub use bus::{BusId, BusLayout};
pub use clock::{Clock, TickClock};
pub use engine::{AudioEngine, Control, EndReason, Event, PlayParams, VoiceId, create};
pub use mixer::{Mixer, MixerConfig, MixerStats};
pub use render::{OfflineRenderer, WavFormat, wav_bytes, write_wav};
pub use sound::SampleBuffer;
pub use spatial::{Attenuation, Listener, Occlusion, Spatial};
pub use stream::{Stream, StreamPump, Streams};
pub use thread::{MixThread, OutputRing};

/// Frames in a mixer block: 5.3 ms at 48 kHz. Commands act at any frame inside a block;
/// gains and positions change at block boundaries, ramped across the block.
pub const BLOCK: usize = 256;

/// The output's channels: interleaved stereo.
pub const CHANNELS: usize = 2;

/// The mixer's default rate in frames a second.
pub const DEFAULT_RATE: u32 = 48_000;

/// Linear gain from decibels.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Decibels from a linear gain (−inf for 0).
pub fn gain_to_db(gain: f32) -> f32 {
    20.0 * gain.log10()
}
