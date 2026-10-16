//! WAV (RIFF WAVE): integer PCM of 8, 16, 24 and 32 bits and IEEE float of 32 and 64 bits,
//! plain or in the extensible header, read a piece at a time.

use std::io::{Read, Seek, SeekFrom};

use super::{DecodeError, Decoder};

/// How a WAV file stores its samples.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavSampleFormat {
    /// Unsigned 8-bit.
    U8,
    /// Signed 16-bit.
    I16,
    /// Signed 24-bit, packed in 3 bytes.
    I24,
    /// Signed 32-bit.
    I32,
    /// IEEE 32-bit float.
    F32,
    /// IEEE 64-bit float.
    F64,
}

impl WavSampleFormat {
    /// Bytes a sample.
    pub fn bytes(self) -> usize {
        match self {
            WavSampleFormat::U8 => 1,
            WavSampleFormat::I16 => 2,
            WavSampleFormat::I24 => 3,
            WavSampleFormat::I32 | WavSampleFormat::F32 => 4,
            WavSampleFormat::F64 => 8,
        }
    }
}

/// What a WAV file's header says.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavInfo {
    /// Channels in a frame.
    pub channels: u16,
    /// Frames a second.
    pub rate: u32,
    /// The samples' format.
    pub format: WavSampleFormat,
    /// Frames in the data chunk.
    pub frames: u64,
}

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_EXTENSIBLE: u16 = 0xFFFE;

/// Decodes a WAV file from a reader.
pub struct WavDecoder<R> {
    reader: R,
    info: WavInfo,
    data_start: u64,
    /// Frames read so far.
    position: u64,
    bytes: Vec<u8>,
}

impl<R: Read + Seek> WavDecoder<R> {
    /// Reads the header and stops at the first sample.
    pub fn new(mut reader: R) -> Result<Self, DecodeError> {
        let mut riff = [0u8; 12];
        reader.read_exact(&mut riff)?;
        if &riff[0..4] != b"RIFF" || &riff[8..12] != b"WAVE" {
            return Err(DecodeError::Format("not a RIFF WAVE file".into()));
        }
        let mut fmt: Option<(u16, u16, u32, u16)> = None;
        let mut data: Option<(u64, u64)> = None;
        loop {
            let mut header = [0u8; 8];
            if let Err(e) = reader.read_exact(&mut header) {
                if e.kind() == std::io::ErrorKind::UnexpectedEof {
                    break;
                }
                return Err(e.into());
            }
            let id = [header[0], header[1], header[2], header[3]];
            let size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as u64;
            let start = reader.stream_position()?;
            match &id {
                b"fmt " => {
                    if size < 16 {
                        return Err(DecodeError::Format("a short fmt chunk".into()));
                    }
                    let mut f = vec![0u8; size as usize];
                    reader.read_exact(&mut f)?;
                    let mut tag = u16::from_le_bytes([f[0], f[1]]);
                    let channels = u16::from_le_bytes([f[2], f[3]]);
                    let rate = u32::from_le_bytes([f[4], f[5], f[6], f[7]]);
                    let bits = u16::from_le_bytes([f[14], f[15]]);
                    if tag == FORMAT_EXTENSIBLE {
                        if size < 40 {
                            return Err(DecodeError::Format("a short extensible fmt chunk".into()));
                        }
                        // The sub-format GUID starts with the plain format's tag.
                        tag = u16::from_le_bytes([f[24], f[25]]);
                    }
                    fmt = Some((tag, channels, rate, bits));
                }
                b"data" => {
                    let end = reader.seek(SeekFrom::End(0))?;
                    // A writer that never came back to fill the size leaves 0 or 0xFFFFFFFF.
                    let length = if size == 0 || size == u32::MAX as u64 || start + size > end {
                        end - start
                    } else {
                        size
                    };
                    data = Some((start, length));
                    if fmt.is_some() {
                        break;
                    }
                }
                _ => {}
            }
            // Chunks are padded to an even size.
            let next = start + size + (size & 1);
            if data.is_some() && fmt.is_some() {
                break;
            }
            reader.seek(SeekFrom::Start(next))?;
        }
        let (tag, channels, rate, bits) =
            fmt.ok_or_else(|| DecodeError::Format("no fmt chunk".into()))?;
        let (data_start, data_len) =
            data.ok_or_else(|| DecodeError::Format("no data chunk".into()))?;
        let format = match (tag, bits) {
            (FORMAT_PCM, 8) => WavSampleFormat::U8,
            (FORMAT_PCM, 16) => WavSampleFormat::I16,
            (FORMAT_PCM, 24) => WavSampleFormat::I24,
            (FORMAT_PCM, 32) => WavSampleFormat::I32,
            (FORMAT_FLOAT, 32) => WavSampleFormat::F32,
            (FORMAT_FLOAT, 64) => WavSampleFormat::F64,
            _ => {
                return Err(DecodeError::Format(format!(
                    "format {tag} with {bits} bits"
                )));
            }
        };
        if channels == 0 || rate == 0 {
            return Err(DecodeError::Format("no channels or no rate".into()));
        }
        let frame_bytes = format.bytes() as u64 * channels as u64;
        reader.seek(SeekFrom::Start(data_start))?;
        Ok(Self {
            reader,
            info: WavInfo {
                channels,
                rate,
                format,
                frames: data_len / frame_bytes,
            },
            data_start,
            position: 0,
            bytes: Vec::new(),
        })
    }

    /// The header's facts.
    pub fn info(&self) -> WavInfo {
        self.info
    }
}

impl<R: Read + Seek + Send> Decoder for WavDecoder<R> {
    fn channels(&self) -> u16 {
        self.info.channels
    }

    fn sample_rate(&self) -> u32 {
        self.info.rate
    }

    fn frames(&self) -> Option<u64> {
        Some(self.info.frames)
    }

    fn read(&mut self, out: &mut [f32]) -> Result<usize, DecodeError> {
        let channels = self.info.channels as usize;
        let left = self.info.frames - self.position;
        let frames = ((out.len() / channels) as u64).min(left) as usize;
        if frames == 0 {
            return Ok(0);
        }
        let size = self.info.format.bytes();
        self.bytes.resize(frames * channels * size, 0);
        self.reader.read_exact(&mut self.bytes)?;
        let samples = &mut out[..frames * channels];
        let b = &self.bytes;
        match self.info.format {
            WavSampleFormat::U8 => {
                for (o, &v) in samples.iter_mut().zip(b) {
                    *o = (v as f32 - 128.0) / 128.0;
                }
            }
            WavSampleFormat::I16 => {
                for (o, v) in samples.iter_mut().zip(b.as_chunks::<2>().0) {
                    *o = i16::from_le_bytes(*v) as f32 / 32768.0;
                }
            }
            WavSampleFormat::I24 => {
                for (o, v) in samples.iter_mut().zip(b.as_chunks::<3>().0) {
                    // Into the top three bytes of an i32, then down: the sign comes along.
                    let x = i32::from_le_bytes([0, v[0], v[1], v[2]]) >> 8;
                    *o = x as f32 / 8_388_608.0;
                }
            }
            WavSampleFormat::I32 => {
                for (o, v) in samples.iter_mut().zip(b.as_chunks::<4>().0) {
                    *o = (i32::from_le_bytes(*v) as f64 / 2_147_483_648.0) as f32;
                }
            }
            WavSampleFormat::F32 => {
                for (o, v) in samples.iter_mut().zip(b.as_chunks::<4>().0) {
                    *o = f32::from_le_bytes(*v);
                }
            }
            WavSampleFormat::F64 => {
                for (o, v) in samples.iter_mut().zip(b.as_chunks::<8>().0) {
                    *o = f64::from_le_bytes(*v) as f32;
                }
            }
        }
        self.position += frames as u64;
        Ok(frames)
    }

    fn rewind(&mut self) -> Result<(), DecodeError> {
        self.reader.seek(SeekFrom::Start(self.data_start))?;
        self.position = 0;
        Ok(())
    }
}
