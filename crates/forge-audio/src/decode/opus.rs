//! Ogg Opus (RFC 7845): the pages through `ogg`, each packet through `opus-rs` (a pure-Rust
//! port of libopus 1.6, BSD-3-Clause).
//!
//! Opus always decodes at 48 kHz. The `OpusHead` packet gives the channels, the output gain
//! and the pre-skip: the frames the encoder put before the sound, dropped from the start.
//! Granule positions count 48 kHz frames from the first decoded one, pre-skip included, so
//! the last page's granule position says where the sound ends: the last packet is cut there.
//! Channel mapping family 0 only (mono and stereo), which is what the mixer plays.

use std::io::{Read, Seek, SeekFrom};

use ogg::PacketReader;

use super::{DecodeError, Decoder};

/// Opus's decoding rate.
const RATE: u32 = 48_000;
/// The longest Opus packet: 120 ms.
const MAX_FRAMES: usize = 5760;

/// Decodes an Ogg Opus stream from a reader.
pub struct OggOpusDecoder<R: Read + Seek> {
    // `None` only while rewinding.
    reader: Option<PacketReader<R>>,
    decoder: opus_rs::OpusDecoder,
    serial: u32,
    channels: u16,
    pre_skip: u64,
    gain: f32,
    /// Frames decoded so far, pre-skip included (the granule positions' count).
    decoded: u64,
    /// The decoded packet not yet handed out, and how far into it.
    pending: Vec<f32>,
    pending_at: usize,
    ended: bool,
}

fn opus_error(e: &'static str) -> DecodeError {
    DecodeError::Format(format!("Opus: {e}"))
}

impl<R: Read + Seek> OggOpusDecoder<R> {
    /// Reads the `OpusHead` and `OpusTags` packets.
    pub fn new(reader: R) -> Result<Self, DecodeError> {
        let mut reader = PacketReader::new(reader);
        let head = reader
            .read_packet()
            .map_err(|e| DecodeError::Format(format!("Ogg: {e}")))?
            .ok_or_else(|| DecodeError::Format("an empty Ogg stream".into()))?;
        let d = &head.data;
        if d.len() < 19 || &d[0..8] != b"OpusHead" {
            return Err(DecodeError::Format("no OpusHead packet".into()));
        }
        if d[8] >> 4 != 0 {
            return Err(DecodeError::Format(format!("OpusHead version {}", d[8])));
        }
        let channels = d[9] as u16;
        let pre_skip = u16::from_le_bytes([d[10], d[11]]) as u64;
        let gain_q8 = i16::from_le_bytes([d[16], d[17]]);
        let family = d[18];
        if family != 0 || channels == 0 || channels > 2 {
            return Err(DecodeError::Channels(channels));
        }
        let serial = head.stream_serial();
        // OpusTags: comments only.
        let tags = reader
            .read_packet()
            .map_err(|e| DecodeError::Format(format!("Ogg: {e}")))?
            .ok_or_else(|| DecodeError::Format("no OpusTags packet".into()))?;
        if !tags.data.starts_with(b"OpusTags") {
            return Err(DecodeError::Format("no OpusTags packet".into()));
        }
        Ok(Self {
            reader: Some(reader),
            decoder: opus_rs::OpusDecoder::new(RATE as i32, channels as usize)
                .map_err(opus_error)?,
            serial,
            channels,
            pre_skip,
            gain: 10f32.powf(gain_q8 as f32 / (20.0 * 256.0)),
            decoded: 0,
            pending: Vec::with_capacity(MAX_FRAMES * channels as usize),
            pending_at: 0,
            ended: false,
        })
    }

    /// Decodes the next audio packet into `pending`; false at the end.
    fn next_packet(&mut self) -> Result<bool, DecodeError> {
        let channels = self.channels as usize;
        let reader = self
            .reader
            .as_mut()
            .expect("an Opus decoder without its reader");
        let packet = loop {
            match reader
                .read_packet()
                .map_err(|e| DecodeError::Format(format!("Ogg: {e}")))?
            {
                Some(p) if p.stream_serial() == self.serial => break p,
                // Another logical stream (a chained or multiplexed file): not ours.
                Some(_) => continue,
                None => return Ok(false),
            }
        };
        self.pending.resize(MAX_FRAMES * channels, 0.0);
        let frames = self
            .decoder
            .decode(&packet.data, MAX_FRAMES, &mut self.pending)
            .map_err(opus_error)?;
        self.pending.truncate(frames * channels);
        let start = self.decoded;
        self.decoded += frames as u64;
        // The end: cut at the last page's granule position.
        let mut end = frames as u64;
        if packet.last_in_stream() {
            end = packet.absgp_page().saturating_sub(start).min(end);
            self.ended = true;
        }
        // The start: drop the pre-skip.
        let skip = self.pre_skip.saturating_sub(start).min(end);
        self.pending.truncate(end as usize * channels);
        self.pending_at = skip as usize * channels;
        if self.gain != 1.0 {
            for x in &mut self.pending[self.pending_at..] {
                *x *= self.gain;
            }
        }
        Ok(true)
    }
}

impl<R: Read + Seek + Send> Decoder for OggOpusDecoder<R> {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        RATE
    }

    fn frames(&self) -> Option<u64> {
        None
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize, DecodeError> {
        let channels = self.channels as usize;
        let want = out.len() / channels * channels;
        let mut written = 0;
        while written < want {
            if self.pending_at >= self.pending.len() {
                if self.ended || !self.next_packet()? {
                    break;
                }
                continue;
            }
            let n = (want - written).min(self.pending.len() - self.pending_at);
            out[written..written + n]
                .copy_from_slice(&self.pending[self.pending_at..self.pending_at + n]);
            written += n;
            self.pending_at += n;
        }
        Ok(written / channels)
    }

    fn rewind(&mut self) -> Result<(), DecodeError> {
        let reader = self
            .reader
            .take()
            .expect("an Opus decoder without its reader");
        let mut inner = reader.into_inner();
        inner.seek(SeekFrom::Start(0))?;
        *self = Self::new(inner)?;
        Ok(())
    }
}
