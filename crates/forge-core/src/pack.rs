//! Packed bytes on disk (#215): what the caches store is compressed, so it takes less room and
//! loads faster from a cold disk, and never slower from a warm one. The compressor is LZ4
//! (`lz4_flex`): the owner chose the fastest decompression over the tightest ratio, since
//! reading matters more than writing.
//!
//! - [`pack`] and [`unpack`] handle one block. It is stored as LZ4, as LZ4 over its byte planes
//!   ([`Codec::Lz4Planes16`]: the bytes of 16-byte records regrouped by their position, in which
//!   LZ4 finds more repeats), or as is when neither is smaller.
//! - [`encode_grid`] and [`decode_grid`] handle a grid of `f32` samples, losslessly (the
//!   floating-point grids compressors do little on).
//!   - Each sample's bits are taken as an integer ordered as the numbers are, and predicted
//!     from its left, upper and upper-left neighbours (the gradient predictor).
//!   - The differences, small near zero, are stored on four byte planes.
//!   - The grid is cut into bands of whole rows that decode on their own, in parallel.
//!   - LZ4 over the planes then finds room raw floats never give it: the island's drawn ground
//!     goes from 268 MB to 71 MB.
//! - [`par_chunks_mut`] spreads the encoders' and decoders' work over the machine's threads.

use std::io;

/// How a block is stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Codec {
    /// As is.
    Raw = 0,
    /// LZ4's block format.
    Lz4 = 1,
    /// LZ4 over the bytes regrouped by their position in 16-byte records ([`planes`]).
    Lz4Planes16 = 2,
}

impl Codec {
    /// The codec stored as `byte`.
    pub fn from_byte(byte: u8) -> io::Result<Self> {
        match byte {
            0 => Ok(Self::Raw),
            1 => Ok(Self::Lz4),
            2 => Ok(Self::Lz4Planes16),
            _ => Err(invalid("a packed block of an unknown codec")),
        }
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// The record width of [`Codec::Lz4Planes16`].
const RECORD: usize = 16;

/// `bytes` regrouped by their position in records of `width` bytes: every record's first byte,
/// then every second byte, and so on. A tail shorter than a record stays as it is, at the end.
#[must_use]
pub fn planes(bytes: &[u8], width: usize) -> Vec<u8> {
    let records = bytes.len() / width;
    let mut out = vec![0; bytes.len()];
    for (r, record) in bytes.chunks_exact(width).enumerate() {
        for (b, &byte) in record.iter().enumerate() {
            out[b * records + r] = byte;
        }
    }
    out[records * width..].copy_from_slice(&bytes[records * width..]);
    out
}

/// The bytes [`planes`] regrouped, put back in their records, into `out` (as long).
pub fn unplanes(planes: &[u8], width: usize, out: &mut [u8]) {
    let records = out.len() / width;
    // Sixteen records at a time: sixteen bytes read in a row from each plane, then written
    // out record by record, so neither side strides a byte at a time across the block.
    const STEP: usize = 16;
    let mut block = [[0_u8; STEP]; 64];
    let whole = if width <= block.len() {
        records / STEP * STEP
    } else {
        0
    };
    for r0 in (0..whole).step_by(STEP) {
        for (b, row) in block[..width].iter_mut().enumerate() {
            row.copy_from_slice(&planes[b * records + r0..][..STEP]);
        }
        for (r, record) in out[r0 * width..(r0 + STEP) * width]
            .chunks_exact_mut(width)
            .enumerate()
        {
            for (byte, row) in record.iter_mut().zip(&block[..width]) {
                *byte = row[r];
            }
        }
    }
    for (r, record) in out.chunks_exact_mut(width).enumerate().skip(whole) {
        for (b, byte) in record.iter_mut().enumerate() {
            *byte = planes[b * records + r];
        }
    }
    out[records * width..].copy_from_slice(&planes[records * width..]);
}

/// `raw` packed: as LZ4, or with `planes16` as LZ4 over its 16-byte planes when that is
/// smaller, or as is when neither is smaller.
#[must_use]
pub fn pack(raw: &[u8], planes16: bool) -> (Codec, Vec<u8>) {
    let mut best = (Codec::Lz4, lz4_flex::block::compress(raw));
    if planes16 {
        let regrouped = lz4_flex::block::compress(&planes(raw, RECORD));
        if regrouped.len() < best.1.len() {
            best = (Codec::Lz4Planes16, regrouped);
        }
    }
    if best.1.len() >= raw.len() {
        best = (Codec::Raw, raw.to_vec());
    }
    best
}

/// Unpacks `packed`, stored with `codec`, into `out`, which must be exactly as long as what
/// was packed.
pub fn unpack(codec: Codec, packed: &[u8], out: &mut [u8]) -> io::Result<()> {
    let lz4 = |packed: &[u8], out: &mut [u8]| match lz4_flex::block::decompress_into(packed, out) {
        Ok(n) if n == out.len() => Ok(()),
        Ok(_) => Err(invalid("a packed block shorter than its length")),
        Err(error) => Err(io::Error::new(io::ErrorKind::InvalidData, error)),
    };
    match codec {
        Codec::Raw if packed.len() == out.len() => {
            out.copy_from_slice(packed);
            Ok(())
        }
        Codec::Raw => Err(invalid("a raw block of another length")),
        Codec::Lz4 => lz4(packed, out),
        Codec::Lz4Planes16 => {
            let mut regrouped = vec![0; out.len()];
            lz4(packed, &mut regrouped)?;
            unplanes(&regrouped, RECORD, out);
            Ok(())
        }
    }
}

/// Threads the parallel loops use: the machine's, less two for the main and render threads.
/// [`par_chunks_mut`] spreads its chunks over this many.
pub fn threads() -> usize {
    std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .saturating_sub(2)
        .max(1)
}

/// Calls `f(index, chunk)` for every chunk of `chunk_len` elements of `data`, the chunks
/// spread over the machine's threads, at least `chunks_per_thread` to a thread (a thread costs
/// tens of microseconds to start: a few small chunks run on the caller's). What it makes
/// depends on the chunks only, never on the threads.
pub fn par_chunks_mut<T: Send>(
    data: &mut [T],
    chunk_len: usize,
    chunks_per_thread: usize,
    f: impl Fn(usize, &mut [T]) + Sync,
) {
    let chunk_len = chunk_len.max(1);
    let chunks = data.len().div_ceil(chunk_len);
    let threads = threads().min(chunks / chunks_per_thread.max(1));
    if threads <= 1 {
        data.chunks_mut(chunk_len)
            .enumerate()
            .for_each(|(i, chunk)| f(i, chunk));
        return;
    }
    let per = chunks.div_ceil(threads);
    let f = &f;
    std::thread::scope(|scope| {
        for (t, group) in data.chunks_mut(per * chunk_len).enumerate() {
            scope.spawn(move || {
                for (i, chunk) in group.chunks_mut(chunk_len).enumerate() {
                    f(t * per + i, chunk);
                }
            });
        }
    });
}

/// A sample's bits as an integer ordered as the numbers are (negative ones below positive).
fn ordered(value: f32) -> u32 {
    let bits = value.to_bits();
    if bits & 0x8000_0000 != 0 {
        !bits
    } else {
        bits | 0x8000_0000
    }
}

/// The sample [`ordered`] made `key` from.
fn from_ordered(key: u32) -> f32 {
    f32::from_bits(if key & 0x8000_0000 != 0 {
        key & 0x7fff_ffff
    } else {
        !key
    })
}

/// Rows a band of a grid `width` samples wide holds: about a megabyte of differences.
fn band_rows(width: usize) -> usize {
    (1_usize << 18).div_ceil(width.max(1))
}

/// The gradient predictor at index `i` of a band `width` wide, from the band's samples before
/// it (`key(j)` for `j < i`): only the left one on the band's first row, only the upper one at
/// the start of a row.
#[inline]
fn predict(i: usize, width: usize, key: impl Fn(usize) -> u32) -> u32 {
    match (!i.is_multiple_of(width), i >= width) {
        (true, true) => key(i - 1)
            .wrapping_add(key(i - width))
            .wrapping_sub(key(i - width - 1)),
        (false, true) => key(i - width),
        (true, false) => key(i - 1),
        (false, false) => 0,
    }
}

/// The bytes [`decode_grid`] reads back, as many as `values` has: the grid of `values`, row
/// by row `width` samples wide (the last row may be short), coded band by band.
#[must_use]
pub fn encode_grid(values: &[f32], width: usize) -> Vec<u8> {
    let width = width.max(1);
    let band = band_rows(width) * width;
    let mut out = vec![0; values.len() * 4];
    par_chunks_mut(&mut out, band * 4, 1, |b, planes| {
        let samples = &values[b * band..b * band + planes.len() / 4];
        let n = samples.len();
        for i in 0..n {
            let key = ordered(samples[i]);
            let difference = key.wrapping_sub(predict(i, width, |j| ordered(samples[j]))) as i32;
            // Zigzag: small differences of either sign become small numbers.
            let zigzag = ((difference << 1) ^ (difference >> 31)) as u32;
            for (plane, byte) in zigzag.to_le_bytes().into_iter().enumerate() {
                planes[plane * n + i] = byte;
            }
        }
    });
    out
}

/// The grid [`encode_grid`] coded into `bytes`, `width` samples wide, into `out` (a quarter of
/// their length).
pub fn decode_grid(bytes: &[u8], width: usize, out: &mut [f32]) -> io::Result<()> {
    if bytes.len() != out.len() * 4 {
        return Err(invalid("a coded grid of another length"));
    }
    let width = width.max(1);
    let band = band_rows(width) * width;
    par_chunks_mut(out, band, 1, |b, samples| {
        let n = samples.len();
        let planes = &bytes[b * band * 4..(b * band + n) * 4];
        for i in 0..n {
            let zigzag = u32::from_le_bytes([
                planes[i],
                planes[n + i],
                planes[2 * n + i],
                planes[3 * n + i],
            ]);
            let difference = (zigzag >> 1) ^ (zigzag & 1).wrapping_neg();
            let predicted = predict(i, width, |j| ordered(samples[j]));
            samples[i] = from_ordered(difference.wrapping_add(predicted));
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_come_back_whole_in_every_codec() {
        let repeats: Vec<u8> = (0..100_000_u32).map(|i| (i / 7 % 13) as u8).collect();
        let records: Vec<u8> = (0..4096_u32)
            .flat_map(|i| {
                [
                    i.to_le_bytes(),
                    7_u32.to_le_bytes(),
                    (i * 3).to_le_bytes(),
                    [1; 4],
                ]
            })
            .flatten()
            .collect();
        let mut rng = crate::seed::SplitMix64::new(9);
        let noise: Vec<u8> = (0..5000).map(|_| (rng.next_u64() >> 56) as u8).collect();
        for (raw, codec) in [
            (&repeats, Codec::Lz4),
            (&records, Codec::Lz4Planes16),
            (&noise, Codec::Raw),
        ] {
            let (got, packed) = pack(raw, true);
            assert_eq!(got, codec);
            let mut out = vec![0; raw.len()];
            unpack(got, &packed, &mut out).unwrap();
            assert_eq!(&out, raw);
            // A wrong length is refused, never filled with something else.
            let mut short = vec![0; raw.len() - 1];
            assert!(unpack(got, &packed, &mut short).is_err());
        }
        assert!(Codec::from_byte(3).is_err());
        // Planes with a tail shorter than a record.
        let odd: Vec<u8> = (0..37).collect();
        let mut back = vec![0; odd.len()];
        unplanes(&planes(&odd, 16), 16, &mut back);
        assert_eq!(back, odd);
    }

    #[test]
    fn grids_come_back_to_the_bit() {
        let width = 301;
        let mut values: Vec<f32> = (0..width * 1000 + 17)
            .map(|i| {
                let (x, y) = ((i % width) as f32, (i / width) as f32);
                (x * 0.37).sin() * 40.0 - y * 0.02
            })
            .collect();
        values[5] = -0.0;
        values[6] = f32::INFINITY;
        values[7] = f32::NEG_INFINITY;
        values[8] = f32::NAN;
        values[9] = f32::MIN_POSITIVE / 2.0;
        for width in [width, 1, 7] {
            let coded = encode_grid(&values, width);
            let mut back = vec![0.0; values.len()];
            decode_grid(&coded, width, &mut back).unwrap();
            assert!(
                back.iter()
                    .zip(&values)
                    .all(|(a, b)| a.to_bits() == b.to_bits())
            );
        }
        // The differences of a smooth grid are small: LZ4 over them beats LZ4 over the floats.
        let coded = lz4_flex::block::compress(&encode_grid(&values, width)).len();
        let floats = lz4_flex::block::compress(bytemuck::cast_slice(&values)).len();
        assert!(coded * 2 < floats, "{coded} against {floats}");
        assert!(decode_grid(&[0; 7], width, &mut [0.0; 2]).is_err());
    }

    #[test]
    fn chunks_are_the_same_on_any_thread_count() {
        let mut data = vec![0_u64; 10_001];
        par_chunks_mut(&mut data, 64, 1, |i, chunk| {
            for (k, v) in chunk.iter_mut().enumerate() {
                *v = (i * 64 + k) as u64;
            }
        });
        assert!(data.iter().enumerate().all(|(i, &v)| v == i as u64));
    }
}
