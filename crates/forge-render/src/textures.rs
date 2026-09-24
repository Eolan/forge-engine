//! Procedural surface textures (issue #20): tileable albedo and normal maps generated from a
//! seed at start-up, with their mip chains, for the material table's triplanar projection.
//!
//! Every texture is square, a power of two and periodic: the noise lattices wrap at the
//! texture's edge, so a repeat shows no seam. Albedo is stored in sRGB (its mips are averaged
//! in linear light), normals in tangent space as `xyz * 0.5 + 0.5` (their mips average the
//! vectors and renormalise them).

use forge_core::hash::{hash_cell2, unit_f32};

/// A texture's pixels: RGBA8 level after level, level 0 first, down to 1 × 1.
#[derive(Clone, Debug)]
pub struct TextureData {
    /// A name for the debugger and the logs.
    pub name: &'static str,
    /// Level 0's side in pixels.
    pub size: u32,
    /// sRGB-encoded colour (albedo), else linear data (normals).
    pub srgb: bool,
    /// Every level's RGBA8 pixels, rows top to bottom.
    pub levels: Vec<Vec<u8>>,
}

/// Value noise that repeats every `period` lattice cells, at `(x, y)` in cells.
fn periodic_noise(seed: u64, x: f32, y: f32, period: u32) -> f32 {
    let (fx, fy) = (x.floor(), y.floor());
    let (tx, ty) = (x - fx, y - fy);
    // Quintic fade: continuous second derivative, so the normals derived from it are smooth.
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (sx, sy) = (fade(tx), fade(ty));
    let p = period as i32;
    let lattice = |i: i32, j: i32| {
        unit_f32(hash_cell2(
            seed,
            (fx as i32 + i).rem_euclid(p),
            (fy as i32 + j).rem_euclid(p),
        ))
    };
    let top = lattice(0, 0) + (lattice(1, 0) - lattice(0, 0)) * sx;
    let bottom = lattice(0, 1) + (lattice(1, 1) - lattice(0, 1)) * sx;
    top + (bottom - top) * sy
}

/// Fractal sum of `octaves` of periodic noise at texture coordinates `(u, v)` in [0, 1): the
/// first octave has `period` cells across the texture, each next one twice as many.
fn fbm(seed: u64, u: f32, v: f32, period: u32, octaves: u32) -> f32 {
    let (mut sum, mut norm, mut amplitude) = (0.0, 0.0, 0.5);
    for octave in 0..octaves {
        let cells = period << octave;
        let s = seed.wrapping_add(u64::from(octave) * 0x9E37_79B9);
        sum += amplitude * periodic_noise(s, u * cells as f32, v * cells as f32, cells);
        norm += amplitude;
        amplitude *= 0.5;
    }
    sum / norm
}

#[cfg(test)]
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn to_byte(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Samples `f(u, v)` at every texel centre of a `size × size` grid.
fn grid<T>(size: u32, f: impl Fn(f32, f32) -> T) -> Vec<T> {
    let mut out = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            out.push(f(
                (x as f32 + 0.5) / size as f32,
                (y as f32 + 0.5) / size as f32,
            ));
        }
    }
    out
}

/// An albedo texture from linear colours, with its mips averaged in linear light.
fn albedo_texture(name: &'static str, size: u32, linear: Vec<[f32; 3]>) -> TextureData {
    let encode = |level: &[[f32; 3]]| {
        level
            .iter()
            .flat_map(|c| {
                [
                    to_byte(linear_to_srgb(c[0])),
                    to_byte(linear_to_srgb(c[1])),
                    to_byte(linear_to_srgb(c[2])),
                    255,
                ]
            })
            .collect::<Vec<u8>>()
    };
    let mut levels = vec![encode(&linear)];
    let (mut current, mut side) = (linear, size);
    while side > 1 {
        let half = side / 2;
        current = downsample(&current, side, |a, b, c, d| {
            [0, 1, 2].map(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25)
        });
        side = half;
        levels.push(encode(&current));
    }
    TextureData {
        name,
        size,
        srgb: true,
        levels,
    }
}

/// A tangent-space normal map from a periodic height field (`heights`, in texels of height
/// per texel of distance times `strength`).
fn normal_texture(name: &'static str, size: u32, heights: &[f32], strength: f32) -> TextureData {
    let at = |x: i64, y: i64| {
        let s = i64::from(size);
        heights[(y.rem_euclid(s) * s + x.rem_euclid(s)) as usize]
    };
    let mut normals = Vec::with_capacity(heights.len());
    for y in 0..i64::from(size) {
        for x in 0..i64::from(size) {
            // Tangent x runs along +u and y along +v (down the rows), the axes the triplanar
            // projection maps to object space.
            let dx = (at(x + 1, y) - at(x - 1, y)) * 0.5 * strength;
            let dy = (at(x, y + 1) - at(x, y - 1)) * 0.5 * strength;
            normals.push(normalize([-dx, -dy, 1.0]));
        }
    }
    let encode = |level: &[[f32; 3]]| {
        level
            .iter()
            .flat_map(|n| {
                [
                    to_byte(n[0] * 0.5 + 0.5),
                    to_byte(n[1] * 0.5 + 0.5),
                    to_byte(n[2] * 0.5 + 0.5),
                    255,
                ]
            })
            .collect::<Vec<u8>>()
    };
    let mut levels = vec![encode(&normals)];
    let (mut current, mut side) = (normals, size);
    while side > 1 {
        current = downsample(&current, side, |a, b, c, d| {
            normalize([0, 1, 2].map(|k| a[k] + b[k] + c[k] + d[k]))
        });
        side /= 2;
        levels.push(encode(&current));
    }
    TextureData {
        name,
        size,
        srgb: false,
        levels,
    }
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length > 0.0 {
        v.map(|c| c / length)
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// Halves a `side × side` level, each texel from its 2 × 2 block.
fn downsample<T: Copy>(level: &[T], side: u32, f: impl Fn(T, T, T, T) -> T) -> Vec<T> {
    let half = side / 2;
    let s = side as usize;
    let mut out = Vec::with_capacity((half * half) as usize);
    for y in 0..half as usize {
        for x in 0..half as usize {
            let i = 2 * y * s + 2 * x;
            out.push(f(level[i], level[i + 1], level[i + s], level[i + s + 1]));
        }
    }
    out
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Bare rock: mottled grey with dark crevices; the relief of weathered stone.
pub fn rock(seed: u64, size: u32) -> [TextureData; 2] {
    let heights = grid(size, |u, v| {
        let base = fbm(seed, u, v, 4, 6);
        let fine = fbm(seed ^ 0xF1E, u, v, 32, 3);
        base * 0.8 + fine * 0.2
    });
    let colours = grid(size, |u, v| {
        let mottle = fbm(seed ^ 0xC01, u, v, 8, 5);
        let grain = fbm(seed ^ 0x6A1, u, v, 64, 2);
        let base = fbm(seed, u, v, 4, 6);
        let crevice = smoothstep(0.30, 0.55, base);
        let grey = 0.30 + 0.25 * mottle + 0.08 * (grain - 0.5);
        let c = grey * (0.55 + 0.45 * crevice);
        [c, c * 0.98, c * 0.95]
    });
    [
        albedo_texture("rock albedo", size, colours),
        normal_texture("rock normal", size, &heights, size as f32 / 24.0),
    ]
}

/// Cast concrete: light grey with stains, pores and faint formwork lines.
pub fn concrete(seed: u64, size: u32) -> [TextureData; 2] {
    let heights = grid(size, |u, v| {
        let pores = fbm(seed ^ 0x9E, u, v, 64, 2);
        let pits = smoothstep(0.72, 0.8, pores);
        0.5 * fbm(seed, u, v, 16, 3) - pits
    });
    let colours = grid(size, |u, v| {
        let stain = fbm(seed ^ 0x57A, u, v, 3, 5);
        let speck = fbm(seed ^ 0x5EC, u, v, 128, 1);
        // Formwork panels: a darker line every quarter of the texture.
        let line =
            (1.0 - smoothstep(0.0, 0.004, (v * 4.0).fract().min(1.0 - (v * 4.0).fract()))) * 0.12;
        let c = (0.42 + 0.18 * stain + 0.06 * (speck - 0.5)) * (1.0 - line);
        [c, c * 0.99, c * 0.97]
    });
    [
        albedo_texture("concrete albedo", size, colours),
        normal_texture("concrete normal", size, &heights, size as f32 / 256.0),
    ]
}

/// Bricks in running bond, 8 across and 16 rows per repeat: fired clay in varied reds and
/// browns, recessed grey mortar.
pub fn brick(seed: u64, size: u32) -> [TextureData; 2] {
    const ACROSS: f32 = 8.0;
    const ROWS: f32 = 16.0;
    const MORTAR: f32 = 0.06; // of a brick's height
    // Where (u, v) falls: its brick's cell and how far inside the brick it is (0 at the
    // mortar's centre line, 1 well inside).
    let cell = |u: f32, v: f32| {
        let row = (v * ROWS).floor();
        let shift = if row as i32 % 2 == 0 { 0.0 } else { 0.5 };
        let x = u * ACROSS + shift;
        let col = x.floor().rem_euclid(ACROSS);
        let (fx, fy) = (x.fract(), (v * ROWS).fract());
        // Distance to the nearest joint in brick heights (a brick is twice as long as tall).
        let edge = (fx.min(1.0 - fx) * 2.0 * ROWS / ACROSS * 0.5).min(fy.min(1.0 - fy));
        (col as i32, row as i32, edge)
    };
    let heights = grid(size, |u, v| {
        let (_, _, edge) = cell(u, v);
        let inset = smoothstep(MORTAR * 0.5, MORTAR, edge);
        inset * 0.8 + 0.2 * fbm(seed ^ 0xB1, u, v, 32, 3)
    });
    let colours = grid(size, |u, v| {
        let (col, row, edge) = cell(u, v);
        let brick = unit_f32(hash_cell2(seed, col, row));
        let tone = unit_f32(hash_cell2(seed ^ 0x70E, col, row));
        let clay = mix3([0.30, 0.09, 0.05], [0.36, 0.17, 0.09], brick);
        let clay = mix3(clay, [0.20, 0.08, 0.05], tone * tone * 0.6);
        let grit = 0.85 + 0.3 * fbm(seed ^ 0x6217, u, v, 64, 2);
        let clay = clay.map(|c| c * grit);
        let mortar = [0.40, 0.38, 0.35].map(|c| c * (0.9 + 0.2 * fbm(seed ^ 0x3A, u, v, 32, 2)));
        mix3(mortar, clay, smoothstep(MORTAR * 0.5, MORTAR, edge))
    });
    [
        albedo_texture("brick albedo", size, colours),
        normal_texture("brick normal", size, &heights, size as f32 / 64.0),
    ]
}

/// Ground: short grass with patches of bare soil.
pub fn grass(seed: u64, size: u32) -> [TextureData; 2] {
    let heights = grid(size, |u, v| fbm(seed ^ 0x6A55, u, v, 32, 4));
    let colours = grid(size, |u, v| {
        let patch = smoothstep(0.6, 0.78, fbm(seed, u, v, 4, 5)) * 0.3;
        let blade = fbm(seed ^ 0xB1AD, u, v, 128, 2);
        let tint = fbm(seed ^ 0x7147, u, v, 8, 3);
        let grass = mix3([0.05, 0.09, 0.025], [0.10, 0.13, 0.04], tint);
        let grass = grass.map(|c| c * (0.7 + 0.6 * blade));
        let soil = [0.13, 0.10, 0.07].map(|c| c * (0.8 + 0.4 * blade));
        mix3(grass, soil, patch)
    });
    [
        albedo_texture("grass albedo", size, colours),
        normal_texture("grass normal", size, &heights, size as f32 / 96.0),
    ]
}

/// A test texture for the mip check (`--mip-check`): level `k` is filled with `k / 16`, so a
/// trilinear sample returns the level of detail the sampler chose, over 16.
pub fn mip_ramp(size: u32) -> TextureData {
    let mut levels = Vec::new();
    let mut side = size;
    let mut level = 0_u32;
    loop {
        let value = to_byte(level as f32 / 16.0);
        levels.push(
            std::iter::repeat_n([value, value, value, 255], (side * side) as usize)
                .flatten()
                .collect(),
        );
        if side == 1 {
            break;
        }
        side /= 2;
        level += 1;
    }
    TextureData {
        name: "mip ramp",
        size,
        srgb: false,
        levels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_repeats_at_its_period() {
        for (x, y) in [(0.3_f32, 0.7_f32), (2.5, 3.25), (7.9, 0.1)] {
            let a = periodic_noise(7, x, y, 8);
            // The shifted coordinate rounds differently in its last bits.
            assert!((a - periodic_noise(7, x + 8.0, y, 8)).abs() < 1e-5);
            assert!((a - periodic_noise(7, x, y - 8.0, 8)).abs() < 1e-5);
        }
        // And fbm over the texture: u = 0 and u = 1 meet.
        let near_edge = fbm(3, 0.999_99, 0.4, 4, 5);
        let wrapped = fbm(3, -0.000_01, 0.4, 4, 5);
        assert!((near_edge - wrapped).abs() < 1e-4);
    }

    #[test]
    fn every_texture_has_its_full_mip_chain() {
        for texture in rock(1, 64)
            .into_iter()
            .chain(concrete(1, 64))
            .chain(brick(1, 64))
            .chain(grass(1, 64))
            .chain([mip_ramp(64)])
        {
            assert_eq!(texture.levels.len(), 7, "{}", texture.name);
            for (k, level) in texture.levels.iter().enumerate() {
                let side = 64 >> k;
                assert_eq!(level.len(), side * side * 4, "{} level {k}", texture.name);
            }
        }
    }

    #[test]
    fn normals_are_unit_and_face_out() {
        let [_, normal] = rock(5, 32);
        for texel in normal.levels[0].chunks(4) {
            let n = [0, 1, 2].map(|k| texel[k] as f32 / 255.0 * 2.0 - 1.0);
            let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            assert!((length - 1.0).abs() < 0.02, "length {length}");
            assert!(n[2] > 0.0);
        }
    }

    #[test]
    fn albedo_mips_keep_the_mean_in_linear_light() {
        let [albedo, _] = brick(9, 64);
        let mean = |level: &[u8]| {
            level
                .chunks(4)
                .map(|t| srgb_to_linear(t[0] as f32 / 255.0))
                .sum::<f32>()
                / (level.len() / 4) as f32
        };
        let top = mean(&albedo.levels[0]);
        let last = mean(albedo.levels.last().unwrap());
        assert!((top - last).abs() < 0.01, "{top} vs {last}");
    }

    #[test]
    fn the_ramp_holds_its_level_in_every_level() {
        let ramp = mip_ramp(16);
        for (k, level) in ramp.levels.iter().enumerate() {
            assert_eq!(level[0], to_byte(k as f32 / 16.0));
        }
    }
}
