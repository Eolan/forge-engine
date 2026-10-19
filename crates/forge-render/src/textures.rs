//! Procedural surface textures (issue #20): tileable albedo and normal maps generated from a
//! seed at start-up, with their mip chains, for the material table's triplanar projection.
//!
//! Every texture is square, a power of two and periodic: the noise lattices wrap at the
//! texture's edge, so a repeat shows no seam. Albedo is stored in sRGB (its mips are averaged
//! in linear light), normals in tangent space as `xyz * 0.5 + 0.5` (their mips average the
//! vectors and renormalise them).

use std::fmt::Debug;
use std::io;
use std::sync::OnceLock;

use forge_core::derived::{DerivedCache, KeyHasher, Sink, Source, Stored};
use forge_core::hash::{hash_cell2, unit_f32};
use forge_core::pack::par_chunks_mut;

/// Where a program keeps the generated textures between its starts ([`keep_in`]).
static KEPT: OnceLock<DerivedCache> = OnceLock::new();

/// Keeps the generated textures in `cache` from now on, each under its generator, its inputs and
/// a digest of the code that makes it: the next start reads them back instead of making them
/// again (#274: 0.4 s of the village's 1.6). Only the first call counts; without one (the
/// tests), every texture is made.
pub fn keep_in(cache: DerivedCache) {
    let _ = KEPT.set(cache);
}

/// `make`'s product made by other code than this file's (`code`, a digest of its source): from
/// the cache [`keep_in`] set when it is in it, else made and stored (the smoke's flipbook,
/// [`crate::smoke_bake::flipbook`]).
pub(crate) fn kept_by<T: Stored>(
    product: &str,
    inputs: &impl Debug,
    code: u64,
    make: impl FnOnce() -> T,
) -> T {
    let Some(cache) = KEPT.get() else {
        return make();
    };
    cache
        .get_or_make(product, KeyHasher::new().debug(inputs).key(code), make)
        .value
}

/// `make`'s textures, from the cache [`keep_in`] set when they are in it, else made and stored.
fn kept<T: Stored>(product: &str, inputs: &impl Debug, make: impl FnOnce() -> T) -> T {
    let Some(cache) = KEPT.get() else {
        return make();
    };
    // This file and the hashes it draws its noise from, their line endings left out.
    static CODE: OnceLock<u64> = OnceLock::new();
    let code = *CODE.get_or_init(|| {
        [
            include_str!("textures.rs"),
            include_str!("../../forge-core/src/hash.rs"),
        ]
        .iter()
        .flat_map(|text| text.bytes())
        .filter(|&b| b != b'\r')
        .fold(0xcbf2_9ce4_8422_2325, |h: u64, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    });
    cache
        .get_or_make(product, KeyHasher::new().debug(inputs).key(code), make)
        .value
}

impl Stored for TextureData {
    fn put(&self, sink: &mut Sink<'_>) -> io::Result<()> {
        self.name.to_string().put(sink)?;
        self.size.put(sink)?;
        self.height.put(sink)?;
        self.srgb.put(sink)?;
        self.levels.put(sink)
    }

    fn take(source: &mut Source<'_>) -> io::Result<Self> {
        Ok(Self {
            name: String::take(source)?.into(),
            size: u32::take(source)?,
            height: u32::take(source)?,
            srgb: bool::take(source)?,
            levels: Vec::take(source)?,
        })
    }
}

/// A texture's pixels: RGBA8 level after level, level 0 first, down to 1 × 1.
#[derive(Clone, Debug)]
pub struct TextureData {
    /// A name for the debugger and the logs.
    pub name: std::borrow::Cow<'static, str>,
    /// Level 0's width in pixels.
    pub size: u32,
    /// Level 0's height in pixels (`size` but for a model's images, #166).
    pub height: u32,
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

/// The fewest rows of a generated level a thread takes (a thread costs tens of microseconds to
/// start).
const ROWS_PER_THREAD: usize = 16;
/// Texels a chunk of [`encode`]'s parallel work.
const ENCODE_CHUNK: usize = 1 << 15;

/// `f(x, y)` for every texel of a level `width` texels wide and `count` texels in all, the rows
/// spread over the threads: each texel's value depends on it alone, never on the threads (the
/// generated textures took 1.4 s of one thread at the village's start).
fn texels<T: Send + Default + Clone>(
    width: usize,
    count: usize,
    f: impl Fn(usize, usize) -> T + Sync,
) -> Vec<T> {
    let mut out = vec![T::default(); count];
    par_chunks_mut(&mut out, width.max(1), ROWS_PER_THREAD, |y, row| {
        for (x, texel) in row.iter_mut().enumerate() {
            *texel = f(x, y);
        }
    });
    out
}

/// A level's RGBA8 bytes, `f` encoding each texel, in chunks the threads share.
fn encode<T: Sync>(level: &[T], f: impl Fn(&T) -> [u8; 4] + Sync) -> Vec<u8> {
    let mut out = vec![0u8; level.len() * 4];
    par_chunks_mut(&mut out, 4 * ENCODE_CHUNK, 1, |i, chunk| {
        for (k, texel) in chunk.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *texel = f(&level[i * ENCODE_CHUNK + k]);
        }
    });
    out
}

/// Samples `f(u, v)` at every texel centre of a `size × size` grid.
fn grid<T: Send + Default + Clone>(size: u32, f: impl Fn(f32, f32) -> T + Sync) -> Vec<T> {
    texels(size as usize, (size * size) as usize, |x, y| {
        f(
            (x as f32 + 0.5) / size as f32,
            (y as f32 + 0.5) / size as f32,
        )
    })
}

/// An albedo texture from linear colours, with its mips averaged in linear light.
fn albedo_texture(name: &'static str, size: u32, linear: Vec<[f32; 3]>) -> TextureData {
    let encode = |level: &[[f32; 3]]| {
        encode(level, |c| {
            [
                to_byte(linear_to_srgb(c[0])),
                to_byte(linear_to_srgb(c[1])),
                to_byte(linear_to_srgb(c[2])),
                255,
            ]
        })
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
        name: name.into(),
        size,
        height: size,
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
    let normals = texels(size as usize, heights.len(), |x, y| {
        let (x, y) = (x as i64, y as i64);
        // Tangent x runs along +u and y along +v (down the rows), the axes the triplanar
        // projection maps to object space.
        let dx = (at(x + 1, y) - at(x - 1, y)) * 0.5 * strength;
        let dy = (at(x, y + 1) - at(x, y - 1)) * 0.5 * strength;
        normalize([-dx, -dy, 1.0])
    });
    let encode = |level: &[[f32; 3]]| {
        encode(level, |n| {
            [
                to_byte(n[0] * 0.5 + 0.5),
                to_byte(n[1] * 0.5 + 0.5),
                to_byte(n[2] * 0.5 + 0.5),
                255,
            ]
        })
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
        name: name.into(),
        size,
        height: size,
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
fn downsample<T: Copy + Send + Sync + Default>(
    level: &[T],
    side: u32,
    f: impl Fn(T, T, T, T) -> T + Sync,
) -> Vec<T> {
    let half = side / 2;
    let s = side as usize;
    texels(half as usize, (half * half) as usize, |x, y| {
        let i = 2 * y * s + 2 * x;
        f(level[i], level[i + 1], level[i + s], level[i + s + 1])
    })
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
    kept("texture-rock", &(seed, size), || {
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
    })
}

/// Cast concrete: light grey with stains, pores and faint formwork lines.
pub fn concrete(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-concrete", &(seed, size), || {
        let heights = grid(size, |u, v| {
            let pores = fbm(seed ^ 0x9E, u, v, 64, 2);
            let pits = smoothstep(0.72, 0.8, pores);
            0.5 * fbm(seed, u, v, 16, 3) - pits
        });
        let colours = grid(size, |u, v| {
            let stain = fbm(seed ^ 0x57A, u, v, 3, 5);
            let speck = fbm(seed ^ 0x5EC, u, v, 128, 1);
            // Formwork panels: a darker line every quarter of the texture.
            let line = (1.0
                - smoothstep(0.0, 0.004, (v * 4.0).fract().min(1.0 - (v * 4.0).fract())))
                * 0.12;
            let c = (0.42 + 0.18 * stain + 0.06 * (speck - 0.5)) * (1.0 - line);
            [c, c * 0.99, c * 0.97]
        });
        [
            albedo_texture("concrete albedo", size, colours),
            normal_texture("concrete normal", size, &heights, size as f32 / 256.0),
        ]
    })
}

/// Bricks in running bond, 8 across and 16 rows per repeat: fired clay in varied reds and
/// browns, recessed grey mortar.
pub fn brick(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-brick", &(seed, size), || {
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
            let mortar =
                [0.40, 0.38, 0.35].map(|c| c * (0.9 + 0.2 * fbm(seed ^ 0x3A, u, v, 32, 2)));
            mix3(mortar, clay, smoothstep(MORTAR * 0.5, MORTAR, edge))
        });
        [
            albedo_texture("brick albedo", size, colours),
            normal_texture("brick normal", size, &heights, size as f32 / 64.0),
        ]
    })
}

/// Stone paving: pale limestone slabs in running bond, 4 across and 6 rows per repeat, each
/// slab its own tint and wear, in fine dark joints; for pavements, lanes and squares.
pub fn paving(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-paving", &(seed, size), || {
        const ACROSS: f32 = 4.0;
        const ROWS: f32 = 6.0;
        const JOINT: f32 = 0.025; // of a slab's height
        let cell = |u: f32, v: f32| {
            let row = (v * ROWS).floor();
            let shift = unit_f32(hash_cell2(seed ^ 0x5A1B, 0, row as i32)) * 0.6 + 0.2;
            let x = u * ACROSS + if row as i32 % 2 == 0 { 0.0 } else { shift };
            let col = x.floor().rem_euclid(ACROSS);
            let (fx, fy) = (x.fract(), (v * ROWS).fract());
            // Distance to the nearest joint in slab heights (a slab is 1.5 times as long as tall).
            let edge = (fx.min(1.0 - fx) * ROWS / ACROSS).min(fy.min(1.0 - fy));
            (col as i32, row as i32, edge)
        };
        let heights = grid(size, |u, v| {
            let (col, row, edge) = cell(u, v);
            let tilt = 0.15 * unit_f32(hash_cell2(seed ^ 0x711, col, row));
            let inset = smoothstep(JOINT * 0.5, JOINT * 1.5, edge);
            inset * (0.7 + tilt) + 0.15 * fbm(seed ^ 0xB2, u, v, 48, 3)
        });
        let colours = grid(size, |u, v| {
            let (col, row, edge) = cell(u, v);
            let stone = unit_f32(hash_cell2(seed, col, row));
            let worn = unit_f32(hash_cell2(seed ^ 0x3E3, col, row));
            let slab = mix3([0.64, 0.60, 0.53], [0.72, 0.68, 0.60], stone);
            let slab = mix3(slab, [0.55, 0.52, 0.47], worn * worn * 0.5);
            let grain = 0.92 + 0.16 * fbm(seed ^ 0x6A1, u, v, 96, 2);
            let blot = 0.94 + 0.12 * fbm(seed ^ 0xB10, u, v, 6, 3);
            let slab = slab.map(|c| c * grain * blot);
            mix3(
                [0.30, 0.29, 0.27],
                slab,
                smoothstep(JOINT * 0.5, JOINT * 1.5, edge),
            )
        });
        [
            albedo_texture("paving albedo", size, colours),
            normal_texture("paving normal", size, &heights, size as f32 / 96.0),
        ]
    })
}

/// Asphalt: a dark binder with light aggregate showing through, worn paler in broad patches.
pub fn asphalt(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-asphalt", &(seed, size), || {
        let heights = grid(size, |u, v| {
            let stones = fbm(seed ^ 0xA66, u, v, 160, 2);
            0.3 * smoothstep(0.55, 0.7, stones) + 0.7 * fbm(seed, u, v, 40, 3)
        });
        let colours = grid(size, |u, v| {
            let stones = fbm(seed ^ 0xA66, u, v, 160, 2);
            let aggregate = smoothstep(0.58, 0.72, stones);
            let wear = fbm(seed ^ 0x3EA, u, v, 4, 4);
            let base = 0.17 + 0.05 * wear;
            let c = base + 0.05 * aggregate;
            [c, c * 0.98, c * 0.95]
        });
        [
            albedo_texture("asphalt albedo", size, colours),
            normal_texture("asphalt normal", size, &heights, size as f32 / 512.0),
        ]
    })
}

/// Bays across and floors up in one repeat of [`facade`]'s textures.
pub const FACADE_CELLS: u32 = 4;

/// A facade over [`FACADE_CELLS`] bays and floors (D-059's city): pale plaster, each bay's
/// window recessed with its frame and sill, louvred shutters either side (closed over the
/// window in about a quarter of them), a string course at each floor line. `u` runs across the
/// bays, `v` down the floors; the row's colour tints it (the plaster's render). The third
/// texture is the windows lit at night, `lit` of those open (the city plan's share of the hour),
/// warm or cool: an emissive map.
pub fn facade(seed: u64, size: u32, lit: f32) -> [TextureData; 3] {
    kept("texture-facade", &(seed, size, lit), || {
        let cells = FACADE_CELLS as f32;
        // A cell's index and the place inside it.
        let cell = |u: f32, v: f32| {
            let (x, y) = (u * cells, v * cells);
            (x.floor() as i32, y.floor() as i32, x.fract(), y.fract())
        };
        let closed = |cx: i32, cy: i32| unit_f32(hash_cell2(seed ^ 0xC105, cx, cy)) < 0.25;
        let share = lit;
        let lit = |cx: i32, cy: i32| unit_f32(hash_cell2(seed ^ 0x117, cx, cy)) < share;
        let window = |u: f32, v: f32| (0.30..0.70).contains(&u) && (0.20..0.78).contains(&v);
        let frame = |u: f32, v: f32| (0.28..0.72).contains(&u) && (0.18..0.80).contains(&v);
        let shutter = |u: f32, v: f32| {
            ((0.17..0.28).contains(&u) || (0.72..0.83).contains(&u)) && (0.18..0.80).contains(&v)
        };
        let sill = |u: f32, v: f32| (0.26..0.74).contains(&u) && (0.80..0.83).contains(&v);
        let course = |v: f32| v > 0.95;
        let slats = |v: f32| ((v * 60.0).fract() - 0.5).abs();
        let heights = grid(size, |u, v| {
            let (cx, cy, fu, fv) = cell(u, v);
            let grain = 0.08 * fbm(seed ^ 0xFA, u, v, 32, 3);
            if window(fu, fv) && closed(cx, cy) {
                0.1 + 0.1 * slats(fv)
            } else if window(fu, fv) {
                -0.8
            } else if frame(fu, fv) {
                -0.3
            } else if shutter(fu, fv) && !closed(cx, cy) {
                0.15 + 0.1 * slats(fv)
            } else if sill(fu, fv) || course(fv) {
                0.35
            } else {
                grain
            }
        });
        let colours = grid(size, |u, v| {
            let (cx, cy, fu, fv) = cell(u, v);
            let grime = 0.9 + 0.15 * fbm(seed ^ 0x6E1, u, v, 16, 3);
            let shutter_colour = |fv: f32| {
                let slat = 0.85 + 0.15 * f32::from(u8::from((fv * 60.0).fract() > 0.5));
                [0.20 * slat, 0.30 * slat, 0.25 * slat]
            };
            if window(fu, fv) && closed(cx, cy) {
                shutter_colour(fv)
            } else if window(fu, fv) {
                // Glass: dark, a little lighter at the top where it catches the sky.
                mix3(
                    [0.03, 0.04, 0.05],
                    [0.10, 0.12, 0.14],
                    1.0 - (fv - 0.2) / 0.58,
                )
            } else if frame(fu, fv) {
                [0.85, 0.85, 0.82]
            } else if shutter(fu, fv) && !closed(cx, cy) {
                shutter_colour(fv)
            } else if sill(fu, fv) || course(fv) {
                [0.88, 0.86, 0.80]
            } else {
                [0.80 * grime, 0.79 * grime, 0.76 * grime]
            }
        });
        let lights = grid(size, |u, v| {
            let (cx, cy, fu, fv) = cell(u, v);
            if window(fu, fv) && !closed(cx, cy) && lit(cx, cy) {
                let warm = unit_f32(hash_cell2(seed ^ 0x3A2, cx, cy));
                // Warm interiors mostly, a few cooler; a little dimmer towards the window's foot.
                let c = mix3(
                    [1.0, 0.72, 0.42],
                    [0.85, 0.85, 0.9],
                    (warm - 0.75).max(0.0) * 4.0,
                );
                c.map(|k| k * (0.75 + 0.25 * (1.0 - fv)))
            } else {
                [0.0; 3]
            }
        });
        [
            albedo_texture("facade albedo", size, colours),
            normal_texture("facade normal", size, &heights, size as f32 / 192.0),
            albedo_texture("facade lights", size, lights),
        ]
    })
}

/// The lit shopfronts at night: [`shopfront`]'s glazing as an emissive map.
pub fn shopfront_lights(size: u32) -> TextureData {
    kept("texture-shopfront_lights", &size, || {
        let glass = |u: f32, v: f32| (0.09..0.91).contains(&u) && (0.22..0.93).contains(&v);
        let colours = grid(size, |u, v| {
            if glass(u, v) && !(0.495..0.505).contains(&u) {
                [1.0, 0.82, 0.6]
            } else {
                [0.0; 3]
            }
        });
        albedo_texture("shopfront lights", size, colours)
    })
}

/// A ground floor's bay of shopfront (D-059's city): a fascia band, a tall glazed front between
/// stone piers, a stone plinth. `u` across the bay, `v` down the floor.
pub fn shopfront(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-shopfront", &(seed, size), || {
        let pier = |u: f32| !(0.07..0.93).contains(&u);
        let fascia = |v: f32| v < 0.17;
        let glass = |u: f32, v: f32| (0.09..0.91).contains(&u) && (0.22..0.93).contains(&v);
        let mullion = |u: f32| (0.495..0.505).contains(&u);
        let heights = grid(size, |u, v| {
            if pier(u) || v > 0.95 {
                0.3 + 0.05 * fbm(seed ^ 0x5701, u, v, 32, 2)
            } else if glass(u, v) && !mullion(u) {
                -0.6
            } else {
                0.0
            }
        });
        let colours = grid(size, |u, v| {
            let stone =
                [0.78, 0.74, 0.66].map(|c| c * (0.9 + 0.15 * fbm(seed ^ 0x5702, u, v, 32, 3)));
            if pier(u) || v > 0.95 {
                stone
            } else if fascia(v) {
                [0.16, 0.24, 0.20]
            } else if glass(u, v) && !mullion(u) {
                // A lit shop's warm interior behind the glass, darker low down.
                mix3([0.20, 0.16, 0.11], [0.06, 0.06, 0.07], (v - 0.22) / 0.71)
            } else {
                [0.22, 0.22, 0.22]
            }
        });
        [
            albedo_texture("shopfront albedo", size, colours),
            normal_texture("shopfront normal", size, &heights, size as f32 / 48.0),
        ]
    })
}

/// Ground: short grass with patches of bare soil.
pub fn grass(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-grass", &(seed, size), || {
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
    })
}

/// The nearest of `cells × cells` jittered points to texture coordinates `(u, v)` (periodic):
/// its distance and the second nearest's, in cells, its cell, and the offset from `(u, v)` to it
/// in cells.
fn worley(seed: u64, u: f32, v: f32, cells: u32) -> (f32, f32, (i32, i32), [f32; 2]) {
    let (x, y) = (u * cells as f32, v * cells as f32);
    let (cx, cy) = (x.floor() as i32, y.floor() as i32);
    let p = cells as i32;
    let mut best = (f32::MAX, f32::MAX, (0, 0), [0.0; 2]);
    for j in -1..=1 {
        for i in -1..=1 {
            let cell = ((cx + i).rem_euclid(p), (cy + j).rem_euclid(p));
            let px = (cx + i) as f32 + 0.1 + 0.8 * unit_f32(hash_cell2(seed, cell.0, cell.1));
            let py =
                (cy + j) as f32 + 0.1 + 0.8 * unit_f32(hash_cell2(seed ^ 0x51DE, cell.0, cell.1));
            let (dx, dy) = (px - x, py - y);
            let d = (dx * dx + dy * dy).sqrt();
            if d < best.0 {
                best = (d, best.0, cell, [dx, dy]);
            } else if d < best.1 {
                best.1 = d;
            }
        }
    }
    best
}

/// A river's bed of rounded cobbles and pebbles (#118): grey, brown and ochre stones packed
/// together, dark sand and silt in the gaps between them.
pub fn gravel(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-gravel", &(seed, size), || {
        // A stone's rise from the gap: rounded, highest at its middle.
        let stones = |u: f32, v: f32, cells: u32, salt: u64| {
            let (f1, f2, cell, _) = worley(seed ^ salt, u, v, cells);
            let edge = f2 - f1;
            (smoothstep(0.0, 0.35, edge).sqrt(), edge, cell)
        };
        let heights = grid(size, |u, v| {
            let (cobble, _, _) = stones(u, v, 20, 0xC0B);
            let (pebble, _, _) = stones(u, v, 56, 0x9EB);
            cobble.max(0.55 * pebble) + 0.05 * fbm(seed ^ 0x6A1, u, v, 64, 2)
        });
        let colours = grid(size, |u, v| {
            let (cobble, edge, cell) = stones(u, v, 20, 0xC0B);
            let (pebble, small_edge, small) = stones(u, v, 56, 0x9EB);
            let tone = |cell: (i32, i32), salt: u64| {
                let h = hash_cell2(seed ^ salt, cell.0, cell.1);
                let kind = unit_f32(h);
                let light = 0.75 + 0.5 * unit_f32(h.rotate_left(17));
                let colour = if kind < 0.5 {
                    [0.30, 0.30, 0.29]
                } else if kind < 0.8 {
                    [0.33, 0.27, 0.21]
                } else {
                    [0.40, 0.32, 0.20]
                };
                colour.map(|c| c * light)
            };
            let grain = 0.85 + 0.3 * fbm(seed ^ 0x6A11, u, v, 128, 2);
            let gap = [0.08, 0.07, 0.055];
            let small_stone = mix3(gap, tone(small, 0x9EB), smoothstep(0.02, 0.1, small_edge));
            let under = mix3(gap, small_stone, f32::from(u8::from(pebble > 0.2)));
            let big = tone(cell, 0xC0B).map(|c| c * (0.8 + 0.2 * cobble));
            mix3(under, big, smoothstep(0.03, 0.1, edge)).map(|c| c * grain)
        });
        [
            albedo_texture("gravel albedo", size, colours),
            normal_texture("gravel normal", size, &heights, size as f32 / 40.0),
        ]
    })
}

/// A shingle beach (#128): rounded, flattened pebbles of grey, blue-grey and brown stone with
/// the odd white quartz, worn smooth by the waves, lying apart on coarse sand. Not the river
/// bed's packed cobbles ([`gravel`]), which read as paving on a beach.
pub fn shingle(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-shingle", &(seed, size), || {
        // A pebble: an ellipse round its cell's point, turned and stretched by the cell's hash,
        // domed; 0 outside it.
        let pebble = |u: f32, v: f32, cells: u32, salt: u64| {
            let (_, _, cell, d) = worley(seed ^ salt, u, v, cells);
            let h = hash_cell2(seed ^ salt ^ 0x7E, cell.0, cell.1);
            let angle = unit_f32(h) * std::f32::consts::PI;
            let stretch = 1.0 + 0.6 * unit_f32(h.rotate_left(11));
            let radius = 0.36 + 0.12 * unit_f32(h.rotate_left(23));
            let (c, s) = (angle.cos(), angle.sin());
            let (a, b) = (d[0] * c + d[1] * s, -d[0] * s + d[1] * c);
            let r = (a / stretch).hypot(b * stretch) / radius;
            ((1.0 - r * r).max(0.0).sqrt(), cell)
        };
        let heights = grid(size, |u, v| {
            let (big, _) = pebble(u, v, 18, 0x5B1);
            let (small, _) = pebble(u, v, 41, 0x5B2);
            big.max(0.6 * small) + 0.03 * fbm(seed ^ 0x5B3, u, v, 128, 2)
        });
        let colours = grid(size, |u, v| {
            let tone = |cell: (i32, i32), salt: u64| {
                let h = hash_cell2(seed ^ salt, cell.0, cell.1);
                let kind = unit_f32(h);
                let light = 0.8 + 0.4 * unit_f32(h.rotate_left(17));
                let colour = if kind < 0.35 {
                    [0.42, 0.42, 0.41]
                } else if kind < 0.6 {
                    [0.35, 0.38, 0.42]
                } else if kind < 0.82 {
                    [0.45, 0.38, 0.3]
                } else if kind < 0.95 {
                    [0.24, 0.24, 0.24]
                } else {
                    [0.72, 0.7, 0.66]
                };
                colour.map(|c| c * light)
            };
            let (big, big_cell) = pebble(u, v, 18, 0x5B1);
            let (small, small_cell) = pebble(u, v, 41, 0x5B2);
            let grain = 0.85 + 0.3 * fbm(seed ^ 0x5B4, u, v, 128, 2);
            let sand = [0.38, 0.34, 0.27].map(|c| c * grain);
            // Each stone darker towards its rim, where it meets the sand.
            let stone = |dome: f32, cell, salt| tone(cell, salt).map(|c| c * (0.75 + 0.25 * dome));
            let under = mix3(
                sand,
                stone(small, small_cell, 0x5B2),
                smoothstep(0.0, 0.15, small),
            );
            mix3(
                under,
                stone(big, big_cell, 0x5B1),
                smoothstep(0.0, 0.12, big),
            )
        });
        [
            albedo_texture("shingle albedo", size, colours),
            normal_texture("shingle normal", size, &heights, size as f32 / 40.0),
        ]
    })
}

/// Granite (#129, D-042): the island's hills, coarse-grained, grey to pink, specked with pink
/// feldspar, white quartz and black mica; weathered smooth into slabs, crossed by the odd
/// curved sheet joint, stained by lichen.
pub fn granite(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-granite", &(seed, size), || {
        // The sheet joints: a few curved cracks, where a slow noise crosses its middle.
        let joint = |u: f32, v: f32| {
            let s = fbm(seed ^ 0x6A7, u, v, 2, 3);
            1.0 - smoothstep(0.0, 0.012, (s - 0.5).abs())
        };
        let heights = grid(size, |u, v| {
            let slab = fbm(seed, u, v, 3, 4);
            let grain = fbm(seed ^ 0x61A, u, v, 128, 1);
            0.7 * slab + 0.06 * grain - 0.25 * joint(u, v)
        });
        let colours = grid(size, |u, v| {
            let (_, _, cell, _) = worley(seed ^ 0x6C2, u, v, 160);
            let h = hash_cell2(seed ^ 0x6C2, cell.0, cell.1);
            let kind = unit_f32(h);
            // The grains: feldspar, quartz, mica, in a grey-pink groundmass.
            let grain = if kind < 0.3 {
                [0.62, 0.46, 0.42]
            } else if kind < 0.5 {
                [0.72, 0.71, 0.69]
            } else if kind < 0.58 {
                [0.1, 0.1, 0.1]
            } else {
                [0.5, 0.47, 0.45]
            };
            let lichen = smoothstep(0.55, 0.75, fbm(seed ^ 0x11C, u, v, 6, 4));
            let weather = 0.85 + 0.3 * fbm(seed ^ 0x3EA, u, v, 4, 4);
            let stained = mix3(grain, [0.36, 0.37, 0.33], 0.6 * lichen);
            stained.map(|c| c * weather * (1.0 - 0.5 * joint(u, v)))
        });
        [
            albedo_texture("granite albedo", size, colours),
            normal_texture("granite normal", size, &heights, size as f32 / 32.0),
        ]
    })
}

/// Grus (#135, D-042): the coarse sand granite rots into on its gentle ground. Loose grains of
/// pink feldspar, white quartz and black mica on a buff to ochre ground, small angular pieces of
/// the granite lying on it, and the odd tuft of dry grass.
pub fn grus(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-grus", &(seed, size), || {
        // A piece of granite: a tilted face round its cell's point, a third of the cells holding one.
        let piece = |u: f32, v: f32| {
            let (f1, f2, cell, offset) = worley(seed ^ 0x6C5, u, v, 22);
            let h = hash_cell2(seed ^ 0x6C6, cell.0, cell.1);
            let (a, b) = (unit_f32(h) - 0.5, unit_f32(h.rotate_left(23)) - 0.5);
            let size = 0.18 + 0.2 * unit_f32(h.rotate_left(41));
            let on = unit_f32(h.rotate_left(7)) < 0.35;
            let inside = if on {
                smoothstep(size, size - 0.06, f1) * smoothstep(0.0, 0.08, f2 - f1)
            } else {
                0.0
            };
            let face = 0.7 + 0.6 * (a * offset[0] + b * offset[1]);
            (inside, face, h)
        };
        let tuft = |u: f32, v: f32| smoothstep(0.62, 0.72, fbm(seed ^ 0x7F7, u, v, 10, 3));
        let heights = grid(size, |u, v| {
            let (inside, face, _) = piece(u, v);
            let sand = 0.12 * fbm(seed ^ 0x6A1, u, v, 160, 2) + 0.2 * fbm(seed, u, v, 6, 3);
            sand + 0.5 * inside * face + 0.15 * tuft(u, v) * fbm(seed ^ 0x7F8, u, v, 96, 1)
        });
        let colours = grid(size, |u, v| {
            // The grains, each a cell of a fine lattice: feldspar, quartz, mica, or the ground's.
            let (_, _, cell, _) = worley(seed ^ 0x6C2, u, v, 140);
            let kind = unit_f32(hash_cell2(seed ^ 0x6C2, cell.0, cell.1));
            let ground = mix3(
                [0.56, 0.47, 0.36],
                [0.62, 0.5, 0.42],
                smoothstep(0.35, 0.65, fbm(seed ^ 0xB0F, u, v, 4, 3)),
            );
            let grain = if kind < 0.18 {
                [0.68, 0.5, 0.43]
            } else if kind < 0.3 {
                [0.78, 0.76, 0.72]
            } else if kind < 0.35 {
                [0.12, 0.11, 0.1]
            } else {
                ground
            };
            let shade = 0.88 + 0.24 * fbm(seed ^ 0x6A1, u, v, 160, 2);
            let sand = grain.map(|c| c * shade);
            let (inside, face, h) = piece(u, v);
            let light = 0.8 + 0.3 * unit_f32(h.rotate_left(13));
            let stone = [0.56, 0.51, 0.48].map(|c| c * light * (0.85 + 0.25 * face));
            let dry = mix3(
                [0.42, 0.37, 0.22],
                [0.55, 0.49, 0.3],
                fbm(seed ^ 0x7F8, u, v, 96, 1),
            );
            mix3(mix3(sand, stone, inside), dry, 0.8 * tuft(u, v))
        });
        [
            albedo_texture("grus albedo", size, colours),
            normal_texture("grus normal", size, &heights, size as f32 / 40.0),
        ]
    })
}

/// Limestone (#129, D-042): the island's low ground and sea cliffs, the old reefs raised with
/// it. Pale cream-grey, fine-grained, pitted where the rain dissolves it, blotched grey and
/// darker where it weathers.
pub fn limestone(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-limestone", &(seed, size), || {
        let pits = |u: f32, v: f32| smoothstep(0.68, 0.78, fbm(seed ^ 0x917, u, v, 48, 2));
        let heights = grid(size, |u, v| {
            0.6 * fbm(seed, u, v, 4, 5) + 0.1 * fbm(seed ^ 0xF17, u, v, 64, 2) - 0.3 * pits(u, v)
        });
        let colours = grid(size, |u, v| {
            let blotch = fbm(seed ^ 0xB10, u, v, 5, 4);
            let grain = fbm(seed ^ 0x6A1, u, v, 128, 1);
            let cream = [0.74, 0.71, 0.63];
            let grey = [0.56, 0.56, 0.54];
            let c = mix3(cream, grey, smoothstep(0.45, 0.75, blotch));
            let shade = (0.92 + 0.12 * (grain - 0.5)) * (1.0 - 0.45 * pits(u, v));
            c.map(|c| c * shade)
        });
        [
            albedo_texture("limestone albedo", size, colours),
            normal_texture("limestone normal", size, &heights, size as f32 / 48.0),
        ]
    })
}

/// Karst pavement (#129, D-042): the limestone's bare ground etched by the rain into blocks
/// (clints) a metre or two across, split by deep fissures (grikes) where moss and grass grow.
pub fn karst(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-karst", &(seed, size), || {
        // A block: its distance to the nearest fissure, the cells' edges, and its cell.
        let block = |u: f32, v: f32| {
            let (f1, f2, cell, _) = worley(seed ^ 0xC71, u, v, 6);
            (f2 - f1, cell)
        };
        let heights = grid(size, |u, v| {
            let (edge, _) = block(u, v);
            let top = smoothstep(0.0, 0.12, edge).sqrt();
            top + 0.08 * fbm(seed ^ 0x917, u, v, 48, 2)
        });
        let colours = grid(size, |u, v| {
            let (edge, cell) = block(u, v);
            let light = 0.85 + 0.25 * unit_f32(hash_cell2(seed ^ 0xC72, cell.0, cell.1));
            let pits = smoothstep(0.7, 0.8, fbm(seed ^ 0x917, u, v, 48, 2));
            let top = [0.72, 0.7, 0.63].map(|c| c * light * (1.0 - 0.4 * pits));
            let moss = mix3(
                [0.05, 0.07, 0.03],
                [0.16, 0.24, 0.08],
                fbm(seed ^ 0x305, u, v, 32, 2),
            );
            mix3(moss, top, smoothstep(0.03, 0.09, edge))
        });
        [
            albedo_texture("karst albedo", size, colours),
            normal_texture("karst normal", size, &heights, size as f32 / 24.0),
        ]
    })
}

/// Scree (#118): broken rock fallen from the walls above, angular fragments of pale grey stone,
/// each a flat face tilted its own way, dark in the cracks between them.
pub fn scree(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-scree", &(seed, size), || {
        // A fragment's face: a plane through its point, tilted by its cell's hash, sunk into the
        // cracks between fragments.
        let fragment = |u: f32, v: f32, cells: u32, salt: u64| {
            let (f1, f2, cell, offset) = worley(seed ^ salt, u, v, cells);
            let h = hash_cell2(seed ^ salt ^ 0x7117, cell.0, cell.1);
            let (a, b) = (unit_f32(h) - 0.5, unit_f32(h.rotate_left(23)) - 0.5);
            let face = 0.6 + 0.5 * (a * offset[0] + b * offset[1]);
            let crack = smoothstep(0.0, 0.12, f2 - f1);
            (face * crack, f2 - f1, cell, f1)
        };
        let heights = grid(size, |u, v| {
            let (big, _, _, _) = fragment(u, v, 14, 0x5C2);
            let (small, _, _, _) = fragment(u, v, 40, 0x5C3);
            big.max(0.6 * small)
        });
        let colours = grid(size, |u, v| {
            let (big, edge, cell, _) = fragment(u, v, 14, 0x5C2);
            let (small, small_edge, small_cell, _) = fragment(u, v, 40, 0x5C3);
            let tone = |cell: (i32, i32), salt: u64| {
                let h = hash_cell2(seed ^ salt, cell.0, cell.1);
                let grey = 0.36 + 0.18 * unit_f32(h);
                let warm = 0.04 * unit_f32(h.rotate_left(11));
                [grey + warm, grey + 0.5 * warm, grey * 0.96]
            };
            let grain = 0.85 + 0.3 * fbm(seed ^ 0x5C4, u, v, 96, 2);
            let crack = [0.07, 0.065, 0.06];
            let chips = mix3(
                crack,
                tone(small_cell, 0x5C3),
                smoothstep(0.01, 0.06, small_edge),
            );
            let under = mix3(crack, chips, f32::from(u8::from(small > 0.15)));
            let face = tone(cell, 0x5C2).map(|c| c * (0.75 + 0.35 * big));
            mix3(under, face, smoothstep(0.02, 0.07, edge)).map(|c| c * grain)
        });
        [
            albedo_texture("scree albedo", size, colours),
            normal_texture("scree normal", size, &heights, size as f32 / 48.0),
        ]
    })
}

/// Scrub on steep ground (#118): rounded shrubs, seven across a repeat, each its own shade of
/// dark green, olive or blue-green, their leaves breaking their outline, over stony soil darker
/// in their shade.
pub fn scrub(seed: u64, size: u32) -> [TextureData; 2] {
    kept("texture-scrub", &(seed, size), || {
        const SHRUBS: u32 = 7;
        // The nearest shrub: how far inside its crown a point is (1 at its heart, 0 at its edge,
        // below 0 outside), and its hash.
        let crown = |u: f32, v: f32| {
            let (f1, _, cell, _) = worley(seed, u, v, SHRUBS);
            let h = hash_cell2(seed ^ 0xC20, cell.0, cell.1);
            let radius = 0.42 + 0.22 * unit_f32(h);
            let ragged = 0.16 * (fbm(seed ^ 0x1EAF, u, v, 64, 2) - 0.5);
            (1.0 - f1 / (radius + ragged), h)
        };
        let leaves = |u: f32, v: f32| {
            let (f1, _, _, _) = worley(seed ^ 0x1EAF, u, v, 112);
            1.0 - smoothstep(0.0, 0.8, f1)
        };
        let heights = grid(size, |u, v| {
            let (c, _) = crown(u, v);
            let dome = c.max(0.0).sqrt();
            dome * (0.85 + 0.15 * leaves(u, v)) + 0.04 * fbm(seed ^ 0x5011, u, v, 48, 2)
        });
        let colours = grid(size, |u, v| {
            let (c, h) = crown(u, v);
            let kind = unit_f32(h.rotate_left(13));
            let green = if kind < 0.45 {
                [0.03, 0.062, 0.024]
            } else if kind < 0.75 {
                [0.058, 0.068, 0.026]
            } else {
                [0.03, 0.058, 0.044]
            };
            let light = 0.8 + 0.4 * unit_f32(h.rotate_left(29));
            let leaf = leaves(u, v);
            // Darker towards the crown's edge, where the leaves shade each other.
            let inner = 0.6 + 0.4 * c.max(0.0).sqrt();
            let foliage = green.map(|g| g * light * inner * (0.85 + 0.6 * leaf * leaf));
            let stone = fbm(seed ^ 0x5701, u, v, 32, 2);
            let soil = mix3(
                [0.11, 0.095, 0.075],
                [0.17, 0.16, 0.14],
                smoothstep(0.55, 0.7, stone),
            );
            // In the shrubs' shade near their crowns.
            let shade = 0.5 + 0.5 * smoothstep(0.0, 0.35, -c);
            mix3(soil.map(|s| s * shade), foliage, smoothstep(-0.03, 0.05, c))
        });
        [
            albedo_texture("scrub albedo", size, colours),
            normal_texture("scrub normal", size, &heights, size as f32 / 40.0),
        ]
    })
}

/// A floor to measure by (#156, the tank's bench): ten squares a repeat each way in two greys
/// (10 cm squares at a metre a repeat), a darker line on the repeat's edges (every metre) and a
/// fainter one through its middle; flat. Its mips keep it from shimmering at a slant.
pub fn checker(size: u32) -> [TextureData; 2] {
    const SQUARES: f32 = 10.0;
    let colours = grid(size, |u, v| {
        let parity = |t: f32| (t * SQUARES).floor() as i32 & 1;
        let odd = (parity(u) ^ parity(v)) as f32;
        let line = |t: f32, width: f32| 1.0 - smoothstep(0.0, width, t.min(1.0 - t));
        let metre = line(u, 0.006).max(line(v, 0.006));
        let half = line((u - 0.5).abs(), 0.003).max(line((v - 0.5).abs(), 0.003));
        let grey = (0.55 + 0.2 * odd) * (1.0 - 0.45 * metre) * (1.0 - 0.2 * half);
        [grey, grey, grey]
    });
    let heights = vec![0.0; (size * size) as usize];
    [
        albedo_texture("checker albedo", size, colours),
        normal_texture("checker normal", size, &heights, 1.0),
    ]
}

/// The sharpness room's floor (#159): eight squares a repeat each way (12.5 cm at a metre a
/// repeat), near black and near white, each square a whole number of texels at a power-of-two
/// `size`, so the squares' edges fall between texels; flat.
pub fn black_and_white(size: u32) -> [TextureData; 2] {
    const SQUARES: f32 = 8.0;
    let colours = grid(size, |u, v| {
        let parity = |t: f32| (t * SQUARES).floor() as i32 & 1;
        let grey = if parity(u) ^ parity(v) == 1 {
            0.8
        } else {
            0.03
        };
        [grey, grey, grey]
    });
    let heights = vec![0.0; (size * size) as usize];
    [
        albedo_texture("black and white albedo", size, colours),
        normal_texture("black and white normal", size, &heights, 1.0),
    ]
}

/// The quarters of the [`leaves`] atlas: the sprays a species' cards are cut from (#296: the
/// species data names its own, [`forge_flora::species::Look`]).
pub use forge_flora::species::LeafSpray;

/// A leaf or a twig of a spray, in its quarter's unit square (`v` down, the stem at the bottom).
struct Stroke {
    /// Where it starts (a leaf's stalk end, a twig's base).
    from: [f32; 2],
    /// Unit direction it grows.
    dir: [f32; 2],
    /// Its length.
    length: f32,
    /// Its half width at the widest (a twig's: all along).
    width: f32,
    /// A leaf's outline: half width `width · sin(π s)^shape` at `s` of its length; a twig's: 0.
    shape: f32,
    /// Lobes along a broad leaf's edge (0: smooth).
    lobes: f32,
    /// Linear colour (before the row's tint).
    colour: [f32; 3],
}

/// One spray's strokes, the twigs first.
fn spray_strokes(seed: u64, spray: LeafSpray) -> Vec<Stroke> {
    let mut strokes = Vec::new();
    let mut n = 0_i32;
    let mut random = || {
        n += 1;
        unit_f32(hash_cell2(seed ^ ((spray.quarter() as u64) << 40), n, 7))
    };
    let rotate = |d: [f32; 2], a: f32| {
        let (s, c) = a.sin_cos();
        [d[0] * c - d[1] * s, d[0] * s + d[1] * c]
    };
    let twig = [0.42, 0.36, 0.28];
    // (leaf length, half width, outline, lobes, spacing along a twig, angle off the twig,
    // side twigs, their length, pairs (both sides at once), needles to a tuft)
    let (length, width, shape, lobes, spacing, angle, sides, side_length, pairs, tuft) = match spray
    {
        LeafSpray::Small => (0.13, 0.046, 0.6, 0.0, 0.045, 0.75, 9, 0.46, false, 0),
        LeafSpray::Broad => (0.24, 0.11, 0.5, 5.0, 0.11, 0.8, 5, 0.44, false, 0),
        LeafSpray::Needles => (0.18, 0.005, 0.25, 0.0, 0.055, 0.0, 8, 0.44, false, 13),
        LeafSpray::Narrow => (0.15, 0.025, 0.85, 0.0, 0.042, 0.62, 8, 0.44, true, 0),
    };
    let stem_from = [0.5, 0.98];
    let up = [0.0, -1.0];
    let stem_length = 0.86;
    // The twigs: the stem from just under the first side twig (a longer bare stem hung under
    // the cards as a thread), then side twigs left and right by turns, shorter towards the tip.
    let bare = 0.12;
    let mut twigs: Vec<([f32; 2], [f32; 2], f32)> = vec![(
        [stem_from[0], stem_from[1] - stem_length * bare],
        up,
        stem_length * (1.0 - bare),
    )];
    for k in 0..sides {
        let t = 0.18 + 0.62 * (k as f32 + 0.5) / sides as f32;
        let side = if k % 2 == 0 { 1.0 } else { -1.0 };
        let at = [stem_from[0], stem_from[1] - stem_length * t];
        let dir = rotate(up, side * (0.75 + 0.35 * random()));
        let reach = side_length * (1.15 - 0.6 * t) * (0.85 + 0.3 * random());
        twigs.push((at, dir, reach));
    }
    for &(from, dir, reach) in &twigs {
        strokes.push(Stroke {
            from,
            dir,
            length: reach,
            width: if reach > 0.5 { 0.007 } else { 0.0045 },
            shape: 0.0,
            lobes: 0.0,
            colour: twig,
        });
    }
    let mut leaf = |at: [f32; 2], dir: [f32; 2], scale: f32, r: f32| {
        let shade = 0.74 + 0.26 * r;
        strokes.push(Stroke {
            from: at,
            dir,
            length: length * scale,
            width: width * scale,
            shape,
            lobes,
            colour: [shade * 0.97, shade, shade * 0.9],
        });
    };
    for &(from, dir, reach) in &twigs {
        // Leaves from a sixth of the way along, alternating, or paired, or in tufts.
        let mut s = reach * 0.16 + spacing * random();
        let mut side = 1.0;
        while s < reach {
            let at = [from[0] + dir[0] * s, from[1] + dir[1] * s];
            let scale = 0.8 + 0.3 * random();
            if tuft > 0 {
                // A fan of needles round the shoot, forward-leaning.
                for k in 0..tuft {
                    let a = (k as f32 / (tuft - 1) as f32 - 0.5) * 2.4 + 0.15 * (random() - 0.5);
                    leaf(at, rotate(dir, a), scale, random());
                }
            } else if pairs {
                leaf(at, rotate(dir, angle), scale, random());
                leaf(at, rotate(dir, -angle), scale, random());
            } else {
                let a = side * (angle + 0.3 * (random() - 0.5));
                leaf(at, rotate(dir, a), scale, random());
                side = -side;
            }
            s += spacing * (0.85 + 0.3 * random());
        }
        // A leaf (or a tuft) at the tip.
        let tip = [from[0] + dir[0] * reach, from[1] + dir[1] * reach];
        if tuft > 0 {
            for k in 0..tuft {
                let a = (k as f32 / (tuft - 1) as f32 - 0.5) * 1.6;
                leaf(tip, rotate(dir, a), 1.0, random());
            }
        } else {
            leaf(tip, dir, 1.0, random());
        }
    }
    // Stretched about the stem's foot, across and up apart, to fill the square to its margins.
    let (mut wide, mut tall) = (1e-3_f32, 1e-3_f32);
    for s in &strokes {
        for p in [
            s.from,
            [
                s.from[0] + s.dir[0] * s.length,
                s.from[1] + s.dir[1] * s.length,
            ],
        ] {
            wide = wide.max((p[0] - stem_from[0]).abs() + s.width);
            tall = tall.max(stem_from[1] - p[1] + s.width);
        }
    }
    let (fx, fy) = (0.47 / wide, (stem_from[1] - 0.03) / tall);
    let place = |p: [f32; 2]| {
        [
            stem_from[0] + (p[0] - stem_from[0]) * fx,
            stem_from[1] + (p[1] - stem_from[1]) * fy,
        ]
    };
    for s in &mut strokes {
        let a = place(s.from);
        let b = place([
            s.from[0] + s.dir[0] * s.length,
            s.from[1] + s.dir[1] * s.length,
        ]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let length = (dx * dx + dy * dy).sqrt().max(1e-6);
        s.from = a;
        s.dir = [dx / length, dy / length];
        s.length = length;
        s.width *= fx.min(fy);
    }
    strokes
}

/// The leaf cards' atlas (D-013): a spray of leaves in each quarter ([`LeafSpray`]), drawn
/// light grey to be tinted by its row, the alpha the share of a texel the leaves and twigs
/// cover (their edges anti-aliased over a texel), for an alpha test at a half. The empty texels
/// take the leaves' mean colour and the mips average the colour weighted by the alpha, so a far
/// card does not darken at its edges. Each mip's alpha is its texels' mean scaled per quarter
/// so the test keeps the share level 0 keeps (Castaño 2010).
pub fn leaves(seed: u64, size: u32) -> TextureData {
    kept("texture-leaves", &(seed, size), || {
        let side = size as usize;
        let half = size as f32 * 0.5;
        // Each spray paints its own quarter, on a thread of its own: no stroke leaves its quarter,
        // so the quarters come out as one thread painting them in turn would leave them.
        let hs = side / 2;
        let paint = |spray: LeafSpray| {
            let q = spray.quarter();
            let (ox, oy) = ((q % 2) as usize * hs, (q / 2) as usize * hs);
            let mut quarter = vec![[0.0_f32; 4]; hs * hs];
            let origin = [(q % 2) as f32 * half, (q / 2) as f32 * half];
            // A quarter's unit square is `half` texels, inside a margin of 2 %.
            let scale = half * 0.96;
            let offset = [origin[0] + half * 0.02, origin[1] + half * 0.02];
            for stroke in spray_strokes(seed, spray) {
                let from = [
                    offset[0] + stroke.from[0] * scale,
                    offset[1] + stroke.from[1] * scale,
                ];
                let (length, width) = (stroke.length * scale, stroke.width * scale);
                let to = [
                    from[0] + stroke.dir[0] * length,
                    from[1] + stroke.dir[1] * length,
                ];
                let reach = width + 1.5;
                let lo = [
                    (from[0].min(to[0]) - reach).floor().max(origin[0]) as usize,
                    (from[1].min(to[1]) - reach).floor().max(origin[1]) as usize,
                ];
                let hi = [
                    ((from[0].max(to[0]) + reach).ceil() as usize).min((origin[0] + half) as usize),
                    ((from[1].max(to[1]) + reach).ceil() as usize).min((origin[1] + half) as usize),
                ];
                for y in lo[1]..hi[1] {
                    for x in lo[0]..hi[0] {
                        let p = [x as f32 + 0.5 - from[0], y as f32 + 0.5 - from[1]];
                        // Along and across the stroke, in texels.
                        let s = p[0] * stroke.dir[0] + p[1] * stroke.dir[1];
                        let t = -p[0] * stroke.dir[1] + p[1] * stroke.dir[0];
                        let (distance, colour) = if stroke.shape == 0.0 {
                            // A twig: a capsule.
                            let along = s.clamp(0.0, length);
                            let d = ((s - along).powi(2) + t * t).sqrt() - width;
                            (d, stroke.colour)
                        } else {
                            let u = s / length;
                            if !(0.0..=1.0).contains(&u) {
                                continue;
                            }
                            let lobe = 1.0 + 0.14 * (stroke.lobes * std::f32::consts::PI * u).sin();
                            let w =
                                width * (std::f32::consts::PI * u).sin().powf(stroke.shape) * lobe;
                            let across = t.abs() / width.max(1e-3);
                            // A lighter midrib, one half a shade darker (the leaf's fold).
                            let rib = if t.abs() < (width * 0.1).max(0.6) {
                                1.12
                            } else if t > 0.0 {
                                0.9
                            } else {
                                1.0
                            };
                            let edge = 1.0 - 0.12 * smoothstep(0.6, 1.0, across);
                            (t.abs() - w, stroke.colour.map(|c| c * rib * edge))
                        };
                        let cover = (0.5 - distance).clamp(0.0, 1.0);
                        if cover <= 0.0 {
                            continue;
                        }
                        let texel = &mut quarter[(y - oy) * hs + (x - ox)];
                        let a = cover + texel[3] * (1.0 - cover);
                        for k in 0..3 {
                            texel[k] = (colour[k] * cover + texel[k] * texel[3] * (1.0 - cover))
                                / a.max(1e-6);
                        }
                        texel[3] = a;
                    }
                }
            }
            quarter
        };
        let quarters: Vec<Vec<[f32; 4]>> = std::thread::scope(|scope| {
            let painting: Vec<_> = LeafSpray::ALL
                .map(|spray| scope.spawn(move || paint(spray)))
                .into_iter()
                .collect();
            painting
                .into_iter()
                .map(|p| p.join().expect("a leaf spray's painting"))
                .collect()
        });
        let mut texels = vec![[0.0_f32; 4]; side * side];
        for (spray, quarter) in LeafSpray::ALL.into_iter().zip(&quarters) {
            let q = spray.quarter() as usize;
            let (ox, oy) = ((q % 2) * hs, (q / 2) * hs);
            for (r, row) in quarter.chunks_exact(hs).enumerate() {
                texels[(oy + r) * side + ox..][..hs].copy_from_slice(row);
            }
        }
        // The empty texels take the covered ones' mean colour.
        let (mut sum, mut weight) = ([0.0_f32; 3], 0.0_f32);
        for t in &texels {
            for k in 0..3 {
                sum[k] += t[k] * t[3];
            }
            weight += t[3];
        }
        let mean = sum.map(|c| c / weight.max(1e-6));
        for t in &mut texels {
            let w = t[3];
            for k in 0..3 {
                t[k] = t[k] * w + mean[k] * (1.0 - w);
            }
        }
        let encode = |level: &[[f32; 4]]| {
            encode(level, |c| {
                [
                    to_byte(linear_to_srgb(c[0])),
                    to_byte(linear_to_srgb(c[1])),
                    to_byte(linear_to_srgb(c[2])),
                    to_byte(c[3]),
                ]
            })
        };
        // The share of quarter `q`'s texels an alpha test at a half keeps, the alpha times `scale`.
        let kept = |level: &[[f32; 4]], side: u32, q: u32, scale: f32| {
            let half = side / 2;
            let (x0, y0) = ((q % 2) * half, (q / 2) * half);
            let mut n = 0;
            for y in y0..y0 + half {
                for x in x0..x0 + half {
                    if level[(y * side + x) as usize][3] * scale >= 0.5 {
                        n += 1;
                    }
                }
            }
            n as f32 / (half * half) as f32
        };
        let cover: [f32; 4] = std::array::from_fn(|q| kept(&texels, size, q as u32, 1.0));
        let mut levels = vec![encode(&texels)];
        let (mut current, mut side) = (texels, size);
        while side > 1 {
            current = downsample(&current, side, |a, b, c, d| {
                let alpha = a[3] + b[3] + c[3] + d[3];
                let colour = [0, 1, 2].map(|k| {
                    if alpha > 1e-6 {
                        (a[k] * a[3] + b[k] * b[3] + c[k] * c[3] + d[k] * d[3]) / alpha
                    } else {
                        (a[k] + b[k] + c[k] + d[k]) * 0.25
                    }
                });
                [colour[0], colour[1], colour[2], alpha * 0.25]
            });
            side /= 2;
            // Each quarter's alpha scaled so the cards' alpha test keeps the share of it level 0
            // keeps (Castaño 2010's coverage-preserving mips): the plain means fall under the
            // cut-off as the leaves blur, and the far crowns thinned to their cores. The means stay
            // unscaled for the next level.
            let mut scaled = current.clone();
            if side >= 2 {
                for q in 0..4 {
                    let (mut lo, mut hi) = (0.25_f32, 16.0_f32);
                    for _ in 0..24 {
                        let mid = (lo * hi).sqrt();
                        if kept(&current, side, q, mid) < cover[q as usize] {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    let half = side / 2;
                    let (x0, y0) = ((q % 2) * half, (q / 2) * half);
                    for y in y0..y0 + half {
                        for x in x0..x0 + half {
                            let t = &mut scaled[(y * side + x) as usize];
                            t[3] = (t[3] * hi).min(1.0);
                        }
                    }
                }
            }
            levels.push(encode(&scaled));
        }
        TextureData {
            name: "leaves".into(),
            size,
            height: size,
            srgb: true,
            levels,
        }
    })
}

/// What a model's image holds, which decides how its mips are made (#166, D-047).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageUse {
    /// sRGB colour (base colour, emissive): averaged in linear light.
    Color,
    /// A tangent-space normal map: the vectors averaged and renormalised.
    Normal,
    /// Linear data (metalness, roughness, occlusion): averaged as stored.
    Data,
}

/// Texels a chunk of [`decode_image`]'s parallel work.
const DECODE_CHUNK: usize = 1 << 16;
/// The fewest rows of a mip a thread takes (a thread costs tens of microseconds to start).
const MIP_ROWS_PER_THREAD: usize = 16;

/// A model's encoded image (PNG or JPEG) as a texture with its mips, for `use`. A side that is
/// not a power of two is resampled up to the next one first, so every level halves exactly.
pub fn decode_image(name: &str, bytes: &[u8], usage: ImageUse) -> image::ImageResult<TextureData> {
    let rgba = image::load_from_memory(bytes)?.to_rgba8();
    let (w, h) = rgba.dimensions();
    let (pw, ph) = (w.next_power_of_two(), h.next_power_of_two());
    // Every texel, every level's encoding and every mip texel stands on its own, so they are
    // worked out in parallel, in chunks the threads share (#220: the planet's 16K colour map took
    // 7 s of one thread at every start), and each byte's value in linear light once. The same
    // bits as one thread texel by texel.
    let linear: [f32; 256] = std::array::from_fn(|b| srgb_to_linear(b as f32 / 255.0));
    let raw = rgba.as_raw();
    let mut texels = vec![[0.0_f32; 4]; raw.len() / 4];
    par_chunks_mut(&mut texels, DECODE_CHUNK, 1, |i, chunk| {
        let first = 4 * i * DECODE_CHUNK;
        for (texel, p) in chunk.iter_mut().zip(raw[first..].as_chunks::<4>().0) {
            let c = [p[0], p[1], p[2], p[3]].map(|b| f32::from(b) / 255.0);
            *texel = match usage {
                ImageUse::Color => [
                    linear[usize::from(p[0])],
                    linear[usize::from(p[1])],
                    linear[usize::from(p[2])],
                    c[3],
                ],
                ImageUse::Normal => {
                    let n = normalize([0, 1, 2].map(|k| c[k] * 2.0 - 1.0));
                    [n[0], n[1], n[2], c[3]]
                }
                ImageUse::Data => c,
            };
        }
    });
    // Resampled in linear light (or as vectors), as the mips are averaged.
    if (pw, ph) != (w, h) {
        let linear =
            image::Rgba32FImage::from_raw(w, h, texels.concat()).expect("one texel per pixel");
        let resized =
            image::imageops::resize(&linear, pw, ph, image::imageops::FilterType::Triangle);
        texels = resized
            .pixels()
            .map(|p| match usage {
                ImageUse::Normal => {
                    let n = normalize([p.0[0], p.0[1], p.0[2]]);
                    [n[0], n[1], n[2], p.0[3]]
                }
                _ => p.0,
            })
            .collect();
    }
    let encode = |level: &[[f32; 4]]| -> Vec<u8> {
        let mut bytes = vec![0_u8; 4 * level.len()];
        par_chunks_mut(&mut bytes, 4 * DECODE_CHUNK, 1, |i, chunk| {
            let first = i * DECODE_CHUNK;
            for (out, c) in chunk.as_chunks_mut::<4>().0.iter_mut().zip(&level[first..]) {
                *out = match usage {
                    ImageUse::Color => [
                        to_byte(linear_to_srgb(c[0])),
                        to_byte(linear_to_srgb(c[1])),
                        to_byte(linear_to_srgb(c[2])),
                        to_byte(c[3]),
                    ],
                    ImageUse::Normal => [
                        to_byte(c[0] * 0.5 + 0.5),
                        to_byte(c[1] * 0.5 + 0.5),
                        to_byte(c[2] * 0.5 + 0.5),
                        to_byte(c[3]),
                    ],
                    ImageUse::Data => c.map(to_byte),
                };
            }
        });
        bytes
    };
    let mut levels = vec![encode(&texels)];
    let (mut current, mut width, mut height) = (texels, pw, ph);
    while width > 1 || height > 1 {
        let (nw, nh) = ((width / 2).max(1), (height / 2).max(1));
        let mut next = vec![[0.0_f32; 4]; (nw * nh) as usize];
        // A row a chunk.
        par_chunks_mut(&mut next, nw as usize, MIP_ROWS_PER_THREAD, |y, row| {
            let y = y as u32;
            for (x, texel) in (0..nw).zip(row.iter_mut()) {
                let at = |dx: u32, dy: u32| {
                    let (sx, sy) = ((2 * x + dx).min(width - 1), (2 * y + dy).min(height - 1));
                    current[(sy * width + sx) as usize]
                };
                let (a, b, c, d) = (at(0, 0), at(1, 0), at(0, 1), at(1, 1));
                let mut mean = [0, 1, 2, 3].map(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25);
                if usage == ImageUse::Normal {
                    let n = normalize([mean[0], mean[1], mean[2]]);
                    mean = [n[0], n[1], n[2], mean[3]];
                }
                *texel = mean;
            }
        });
        (current, width, height) = (next, nw, nh);
        levels.push(encode(&current));
    }
    Ok(TextureData {
        name: name.to_owned().into(),
        size: pw,
        height: ph,
        srgb: usage == ImageUse::Color,
        levels,
    })
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
        name: "mip ramp".into(),
        size,
        height: size,
        srgb: false,
        levels,
    }
}

/// A decal's texture (D-074, `forge_render::decals`) from its linear colour and alpha per texel:
/// sRGB colour premultiplied by its alpha, each mip the plain average of its four texels' (colour
/// times alpha, and alpha), so a soft edge fades into the surface's own colour with distance
/// rather than into the colour of the texels it leaves bare.
fn premultiplied_texture(name: &'static str, size: u32, linear: Vec<[f32; 4]>) -> TextureData {
    let mut current: Vec<[f32; 4]> = linear
        .into_iter()
        .map(|[r, g, b, a]| [r * a, g * a, b * a, a])
        .collect();
    let encode = |level: &[[f32; 4]]| {
        encode(level, |c| {
            [
                to_byte(linear_to_srgb(c[0])),
                to_byte(linear_to_srgb(c[1])),
                to_byte(linear_to_srgb(c[2])),
                to_byte(c[3]),
            ]
        })
    };
    let mut levels = vec![encode(&current)];
    let mut side = size;
    while side > 1 {
        current = downsample(&current, side, |a, b, c, d| {
            [0, 1, 2, 3].map(|k| (a[k] + b[k] + c[k] + d[k]) * 0.25)
        });
        side /= 2;
        levels.push(encode(&current));
    }
    TextureData {
        name: name.into(),
        size,
        height: size,
        srgb: true,
        levels,
    }
}

/// Grime at the foot of a wall (D-074): rain splashed up with soil, densest at the ground
/// (`v` = 1) and gone by the top (`v` = 0), its upper edge wandering and speckled where the
/// splashes thin out; it repeats along `u`, for a strip 2 m a repeat and 1.1 m high whose foot
/// stands 0.1 m under the ground.
pub fn decal_grime(seed: u64, size: u32) -> TextureData {
    kept("texture-decal-grime", &(seed, size), || {
        let texels = grid(size, |u, v| {
            // Metres over the ground, and the band's top along the wall.
            let h = 1.0 - 1.1 * v;
            let edge = 0.28 + 0.34 * fbm(seed, u, 0.37, 3, 3);
            let band = 1.0 - smoothstep(edge - 0.32, edge + 0.05, h);
            let foot = 1.0 - smoothstep(0.0, 0.22, h);
            let mottle = fbm(seed ^ 0x6A1, u, v, 8, 4);
            let speck = smoothstep(0.62, 0.78, fbm(seed ^ 0x5EC, u, v, 48, 2))
                * (1.0 - smoothstep(edge, edge + 0.22, h))
                * smoothstep(edge - 0.2, edge, h);
            let alpha =
                ((0.58 * band + 0.3 * foot) * (0.55 + 0.45 * mottle) + 0.3 * speck).clamp(0.0, 0.9);
            // Brown soil, greener and darker at the very foot where it stays damp.
            let soil = [0.19, 0.16, 0.12];
            let damp = [0.11, 0.12, 0.09];
            let c: [f32; 3] = std::array::from_fn(|k| soil[k] + (damp[k] - soil[k]) * foot);
            [c[0], c[1], c[2], alpha]
        });
        premultiplied_texture("decal grime", size, texels)
    })
}

/// A stain on a paving (D-074): a soft, ragged patch of grime where water stood and dried, its
/// rim a little darker, faint and mottled; a stamp clamped at its edges, gone before them.
pub fn decal_stain(seed: u64, size: u32) -> TextureData {
    kept("texture-decal-stain", &(seed, size), || {
        let texels = grid(size, |u, v| {
            // 0 in the middle, 1 at the middle of an edge, ragged by broad noise.
            let r = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() * 2.0;
            let reach = r + 0.4 * (fbm(seed, u, v, 3, 4) - 0.5);
            let patch = 1.0 - smoothstep(0.35, 0.75, reach);
            let rim = smoothstep(0.45, 0.68, reach) * patch;
            let mottle = fbm(seed ^ 0x57A, u, v, 12, 3);
            let alpha = ((patch * (0.32 + 0.3 * mottle) + rim * 0.15)
                * (1.0 - smoothstep(0.85, 1.0, r)))
            .clamp(0.0, 0.7);
            [0.09, 0.08, 0.065, alpha]
        });
        premultiplied_texture("decal stain", size, texels)
    })
}

/// Cracks in a paving or a render (D-074): a few dark lines along the edges of a Voronoi tiling,
/// thinning and fading out towards the stamp's rim; a stamp clamped at its edges.
pub fn decal_cracks(seed: u64, size: u32) -> TextureData {
    kept("texture-decal-cracks", &(seed, size), || {
        const CELLS: i32 = 5;
        let point = |i: i32, j: i32| {
            let h = |s: u64| unit_f32(hash_cell2(seed ^ s, i, j));
            [
                i as f32 + 0.15 + 0.7 * h(0x51),
                j as f32 + 0.15 + 0.7 * h(0x52),
            ]
        };
        let texels = grid(size, |u, v| {
            let (x, y) = (u * CELLS as f32, v * CELLS as f32);
            let (ci, cj) = (x.floor() as i32, y.floor() as i32);
            // The nearest two points: a crack where they are equally near.
            let (mut d1, mut d2) = (f32::MAX, f32::MAX);
            for j in cj - 1..=cj + 1 {
                for i in ci - 1..=ci + 1 {
                    let p = point(i, j);
                    let d = ((p[0] - x).powi(2) + (p[1] - y).powi(2)).sqrt();
                    if d < d1 {
                        d2 = d1;
                        d1 = d;
                    } else if d < d2 {
                        d2 = d;
                    }
                }
            }
            let rim = ((u - 0.5).powi(2) + (v - 0.5).powi(2)).sqrt() * 2.0;
            let width = 0.06 * (1.0 - rim).max(0.0);
            let line = 1.0 - smoothstep(width * 0.3, width, d2 - d1);
            let wander = fbm(seed ^ 0xC4A, u, v, 8, 3);
            let alpha = (line * smoothstep(0.35, 0.65, wander + 0.3 * (1.0 - rim))).clamp(0.0, 1.0);
            [0.05, 0.045, 0.04, alpha * 0.9]
        });
        premultiplied_texture("decal cracks", size, texels)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_textures_read_back_byte_for_byte() {
        let dir = std::env::temp_dir().join(format!("forge-kept-textures-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cache = DerivedCache::new(&dir);
        let key = KeyHasher::new().debug(&(5, 64)).key(1);
        let made = cache.get_or_make("texture-facade", key, || facade(5, 64, 0.4));
        let read = cache.get_or_make("texture-facade", key, || facade(5, 64, 0.4));
        assert!(!made.from_cache && read.from_cache);
        for (a, b) in made.value.iter().zip(&read.value) {
            assert_eq!(
                (&a.name, a.size, a.height, a.srgb),
                (&b.name, b.size, b.height, b.srgb)
            );
            assert_eq!(a.levels, b.levels);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        // So do the cells of the stones, the fragments and the shrubs.
        let (a, b) = (worley(3, 0.999_99, 0.4, 14), worley(3, -0.000_01, 0.4, 14));
        assert!((a.0 - b.0).abs() < 1e-3 && a.2 == b.2);
    }

    #[test]
    fn every_texture_has_its_full_mip_chain() {
        for texture in rock(1, 64)
            .into_iter()
            .chain(concrete(1, 64))
            .chain(brick(1, 64))
            .chain(grass(1, 64))
            .chain(gravel(1, 64))
            .chain(scree(1, 64))
            .chain(scrub(1, 64))
            .chain([mip_ramp(64)])
        {
            assert_eq!(texture.levels.len(), 7, "{}", texture.name);
            for (k, level) in texture.levels.iter().enumerate() {
                let side = 64 >> k;
                assert_eq!(level.len(), side * side * 4, "{} level {k}", texture.name);
            }
        }
    }

    /// Each quarter of the leaf atlas is a spray covering a third to two thirds of it, nothing
    /// on its edges (the cards' margins), and an alpha test at a half keeps the same share of
    /// it at every mip down to 4 × 4 texels a quarter. `FORGE_LEAVES_PNG=path` writes level 0.
    #[test]
    fn the_leaf_atlas_s_sprays_cover_their_quarters_and_keep_it_in_the_mips() {
        let size = 256;
        let atlas = leaves(3, size);
        assert_eq!(atlas.levels.len(), 9);
        if let Ok(path) = std::env::var("FORGE_LEAVES_PNG") {
            image::save_buffer(&path, &atlas.levels[0], size, size, image::ColorType::Rgba8)
                .expect("written");
        }
        let alpha = |level: usize, x: u32, y: u32| {
            let side = size >> level;
            f32::from(atlas.levels[level][((y * side + x) * 4 + 3) as usize]) / 255.0
        };
        let half = size / 2;
        for spray in LeafSpray::ALL {
            let q = spray.quarter();
            let (x0, y0) = ((q % 2) * half, (q / 2) * half);
            let mut cover = 0.0;
            for y in y0..y0 + half {
                for x in x0..x0 + half {
                    cover += alpha(0, x, y);
                }
            }
            cover /= (half * half) as f32;
            assert!((0.3..0.7).contains(&cover), "{spray:?}: {cover:.2}");
            for k in 0..half {
                for (x, y) in [(x0 + k, y0), (x0, y0 + k), (x0 + k, y0 + half - 1)] {
                    assert_eq!(alpha(0, x, y), 0.0, "{spray:?} at {x}, {y}");
                }
            }
            // The share the test keeps at each level, against level 0's.
            let kept = |level: usize| {
                let quarter = half >> level;
                let mut n = 0;
                for y in 0..quarter {
                    for x in 0..quarter {
                        if alpha(level, (x0 >> level) + x, (y0 >> level) + y) >= 0.5 {
                            n += 1;
                        }
                    }
                }
                n as f32 / (quarter * quarter) as f32
            };
            let base = kept(0);
            for level in 1..=5 {
                let share = kept(level);
                // A quarter of 4 × 4 texels keeps whole sixteenths.
                assert!(
                    (share - base).abs() <= 0.04_f32.max(1.0 / (half >> level).pow(2) as f32),
                    "{spray:?} at level {level}: {share:.3} of it kept, {base:.3} at level 0"
                );
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

    /// `width × height` RGBA texels from `f(x, y)`, encoded as a PNG.
    fn png(width: u32, height: u32, f: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_fn(width, height, |x, y| image::Rgba(f(x, y)));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    /// A model's image of any size: resampled to powers of two, every level down to 1 × 1 on
    /// both sides, a colour's mean kept in linear light, normals unit.
    #[test]
    fn a_model_s_image_gets_its_whole_chain() {
        // 100 × 30: 128 × 32, eight levels (the last 1 × 1).
        let checker = png(100, 30, |x, y| {
            let v = if (x / 5 + y / 5) % 2 == 0 { 255 } else { 0 };
            [v, v, v, 255]
        });
        let color = decode_image("checker", &checker, ImageUse::Color).unwrap();
        assert_eq!((color.size, color.height), (128, 32));
        assert!(color.srgb);
        assert_eq!(color.levels.len(), 8);
        for (k, level) in color.levels.iter().enumerate() {
            let (w, h) = ((128 >> k).max(1), (32 >> k).max(1));
            assert_eq!(level.len(), w * h * 4, "level {k}");
        }
        // Black and white in equal parts: half the light, so about 188 in sRGB, not 128.
        let last = color.levels.last().unwrap()[0];
        assert!((180..=196).contains(&last), "{last}");
        let data = decode_image("checker", &checker, ImageUse::Data).unwrap();
        assert!(!data.srgb);
        let last = data.levels.last().unwrap()[0];
        assert!((120..=136).contains(&last), "{last}");
        // Normals tilting either way average to straight up, unit.
        let bumps = png(8, 8, |x, _| {
            if x % 2 == 0 {
                [218, 128, 218, 255]
            } else {
                [38, 128, 218, 255]
            }
        });
        let normal = decode_image("bumps", &bumps, ImageUse::Normal).unwrap();
        let n = &normal.levels.last().unwrap()[..3];
        assert!((n[0] as i32 - 128).abs() <= 1 && n[2] >= 254, "{n:?}");
        assert!(decode_image("garbage", b"not an image", ImageUse::Color).is_err());
    }

    #[test]
    fn the_ramp_holds_its_level_in_every_level() {
        let ramp = mip_ramp(16);
        for (k, level) in ramp.levels.iter().enumerate() {
            assert_eq!(level[0], to_byte(k as f32 / 16.0));
        }
    }
}
