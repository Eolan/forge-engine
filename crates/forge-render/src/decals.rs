//! Decals on static surfaces (D-074, #304): boxes that colour what stands inside them, applied in
//! the resolve between a row's surface and its lighting, so the sun, the probes, the ambient
//! occlusion and the reflections take them as part of the surface. A pixel finds the decals over
//! it in a world grid of [`CELL`] columns built once on the CPU ([`DecalSet::tables`]), each
//! column's list in painter's order (priority, then the order they were added). No geometry: no
//! z-fighting, and the texture's gradients come from the resolve's analytic derivatives, so its
//! mips and anisotropy hold at depth edges.
//!
//! A decal's box has its own frame: `x` along its texture's `u`, `z` along `v`, `y` the axis it
//! projects along (out of the surface it marks). Kinds take their colour from a texture of their
//! own ([`crate::textures::decal_grime`] and the like), premultiplied by its alpha in every mip,
//! clamped at its edges (a stamp) or repeating along `u` (a strip: the foot of a wall). A row
//! decal (`DecalKind::row`) takes from its texture only its coverage (its alpha, or its red with
//! `DecalAlpha::Mask`), a mask over a material row: where it masks, the surface is that row's (its
//! colour and its normal), as a layered ground's layer is; the layered class alone takes them for
//! now. Textures made elsewhere (a production's
//! block-compressed ones) come with their alpha apart (`DecalAlpha::Straight`) or as a mask over
//! the kind's tint (`DecalAlpha::Mask`), and a kind may show one cell of an atlas
//! (`DecalKind::uv_rect`). A colour decal may carry a normal map (`DecalKind::normal`), whose
//! slopes bend the surface's normal where it covers it.

use crate::columns::{ColumnTables, place};
use bytemuck::{Pod, Zeroable};
use forge_core::material::{MaterialId, TextureId};
use glam::{Quat, Vec3};

/// A grid column's side in the scene's frame, metres (`x` and `z`; a column holds every height).
pub const CELL: f32 = 4.0;
/// The most decals a column lists (a pixel walks at most these); more are left out, the lowest
/// priorities first, with a warning.
pub const MAX_PER_CELL: usize = 32;

/// How a kind's texture gives its colour and how much of the surface it covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecalAlpha {
    /// RGBA, its colour premultiplied by its alpha in every mip (Forge's own decal textures).
    #[default]
    Premultiplied,
    /// RGBA, its colour and its alpha apart (a block-compressed texture's), multiplied as it is
    /// read: its mips' edges darken a little where the colour under its clear texels is dark.
    Straight,
    /// Its red channel alone: how much of the kind's tint covers the surface (a mask over a flat
    /// colour; a one-channel texture's). The texture is uploaded linear: decoded as sRGB, a half
    /// coverage would read as about a fifth.
    Mask,
}

/// A kind of decal: how its texture marks a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecalKind {
    /// Its texture in the scene's [`crate::TextureSet`] (`TextureSet::add`), read as `alpha` says.
    pub texture: TextureId,
    /// How its texture gives its colour and its coverage.
    pub alpha: DecalAlpha,
    /// The part of its texture it shows: `u`, `v` of its corner and its width and height, in the
    /// texture's coordinates (an atlas's cell); `[0, 0, 1, 1]` for all of it. A strip repeats its
    /// cell along `u`, from its phase, within the cell. A stamp's edges and coarser mips read a
    /// little of the cells beside it where the atlas leaves no gutter.
    pub uv_rect: [f32; 4],
    /// What its colour is multiplied by (rgb), and its strength (a).
    pub tint: [f32; 4],
    /// Its texture repeats along the box's `x` (a strip: [`Decal::repeat`] times, from the decal's
    /// [`Decal::phase`]), else it is a stamp clamped at its edges.
    pub strip: bool,
    /// Its place among the decals a pixel walks: higher paints over lower.
    pub priority: u8,
    /// The share of the box's depth (`y`) over which it fades out towards both faces, 0 to 1.
    pub depth_fade: f32,
    /// The cosines between a surface's normal and the box's `y` under which the decal is gone and
    /// over which it is whole: it marks the faces it looks at, not those at right angles (the
    /// ground under a wall's grime).
    pub facing: [f32; 2],
    /// A row decal's material row (D-074, a mask over a material row): where its texture's alpha
    /// masks, the surface takes that row's colour (times [`Self::tint`]) and normal, as a layered
    /// ground's layer; the layered class alone draws them for now. `None`: a colour decal.
    pub row: Option<MaterialId>,
    /// A colour decal's tangent-space normal map (D-074), read as its texture is (its atlas cell,
    /// its strip's repeats): its red and green the slopes along the box's `x` (the texture's `u`)
    /// and up the image (towards −`v`, the box's −`z`), as Forge reads a mesh's maps. Where the
    /// decal covers the surface (its alpha times its strength) it bends the surface's normal,
    /// whatever the surface's own map: the cracks and dents a decal carries catch the light. A
    /// decal painted over it covers its relief as it covers its colour. The map is linear (uploaded
    /// with `srgb` false: read as sRGB, a flat texel would tilt by about 30°), and its coverage is
    /// the kind's texture's alpha, colour and relief together (no decal of relief alone yet).
    /// `None`: the surface's normal kept (Forge's own decals).
    pub normal: Option<TextureId>,
    /// The slopes of [`Self::normal`] times this (1: as the map gives them).
    pub normal_strength: f32,
    /// A colour decal's roughness map (D-074, #393), read as its texture is: its red plus
    /// [`Self::roughness_bias`], clamped to 0 to 1, is the (perceptual) roughness it lays where it
    /// covers the surface (its alpha times [`Self::roughness_strength`], with its fades), painted
    /// over the surface's own as its colour is: a stain of wet blood glossy on dry asphalt, a
    /// logo's paint on a matt wall. The map is linear (uploaded with `srgb` false). Every class
    /// takes it, the layered ground too.
    /// `None`: the surface's own roughness where it covers (Forge's own decals): it covers a
    /// roughness another decal laid under it, as it covers that decal's relief.
    pub roughness: Option<TextureId>,
    /// Added to [`Self::roughness`]'s red before it is clamped (0: as the map gives it).
    pub roughness_bias: f32,
    /// The share of its coverage [`Self::roughness`] takes: its texture's alpha times this, with
    /// its facing and depth fades (#395: a puddle's colour a third over the ground, its smoothness
    /// whole, as the source's material gives it). None: its strength (`tint`'s alpha), as its
    /// colour's. A standard row's pixel whose decals lower its roughness into the mirror rays'
    /// reach traces one, its share by how much they laid (#395, `r.reflections.decals`); the
    /// layered ground and the other classes keep their rows' reflections.
    pub roughness_strength: Option<f32>,
}

/// A decal: a box in the scene's frame and its kind.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Decal {
    /// The box's middle in the scene's frame (relative to the scene's origin, as an instance's
    /// transform), metres.
    pub centre: Vec3,
    /// The box's turn: its `x`, `y` and `z` axes in the scene's frame.
    pub rotation: Quat,
    /// The box's half sizes along its axes, metres.
    pub half: Vec3,
    /// Its kind: an index into [`DecalSet::kinds`].
    pub kind: u16,
    /// A strip's repeats of its texture along `x` (1 for a stamp).
    pub repeat: f32,
    /// Where a strip's texture starts along `x`, in repeats: a run of strips along a bank goes on
    /// where the one before it ended, and lone strips start apart so that neighbours do not
    /// repeat it.
    pub phase: f32,
}

/// A scene's decals and their kinds.
#[derive(Clone, Debug, Default)]
pub struct DecalSet {
    /// The kinds, indexed by [`Decal::kind`].
    pub kinds: Vec<DecalKind>,
    /// The decals.
    pub decals: Vec<Decal>,
}

/// Mirrors `Decal` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuDecal {
    centre: [f32; 3],
    kind: u32,
    rotation: [f32; 4],
    half: [f32; 3],
    repeat: f32,
    phase: f32,
    pad: [u32; 3],
}

/// Mirrors `DecalKind` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuDecalKind {
    texture: u32,
    /// The sampler: anisotropic, clamped (a stamp) or repeating along `u` (a strip).
    sampler: u32,
    depth_fade: f32,
    /// A row decal's material row, `u32::MAX` for a colour decal.
    row: u32,
    tint: [f32; 4],
    facing: [f32; 4],
    /// `DecalAlpha`: 0 premultiplied, 1 straight, 2 a red mask.
    alpha: u32,
    /// 1: a strip, its `u` wrapped within its cell.
    strip: u32,
    /// Its normal map, `TEXTURE_NONE` without one.
    normal: u32,
    normal_strength: f32,
    uv_rect: [f32; 4],
    /// Its roughness map, `TEXTURE_NONE` without one, the bias added to its red, and the share of
    /// its coverage it takes.
    roughness: u32,
    roughness_bias: f32,
    roughness_strength: f32,
    pad: u32,
}

const _: () = assert!(std::mem::size_of::<GpuDecalKind>() == 96);

/// Mirrors `DecalGrid` in `meshlet.slang`: where the tables stand and the hash's size.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct GpuDecalGrid {
    pub slots: u64,
    pub indices: u64,
    pub decals: u64,
    pub kinds: u64,
    /// The slots' count less one (a power of two less one).
    pub slot_mask: u32,
    pub cell: f32,
    pub count: u32,
    pub pad: u32,
}

/// `SAMPLER_ANISOTROPIC_WRAP` in `bindless.slang` and its wrap modes (u, v): 0 repeat, 1 clamp.
const ANISOTROPIC_WRAP: u32 = 6;

/// The tables the GPU reads, as bytes, each at its offset in one buffer: the hash's slots
/// (`x`, `z`, first, count; count 0 an empty slot), the columns' decal indices, the decals and
/// the kinds.
pub(crate) struct DecalTables {
    pub bytes: Vec<u8>,
    pub slots: u64,
    pub indices: u64,
    pub decals: u64,
    pub kinds: u64,
    pub slot_mask: u32,
    pub count: u32,
}

impl DecalSet {
    /// The columns of [`CELL`] a decal's box overlaps, as (`x`, `z`) cell indices.
    fn columns(d: &Decal) -> impl Iterator<Item = (i32, i32)> {
        let (lo, hi) = [-1.0_f32, 1.0]
            .iter()
            .flat_map(|&x| [-1.0_f32, 1.0].map(move |y| (x, y)))
            .flat_map(|(x, y)| [-1.0_f32, 1.0].map(move |z| Vec3::new(x, y, z)))
            .map(|c| d.centre + d.rotation * (c * d.half))
            .fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(p), hi.max(p)),
            );
        let (x0, x1) = ((lo.x / CELL).floor() as i32, (hi.x / CELL).floor() as i32);
        let (z0, z1) = ((lo.z / CELL).floor() as i32, (hi.z / CELL).floor() as i32);
        (x0..=x1).flat_map(move |x| (z0..=z1).map(move |z| (x, z)))
    }

    /// The GPU's tables: each column's decals sorted by (priority, index), [`MAX_PER_CELL`] at
    /// most, a kind's texture by its `sampled` index; `None` without decals.
    pub(crate) fn tables(&self, sampled: impl Fn(TextureId) -> u32) -> Option<DecalTables> {
        if self.decals.is_empty() {
            return None;
        }
        let mut columns: std::collections::BTreeMap<(i32, i32), Vec<u32>> = Default::default();
        for (i, d) in self.decals.iter().enumerate() {
            if self.kinds.get(usize::from(d.kind)).is_none() {
                continue;
            }
            for c in Self::columns(d) {
                columns.entry(c).or_default().push(i as u32);
            }
        }
        let priority = |i: u32| self.kinds[usize::from(self.decals[i as usize].kind)].priority;
        let mut dropped = 0;
        for list in columns.values_mut() {
            list.sort_by_key(|&i| (priority(i), i));
            if list.len() > MAX_PER_CELL {
                dropped += list.len() - MAX_PER_CELL;
                list.drain(..list.len() - MAX_PER_CELL);
            }
        }
        if dropped > 0 {
            tracing::warn!(dropped, "decals beyond a column's {MAX_PER_CELL} left out");
        }
        let columns = ColumnTables::new(&columns);
        let decals: Vec<GpuDecal> = self
            .decals
            .iter()
            .map(|d| GpuDecal {
                centre: d.centre.to_array(),
                kind: u32::from(d.kind),
                rotation: d.rotation.normalize().to_array(),
                half: d.half.max(Vec3::splat(1e-3)).to_array(),
                repeat: d.repeat.max(1e-3),
                phase: d.phase,
                pad: [0; 3],
            })
            .collect();
        let kinds: Vec<GpuDecalKind> = self
            .kinds
            .iter()
            .map(|k| GpuDecalKind {
                texture: sampled(k.texture),
                // Wrap modes (u, v): a strip repeats along u; both clamp on a stamp.
                sampler: ANISOTROPIC_WRAP + 3 * u32::from(!k.strip) + 1,
                depth_fade: k.depth_fade.clamp(0.0, 1.0),
                row: k.row.map_or(u32::MAX, |r| r.0),
                tint: k.tint,
                facing: [k.facing[0], k.facing[1], 0.0, 0.0],
                alpha: match k.alpha {
                    DecalAlpha::Premultiplied => 0,
                    DecalAlpha::Straight => 1,
                    DecalAlpha::Mask => 2,
                },
                strip: u32::from(k.strip),
                normal: k.normal.map_or(crate::material::TEXTURE_NONE, &sampled),
                normal_strength: if k.normal.is_some() {
                    k.normal_strength
                } else {
                    0.0
                },
                uv_rect: k.uv_rect,
                roughness: k.roughness.map_or(crate::material::TEXTURE_NONE, &sampled),
                roughness_bias: if k.roughness.is_some() {
                    k.roughness_bias
                } else {
                    0.0
                },
                roughness_strength: k
                    .roughness_strength
                    .filter(|s| s.is_finite())
                    .unwrap_or(k.tint[3])
                    .max(0.0),
                pad: 0,
            })
            .collect();
        let mut bytes: Vec<u8> = Vec::new();
        let slots_at = place(&mut bytes, bytemuck::cast_slice(&columns.slots));
        let indices_at = place(&mut bytes, bytemuck::cast_slice(&columns.indices));
        let decals_at = place(&mut bytes, bytemuck::cast_slice(&decals));
        let kinds_at = place(&mut bytes, bytemuck::cast_slice(&kinds));
        Some(DecalTables {
            bytes,
            slots: slots_at,
            indices: indices_at,
            decals: decals_at,
            kinds: kinds_at,
            slot_mask: columns.mask,
            count: self.decals.len() as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(priority: u8) -> DecalKind {
        DecalKind {
            texture: TextureId(0),
            alpha: DecalAlpha::Premultiplied,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            tint: [1.0; 4],
            strip: false,
            priority,
            depth_fade: 0.2,
            facing: [0.2, 0.5],
            row: None,
            normal: None,
            normal_strength: 1.0,
            roughness: None,
            roughness_bias: 0.0,
            roughness_strength: None,
        }
    }

    fn decal(x: f32, z: f32, kind: u16) -> Decal {
        Decal {
            centre: Vec3::new(x, 0.0, z),
            rotation: Quat::IDENTITY,
            half: Vec3::new(1.0, 0.2, 1.0),
            kind,
            repeat: 1.0,
            phase: 0.0,
        }
    }

    /// Reads a column's list back from the tables as the shader does.
    fn column(t: &DecalTables, x: i32, z: i32) -> Vec<u32> {
        let slots: &[[u32; 4]] =
            bytemuck::cast_slice(&t.bytes[t.slots as usize..t.indices as usize]);
        let indices: &[u32] = bytemuck::cast_slice(&t.bytes[t.indices as usize..t.decals as usize]);
        let mut s = crate::columns::slot_of(x, z, t.slot_mask);
        for _ in 0..=t.slot_mask {
            let slot = slots[s as usize];
            if slot[3] == 0 {
                return Vec::new();
            }
            if slot[0] == x as u32 && slot[1] == z as u32 {
                return indices[slot[2] as usize..(slot[2] + slot[3]) as usize].to_vec();
            }
            s = (s + 1) & t.slot_mask;
        }
        Vec::new()
    }

    #[test]
    fn a_pixel_finds_the_decals_over_it_in_painters_order() {
        let set = DecalSet {
            kinds: vec![kind(2), kind(1)],
            decals: vec![decal(0.5, 1.0, 0), decal(2.0, 2.0, 1), decal(30.0, 30.0, 0)],
        };
        let t = set.tables(|id| id.0).expect("tables");
        // The first two overlap column (0, 0), the lower priority first.
        assert_eq!(column(&t, 0, 0), [1, 0]);
        assert_eq!(column(&t, 7, 7), [2]);
        assert!(column(&t, 3, 3).is_empty());
        // A box straddling a column's edge lists in both.
        assert_eq!(column(&t, -1, 0), [0]);
    }

    #[test]
    fn a_column_lists_at_most_its_share() {
        let decals = (0..MAX_PER_CELL + 4)
            .map(|i| decal(2.0, 2.0, (i % 2) as u16))
            .collect();
        let set = DecalSet {
            kinds: vec![kind(0), kind(1)],
            decals,
        };
        let t = set.tables(|id| id.0).expect("tables");
        let list = column(&t, 0, 0);
        assert_eq!(list.len(), MAX_PER_CELL);
        // The lowest priorities went first.
        assert!(list.iter().filter(|&&i| i % 2 == 0).count() < MAX_PER_CELL / 2);
    }

    #[test]
    fn a_kind_keeps_how_its_texture_reads_and_its_atlas_cell() {
        let atlas = DecalKind {
            alpha: DecalAlpha::Mask,
            uv_rect: [0.5, 0.25, 0.5, 0.25],
            ..kind(0)
        };
        let straight = DecalKind {
            alpha: DecalAlpha::Straight,
            ..kind(1)
        };
        let set = DecalSet {
            kinds: vec![kind(0), straight, atlas],
            decals: vec![decal(0.0, 0.0, 2)],
        };
        let t = set.tables(|id| id.0).expect("tables");
        let kinds: &[GpuDecalKind] = bytemuck::cast_slice(&t.bytes[t.kinds as usize..]);
        assert_eq!(kinds.iter().map(|k| k.alpha).collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(kinds[0].uv_rect, [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(kinds[2].uv_rect, [0.5, 0.25, 0.5, 0.25]);
    }

    #[test]
    fn a_kind_without_a_normal_map_bends_nothing() {
        let bumped = DecalKind {
            normal: Some(TextureId(7)),
            normal_strength: 0.5,
            ..kind(1)
        };
        let set = DecalSet {
            kinds: vec![kind(0), bumped],
            decals: vec![decal(0.0, 0.0, 1)],
        };
        let t = set.tables(|id| id.0 + 100).expect("tables");
        let kinds: &[GpuDecalKind] = bytemuck::cast_slice(&t.bytes[t.kinds as usize..]);
        assert_eq!(
            (kinds[0].normal, kinds[0].normal_strength),
            (crate::material::TEXTURE_NONE, 0.0)
        );
        assert_eq!((kinds[1].normal, kinds[1].normal_strength), (107, 0.5));
    }

    #[test]
    fn a_kind_without_a_roughness_map_lays_none() {
        let glossy = DecalKind {
            roughness: Some(TextureId(9)),
            roughness_bias: -0.7,
            ..kind(1)
        };
        let set = DecalSet {
            kinds: vec![kind(0), glossy],
            decals: vec![decal(0.0, 0.0, 1)],
        };
        let t = set.tables(|id| id.0 + 100).expect("tables");
        let kinds: &[GpuDecalKind] = bytemuck::cast_slice(&t.bytes[t.kinds as usize..]);
        assert_eq!(
            (kinds[0].roughness, kinds[0].roughness_bias),
            (crate::material::TEXTURE_NONE, 0.0)
        );
        assert_eq!((kinds[1].roughness, kinds[1].roughness_bias), (109, -0.7));
    }

    #[test]
    fn a_kind_lays_its_roughness_at_its_own_share() {
        // Its strength by default, its own when given (#395).
        let glossy = DecalKind {
            roughness: Some(TextureId(9)),
            roughness_bias: -0.7,
            ..kind(1)
        };
        let set = DecalSet {
            kinds: vec![glossy],
            decals: vec![decal(0.0, 0.0, 0)],
        };
        let t = set.tables(|id| id.0).expect("tables");
        let kinds: &[GpuDecalKind] = bytemuck::cast_slice(&t.bytes[t.kinds as usize..]);
        assert_eq!(kinds[0].roughness_strength, 1.0);
        let puddle = DecalKind {
            tint: [0.02, 0.02, 0.02, 0.3],
            roughness_strength: Some(1.0),
            ..glossy
        };
        let faint = DecalKind {
            tint: [1.0, 1.0, 1.0, 0.3],
            ..glossy
        };
        let set = DecalSet {
            kinds: vec![puddle, faint],
            decals: vec![decal(0.0, 0.0, 1)],
        };
        let t = set.tables(|id| id.0).expect("tables");
        let kinds: &[GpuDecalKind] = bytemuck::cast_slice(&t.bytes[t.kinds as usize..]);
        assert_eq!(
            (kinds[0].roughness_strength, kinds[1].roughness_strength),
            (1.0, 0.3)
        );
    }
}
