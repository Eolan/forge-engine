//! Decoders: WAV in our own code, Ogg Vorbis through `lewton`, Ogg Opus through `opus-rs`
//! (all pure Rust).
//!
//! A [`Decoder`] reads interleaved `f32` frames from a byte source a piece at a time, so the
//! same decoder serves a sound loaded whole ([`load`]) and one streamed while it plays
//! ([`crate::Streams::open`]). Decoders run on the game thread or a decode thread, never on
//! the audio thread: they read files and allocate.

mod opus;
mod vorbis;
mod wav;

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub use opus::OggOpusDecoder;
pub use vorbis::VorbisDecoder;
pub use wav::{WavDecoder, WavInfo, WavSampleFormat};

use crate::SampleBuffer;

/// Why a sound could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum DecodeError {
    /// Reading the source failed.
    #[error("reading the sound failed: {0}")]
    Io(#[from] std::io::Error),
    /// The bytes are not a sound this decoder reads.
    #[error("not a sound this decoder reads: {0}")]
    Format(String),
    /// The Vorbis stream is damaged.
    #[error("Vorbis: {0}")]
    Vorbis(#[from] lewton::VorbisError),
    /// More channels than the mixer plays (1 or 2).
    #[error("{0} channels: sounds are mono or stereo")]
    Channels(u16),
}

/// A source of decoded frames.
pub trait Decoder: Send {
    /// Channels in a frame.
    fn channels(&self) -> u16;
    /// Frames a second.
    fn sample_rate(&self) -> u32;
    /// The length in frames, when the format says.
    fn frames(&self) -> Option<u64>;
    /// Decodes up to `out.len() / channels` frames into `out`, interleaved, and returns how
    /// many it wrote: 0 only at the end. `out`'s length must be a whole number of frames.
    fn read(&mut self, out: &mut [f32]) -> Result<usize, DecodeError>;
    /// Goes back to the first frame (for a looping stream).
    fn rewind(&mut self) -> Result<(), DecodeError>;
}

/// Decodes everything a decoder has into a sound in memory.
pub fn decode_all(decoder: &mut dyn Decoder) -> Result<SampleBuffer, DecodeError> {
    let channels = decoder.channels();
    if channels == 0 || channels > 2 {
        return Err(DecodeError::Channels(channels));
    }
    let mut data = Vec::with_capacity(decoder.frames().unwrap_or(0) as usize * channels as usize);
    let mut chunk = vec![0.0f32; 4096 * channels as usize];
    loop {
        let frames = decoder.read(&mut chunk)?;
        if frames == 0 {
            break;
        }
        data.extend_from_slice(&chunk[..frames * channels as usize]);
    }
    Ok(SampleBuffer::new(decoder.sample_rate(), channels, data))
}

/// The decoder for a reader, chosen by its first bytes: `RIFF` for WAV; `OggS` for Ogg, then
/// Opus when the first packet is an `OpusHead`, Vorbis otherwise.
pub fn open<R: Read + Seek + Send + 'static>(
    mut reader: R,
) -> Result<Box<dyn Decoder>, DecodeError> {
    let mut head = [0u8; 64];
    let mut got = 0;
    while got < head.len() {
        match reader.read(&mut head[got..])? {
            0 => break,
            n => got += n,
        }
    }
    reader.seek(SeekFrom::Start(0))?;
    let head = &head[..got];
    match head.get(0..4) {
        Some(b"RIFF") => Ok(Box::new(WavDecoder::new(reader)?)),
        Some(b"OggS") if head.windows(8).any(|w| w == b"OpusHead") => {
            Ok(Box::new(OggOpusDecoder::new(reader)?))
        }
        Some(b"OggS") => Ok(Box::new(VorbisDecoder::new(reader)?)),
        _ => Err(DecodeError::Format(format!(
            "unknown signature {:?}",
            &head[..got.min(4)]
        ))),
    }
}

/// The decoder for a file, buffered.
pub fn open_file(path: impl AsRef<Path>) -> Result<Box<dyn Decoder>, DecodeError> {
    open(BufReader::new(File::open(path)?))
}

/// A whole file decoded into memory.
pub fn load(path: impl AsRef<Path>) -> Result<SampleBuffer, DecodeError> {
    decode_all(open_file(path)?.as_mut())
}
