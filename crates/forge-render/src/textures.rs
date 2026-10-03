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
}

/// A shingle beach (#128): rounded, flattened pebbles of grey, blue-grey and brown stone with
/// the odd white quartz, worn smooth by the waves, lying apart on coarse sand. Not the river
/// bed's packed cobbles ([`gravel`]), which read as paving on a beach.
pub fn shingle(seed: u64, size: u32) -> [TextureData; 2] {
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
}

/// Granite (#129, D-042): the island's hills, coarse-grained, grey to pink, specked with pink
/// feldspar, white quartz and black mica; weathered smooth into slabs, crossed by the odd
/// curved sheet joint, stained by lichen.
pub fn granite(seed: u64, size: u32) -> [TextureData; 2] {
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
}

/// Grus (#135, D-042): the coarse sand granite rots into on its gentle ground. Loose grains of
/// pink feldspar, white quartz and black mica on a buff to ochre ground, small angular pieces of
/// the granite lying on it, and the odd tuft of dry grass.
pub fn grus(seed: u64, size: u32) -> [TextureData; 2] {
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
}

/// Limestone (#129, D-042): the island's low ground and sea cliffs, the old reefs raised with
/// it. Pale cream-grey, fine-grained, pitted where the rain dissolves it, blotched grey and
/// darker where it weathers.
pub fn limestone(seed: u64, size: u32) -> [TextureData; 2] {
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
}

/// Karst pavement (#129, D-042): the limestone's bare ground etched by the rain into blocks
/// (clints) a metre or two across, split by deep fissures (grikes) where moss and grass grow.
pub fn karst(seed: u64, size: u32) -> [TextureData; 2] {
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
}

/// Scree (#118): broken rock fallen from the walls above, angular fragments of pale grey stone,
/// each a flat face tilted its own way, dark in the cracks between them.
pub fn scree(seed: u64, size: u32) -> [TextureData; 2] {
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
}

/// Scrub on steep ground (#118): rounded shrubs, seven across a repeat, each its own shade of
/// dark green, olive or blue-green, their leaves breaking their outline, over stony soil darker
/// in their shade.
pub fn scrub(seed: u64, size: u32) -> [TextureData; 2] {
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
