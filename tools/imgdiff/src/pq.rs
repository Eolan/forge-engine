//! The HDR10 captures' PQ codes as light, for HDR-ꟻLIP (issue #126).
//!
//! A capture's 16-bit `-pq.png` (issue #94) holds the Rec.2100 PQ signal in Rec.2020
//! primaries, each 10-bit code widened to 16 bits (`c << 6 | c >> 4`, `forge-app`'s
//! `a2b10g10r10_to_rgb16`). Decoded here to linear Rec.709 with 1.0 = 100 nits, the white of
//! an SDR display: what ꟻLIP's colour space expects. Colours outside Rec.709 lose their
//! negative channels, as an sRGB display would show them.

use crate::flip::Color;

/// The nits of linear 1.0.
pub const NITS_PER_UNIT: f64 = 100.0;

// SMPTE ST 2084, as `forge_render::aces2::pq_decode`.
const M1: f64 = 2610.0 / 16384.0;
const M2: f64 = 2523.0 / 4096.0 * 128.0;
const C1: f64 = 3424.0 / 4096.0;
const C2: f64 = 2413.0 / 4096.0 * 32.0;
const C3: f64 = 2392.0 / 4096.0 * 32.0;

/// Rec.2020 to Rec.709, as `forge_render::display::REC2020_TO_REC709` (ITU-R BT.2087).
const REC2020_TO_REC709: [[f64; 3]; 3] = [
    [1.660_491, -0.587_641_1, -0.072_849_86],
    [-0.124_550_47, 1.132_899_9, -0.008_349_42],
    [-0.018_150_76, -0.100_578_9, 1.118_729_7],
];

/// The nits of a PQ signal in [0, 1].
pub fn decode(signal: f64) -> f64 {
    let e = signal.clamp(0.0, 1.0).powf(1.0 / M2);
    ((e - C1).max(0.0) / (C2 - C3 * e)).powf(1.0 / M1) * 10_000.0
}

/// A 16-bit PQ capture as linear Rec.709 light, 1.0 = [`NITS_PER_UNIT`].
pub fn to_linear(image: &image::ImageBuffer<image::Rgba<u16>, Vec<u16>>) -> Vec<Color> {
    let nits: Vec<f64> = (0..1024)
        .map(|code| decode(f64::from(code) / 1023.0))
        .collect();
    image
        .pixels()
        .map(|p| {
            let rec2020 = [0, 1, 2].map(|c| nits[usize::from(p[c] >> 6)]);
            REC2020_TO_REC709.map(|row| {
                let v = row[0] * rec2020[0] + row[1] * rec2020[1] + row[2] * rec2020[2];
                (v.max(0.0) / NITS_PER_UNIT) as f32
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(nits: f64) -> f64 {
        let p = (nits / 10_000.0).powf(M1);
        ((C1 + C2 * p) / (1.0 + C3 * p)).powf(M2)
    }

    #[test]
    fn pq_decodes_st_2084() {
        assert_eq!(decode(0.0), 0.0);
        assert!((decode(1.0) - 10_000.0).abs() < 1e-6);
        // BT.2100's reference white and BT.2408's graphics white.
        for nits in [100.0, 203.0, 1000.0] {
            assert!((decode(encode(nits)) - nits).abs() < 1e-9 * nits, "{nits}");
        }
        assert!(
            (encode(203.0) - 0.58).abs() < 0.001,
            "BT.2408: 203 nits at 58 %"
        );
    }

    #[test]
    fn a_capture_decodes_to_rec_709() {
        let widen = |c: u16| (c << 6) | (c >> 4);
        // White at 203 nits, and Rec.2020's pure green (outside Rec.709).
        let white = widen((encode(203.0) * 1023.0).round() as u16);
        let green = widen((encode(100.0) * 1023.0).round() as u16);
        let image = image::ImageBuffer::from_vec(
            2,
            1,
            vec![white, white, white, u16::MAX, 0, green, 0, u16::MAX],
        )
        .unwrap();
        let linear = to_linear(&image);
        for c in linear[0] {
            assert!((f64::from(c) - 2.03).abs() < 0.01, "{:?}", linear[0]);
        }
        assert_eq!(linear[1][0], 0.0, "red below zero: clipped");
        assert!(linear[1][1] > 1.0, "{:?}", linear[1]);
    }
}
