//! Block-compressed textures from DDS files (D-083 step 1, the Bistro): BC1, BC3, BC5 and BC7
//! kept as their blocks with the file's mips, uploaded as they are ([`crate::material::TextureSet::add_compressed`]).
//! The legacy header's four-character codes (DXT1, DXT5, ATI2, BC5U) and the DX10 header's DXGI
//! formats are read; other formats are an error.
//!
//! A DirectX normal map points its green down the image: [`flip_green`] turns a BC5 map's green
//! over on its blocks, so it reads as the glTF maps the shaders expect. A BC5 map has no blue:
//! it is sampled through a view reading blue as one (`TextureSet::add_compressed`).
//!
//! A BC5 normal map's levels hold unit normals, so the spread of the bumps a coarse level averages
//! is lost; [`bc5_normals_to_bc7`] rebuilds its levels from the unit normals, unnormalised, as BC7
//! (Intel's ISPC Texture Compressor through `intel_tex_2`), and [`write_dds`] keeps them on disk
//! (D-086).

use std::fmt;

use forge_gpu::vk;
use glam::Vec3;

/// A block-compressed format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFormat {
    /// Colour with a one-bit alpha, 8 bytes a block (DXT1).
    Bc1,
    /// Colour with an interpolated alpha, 16 bytes a block (DXT5).
    Bc3,
    /// Two channels, red and green, 16 bytes a block (ATI2): normal maps.
    Bc5,
    /// Colour with alpha, 16 bytes a block.
    Bc7,
}

impl BlockFormat {
    /// Bytes a 4 × 4 block.
    pub fn block_bytes(self) -> usize {
        match self {
            BlockFormat::Bc1 => 8,
            BlockFormat::Bc3 | BlockFormat::Bc5 | BlockFormat::Bc7 => 16,
        }
    }

    /// The Vulkan format, sRGB-encoded colour when `srgb` (BC5 is always linear).
    pub fn vk_format(self, srgb: bool) -> vk::Format {
        match (self, srgb) {
            (BlockFormat::Bc1, true) => vk::Format::BC1_RGBA_SRGB_BLOCK,
            (BlockFormat::Bc1, false) => vk::Format::BC1_RGBA_UNORM_BLOCK,
            (BlockFormat::Bc3, true) => vk::Format::BC3_SRGB_BLOCK,
            (BlockFormat::Bc3, false) => vk::Format::BC3_UNORM_BLOCK,
            (BlockFormat::Bc5, _) => vk::Format::BC5_UNORM_BLOCK,
            (BlockFormat::Bc7, true) => vk::Format::BC7_SRGB_BLOCK,
            (BlockFormat::Bc7, false) => vk::Format::BC7_UNORM_BLOCK,
        }
    }

    /// Bytes of a level `width` × `height` texels.
    pub fn level_bytes(self, width: u32, height: u32) -> usize {
        width.div_ceil(4).max(1) as usize * height.div_ceil(4).max(1) as usize * self.block_bytes()
    }
}

/// A texture kept as its blocks, every level the file holds.
#[derive(Clone)]
pub struct CompressedTexture {
    /// A name for the debugger and the logs.
    pub name: String,
    /// Level 0's width and height in texels.
    pub width: u32,
    /// Level 0's height.
    pub height: u32,
    /// Its blocks' format.
    pub format: BlockFormat,
    /// Sampled as sRGB-encoded colour (base colour, emission), else linear data.
    pub srgb: bool,
    /// Each level's blocks, rows of blocks top to bottom, level 0 first.
    pub levels: Vec<Vec<u8>>,
}

impl fmt::Debug for CompressedTexture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompressedTexture")
            .field("name", &self.name)
            .field("width", &self.width)
            .field("height", &self.height)
            .field("format", &self.format)
            .field("srgb", &self.srgb)
            .field("levels", &self.levels.len())
            .finish()
    }
}

/// Why a DDS file does not read.
#[derive(Debug)]
pub struct DdsError(String);

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DdsError {}

fn error(message: impl Into<String>) -> DdsError {
    DdsError(message.into())
}

/// The widest and highest a texture read may be, texels: what every Vulkan device samples
/// (`maxImageDimension2D` is at least 16384 on the GPUs Forge runs on).
const MAX_SIDE: u32 = 16384;

/// The little-endian word at `at`.
fn word(bytes: &[u8], at: usize) -> Result<u32, DdsError> {
    bytes
        .get(at..at + 4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .ok_or_else(|| error("truncated header"))
}

/// The texture in DDS file `bytes`, called `name`, sampled as sRGB colour when `srgb`: its
/// blocks and every level the file holds (one where it says none).
pub fn read_dds(name: &str, bytes: &[u8], srgb: bool) -> Result<CompressedTexture, DdsError> {
    if bytes.get(0..4) != Some(b"DDS ") {
        return Err(error("not a DDS file"));
    }
    if word(bytes, 4)? != 124 {
        return Err(error("a header that is not 124 bytes"));
    }
    let height = word(bytes, 12)?;
    let width = word(bytes, 16)?;
    // The level count only where the header says it holds one (DDSD_MIPMAPCOUNT), and never more
    // than the image has.
    let mips = if word(bytes, 8)? & 0x2_0000 != 0 {
        word(bytes, 28)?.max(1)
    } else {
        1
    };
    let four_cc = bytes.get(84..88).ok_or_else(|| error("truncated header"))?;
    let (format, data) = match four_cc {
        b"DXT1" => (BlockFormat::Bc1, 128),
        b"DXT5" => (BlockFormat::Bc3, 128),
        b"ATI2" | b"BC5U" => (BlockFormat::Bc5, 128),
        b"DX10" => {
            let dxgi = word(bytes, 128)?;
            let format = match dxgi {
                71 | 72 => BlockFormat::Bc1,
                77 | 78 => BlockFormat::Bc3,
                83 => BlockFormat::Bc5,
                98 | 99 => BlockFormat::Bc7,
                other => return Err(error(format!("DXGI format {other} is not read"))),
            };
            (format, 148)
        }
        other => {
            return Err(error(format!(
                "format {:?} is not read",
                String::from_utf8_lossy(other)
            )));
        }
    };
    if width == 0 || height == 0 {
        return Err(error("an empty image"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(error(format!(
            "{width} × {height} is larger than {MAX_SIDE} a side"
        )));
    }
    let mips = mips.min(32 - width.max(height).leading_zeros());
    let mut levels = Vec::with_capacity(mips as usize);
    let mut at = data;
    for level in 0..mips {
        let (w, h) = ((width >> level).max(1), (height >> level).max(1));
        let size = format.level_bytes(w, h);
        let Some(blocks) = bytes.get(at..at + size) else {
            // A file that holds fewer levels than it says keeps those it holds.
            if levels.is_empty() {
                return Err(error("truncated level 0"));
            }
            break;
        };
        levels.push(blocks.to_vec());
        at += size;
        if w == 1 && h == 1 {
            break;
        }
    }
    Ok(CompressedTexture {
        name: name.to_owned(),
        width,
        height,
        format,
        srgb: srgb && format != BlockFormat::Bc5,
        levels,
    })
}

/// The palette index map that turns a BC4 channel block's values over (v → 255 − v) once its
/// endpoints are swapped and turned over: with e0 > e1, eight values from e0 to e1; else six
/// and 0 and 255.
const FLIP_EIGHT: [u8; 8] = [1, 0, 7, 6, 5, 4, 3, 2];
const FLIP_SIX: [u8; 8] = [1, 0, 5, 4, 3, 2, 7, 6];

/// BC4 channel block `block` (8 bytes) with its values turned over (v → 255 − v, to a unit of
/// the interpolation's rounding): the endpoints swapped and turned over keep the block's mode,
/// and its indices are remapped to the matching entries.
fn flip_channel(block: &mut [u8]) {
    let (e0, e1) = (block[0], block[1]);
    let map = if e0 > e1 { FLIP_EIGHT } else { FLIP_SIX };
    block[0] = 255 - e1;
    block[1] = 255 - e0;
    let mut bits = block[2..8]
        .iter()
        .rev()
        .fold(0_u64, |acc, &b| (acc << 8) | u64::from(b));
    let mut flipped = 0_u64;
    for texel in 0..16 {
        let index = (bits & 7) as u8;
        bits >>= 3;
        flipped |= u64::from(map[usize::from(index)]) << (3 * texel);
    }
    for (k, b) in block[2..8].iter_mut().enumerate() {
        *b = (flipped >> (8 * k)) as u8;
    }
}

/// `texture` (BC5) with its green turned over on every level: a DirectX normal map read as an
/// OpenGL (glTF) one. Other formats are left as they are.
pub fn flip_green(texture: &mut CompressedTexture) {
    if texture.format != BlockFormat::Bc5 {
        return;
    }
    for level in &mut texture.levels {
        for block in level.as_chunks_mut::<16>().0 {
            flip_channel(&mut block[8..16]);
        }
    }
}

/// A BC4-style alpha or channel block's eight values.
fn channel_palette(e0: u8, e1: u8) -> [u8; 8] {
    let (a, b) = (u32::from(e0), u32::from(e1));
    if e0 > e1 {
        let mix = |k: u32| ((a * (7 - k) + b * k + 3) / 7) as u8;
        [e0, e1, mix(1), mix(2), mix(3), mix(4), mix(5), mix(6)]
    } else {
        let mix = |k: u32| ((a * (5 - k) + b * k + 2) / 5) as u8;
        [e0, e1, mix(1), mix(2), mix(3), mix(4), 0, 255]
    }
}

/// The 16 values of BC4 channel block `block` (8 bytes), row by row.
fn channel_values(block: &[u8]) -> [u8; 16] {
    let palette = channel_palette(block[0], block[1]);
    let bits = block[2..8]
        .iter()
        .rev()
        .fold(0_u64, |acc, &b| (acc << 8) | u64::from(b));
    std::array::from_fn(|t| palette[((bits >> (3 * t)) & 7) as usize])
}

/// The alpha of `texture`'s level 0, a byte a texel, rows top to bottom: BC3's alpha blocks and
/// BC1's cut-out texels. `None` for BC5 and BC7 (not read here).
pub fn alpha_plane(texture: &CompressedTexture) -> Option<Vec<u8>> {
    let level = texture.levels.first()?;
    let (w, h) = (texture.width as usize, texture.height as usize);
    let blocks_x = w.div_ceil(4).max(1);
    let mut alpha = vec![255_u8; w * h];
    let mut put = |bx: usize, by: usize, values: [u8; 16]| {
        for (t, &v) in values.iter().enumerate() {
            let (x, y) = (4 * bx + t % 4, 4 * by + t / 4);
            if x < w && y < h {
                alpha[y * w + x] = v;
            }
        }
    };
    match texture.format {
        BlockFormat::Bc3 => {
            for (i, block) in level.as_chunks::<16>().0.iter().enumerate() {
                put(i % blocks_x, i / blocks_x, channel_values(&block[0..8]));
            }
        }
        BlockFormat::Bc1 => {
            for (i, block) in level.as_chunks::<8>().0.iter().enumerate() {
                let c0 = u16::from_le_bytes([block[0], block[1]]);
                let c1 = u16::from_le_bytes([block[2], block[3]]);
                let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
                // Index 3 is transparent black in the three-colour mode alone.
                let values = std::array::from_fn(|t| {
                    if c0 <= c1 && (indices >> (2 * t)) & 3 == 3 {
                        0
                    } else {
                        255
                    }
                });
                put(i % blocks_x, i / blocks_x, values);
            }
        }
        BlockFormat::Bc5 | BlockFormat::Bc7 => return None,
    }
    Some(alpha)
}

/// The 16 colours of a BC1 or BC3 colour block (BC3's after its 8 alpha bytes), 8-bit RGB, rows
/// of four top to bottom: BC1's three-colour mode gives black for index 3 (its cut-out texels).
fn colour_block(format: BlockFormat, colour: &[u8]) -> [[u8; 3]; 16] {
    let expand = |c: u16| {
        let (r, g, b) = ((c >> 11) & 31, (c >> 5) & 63, c & 31);
        [
            (r << 3) | (r >> 2),
            (g << 2) | (g >> 4),
            (b << 3) | (b >> 2),
        ]
        .map(|v| v as u8)
    };
    let c0 = u16::from_le_bytes([colour[0], colour[1]]);
    let c1 = u16::from_le_bytes([colour[2], colour[3]]);
    let (e0, e1) = (expand(c0), expand(c1));
    let mix = |a: u16, b: u16, d: u16| -> [u8; 3] {
        std::array::from_fn(|k| ((a * u16::from(e0[k]) + b * u16::from(e1[k])) / d) as u8)
    };
    // BC3's colour block always has four colours; BC1's three and black where c0 <= c1.
    let palette = if c0 > c1 || format == BlockFormat::Bc3 {
        [e0, e1, mix(2, 1, 3), mix(1, 2, 3)]
    } else {
        [e0, e1, mix(1, 1, 2), [0; 3]]
    };
    let indices = u32::from_le_bytes([colour[4], colour[5], colour[6], colour[7]]);
    std::array::from_fn(|t| palette[((indices >> (2 * t)) & 3) as usize])
}

/// The colour of a BC1 or BC3 texture's level 0, 8-bit RGB as stored (sRGB-encoded where the
/// texture is), a texel each, rows top to bottom; `None` for BC5 and BC7 (not read here).
pub fn color_plane(texture: &CompressedTexture) -> Option<Vec<[u8; 3]>> {
    let (size, at) = match texture.format {
        BlockFormat::Bc1 => (8, 0),
        BlockFormat::Bc3 => (16, 8),
        BlockFormat::Bc5 | BlockFormat::Bc7 => return None,
    };
    let level = texture.levels.first()?;
    let (w, h) = (texture.width as usize, texture.height as usize);
    let blocks_x = w.div_ceil(4).max(1);
    let mut plane = vec![[0_u8; 3]; w * h];
    for (i, block) in level.chunks_exact(size).enumerate() {
        let (bx, by) = (i % blocks_x, i / blocks_x);
        for (t, c) in colour_block(texture.format, &block[at..at + 8])
            .into_iter()
            .enumerate()
        {
            let (x, y) = (4 * bx + t % 4, 4 * by + t / 4);
            if x < w && y < h {
                plane[y * w + x] = c;
            }
        }
    }
    Some(plane)
}

/// The one colour every texel of a BC1 or BC3 texture's first level holds, 8-bit RGB (its alpha
/// aside), or `None` where they differ or the format is another: a map of one value, better
/// read as its material's factor.
pub fn uniform_color(texture: &CompressedTexture) -> Option<[u8; 3]> {
    let (size, at) = match texture.format {
        BlockFormat::Bc1 => (8, 0),
        BlockFormat::Bc3 => (16, 8),
        BlockFormat::Bc5 | BlockFormat::Bc7 => return None,
    };
    let mut seen = None;
    for block in texture.levels.first()?.chunks_exact(size) {
        for c in colour_block(texture.format, &block[at..at + 8]) {
            match seen {
                None => seen = Some(c),
                Some(s) if s != c => return None,
                Some(_) => {}
            }
        }
    }
    seen
}

/// The levels of a normal map from its level 0's unit normals (`width` × `height`, rows top to
/// bottom): each smaller level the mean of the four texels above it (two, one at an odd edge),
/// left unnormalised, so a level's normals shorten where those under them part.
pub fn normal_levels(
    width: usize,
    height: usize,
    level0: Vec<Vec3>,
) -> Vec<(usize, usize, Vec<Vec3>)> {
    let mut levels = vec![(width, height, level0)];
    loop {
        let (w, h, above) = levels.last().expect("level 0");
        if *w == 1 && *h == 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut level = Vec::with_capacity(nw * nh);
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = Vec3::ZERO;
                let mut count = 0.0;
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = (2 * x + dx, 2 * y + dy);
                    if sx < *w && sy < *h {
                        sum += above[sy * w + sx];
                        count += 1.0;
                    }
                }
                level.push(sum / count);
            }
        }
        levels.push((nw, nh, level));
    }
    levels
}

/// A BC5 normal map (its green already turned over, [`flip_green`]) as BC7, every level (D-086):
/// level 0's normals made whole (z rebuilt, unit length), the smaller levels their unnormalised
/// means ([`normal_levels`]), each texel's xyz × 0.5 + 0.5 in RGB, alpha 255. The shader reads
/// the shortening as the normals' spread (Toksvig, #331), as it does an RGB map's; a BC5 map's
/// coarse levels said a bumpy surface was flat, its highlights too sharp and bright. `None` for
/// another format.
pub fn bc5_normals_to_bc7(texture: &CompressedTexture) -> Option<CompressedTexture> {
    if texture.format != BlockFormat::Bc5 {
        return None;
    }
    let (w, h) = (texture.width as usize, texture.height as usize);
    let blocks_x = w.div_ceil(4).max(1);
    let mut level0 = vec![Vec3::Z; w * h];
    for (i, block) in texture.levels.first()?.as_chunks::<16>().0.iter().enumerate() {
        let (bx, by) = (i % blocks_x, i / blocks_x);
        let (red, green) = (channel_values(&block[0..8]), channel_values(&block[8..16]));
        for t in 0..16 {
            let (x, y) = (4 * bx + t % 4, 4 * by + t / 4);
            if x < w && y < h {
                let nx = f32::from(red[t]) / 255.0 * 2.0 - 1.0;
                let ny = f32::from(green[t]) / 255.0 * 2.0 - 1.0;
                let nz = (1.0 - nx * nx - ny * ny).max(0.0).sqrt();
                level0[y * w + x] = Vec3::new(nx, ny, nz).normalize_or(Vec3::Z);
            }
        }
    }
    let settings = intel_tex_2::bc7::opaque_fast_settings();
    let levels = normal_levels(w, h, level0)
        .into_iter()
        .map(|(lw, lh, normals)| {
            // Padded to whole blocks with the edge's texels.
            let (pw, ph) = (lw.div_ceil(4) * 4, lh.div_ceil(4) * 4);
            let mut rgba = vec![0_u8; pw * ph * 4];
            for y in 0..ph {
                for x in 0..pw {
                    let n = normals[y.min(lh - 1) * lw + x.min(lw - 1)];
                    let at = (y * pw + x) * 4;
                    for (k, v) in n.to_array().into_iter().enumerate() {
                        rgba[at + k] = ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    rgba[at + 3] = 255;
                }
            }
            intel_tex_2::bc7::compress_blocks(
                &settings,
                &intel_tex_2::RgbaSurface {
                    data: &rgba,
                    width: pw as u32,
                    height: ph as u32,
                    stride: pw as u32 * 4,
                },
            )
        })
        .collect();
    Some(CompressedTexture {
        name: texture.name.clone(),
        width: texture.width,
        height: texture.height,
        format: BlockFormat::Bc7,
        srgb: false,
        levels,
    })
}

/// `texture` as a DDS file [`read_dds`] reads back: the DX10 header's DXGI format (BC1, BC3, BC5 or
/// BC7, sRGB where it is) and every level.
pub fn write_dds(texture: &CompressedTexture) -> Vec<u8> {
    let dxgi: u32 = match (texture.format, texture.srgb) {
        (BlockFormat::Bc1, false) => 71,
        (BlockFormat::Bc1, true) => 72,
        (BlockFormat::Bc3, false) => 77,
        (BlockFormat::Bc3, true) => 78,
        (BlockFormat::Bc5, _) => 83,
        (BlockFormat::Bc7, false) => 98,
        (BlockFormat::Bc7, true) => 99,
    };
    let mut bytes = vec![0_u8; 148];
    let mut put = |at: usize, v: u32| bytes[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(4, 124);
    // CAPS, HEIGHT, WIDTH, PIXELFORMAT, MIPMAPCOUNT, LINEARSIZE.
    put(8, 0x1 | 0x2 | 0x4 | 0x1000 | 0x2_0000 | 0x8_0000);
    put(12, texture.height);
    put(16, texture.width);
    put(20, texture.levels.first().map_or(0, Vec::len) as u32);
    put(28, texture.levels.len() as u32);
    put(76, 32);
    put(80, 0x4); // DDPF_FOURCC
    put(108, 0x1000 | 0x40_0000 | 0x8); // TEXTURE, MIPMAP, COMPLEX
    put(128, dxgi);
    put(132, 3); // a 2D texture
    put(140, 1); // one in the array
    bytes[0..4].copy_from_slice(b"DDS ");
    bytes[84..88].copy_from_slice(b"DX10");
    for level in &texture.levels {
        bytes.extend_from_slice(level);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DDS file of `width` × `height` in `four_cc` with `levels` levels of `fill` bytes.
    fn dds(four_cc: &[u8; 4], width: u32, height: u32, levels: u32, block: usize) -> Vec<u8> {
        let mut bytes = vec![0_u8; 128];
        bytes[0..4].copy_from_slice(b"DDS ");
        bytes[4..8].copy_from_slice(&124_u32.to_le_bytes());
        // DDSD_MIPMAPCOUNT.
        bytes[8..12].copy_from_slice(&0x2_0000_u32.to_le_bytes());
        bytes[12..16].copy_from_slice(&height.to_le_bytes());
        bytes[16..20].copy_from_slice(&width.to_le_bytes());
        bytes[28..32].copy_from_slice(&levels.to_le_bytes());
        bytes[84..88].copy_from_slice(four_cc);
        for level in 0..levels.max(1) {
            let (w, h) = ((width >> level).max(1), (height >> level).max(1));
            let n = w.div_ceil(4) as usize * h.div_ceil(4) as usize * block;
            bytes.extend(std::iter::repeat_n(level as u8, n));
        }
        bytes
    }

    #[test]
    fn a_dds_file_keeps_its_blocks_and_levels() {
        // 16 × 8 in BC1: 4 × 2 blocks of 8 bytes, then 2 × 1, 1 × 1 (4 × 2 texels), 1 × 1 twice.
        let t = read_dds("t", &dds(b"DXT1", 16, 8, 5, 8), true).expect("reads");
        assert_eq!(
            (t.width, t.height, t.format, t.srgb),
            (16, 8, BlockFormat::Bc1, true)
        );
        assert_eq!(
            t.levels.iter().map(Vec::len).collect::<Vec<_>>(),
            [64, 16, 8, 8, 8]
        );
        assert!(t.levels[3].iter().all(|&b| b == 3));
        // BC5 is never sRGB; a level count of 0 is one level.
        let n = read_dds("n", &dds(b"ATI2", 8, 8, 0, 16), true).expect("reads");
        assert_eq!(
            (n.format, n.srgb, n.levels.len()),
            (BlockFormat::Bc5, false, 1)
        );
        // Without DDSD_MIPMAPCOUNT, one level whatever the count says; a count past the
        // image's own is cut to it.
        let mut flagless = dds(b"DXT1", 16, 8, 5, 8);
        flagless[8..12].fill(0);
        assert_eq!(
            read_dds("f", &flagless, false).expect("reads").levels.len(),
            1
        );
        let mut huge = dds(b"DXT1", 16, 8, 5, 8);
        huge[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(read_dds("h", &huge, false).expect("reads").levels.len(), 5);
        assert!(read_dds("x", &dds(b"DXT1", 32768, 4, 1, 8), false).is_err());
        assert!(read_dds("x", &dds(b"DXT3", 8, 8, 1, 16), false).is_err());
        assert!(read_dds("x", b"PNG...", false).is_err());
    }

    #[test]
    fn a_flipped_channel_reads_turned_over() {
        // Both modes, every index, endpoints in both orders.
        for (e0, e1) in [(200_u8, 40_u8), (40, 200), (255, 0), (0, 255), (90, 90)] {
            let mut block = [e0, e1, 0, 0, 0, 0, 0, 0];
            // Indices 0..7 then 7..0 over the 16 texels.
            let indices: Vec<u64> = (0..8).chain((0..8).rev()).collect();
            let bits = indices
                .iter()
                .enumerate()
                .fold(0_u64, |acc, (t, &i)| acc | (i << (3 * t)));
            for k in 0..6 {
                block[2 + k] = (bits >> (8 * k)) as u8;
            }
            let before = channel_values(&block);
            flip_channel(&mut block);
            let after = channel_values(&block);
            for (b, a) in before.iter().zip(after) {
                assert!(
                    (i32::from(255 - b) - i32::from(a)).abs() <= 1,
                    "{e0} {e1}: {b} -> {a}"
                );
            }
            // Twice is the block itself.
            let mut again = block;
            flip_channel(&mut again);
            let mut back = again;
            flip_channel(&mut back);
            assert_eq!(back, block);
        }
    }

    #[test]
    fn alpha_planes_follow_the_blocks() {
        // BC3: one block whose alpha is 0 at index 6 of the six-value mode.
        let mut t = CompressedTexture {
            name: "a".into(),
            width: 4,
            height: 4,
            format: BlockFormat::Bc3,
            srgb: true,
            levels: vec![vec![0; 16]],
        };
        // Endpoints 10 ≤ 250: index 6 is 0, index 7 is 255; texel 0 takes 6, the rest 7.
        let bits: u64 = 6 | (0o7777777777777777_u64 << 3) & ((1 << 48) - 1);
        t.levels[0][0] = 10;
        t.levels[0][1] = 250;
        for k in 0..6 {
            t.levels[0][2 + k] = (bits >> (8 * k)) as u8;
        }
        let plane = alpha_plane(&t).expect("BC3 has alpha");
        assert_eq!(plane[0], 0);
        assert!(plane[1..].iter().all(|&a| a == 255));
        // BC1 in its three-colour mode: index 3 is cut out.
        t.format = BlockFormat::Bc1;
        t.levels = vec![vec![0, 0, 1, 0, 0b11, 0, 0, 0]];
        let plane = alpha_plane(&t).expect("BC1 has alpha");
        assert_eq!(plane[0], 0);
        assert!(plane[1..].iter().all(|&a| a == 255));
    }

    #[test]
    fn a_map_of_one_colour_reads_as_that_colour() {
        // Two BC3 blocks, each its colour block's endpoints both (0, 47, 31) in 5:6:5, every
        // texel at index 0: green 47 expands to 190, blue 31 to 255.
        let c = (47_u16 << 5) | 31;
        let mut block = [0_u8; 16];
        block[8..10].copy_from_slice(&c.to_le_bytes());
        block[10..12].copy_from_slice(&c.to_le_bytes());
        let mut t = CompressedTexture {
            name: "specular".into(),
            width: 8,
            height: 4,
            format: BlockFormat::Bc3,
            srgb: false,
            levels: vec![[block, block].concat()],
        };
        assert_eq!(uniform_color(&t), Some([0, 190, 255]));
        // One texel of the second block on index 1, another colour: no longer one value.
        t.levels[0][16 + 8] = 0;
        t.levels[0][16 + 12] = 1;
        assert_eq!(uniform_color(&t), None);
        // BC1's three-colour mode: index 3 is black.
        t.format = BlockFormat::Bc1;
        t.levels = vec![vec![0, 0, 1, 0, 0xFF, 0xFF, 0xFF, 0xFF]];
        assert_eq!(uniform_color(&t), Some([0, 0, 0]));
        t.levels[0][4] = 0b10;
        assert_eq!(uniform_color(&t), None);
    }

    #[test]
    fn normal_levels_shorten_where_normals_part() {
        // 2 × 2: two normals leaning 60° apart each way along x, two flat.
        let lean = Vec3::new(0.5_f32.sqrt(), 0.0, 0.5_f32.sqrt());
        let levels = normal_levels(
            2,
            2,
            vec![lean, Vec3::new(-lean.x, 0.0, lean.z), Vec3::Z, Vec3::Z],
        );
        assert_eq!(levels.len(), 2);
        let (w, h, top) = &levels[1];
        assert_eq!((*w, *h), (1, 1));
        // The mean: z (0.707 + 0.707 + 1 + 1) / 4, x cancelled.
        assert!(
            (top[0] - Vec3::new(0.0, 0.0, 0.8536)).length() < 1e-3,
            "{}",
            top[0]
        );
        // An odd edge: 3 × 1 halves to 1 × 1 from its first two.
        let odd = normal_levels(3, 1, vec![Vec3::X, Vec3::Z, Vec3::Y]);
        assert_eq!(
            odd.iter().map(|l| (l.0, l.1)).collect::<Vec<_>>(),
            [(3, 1), (1, 1)]
        );
        assert!((odd[1].2[0] - Vec3::new(0.5, 0.0, 0.5)).length() < 1e-6);
    }

    #[test]
    fn a_bc5_normal_map_becomes_bc7_with_every_level() {
        // 8 × 4 of flat normals (128, 128): every level, BC7's 16 bytes a block.
        let flat = [128_u8, 128, 0, 0, 0, 0, 0, 0];
        let t = CompressedTexture {
            name: "normal".into(),
            width: 8,
            height: 4,
            format: BlockFormat::Bc5,
            srgb: false,
            levels: vec![[flat, flat, flat, flat].concat()],
        };
        let bc7 = bc5_normals_to_bc7(&t).expect("BC5 transcodes");
        assert_eq!(bc7.format, BlockFormat::Bc7);
        assert_eq!(
            bc7.levels.iter().map(Vec::len).collect::<Vec<_>>(),
            [32, 16, 16, 16]
        );
        // Back through a DDS file.
        let back = read_dds("normal", &write_dds(&bc7), false).expect("reads back");
        assert_eq!(
            (back.width, back.height, back.format),
            (8, 4, BlockFormat::Bc7)
        );
        assert_eq!(back.levels, bc7.levels);
        assert!(bc5_normals_to_bc7(&back).is_none());
    }

    #[test]
    fn a_colour_plane_follows_the_blocks() {
        // 8 × 4 in BC1, four-colour mode: the first block white throughout, the second red
        // (31, 0, 0) but for its texel (1, 2) on index 1, black.
        let white = [0xFF, 0xFF, 0, 0, 0, 0, 0, 0];
        let red = 31_u16 << 11;
        let mut second = [0_u8; 8];
        second[0..2].copy_from_slice(&red.to_le_bytes());
        // Texel 9 (x 1, y 2) on index 1.
        second[4..8].copy_from_slice(&(1_u32 << (2 * 9)).to_le_bytes());
        let t = CompressedTexture {
            name: "emissive".into(),
            width: 8,
            height: 4,
            format: BlockFormat::Bc1,
            srgb: true,
            levels: vec![[white, second].concat()],
        };
        let plane = color_plane(&t).expect("BC1 reads");
        assert_eq!(plane.len(), 32);
        assert_eq!(plane[0], [255; 3]);
        assert_eq!(plane[3 * 8 + 3], [255; 3]);
        assert_eq!(plane[4], [255, 0, 0]);
        assert_eq!(plane[2 * 8 + 5], [0, 0, 0]);
        assert_eq!(plane[3 * 8 + 7], [255, 0, 0]);
    }
}
