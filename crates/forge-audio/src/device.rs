//! The audio device (#318): the default output through `cpal` (WASAPI), fed by the mixer on
//! its own thread ([`MixThread`]). Behind the `device` feature, on Windows only for now (ALSA
//! needs system headers the Linux CI does not install).
//!
//! The device's callback only copies frames out of the mix thread's ring
//! ([`OutputRing::pull`]): stereo into the first two channels and silence in the others, the
//! mean of the two on a mono device. It never allocates, locks or waits.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};

use crate::{MixThread, Mixer, OutputRing};

/// Why the device did not open.
#[derive(Debug, thiserror::Error)]
pub enum DeviceError {
    /// The system has no default output device.
    #[error("no output device")]
    NoDevice,
    /// `cpal` failed: the device's format, the stream or starting it.
    #[error("the audio device: {0}")]
    Device(String),
    /// The device wants samples the callback does not write.
    #[error("the device wants {0} samples; f32, i16 and i32 are played")]
    Format(String),
}

/// The default output device and its format, before it plays.
pub struct Device {
    device: cpal::Device,
    config: cpal::StreamConfig,
    format: cpal::SampleFormat,
    name: String,
}

impl Device {
    /// The system's default output device in its own format (the mixer then runs at the
    /// device's rate: no resampling at the output).
    pub fn default_output() -> Result<Device, DeviceError> {
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or(DeviceError::NoDevice)?;
        let supported = device
            .default_output_config()
            .map_err(|e| DeviceError::Device(e.to_string()))?;
        let format = supported.sample_format();
        let name = device.to_string();
        Ok(Device {
            device,
            config: supported.into(),
            format,
            name,
        })
    }

    /// Frames a second.
    pub fn sample_rate(&self) -> u32 {
        self.config.sample_rate
    }

    /// The device's channels.
    pub fn channels(&self) -> u16 {
        self.config.channels
    }

    /// The device's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Starts `mixer` on a thread of its own, `ring` frames ahead of the device, and the
    /// device's stream reading from it.
    pub fn play(self, mixer: Mixer, ring: usize) -> Result<Output, DeviceError> {
        let (mix, ring) = MixThread::spawn(mixer, ring);
        let short = Arc::new(AtomicU64::new(0));
        let stream = match self.format {
            cpal::SampleFormat::F32 => open::<f32>(&self.device, self.config, ring, &short),
            cpal::SampleFormat::I16 => open::<i16>(&self.device, self.config, ring, &short),
            cpal::SampleFormat::I32 => open::<i32>(&self.device, self.config, ring, &short),
            other => return Err(DeviceError::Format(format!("{other:?}"))),
        }?;
        stream
            .play()
            .map_err(|e| DeviceError::Device(e.to_string()))?;
        Ok(Output {
            _stream: stream,
            _mix: mix,
            short,
            name: self.name,
            rate: self.config.sample_rate,
        })
    }
}

/// A playing device. Dropping it stops the stream, then the mix thread.
pub struct Output {
    // Dropped first: the callback stops before the ring's other end goes.
    _stream: cpal::Stream,
    _mix: MixThread,
    short: Arc<AtomicU64>,
    name: String,
    rate: u32,
}

impl Output {
    /// The device's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Frames a second.
    pub fn sample_rate(&self) -> u32 {
        self.rate
    }

    /// Device callbacks the ring could not fill (heard as clicks).
    pub fn short_pulls(&self) -> u64 {
        self.short.load(Ordering::Relaxed)
    }
}

/// The device's stream of samples of type `T`.
fn open<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut ring: OutputRing,
    short: &Arc<AtomicU64>,
) -> Result<cpal::Stream, DeviceError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let short = Arc::clone(short);
    // Sized here: the callback never allocates.
    let mut stereo = vec![0.0f32; 8192];
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                for chunk in data.chunks_mut(channels * stereo.len() / 2) {
                    let frames = chunk.len() / channels;
                    let s = &mut stereo[..frames * 2];
                    if ring.pull(s) < frames {
                        short.fetch_add(1, Ordering::Relaxed);
                    }
                    for (frame, out) in s.as_chunks::<2>().0.iter().zip(chunk.chunks_mut(channels))
                    {
                        if channels == 1 {
                            out[0] = T::from_sample(0.5 * (frame[0] + frame[1]));
                            continue;
                        }
                        for (c, o) in out.iter_mut().enumerate() {
                            *o = if c < 2 {
                                T::from_sample(frame[c])
                            } else {
                                T::EQUILIBRIUM
                            };
                        }
                    }
                }
            },
            // A device unplugged: the crate has no logger.
            |e| eprintln!("the audio device: {e}"),
            None,
        )
        .map_err(|e| DeviceError::Device(e.to_string()))
}
