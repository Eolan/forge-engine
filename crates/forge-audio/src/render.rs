//! Rendering without a device: seconds of a scene into a buffer or a WAV file.
//!
//! An [`OfflineRenderer`] runs the mixer on the calling thread as fast as it goes, a block at
//! a time, and refills the streams before each block, so what it renders depends only on the
//! commands: the same scene gives the same samples to the bit, on any machine fast or slow
//! (D-016's rule within one binary). The tests assert on its samples; the owner listens to
//! its WAVs.

use std::io::Write;
use std::path::Path;

use crate::stream::StreamPump;
use crate::{BLOCK, CHANNELS, Mixer};

/// The mixer run offline.
pub struct OfflineRenderer {
    mixer: Mixer,
    pump: Option<StreamPump>,
}

impl OfflineRenderer {
    /// Renders `mixer`, without streams.
    pub fn new(mixer: Mixer) -> Self {
        Self { mixer, pump: None }
    }

    /// Renders `mixer` and pumps `pump`'s streams before each block.
    pub fn with_streams(mixer: Mixer, pump: StreamPump) -> Self {
        Self {
            mixer,
            pump: Some(pump),
        }
    }

    /// The mixer.
    pub fn mixer(&mut self) -> &mut Mixer {
        &mut self.mixer
    }

    /// Renders `frames` frames, interleaved stereo.
    pub fn render(&mut self, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0; frames * CHANNELS];
        self.render_into(&mut out);
        out
    }

    /// Renders `seconds` of sound.
    pub fn render_seconds(&mut self, seconds: f64) -> Vec<f32> {
        let frames = (seconds * self.mixer.sample_rate() as f64).round() as usize;
        self.render(frames)
    }

    /// Fills `out` (interleaved stereo), a block at a time.
    pub fn render_into(&mut self, out: &mut [f32]) {
        for chunk in out.chunks_mut(BLOCK * CHANNELS) {
            if let Some(pump) = &mut self.pump {
                pump.pump();
            }
            self.mixer.render(chunk);
        }
    }
}

/// How a WAV file stores its samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavFormat {
    /// 16-bit integers: the sample times 32 768, rounded and clipped (no dither, so the file
    /// is the same every time; the reader divides by the same number).
    Pcm16,
    /// 24-bit integers, rounded.
    Pcm24,
    /// 32-bit floats: the mixer's samples exactly.
    Float32,
}

/// A WAV file's bytes for interleaved samples.
pub fn wav_bytes(samples: &[f32], channels: u16, rate: u32, format: WavFormat) -> Vec<u8> {
    let (tag, bits): (u16, u16) = match format {
        WavFormat::Pcm16 => (1, 16),
        WavFormat::Pcm24 => (1, 24),
        WavFormat::Float32 => (3, 32),
    };
    let block_align = channels * bits / 8;
    let data_len = samples.len() as u32 * (bits / 8) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&tag.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * block_align as u32).to_le_bytes());
    out.extend_from_slice(&block_align.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for &x in samples {
        match format {
            WavFormat::Pcm16 => {
                let v = (x as f64 * 32_768.0).round().clamp(-32_768.0, 32_767.0) as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
            WavFormat::Pcm24 => {
                let v = (x as f64 * 8_388_608.0)
                    .round()
                    .clamp(-8_388_608.0, 8_388_607.0) as i32;
                out.extend_from_slice(&v.to_le_bytes()[..3]);
            }
            WavFormat::Float32 => out.extend_from_slice(&x.to_le_bytes()),
        }
    }
    out
}

/// Writes interleaved samples to a WAV file.
pub fn write_wav(
    path: impl AsRef<Path>,
    samples: &[f32],
    channels: u16,
    rate: u32,
    format: WavFormat,
) -> std::io::Result<()> {
    let path = path.as_ref();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = std::fs::File::create(path)?;
    file.write_all(&wav_bytes(samples, channels, rate, format))
}
