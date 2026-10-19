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
//! clamped at its edges (a stamp) or repeating along `u` (a strip: the foot of a wall).

use crate::columns::{ColumnTables, place};
use bytemuck::{Pod, Zeroable};
use forge_core::material::TextureId;
use glam::{Quat, Vec3};

/// A grid column's side in the scene's frame, metres (`x` and `z`; a column holds every height).
pub const CELL: f32 = 4.0;
/// The most decals a column lists (a pixel walks at most these); more are left out, the lowest
/// priorities first, with a warning.
pub const MAX_PER_CELL: usize = 16;

/// A kind of decal: how its texture marks a surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DecalKind {
    /// Its texture in the scene's [`crate::TextureSet`] (`TextureSet::add`): RGBA, its colour
    /// premultiplied by its alpha in every mip.
    pub texture: TextureId,
    /// What its colour is multiplied by (rgb), and its strength (a).
    pub tint: [f32; 4],
    /// Its texture repeats along the box's `x` (a strip: [`Decal::repeat`] times, each decal from
    /// a phase of its own so that neighbours do not repeat it), else it is a stamp clamped at its
    /// edges.
    pub strip: bool,
    /// Its place among the decals a pixel walks: higher paints over lower.
    pub priority: u8,
    /// The share of the box's depth (`y`) over which it fades out towards both faces, 0 to 1.
    pub depth_fade: f32,
    /// The cosines between a surface's normal and the box's `y` under which the decal is gone and
    /// over which it is whole: it marks the faces it looks at, not those at right angles (the
    /// ground under a wall's grime).
    pub facing: [f32; 2],
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
}

/// Mirrors `DecalKind` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuDecalKind {
    texture: u32,
    /// The sampler: anisotropic, clamped (a stamp) or repeating along `u` (a strip).
    sampler: u32,
    depth_fade: f32,
    /// 1 for a strip: its texture repeats along `u`, each decal from a phase of its own.
    strip: u32,
    tint: [f32; 4],
    facing: [f32; 4],
}

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
                strip: u32::from(k.strip),
                tint: k.tint,
                facing: [k.facing[0], k.facing[1], 0.0, 0.0],
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
            tint: [1.0; 4],
            strip: false,
            priority,
            depth_fade: 0.2,
            facing: [0.2, 0.5],
        }
    }

    fn decal(x: f32, z: f32, kind: u16) -> Decal {
        Decal {
            centre: Vec3::new(x, 0.0, z),
            rotation: Quat::IDENTITY,
            half: Vec3::new(1.0, 0.2, 1.0),
            kind,
            repeat: 1.0,
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
}
