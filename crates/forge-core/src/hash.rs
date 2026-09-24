//! Position hashes, specified to the bit, shared by CPU and GPU code.
//!
//! `pcg3d` / `pcg4d` are the multi-dimensional hashes recommended by Jarzynski & Olano, "Hash
//! Functions for GPU Rendering" (JCGT 2020) — see `docs/RESEARCH.md` §3. They are written the
//! same way in the Slang shaders so a grass blade or a scatter cell hashes identically on both
//! sides. The 64-bit mixers are the SplitMix64 finaliser (Stafford's mix13).

/// PCG3D: three 32-bit inputs to three 32-bit outputs, GPU-friendly, passes TestU01.
#[inline]
#[must_use]
pub fn pcg3d(mut v: [u32; 3]) -> [u32; 3] {
    for x in &mut v {
        *x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    }
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    for x in &mut v {
        *x ^= *x >> 16;
    }
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[2]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v
}

/// PCG4D: four 32-bit inputs to four 32-bit outputs.
#[inline]
#[must_use]
pub fn pcg4d(mut v: [u32; 4]) -> [u32; 4] {
    for x in &mut v {
        *x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    }
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[3]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v[3] = v[3].wrapping_add(v[1].wrapping_mul(v[2]));
    for x in &mut v {
        *x ^= *x >> 16;
    }
    v[0] = v[0].wrapping_add(v[1].wrapping_mul(v[3]));
    v[1] = v[1].wrapping_add(v[2].wrapping_mul(v[0]));
    v[2] = v[2].wrapping_add(v[0].wrapping_mul(v[1]));
    v[3] = v[3].wrapping_add(v[1].wrapping_mul(v[2]));
    v
}

/// 64-bit mixer (SplitMix64 finaliser). Bijective, good avalanche.
#[inline]
#[must_use]
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash of a seed and a 2-D integer cell, for scatter and noise lattices.
#[inline]
#[must_use]
pub fn hash_cell2(seed: u64, x: i32, y: i32) -> u64 {
    let [a, b, c] = pcg3d([x as u32, y as u32, seed as u32 ^ (seed >> 32) as u32]);
    mix64((u64::from(a) << 32) | u64::from(b) ^ (u64::from(c) << 7))
}

/// Hash of a seed and a 3-D integer cell.
#[inline]
#[must_use]
pub fn hash_cell3(seed: u64, x: i32, y: i32, z: i32) -> u64 {
    let [a, b, c, _] = pcg4d([
        x as u32,
        y as u32,
        z as u32,
        seed as u32 ^ (seed >> 32) as u32,
    ]);
    mix64((u64::from(a) << 32) | u64::from(b) ^ (u64::from(c) << 7))
}

/// Maps 64 random bits to a float uniform in `[0, 1)`.
#[inline]
#[must_use]
pub fn unit_f32(bits: u64) -> f32 {
    (bits >> 40) as f32 * (1.0 / (1_u32 << 24) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcg3d_is_deterministic_and_sensitive() {
        assert_eq!(pcg3d([1, 2, 3]), pcg3d([1, 2, 3]));
        assert_ne!(pcg3d([1, 2, 3]), pcg3d([1, 2, 4]));
        assert_ne!(pcg3d([0, 0, 0]), [0, 0, 0]);
    }

    #[test]
    fn cell_hashes_differ_between_neighbours_and_seeds() {
        assert_ne!(hash_cell2(1, 0, 0), hash_cell2(1, 1, 0));
        assert_ne!(hash_cell2(1, 0, 0), hash_cell2(2, 0, 0));
        assert_ne!(hash_cell3(1, 0, 0, 0), hash_cell3(1, 0, 0, 1));
    }

    #[test]
    fn unit_float_is_in_range() {
        for i in 0..10_000_u64 {
            let f = unit_f32(mix64(i));
            assert!((0.0..1.0).contains(&f));
        }
    }
}
