//! Ogg Vorbis through `lewton` (pure Rust, MIT OR Apache-2.0), a packet at a time.
//!
//! A page's granule position is the frames due by its end. When the first audio page decodes
//! to more, the excess is padding: at the start when more pages follow, at the end when it
//! is also the last page (the Vorbis I specification, A.2). `lewton` trims the end only from
//! the second page on (it has no granule before), so the decoder decodes the first page ahead
//! and trims it itself: FFmpeg writes a second of sound as one page and leaves 704 frames of
//! padding at its end.

use std::io::{Read, Seek, SeekFrom};

use lewton::inside_ogg::OggStreamReader;
use lewton::samples::InterleavedSamples;

use super::{DecodeError, Decoder};

/// Decodes an Ogg Vorbis stream from a reader.
pub struct VorbisDecoder<R: Read + Seek> {
    // `None` only while rewinding.
    reader: Option<OggStreamReader<R>>,
    channels: u16,
    rate: u32,
    /// The decoded packet not yet handed out, and how far into it.
    pending: Vec<f32>,
    pending_at: usize,
}

impl<R: Read + Seek> VorbisDecoder<R> {
    /// Reads the three Vorbis headers and the first page of audio.
    pub fn new(reader: R) -> Result<Self, DecodeError> {
        let reader = OggStreamReader::new(reader)?;
        let channels = reader.ident_hdr.audio_channels as u16;
        let rate = reader.ident_hdr.audio_sample_rate;
        if channels == 0 || rate == 0 {
            return Err(DecodeError::Format(
                "a Vorbis stream without channels or rate".into(),
            ));
        }
        let mut decoder = Self {
            reader: Some(reader),
            channels,
            rate,
            pending: Vec::new(),
            pending_at: 0,
        };
        decoder.prime()?;
        Ok(decoder)
    }

    /// Decodes the first page of audio and drops the padding its granule position leaves
    /// out.
    fn prime(&mut self) -> Result<(), DecodeError> {
        let channels = self.channels as usize;
        let reader = self
            .reader
            .as_mut()
            .expect("a Vorbis decoder without its reader");
        let mut decoded = Vec::new();
        while let Some(packet) = reader.read_dec_packet_generic::<InterleavedSamples<f32>>()? {
            decoded.extend_from_slice(&packet.samples);
            if let Some(granule) = reader.get_last_absgp() {
                let frames = (decoded.len() / channels) as u64;
                if frames > granule {
                    match reader.read_dec_packet_generic::<InterleavedSamples<f32>>()? {
                        // The first page was also the last: the excess is at the end.
                        None => decoded.truncate(granule as usize * channels),
                        // Padding at the start.
                        Some(next) => {
                            decoded.drain(..(frames - granule) as usize * channels);
                            decoded.extend_from_slice(&next.samples);
                        }
                    }
                }
                break;
            }
        }
        self.pending = decoded;
        self.pending_at = 0;
        Ok(())
    }
}

impl<R: Read + Seek + Send> Decoder for VorbisDecoder<R> {
    fn channels(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.rate
    }

    fn frames(&self) -> Option<u64> {
        None
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize, DecodeError> {
        let channels = self.channels as usize;
        let want = out.len() / channels * channels;
        let mut written = 0;
        while written < want {
            if self.pending_at == self.pending.len() {
                let reader = self
                    .reader
                    .as_mut()
                    .expect("a Vorbis decoder without its reader");
                match reader.read_dec_packet_generic::<InterleavedSamples<f32>>()? {
                    Some(packet) => {
                        self.pending = packet.samples;
                        self.pending_at = 0;
                        continue;
                    }
                    None => break,
                }
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
        // Back to byte 0 and the headers again: exact, where a granule seek lands on a page.
        let reader = self
            .reader
            .take()
            .expect("a Vorbis decoder without its reader");
        let mut inner = reader.into_inner().into_inner();
        inner.seek(SeekFrom::Start(0))?;
        self.reader = Some(OggStreamReader::new(inner)?);
        self.prime()
    }
}
