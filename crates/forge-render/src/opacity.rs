//! Opacity micromaps for the rays' cut-out triangles (#333, `VK_EXT_opacity_micromap`): each
//! not-opaque triangle of a cut ([`Cut::split`]) is divided into 4^level micro-triangles, and each
//! takes one of four states from the alpha its row's texture holds there: transparent (every ray
//! passes it, with no query loop), opaque (every ray meets it, with none), or unknown (the loop's
//! alpha test decides, as without the maps). A triangle wholly in one state takes a special index
//! instead of a map.
//!
//! The states are conservative: a micro-triangle is transparent or opaque only where every point
//! of it would fail or pass `ray_candidate_kept`'s test, which reads the texture at level 0
//! through its sampler (bilinear): over the box of texels its taps reach, every alpha on one side
//! of the cutoff, by a margin for the filter's rounding. The maps follow the cut-out rows a mesh's
//! instances read alike (`GpuMaterial::same_ray_alpha`); an instance whose rows read otherwise
//! turns them off (`tlas_instances_main`). A mesh the probes' rays see (they keep a quarter of a
//! cut-out's triangles, `probe_leaf_kept`) has no opaque cut-out micro-triangle: the clear ones are
//! still skipped. Which micro-triangle a point falls in follows the Vulkan specification's
//! `BarycentricsToSpaceFillingCurveIndex` ([`curve_index`]).

use std::collections::HashMap;
use std::sync::Arc;

use forge_gpu::{MicromapTriangle, opacity_index};

use crate::material::{AlphaPlane, GpuMaterial, RayAlpha, TextureAlpha, TextureSet};
use crate::raytrace::Cut;

/// The micro-triangles' edge the bake aims at, in texels: finer adds memory for little, as the
/// bilinear taps reach a texel past it anyway.
const TARGET_EDGE: f32 = 2.0;
/// The finest subdivision level it makes (4^9 micro-triangles, 64 KiB a map), and the device's
/// limit below that.
pub(crate) const MAX_LEVEL: u32 = 9;
/// The coarsest map: 4 micro-triangles, a byte.
const MIN_LEVEL: u32 = 1;
/// The bytes the maps may take in all: past them, the finest level comes down.
const BUDGET: usize = 32 << 20;
/// How far an alpha must stand from the cutoff for its side to be sure through the filter's
/// rounding.
const MARGIN: f32 = 1.0 / 512.0;

/// A micro-triangle's 4-state values.
const TRANSPARENT: u8 = 0;
const OPAQUE: u8 = 1;
const UNKNOWN_TRANSPARENT: u8 = 2;
const UNKNOWN_OPAQUE: u8 = 3;

/// The micromap's triangles and states, and what each cut's not-opaque triangles index there.
pub(crate) struct Baked {
    /// The micromap's triangles.
    pub triangles: Vec<MicromapTriangle>,
    /// Their states.
    pub data: Vec<u8>,
    /// Per cut: its maps; none for a cut without.
    pub cuts: Vec<Option<CutMaps>>,
    /// What the bake made, for the log.
    pub stats: Stats,
}

/// A cut's maps: what its not-opaque triangles index, the rows they follow, and whether the
/// probes' rays may use them.
pub(crate) struct CutMaps {
    /// Per not-opaque triangle, a map in the micromap or a special index.
    pub indices: Vec<i32>,
    /// The instance material whose rows the maps follow: an instance whose cut-out rows read the
    /// same (`GpuMaterial::same_ray_alpha`) keeps them.
    pub rows: u32,
    /// No cut-out row's micro-triangle is opaque (some instance of the mesh is seen by the
    /// probes' rays, which keep a quarter of a cut-out's triangles, `probe_leaf_kept`): the
    /// clear ones are still skipped.
    pub probes: bool,
}

/// What a bake made.
#[derive(Clone, Debug, Default)]
pub(crate) struct Stats {
    /// Not-opaque triangles over the cuts with maps.
    pub triangles: u64,
    /// Of them, wholly transparent, wholly opaque, wholly unknown, and with a map.
    pub transparent: u64,
    pub opaque: u64,
    pub unknown: u64,
    pub mapped: u64,
    /// Cuts with cut-out triangles left without maps: their instances' cut-out rows differ.
    pub mixed: u32,
    /// Cuts whose maps keep the probes' rule (no opaque cut-out micro-triangle).
    pub probes: u32,
    /// Cuts left without maps because every triangle of theirs is unknown.
    pub unknown_cuts: u32,
    /// Distinct maps, their micro-triangles in each state, and the finest level made.
    pub maps: u32,
    pub micro: [u64; 4],
    pub level: u32,
    /// Milliseconds the bake took.
    pub ms: f64,
}

/// What a not-opaque triangle's map comes from: its row's test and its UVs.
struct Job {
    plane: Arc<AlphaPlane>,
    /// Address modes (u, v): 0 repeat, 1 clamp, 2 mirrored repeat.
    modes: [u32; 2],
    uv: [[f32; 3]; 2],
    alpha: f32,
    cutoff: f32,
    corners: [[f32; 2]; 3],
    /// Opaque micro-triangles left unknown ([`CutMaps::probes`]).
    no_opaque: bool,
}

/// A triangle's result: a special index, or a map at a level.
enum Classified {
    Special(i32),
    Map(u32, Vec<u8>),
}

/// The special index of a triangle wholly in `state`.
fn special(state: u8) -> i32 {
    match state {
        TRANSPARENT => opacity_index::TRANSPARENT,
        OPAQUE => opacity_index::OPAQUE,
        UNKNOWN_TRANSPARENT => opacity_index::UNKNOWN_TRANSPARENT,
        _ => opacity_index::UNKNOWN_OPAQUE,
    }
}

/// The state of a cut-out row's hit where the test keeps everywhere (`true`) or nowhere; kept
/// everywhere but `no_opaque`, unknown.
fn constant(kept: bool, no_opaque: bool) -> i32 {
    special(match (kept, no_opaque) {
        (false, _) => TRANSPARENT,
        (true, false) => OPAQUE,
        (true, true) => UNKNOWN_OPAQUE,
    })
}

/// Per cut, per not-opaque triangle: a special index or a job's; the rows the cut follows and
/// whether its maps keep no opaque cut-out micro-triangle.
type Entries = (Vec<Result<i32, usize>>, u32, bool);

/// Bakes the micromaps of `cuts`' not-opaque triangles, from the rows `materials` gives the
/// instances `placed` (each a mesh, its material, and whether the probes' rays see it) and the
/// textures' alpha in `textures`, at most `max_level`.
pub(crate) fn bake(
    cuts: &[Cut],
    placed: &[(u32, u32, bool)],
    materials: &[GpuMaterial],
    textures: &TextureSet,
    max_level: u32,
) -> Baked {
    let start = std::time::Instant::now();
    let max_level = max_level.min(MAX_LEVEL);
    // Each mesh's instances' materials, and whether the probes see any of them.
    let mut bases: Vec<(Vec<u32>, bool)> = vec![(Vec::new(), false); cuts.len()];
    for &(mesh, material, probes) in placed {
        let Some((list, seen)) = bases.get_mut(mesh as usize) else {
            continue;
        };
        if !list.contains(&material) {
            list.push(material);
        }
        *seen |= probes;
    }
    let mut stats = Stats::default();
    let mut entries: Vec<Option<Entries>> = Vec::with_capacity(cuts.len());
    let mut jobs: Vec<Job> = Vec::new();
    let mut unique: HashMap<(u32, [u32; 6], bool), usize> = HashMap::new();
    for (cut, (list, probes)) in cuts.iter().zip(&bases) {
        let triangles = cut.sections.len() as u32;
        let Some(&base) = list.first() else {
            entries.push(None);
            continue;
        };
        if cut.dynamic || cut.masked == 0 || cut.opaque >= triangles {
            entries.push(None);
            continue;
        }
        // Every instance's cut-out rows read as the first's, or no maps.
        let row = |b: u32, s: u32| materials.get((b + s) as usize);
        let same = list.iter().all(|&b| {
            (0..32).filter(|s| cut.masked & (1 << s) != 0).all(|s| {
                match (row(b, s), row(base, s)) {
                    (Some(a), Some(r)) => a.same_ray_alpha(r),
                    _ => false,
                }
            })
        });
        if !same {
            stats.mixed += 1;
            entries.push(None);
            continue;
        }
        let no_opaque = *probes;
        let mut list = Vec::with_capacity((triangles - cut.opaque) as usize);
        for t in cut.opaque as usize..triangles as usize {
            let row_index = base + cut.sections[t];
            let entry = match materials
                .get(row_index as usize)
                .map(GpuMaterial::ray_alpha)
            {
                None | Some(RayAlpha::Loop) => Ok(opacity_index::UNKNOWN_OPAQUE),
                // Not cut out: the probes' rays keep it too.
                Some(RayAlpha::Kept) => Ok(opacity_index::OPAQUE),
                Some(RayAlpha::Constant(kept)) => Ok(constant(kept, no_opaque)),
                // On a mesh without UVs, the factor alone.
                Some(RayAlpha::Texture { alpha, cutoff, .. }) if cut.uvs.is_empty() => {
                    Ok(constant(alpha >= cutoff, no_opaque))
                }
                Some(RayAlpha::Texture {
                    texture,
                    sampler,
                    uv,
                    alpha,
                    cutoff,
                }) => match (textures.alpha(texture), wrap_modes(sampler)) {
                    (TextureAlpha::Unknown, _) | (_, None) => Ok(opacity_index::UNKNOWN_OPAQUE),
                    (TextureAlpha::Opaque, _) => Ok(sure(alpha, alpha, cutoff)
                        .map_or(opacity_index::UNKNOWN_OPAQUE, |s| {
                            constant(s == OPAQUE, no_opaque)
                        })),
                    (TextureAlpha::Plane(plane), Some(modes)) => {
                        let corners: [[f32; 2]; 3] =
                            std::array::from_fn(|k| cut.uvs[cut.indices[t * 3 + k] as usize]);
                        let bits = [
                            corners[0][0].to_bits(),
                            corners[0][1].to_bits(),
                            corners[1][0].to_bits(),
                            corners[1][1].to_bits(),
                            corners[2][0].to_bits(),
                            corners[2][1].to_bits(),
                        ];
                        let key = (row_index, bits, no_opaque);
                        let job = *unique.entry(key).or_insert_with(|| {
                            jobs.push(Job {
                                plane,
                                modes,
                                uv,
                                alpha,
                                cutoff,
                                corners,
                                no_opaque,
                            });
                            jobs.len() - 1
                        });
                        Err(job)
                    }
                },
            };
            list.push(entry);
        }
        entries.push(Some((list, base, no_opaque)));
    }
    // Each job's level by its longest edge in texels, the finest brought down until the maps fit
    // the budget.
    let levels: Vec<u32> = jobs.iter().map(|j| level_for(j, max_level)).collect();
    let mut cap = max_level;
    while cap > MIN_LEVEL && levels.iter().map(|&l| map_bytes(l.min(cap))).sum::<usize>() > BUDGET {
        cap -= 1;
    }
    let mut results: Vec<Option<Classified>> =
        std::iter::repeat_with(|| None).take(jobs.len()).collect();
    forge_core::pack::par_chunks_mut(&mut results, 1, 4, |i, slot| {
        slot[0] = Some(classify(&jobs[i], levels[i].min(cap)));
    });
    // The maps, the same states once.
    let mut triangles: Vec<MicromapTriangle> = Vec::new();
    let mut data: Vec<u8> = Vec::new();
    let mut maps: HashMap<(u32, Vec<u8>), i32> = HashMap::new();
    let mut job_index: Vec<i32> = Vec::with_capacity(results.len());
    for result in results {
        let index = match result.expect("every job classified") {
            Classified::Special(index) => index,
            Classified::Map(level, bytes) => {
                for (k, count) in stats.micro.iter_mut().enumerate() {
                    *count += states(&bytes, level)
                        .filter(|&s| usize::from(s) == k)
                        .count() as u64;
                }
                stats.level = stats.level.max(level);
                *maps
                    .entry((level, bytes))
                    .or_insert_with_key(|(level, bytes)| {
                        triangles.push(MicromapTriangle {
                            offset: data.len() as u32,
                            level: *level as u16,
                        });
                        data.extend_from_slice(bytes);
                        triangles.len() as i32 - 1
                    })
            }
        };
        job_index.push(index);
    }
    stats.maps = triangles.len() as u32;
    let cuts = entries
        .into_iter()
        .map(|e| {
            e.and_then(|(list, rows, probes)| {
                let indices: Vec<i32> = list
                    .into_iter()
                    .map(|e| e.unwrap_or_else(|job| job_index[job]))
                    .collect();
                // A cut whose triangles are all unknown gains nothing: no maps, its structure
                // built as without them.
                if indices.iter().all(|&i| {
                    i == opacity_index::UNKNOWN_OPAQUE || i == opacity_index::UNKNOWN_TRANSPARENT
                }) {
                    stats.unknown_cuts += 1;
                    return None;
                }
                for &i in &indices {
                    stats.triangles += 1;
                    match i {
                        opacity_index::TRANSPARENT => stats.transparent += 1,
                        opacity_index::OPAQUE => stats.opaque += 1,
                        i if i < 0 => stats.unknown += 1,
                        _ => stats.mapped += 1,
                    }
                }
                stats.probes += u32::from(probes);
                Some(CutMaps {
                    indices,
                    rows,
                    probes,
                })
            })
        })
        .collect();
    stats.ms = start.elapsed().as_secs_f64() * 1e3;
    Baked {
        triangles,
        data,
        cuts,
        stats,
    }
}

/// The address modes (u, v) of wrapping sampler `sampler` (`SAMPLER_ANISOTROPIC_WRAP` plus
/// `3 u + v`); none for another sampler.
fn wrap_modes(sampler: u32) -> Option<[u32; 2]> {
    let wrap = sampler.checked_sub(forge_gpu::SamplerKind::AnisotropicWrap as u32)?;
    (wrap < 9).then_some([wrap / 3, wrap % 3])
}

/// Bytes of a map at `level`: 2 bits for each of its 4^level micro-triangles.
fn map_bytes(level: u32) -> usize {
    (1_usize << (2 * level)) / 4
}

/// The level `job`'s triangle takes: micro-triangles about [`TARGET_EDGE`] texels on a side.
fn level_for(job: &Job, max_level: u32) -> u32 {
    let c = texel_corners(job);
    let edge = (0..3)
        .map(|k| {
            let (a, b) = (c[k], c[(k + 1) % 3]);
            ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
        })
        .fold(0.0_f32, f32::max);
    if !edge.is_finite() || edge <= TARGET_EDGE {
        return MIN_LEVEL;
    }
    ((edge / TARGET_EDGE).log2().ceil() as u32).clamp(MIN_LEVEL, max_level.max(MIN_LEVEL))
}

/// `job`'s corners in texels, as the shader reads them: its UVs through the row's transform,
/// texel centres at halves.
fn texel_corners(job: &Job) -> [[f32; 2]; 3] {
    let (w, h) = (job.plane.width as f32, job.plane.height as f32);
    job.corners.map(|[u, v]| {
        let t = |r: [f32; 3]| r[0] * u + r[1] * v + r[2];
        [t(job.uv[0]) * w - 0.5, t(job.uv[1]) * h - 0.5]
    })
}

/// The state every value from `lo` to `hi` (the alpha test's input, the factor applied) gives
/// against `cutoff`, when sure.
fn sure(lo: f32, hi: f32, cutoff: f32) -> Option<u8> {
    if lo >= cutoff + MARGIN {
        Some(OPAQUE)
    } else if hi < cutoff - MARGIN {
        Some(TRANSPARENT)
    } else {
        None
    }
}

/// `job`'s triangle's states at `level`, or its special index when it is wholly one.
fn classify(job: &Job, level: u32) -> Classified {
    let c = texel_corners(job);
    let state = |a: [f32; 2], b: [f32; 2], d: [f32; 2]| -> u8 {
        let (lo, hi) = alpha_range(job, a, b, d);
        let (lo, hi) = (
            job.alpha * f32::from(lo) / 255.0,
            job.alpha * f32::from(hi) / 255.0,
        );
        match sure(lo, hi, job.cutoff) {
            Some(OPAQUE) if job.no_opaque => UNKNOWN_OPAQUE,
            Some(s) => s,
            None if (lo + hi) * 0.5 >= job.cutoff => UNKNOWN_OPAQUE,
            None => UNKNOWN_TRANSPARENT,
        }
    };
    let whole = state(c[0], c[1], c[2]);
    if whole == TRANSPARENT || whole == OPAQUE {
        return Classified::Special(special(whole));
    }
    let n = 1_u32 << level;
    let at = |u: u32, v: u32| -> [f32; 2] {
        let (u, v) = (u as f32 / n as f32, v as f32 / n as f32);
        std::array::from_fn(|k| c[0][k] + u * (c[1][k] - c[0][k]) + v * (c[2][k] - c[0][k]))
    };
    let mut bytes = vec![0_u8; map_bytes(level)];
    let mut put = |index: u32, s: u8| bytes[(index / 4) as usize] |= s << ((index % 4) * 2);
    let third = |i: u32, k: f32| (i as f32 + k) / n as f32;
    for i in 0..n {
        for j in 0..n - i {
            // Upright: (i, j), (i + 1, j), (i, j + 1).
            let s = state(at(i, j), at(i + 1, j), at(i, j + 1));
            put(
                curve_index(third(i, 1.0 / 3.0), third(j, 1.0 / 3.0), level),
                s,
            );
            // Inverted: (i + 1, j), (i, j + 1), (i + 1, j + 1).
            if i + j + 1 < n {
                let s = state(at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
                put(
                    curve_index(third(i, 2.0 / 3.0), third(j, 2.0 / 3.0), level),
                    s,
                );
            }
        }
    }
    let first = bytes[0] & 3;
    if states(&bytes, level).all(|s| s == first) {
        return Classified::Special(special(first));
    }
    if states(&bytes, level).all(|s| s >= UNKNOWN_TRANSPARENT) {
        return Classified::Special(opacity_index::UNKNOWN_OPAQUE);
    }
    Classified::Map(level, bytes)
}

/// The states of a map at `level`, in the curve's order.
fn states(bytes: &[u8], level: u32) -> impl Iterator<Item = u8> + '_ {
    (0..1_u32 << (2 * level)).map(|i| (bytes[(i / 4) as usize] >> ((i % 4) * 2)) & 3)
}

/// The least and greatest alpha among the texels a bilinear tap anywhere in the triangle `a`,
/// `b`, `d` (texel coordinates, centres at whole numbers) reads: those of its box and one past.
fn alpha_range(job: &Job, a: [f32; 2], b: [f32; 2], d: [f32; 2]) -> (u8, u8) {
    let p = &job.plane;
    let lo = |k: usize| a[k].min(b[k]).min(d[k]).floor() as i64;
    let hi = |k: usize| a[k].max(b[k]).max(d[k]).floor() as i64 + 1;
    let (mut least, mut most) = (u8::MAX, u8::MIN);
    let width = p.width as usize;
    each_index(lo(1), hi(1), p.height, job.modes[1], |y| {
        let row = &p.texels[y * width..(y + 1) * width];
        each_index(lo(0), hi(0), p.width, job.modes[0], |x| {
            least = least.min(row[x]);
            most = most.max(row[x]);
        });
    });
    (least, most)
}

/// Calls `f` with the texel index each of `lo..=hi` reads along an axis of `n` texels under
/// address mode `mode` (0 repeat, 1 clamp, 2 mirrored repeat), each once when they cover it all.
fn each_index(lo: i64, hi: i64, n: u32, mode: u32, mut f: impl FnMut(usize)) {
    let n = i64::from(n.max(1));
    match mode {
        1 => (lo.clamp(0, n - 1)..=hi.clamp(0, n - 1)).for_each(|i| f(i as usize)),
        _ if hi - lo + 1 >= if mode == 0 { n } else { 2 * n } => {
            (0..n).for_each(|i| f(i as usize));
        }
        0 => (lo..=hi).for_each(|i| f(i.rem_euclid(n) as usize)),
        _ => (lo..=hi).for_each(|i| {
            let m = i.rem_euclid(2 * n);
            f(if m < n { m } else { 2 * n - 1 - m } as usize)
        }),
    }
}

/// The micro-triangle at barycentrics `(u, v)` of a triangle divided at `level`: its index in
/// the order the micromap stores states, a space-filling curve. The Vulkan specification's
/// reference code (`BarycentricsToSpaceFillingCurveIndex`, appendix `VK_EXT_opacity_micromap`,
/// © The Khronos Group Inc., CC-BY-4.0), ported to Rust.
pub(crate) fn curve_index(u: f32, v: f32, level: u32) -> u32 {
    let u = u.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let n = 1_u32 << level;
    let fu = u * n as f32;
    let fv = v * n as f32;
    let mut iu = fu as u32;
    let mut iv = fv as u32;
    let uf = fu - iu as f32;
    let vf = fv - iv as f32;
    if iu >= n {
        iu = n - 1;
    }
    if iv >= n {
        iv = n - 1;
    }
    let iuv = iu + iv;
    if iuv >= n {
        iu -= iuv - n + 1;
    }
    let mut iw = !(iu + iv);
    if uf + vf >= 1.0 && iuv < n - 1 {
        iw = iw.wrapping_sub(1);
    }
    let mut b0 = !(iu ^ iw);
    b0 &= n - 1;
    let t = (iu ^ iv) & b0;
    let mut f = t;
    f ^= f >> 1;
    f ^= f >> 2;
    f ^= f >> 4;
    f ^= f >> 8;
    let mut b1 = ((f ^ iu) & !b0) | t;
    // Interleave the bits.
    b0 = (b0 | (b0 << 8)) & 0x00ff_00ff;
    b0 = (b0 | (b0 << 4)) & 0x0f0f_0f0f;
    b0 = (b0 | (b0 << 2)) & 0x3333_3333;
    b0 = (b0 | (b0 << 1)) & 0x5555_5555;
    b1 = (b1 | (b1 << 8)) & 0x00ff_00ff;
    b1 = (b1 | (b1 << 4)) & 0x0f0f_0f0f;
    b1 = (b1 | (b1 << 2)) & 0x3333_3333;
    b1 = (b1 | (b1 << 1)) & 0x5555_5555;
    b0 | (b1 << 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The micro-triangles of a level, as the bake walks them: each upright and inverted one's
    /// corners on the grid, and its index on the curve.
    fn grid(level: u32) -> Vec<([[u32; 2]; 3], u32)> {
        let n = 1_u32 << level;
        let third = |i: u32, k: f32| (i as f32 + k) / n as f32;
        let mut out = Vec::new();
        for i in 0..n {
            for j in 0..n - i {
                out.push((
                    [[i, j], [i + 1, j], [i, j + 1]],
                    curve_index(third(i, 1.0 / 3.0), third(j, 1.0 / 3.0), level),
                ));
                if i + j + 1 < n {
                    out.push((
                        [[i + 1, j], [i, j + 1], [i + 1, j + 1]],
                        curve_index(third(i, 2.0 / 3.0), third(j, 2.0 / 3.0), level),
                    ));
                }
            }
        }
        out
    }

    /// Every micro-triangle has its own index, the indices fill 0..4^level, and the curve goes
    /// from each to the next through a shared corner. At level 1, the specification's order: the
    /// sub-triangle nearest vertex 0, the middle one, nearest vertex 1 (u = 1), nearest vertex 2.
    #[test]
    fn the_curve_numbers_every_micro_triangle_once_in_the_specifications_order() {
        for level in 0..=6 {
            let mut cells = grid(level);
            cells.sort_by_key(|c| c.1);
            let count = 1_usize << (2 * level);
            assert_eq!(cells.len(), count);
            for (k, (_, index)) in cells.iter().enumerate() {
                assert_eq!(*index as usize, k, "level {level}");
            }
            for pair in cells.windows(2) {
                let shared = pair[0].0.iter().filter(|p| pair[1].0.contains(p)).count();
                assert!(
                    shared >= 1,
                    "level {level}: {:?} then {:?}",
                    pair[0],
                    pair[1]
                );
            }
        }
        let mut one = grid(1);
        one.sort_by_key(|c| c.1);
        let corners: Vec<[[u32; 2]; 3]> = one.into_iter().map(|c| c.0).collect();
        assert_eq!(
            corners,
            vec![
                [[0, 0], [1, 0], [0, 1]],
                [[1, 0], [0, 1], [1, 1]],
                [[1, 0], [2, 0], [1, 1]],
                [[0, 1], [1, 1], [0, 2]],
            ]
        );
    }

    fn plane(width: u32, height: u32, alpha: impl Fn(u32, u32) -> u8) -> Arc<AlphaPlane> {
        Arc::new(AlphaPlane {
            width,
            height,
            texels: (0..height)
                .flat_map(|y| (0..width).map(move |x| (x, y)))
                .map(|(x, y)| alpha(x, y))
                .collect(),
        })
    }

    fn job(plane: Arc<AlphaPlane>, corners: [[f32; 2]; 3]) -> Job {
        Job {
            plane,
            modes: [1, 1],
            uv: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            alpha: 1.0,
            cutoff: 0.5,
            corners,
            no_opaque: false,
        }
    }

    /// A texture solid on its left half and clear on its right: a triangle over its left quarter
    /// is opaque, one over its right quarter transparent, and one across the middle has a map
    /// whose micro-triangles take the side each lies on, the ones the middle crosses unknown.
    #[test]
    fn micro_triangles_take_the_side_of_the_cutoff_their_texels_lie_on() {
        let half = plane(64, 64, |x, _| if x < 32 { 255 } else { 0 });
        let left = job(half.clone(), [[0.0, 0.0], [0.25, 0.0], [0.0, 1.0]]);
        assert!(matches!(
            classify(&left, 4),
            Classified::Special(opacity_index::OPAQUE)
        ));
        let right = job(half.clone(), [[0.75, 0.0], [1.0, 0.0], [1.0, 1.0]]);
        assert!(matches!(
            classify(&right, 4),
            Classified::Special(opacity_index::TRANSPARENT)
        ));
        let across = job(half, [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        let Classified::Map(level, bytes) = classify(&across, 4) else {
            panic!("a map");
        };
        // Each micro-triangle's state against where its texels lie: u is x.
        for (corners, index) in grid(level) {
            let s = (bytes[(index / 4) as usize] >> ((index % 4) * 2)) & 3;
            let n = (1_u32 << level) as f32;
            let xs = corners.map(|[u, _]| u as f32 / n * 64.0 - 0.5);
            let (lo, hi) = (
                xs.iter().copied().fold(f32::MAX, f32::min).floor(),
                xs.iter().copied().fold(f32::MIN, f32::max).floor() + 1.0,
            );
            let want = if hi < 32.0 {
                OPAQUE
            } else if lo >= 32.0 {
                TRANSPARENT
            } else {
                s.max(UNKNOWN_TRANSPARENT)
            };
            assert_eq!(s, want, "{corners:?}");
        }
    }

    /// For a mesh the probes' rays see, no cut-out micro-triangle is opaque: the solid side is
    /// unknown, the clear side still transparent.
    #[test]
    fn a_mesh_the_probes_see_keeps_no_opaque_micro_triangle() {
        let half = plane(64, 64, |x, _| if x < 32 { 255 } else { 0 });
        let mut left = job(half.clone(), [[0.0, 0.0], [0.25, 0.0], [0.0, 1.0]]);
        left.no_opaque = true;
        assert!(matches!(
            classify(&left, 4),
            Classified::Special(opacity_index::UNKNOWN_OPAQUE)
        ));
        let mut across = job(half, [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]]);
        across.no_opaque = true;
        let Classified::Map(level, bytes) = classify(&across, 4) else {
            panic!("a map");
        };
        let counts = states(&bytes, level).fold([0; 4], |mut c, s| {
            c[usize::from(s)] += 1;
            c
        });
        assert_eq!(counts[usize::from(OPAQUE)], 0);
        assert!(counts[usize::from(TRANSPARENT)] > 0);
    }

    /// A tap reads the texel past the box too: a triangle whose texels are all solid but whose
    /// edge stands within a texel of a clear one is not sure.
    #[test]
    fn the_bilinear_taps_reach_a_texel_past_the_box() {
        let half = plane(64, 64, |x, _| if x < 32 { 255 } else { 0 });
        // Texel centres up to 31.4: the taps at 31.4 blend texels 31 and 32.
        let edge = 31.9 / 64.0;
        let near = job(half, [[0.0, 0.0], [edge, 0.0], [0.0, 1.0]]);
        let Classified::Map(..) = classify(&near, 3) else {
            panic!("its edge micro-triangles unknown");
        };
    }

    #[test]
    fn address_modes_read_the_texels_the_sampler_reads() {
        let read = |lo, hi, mode| {
            let mut v = Vec::new();
            each_index(lo, hi, 4, mode, |i| v.push(i));
            v
        };
        assert_eq!(read(-2, 0, 0), vec![2, 3, 0]);
        assert_eq!(read(-2, 1, 0), vec![0, 1, 2, 3]);
        assert_eq!(read(-2, 1, 1), vec![0, 1]);
        assert_eq!(read(-2, 1, 2), vec![1, 0, 0, 1]);
        assert_eq!(read(3, 5, 2), vec![3, 3, 2]);
        assert_eq!(read(-100, 100, 0), vec![0, 1, 2, 3]);
    }
}
