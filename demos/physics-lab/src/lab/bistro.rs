//! `physics-lab --lab bistro` (D-083 step 1): the Amazon Lumberyard Bistro's exterior, an
//! authored scene (ORCA, CC BY 4.0) to measure Forge's image, load and frame against: by
//! day under its authors' HDR sky and the sun taken out of it (D-084), by night lit by its
//! emissive surfaces alone (`--time-of-day 1.5`).
//!
//! The archive is fetched by hand into `assets/external/bistro/` (git-ignored, never shipped)
//! and converted once by `tools/fbx-to-gltf.py` into `converted/`: the meshes and nodes as glTF
//! without images, each material's DDS maps by role in `BistroExterior.textures.json`, and the
//! FBX's camera path in `BistroExterior.camera.json`. The maps are read here as their blocks
//! (`forge_render::dds`): base colour (alpha = opacity), "specular" (roughness in green,
//! metalness in blue; its red, said to be occlusion, is 0 throughout and left out) and the
//! DirectX normal maps, their green turned over.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::{Context, Result};
use forge_geom::city::{Imported, PropKind, PropSpec};
use forge_geom::model::{Model, load_gltf};
use forge_render::dds::{
    BlockFormat, alpha_plane, color_plane, flip_green, read_dds, uniform_color,
};
use forge_render::lamps::{EVERY_REGION, Emission, Lamp, WHITE_BALANCE};
use forge_render::material::{ExternalMaps, external_layer, upload_compressed};
use glam::{Mat4, Quat, Vec3};

use city_blocks::CityMaterials;
use city_blocks::scenario::Environment;

/// The luminance of an emissive map's full colour, cd/m² (a lit shop sign or lamp shade): the
/// night is lit by these alone. As the authors' renders have it: their renderer takes the
/// emissive maps at 1 and the sky's photograph at 10 (`BistroExterior.pyscene`), so a full
/// emissive texel shines as a photograph texel of 0.1; Forge scales the photograph so that its
/// sun (4.458 of its units) gives the sun's 128,000 lux (D-084): 0.1 × 128,000 / 4.458.
const EMISSIVE_LUMINANCE: f32 = 2870.0;
/// Cut out under this base colour alpha where the map has any (the archive's alpha is opacity,
/// tested at a half as its own renderer does).
const ALPHA_CUTOFF: f32 = 0.5;

/// Materials made of single sheets, seen from both sides (the terrace sees the awnings from
/// below).
const SHEETS: [&str; 3] = ["Awning", "Banner", "Curtains"];

/// The share of a material's edges that only one of its triangles uses from which it is drawn
/// from both sides: an open sheet (a chain's links, a pane, a sign's letters, petals) would
/// vanish from behind, and the sun's rays would pass its back. A closed surface has none; the
/// archive's walls and kerbs, open where they meet other pieces, 10 to 26 %.
const OPEN_SHEET: f64 = 0.3;

/// Per material, the share of its edges that only one of its triangles uses, over every mesh,
/// their corners welded within a millimetre.
fn open_edges(model: &Model) -> HashMap<String, f64> {
    // An edge by its two corners in millimetres, the lesser first.
    type Edge = ([i32; 3], [i32; 3]);
    let mut counts: HashMap<String, [usize; 2]> = HashMap::new();
    for m in &model.meshes {
        let corner = |i: u32| {
            (Vec3::from(m.mesh.positions[i as usize]) * 1000.0)
                .round()
                .as_ivec3()
                .to_array()
        };
        let mut uses: Vec<HashMap<Edge, u32>> = vec![HashMap::new(); m.materials.len()];
        for (t, tri) in m.mesh.indices.as_chunks::<3>().0.iter().enumerate() {
            let Some(edges) = uses.get_mut(usize::from(m.mesh.section(t))) else {
                continue;
            };
            for e in 0..3 {
                let (a, c) = (corner(tri[e]), corner(tri[(e + 1) % 3]));
                *edges.entry((a.min(c), a.max(c))).or_default() += 1;
            }
        }
        for (material, edges) in m.materials.iter().zip(uses) {
            let count = counts.entry(material.name.clone()).or_default();
            count[0] += edges.len();
            count[1] += edges.values().filter(|&&n| n == 1).count();
        }
    }
    counts
        .into_iter()
        .map(|(name, [edges, open])| (name, open as f64 / edges.max(1) as f64))
        .collect()
}

/// An authored scene of the lab (D-083): the Bistro's exterior, or the material test chart
/// (`tools/material-chart.py`): spheres and tiles of known materials under generated skies, to
/// compare with Falcor's render of the same file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Authored {
    /// The Amazon Lumberyard Bistro's exterior.
    Bistro,
    /// The material test chart.
    Chart,
}

impl Authored {
    /// The converted scene's folder, and the one its maps' paths are relative to.
    fn folders(self) -> (PathBuf, PathBuf) {
        let external =
            forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR")).join("assets/external");
        match self {
            Authored::Bistro => (
                external.join("bistro/converted"),
                external.join("bistro/Bistro_v5_2"),
            ),
            Authored::Chart => (external.join("chart"), external.join("chart")),
        }
    }

    /// Its glTF file's stem.
    fn stem(self) -> &'static str {
        match self {
            Authored::Bistro => "BistroExterior",
            Authored::Chart => "chart",
        }
    }
}

/// A material's maps by role, as `BistroExterior.textures.json` names them.
#[derive(Clone, Debug, Default, serde::Deserialize)]
struct Roles {
    base_color: Option<String>,
    specular: Option<String>,
    normal: Option<String>,
    emissive: Option<String>,
}

/// The FBX's camera path: one `[x, y, z, qx, qy, qz, qw]` a frame, in glTF's frame (+Y up) with
/// the camera's own axes left as Blender's (it looks down its local −Z, its up +Y).
#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CameraPath {
    pub frames: Vec<[f32; 7]>,
}

impl CameraPath {
    /// Frame `k`'s position, and its yaw and pitch as the fly camera takes them (yaw 0 looks
    /// down −Z, turning to +X as it grows less; pitch up).
    pub(crate) fn pose(&self, k: usize) -> Option<(Vec3, f32, f32)> {
        let f = self.frames.get(k)?;
        let q = Quat::from_xyzw(f[3], f[4], f[5], f[6]).normalize();
        // Blender's camera axes taken through the axis change into glTF's: its −Z is −Y there.
        let forward = q * Vec3::NEG_Y;
        let yaw = (-forward.x).atan2(-forward.z);
        let pitch = forward.y.clamp(-1.0, 1.0).asin();
        Some((Vec3::new(f[0], f[1], f[2]), yaw, pitch))
    }
}

/// The exterior: its model, its meshes' cache key and prop names, its maps by material and its
/// camera path.
pub(super) struct Bistro {
    pub model: Model,
    key: String,
    pub props: Vec<&'static str>,
    roles: HashMap<String, Roles>,
    pub camera: Option<CameraPath>,
}

/// The scene, read once; `None` (logged) where its converted files are missing.
pub(super) fn scene(which: Authored) -> Option<&'static Bistro> {
    static BISTRO: OnceLock<Option<Bistro>> = OnceLock::new();
    static CHART: OnceLock<Option<Bistro>> = OnceLock::new();
    let cell = match which {
        Authored::Bistro => &BISTRO,
        Authored::Chart => &CHART,
    };
    cell.get_or_init(|| match read(which) {
        Ok(b) => Some(b),
        Err(e) => {
            tracing::warn!(
                "no {which:?}: {e:#} (the Bistro: fetch the archive into assets/external/bistro/ \
                 and run tools/fbx-to-gltf.py; the chart: run tools/material-chart.py; D-083)"
            );
            None
        }
    })
    .as_ref()
}

fn read(which: Authored) -> Result<Bistro> {
    let (converted, _) = which.folders();
    let stem = which.stem();
    let gltf = converted.join(format!("{stem}.gltf"));
    let model = load_gltf(&gltf).with_context(|| format!("{}", gltf.display()))?;
    // The cache key: the files' sizes and times, not 130 MB of bytes hashed at every start.
    let stamp = |name: &str| -> String {
        std::fs::metadata(converted.join(name)).map_or_else(
            |_| "?".to_owned(),
            |m| {
                let time = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                format!("{}-{time}", m.len())
            },
        )
    };
    let key = format!(
        "{stem}#{}#{}",
        stamp(&format!("{stem}.gltf")),
        stamp(&format!("{stem}.bin"))
    );
    // The Bistro's maps by material; the chart has factors alone.
    let roles: HashMap<String, Roles> = match which {
        Authored::Bistro => serde_json::from_slice(
            &std::fs::read(converted.join("BistroExterior.textures.json"))
                .context("BistroExterior.textures.json")?,
        )
        .context("BistroExterior.textures.json")?,
        Authored::Chart => HashMap::new(),
    };
    let camera = std::fs::read(converted.join(format!("{stem}.camera.json")))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CameraPath>(&bytes).ok());
    let props = (0..model.meshes.len())
        .map(|k| &*Box::leak(format!("lab-{stem}-{k}").into_boxed_str()))
        .collect();
    tracing::info!(
        meshes = model.meshes.len(),
        triangles = model
            .meshes
            .iter()
            .map(|m| m.mesh.triangle_count())
            .sum::<usize>(),
        materials = roles.len(),
        camera_frames = camera.as_ref().map_or(0, |c| c.frames.len()),
        "an authored scene"
    );
    Ok(Bistro {
        model,
        key,
        props,
        roles,
        camera,
    })
}

/// The scene's props: every mesh of the exterior, in order.
pub(super) fn props(which: Authored) -> Vec<PropSpec> {
    let Some(b) = scene(which) else {
        return Vec::new();
    };
    b.model
        .meshes
        .iter()
        .enumerate()
        .map(|(k, mesh)| PropSpec {
            name: b.props[k].to_owned(),
            kind: PropKind::Imported(Imported {
                key: format!("{} {k}", b.key),
                mesh: Arc::new(mesh.mesh.clone()),
                normal_weight: None,
            }),
        })
        .collect()
}

/// What the scene draws, its props from `first`: every mesh where the file places it.
pub(super) fn build(which: Authored, first: usize) -> Vec<(usize, Mat4)> {
    scene(which).map_or_else(Vec::new, |b| {
        (0..b.model.meshes.len())
            .map(|k| (first + k, Mat4::IDENTITY))
            .collect()
    })
}

/// The scene's sky and sun as its authors' renders were lit (D-084): the archive's HDR
/// photograph alone, its sun taken out of it to be the scene's (`BistroExterior.pyscene` lights
/// by it; the FBX's directional light is for rasterisers, its readme says, and NVIDIA's
/// path-traced renders leave it out). The chart's: the sky `FORGE_CHART_SKY` names among
/// `tools/material-chart.py`'s (`sky-side-30.hdr` without it), as its Falcor scene reads it.
/// `None` without the file.
pub(crate) fn environment(which: Authored) -> Option<Environment> {
    let (_, archive) = which.folders();
    let path = match which {
        Authored::Bistro => archive.join("san_giuseppe_bridge_4k.hdr"),
        Authored::Chart => archive.join(
            std::env::var("FORGE_CHART_SKY").unwrap_or_else(|_| "sky-side-30.hdr".to_owned()),
        ),
    };
    if !path.exists() {
        tracing::warn!(path = %path.display(), "no sky photograph: lit by Forge's");
        return None;
    }
    Some(Environment {
        path: Box::leak(path.to_string_lossy().into_owned().into_boxed_str()),
        scale: 1.0,
    })
}

/// Where the camera starts: the path's first frame (the street before the café), else above
/// the scene's middle.
pub(crate) fn camera(which: Authored) -> (Vec3, f32, f32) {
    scene(which)
        .and_then(|b| b.camera.as_ref())
        .and_then(|c| c.pose(0))
        .unwrap_or((Vec3::new(0.0, 3.0, 0.0), 0.0, 0.0))
}

/// The rows of every mesh's materials, their maps read from the archive and uploaded once each:
/// the base colour and emission sRGB, the normal maps' green turned over, a material cut out
/// where its base colour's map has any alpha.
pub(super) fn rows(
    which: Authored,
    device: &Arc<forge_gpu::Device>,
    materials: &mut CityMaterials,
) -> Result<()> {
    let Some(b) = scene(which) else {
        return Ok(());
    };
    let started = std::time::Instant::now();
    let (_, archive) = which.folders();
    // Each map once, by its path and whether it is colour.
    let mut wanted: Vec<(String, bool, bool)> = Vec::new();
    for r in b.roles.values() {
        for (path, srgb, normal) in [
            (&r.base_color, true, false),
            (&r.emissive, true, false),
            (&r.specular, false, false),
            (&r.normal, false, true),
        ] {
            if let Some(p) = path
                && !wanted.iter().any(|w| w.0 == *p)
            {
                wanted.push((p.clone(), srgb, normal));
            }
        }
    }
    let mut read = Vec::with_capacity(wanted.len());
    let mut missing = 0;
    for (path, srgb, normal) in &wanted {
        let bytes = match std::fs::read(archive.join(path)) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!(map = path, "a Bistro map does not read: {e}");
                missing += 1;
                continue;
            }
        };
        match read_dds(path, &bytes, *srgb) {
            Ok(mut t) => {
                if *normal {
                    if t.format == BlockFormat::Bc5 {
                        flip_green(&mut t);
                    } else {
                        tracing::warn!(map = path, format = ?t.format, "a normal map not in BC5 keeps its DirectX green");
                    }
                }
                read.push(t);
            }
            Err(e) => {
                tracing::warn!(map = path, "a Bistro map does not decode: {e}");
                missing += 1;
            }
        }
    }
    let read_time = started.elapsed();
    // The "Specular" maps of one value (most are 16 × 16 of one texel): their material's
    // factors instead. A row's own roughness decides its mirror rays (D-084: the glass, at 0,
    // reflected nothing of the street under a map's factor of 1).
    let flat: HashMap<String, [u8; 3]> = read
        .iter()
        .filter(|t| !t.srgb)
        .filter_map(|t| uniform_color(t).map(|c| (t.name.clone(), c)))
        .collect();
    let uploaded = upload_compressed(device, &read)?;
    let mut ids = HashMap::new();
    let mut cut_out = HashMap::new();
    let mut bytes = 0_u64;
    for ((image, size), texture) in uploaded.into_iter().zip(&read) {
        bytes += size;
        let id = materials.textures.add_compressed(image, size, texture)?;
        let alpha = alpha_plane(texture).is_some_and(|a| a.iter().any(|&a| a < 255));
        cut_out.insert(texture.name.clone(), alpha);
        ids.insert(texture.name.clone(), id);
    }
    let map = |p: &Option<String>| p.as_ref().and_then(|p| ids.get(p).copied());
    let measured = std::time::Instant::now();
    let open = open_edges(&b.model);
    let open_time = measured.elapsed();
    let mut rows_made = 0;
    for (k, mesh) in b.model.meshes.iter().enumerate() {
        let uvs = !mesh.mesh.uvs.is_empty();
        let rows = mesh
            .materials
            .iter()
            .map(|m| {
                let r = b.roles.get(&m.name).cloned().unwrap_or_default();
                let factors = r.specular.as_ref().and_then(|p| flat.get(p)).copied();
                let maps = if uvs {
                    ExternalMaps {
                        base_color: map(&r.base_color),
                        metal_rough: map(&r.specular).filter(|_| factors.is_none()),
                        // Its red, "occlusion" by the archive's readme, is 0 throughout: its own
                        // renderer reads none, and taken here it blacked out every shadow.
                        occlusion: None,
                        normal: map(&r.normal),
                        emissive: map(&r.emissive),
                        emissive_luminance: EMISSIVE_LUMINANCE,
                        alpha_cutoff: r
                            .base_color
                            .as_ref()
                            .filter(|p| cut_out.get(*p).copied().unwrap_or(false))
                            .map(|_| ALPHA_CUTOFF),
                    }
                } else {
                    ExternalMaps::default()
                };
                rows_made += 1;
                // Both sides drawn where its authors named it so (".DoubleSided"), where it is cut
                // out, on the single sheets seen from below (the awnings, the banner, the
                // curtains), and where its edges are largely open (`OPEN_SHEET`). The exporter
                // wrote every material double-sided (Blender's import default), which skipped the
                // cone cull and the rays' back faces everywhere.
                let mut m = m.clone();
                if let Some([_, roughness, metal]) = factors {
                    m.roughness = f32::from(roughness) / 255.0;
                    m.metallic = f32::from(metal) / 255.0;
                }
                m.double_sided = m.name.ends_with(".DoubleSided")
                    || maps.alpha_cutoff.is_some()
                    || SHEETS.iter().any(|s| m.name.contains(s))
                    || open.get(&m.name).is_some_and(|&o| o >= OPEN_SHEET);
                let mut layer = external_layer(&m, maps);
                // D-087's prototype: a glowing piece lights as a lamp, and the probes' rays take
                // none of its glow.
                layer.lamp = maps.emissive.is_some() && lamps_on();
                (m.name.clone(), layer)
            })
            .collect();
        materials.add_rows(b.props[k], rows);
    }
    tracing::info!(
        maps = read.len(),
        missing,
        megabytes = bytes / (1 << 20),
        rows = rows_made,
        read_ms = read_time.as_millis() as u64,
        open_edges_ms = open_time.as_millis() as u64,
        sheets = open.values().filter(|&&o| o >= OPEN_SHEET).count(),
        total_ms = started.elapsed().as_millis() as u64,
        "the Bistro's maps"
    );
    Ok(())
}

/// Whether the Bistro's glowing pieces light the scene as lamps (D-087's prototype,
/// `FORGE_BISTRO_LAMPS=1`): each a Forge lamp of its own light ([`lamps`]), its glow left out of
/// the probes' rays (`RenderLayer::lamp`). Without it the night is lit by the probes' rays
/// meeting the glow, one ray in hundreds: dim, and flickering (D-087).
pub(super) fn lamps_on() -> bool {
    std::env::var("FORGE_BISTRO_LAMPS").is_ok_and(|v| v == "1")
}

/// Under this luminance, cd/m², a glowing piece is too dim to light anything worth a lamp.
const LEAST_GLOW: f32 = 1.0;
/// A closed glowing piece seen from above under this share of its area seen from round the
/// horizon, and from below under `LANTERN_DOWN`'s, is a lantern ([`Emission::LANTERN`]): the
/// Bistro's street lanterns, 0 and 0.32; its bulbs and globes 0.84 to 1.1.
const LANTERN_UP: f32 = 0.25;
/// See [`LANTERN_UP`].
const LANTERN_DOWN: f32 = 0.6;
/// Under this share of its area its normals' sum keeps, a glowing piece shines every way (a bulb,
/// a lantern's panes); over it, it is flat and shines forward (a sign, a spotlight's glass).
const FLAT_PIECE: f32 = 0.5;

/// One connected piece of an emissive material, its corners welded within a millimetre: its area,
/// its middle, its normals' sum (each triangle's by its area, so a closed piece's sums to none),
/// its luminance per channel, cd/m², its map's mean over its area, its bounds, and the share of
/// its area that faces away from its middle less the share that faces it (1 out, -1 in).
#[derive(Clone, Copy, Debug, Default)]
struct Glow {
    area: f32,
    middle: Vec3,
    normal: Vec3,
    luminance: Vec3,
    low: Vec3,
    high: Vec3,
    outward: f32,
    beams: [f32; BEAMS],
}

/// The directions a glowing piece's light is measured in (`Glow::beams`): up, down, then eight
/// round the horizon.
const BEAMS: usize = 10;

/// The unit direction of `Glow::beams`' entry `k`.
fn beam_direction(k: usize) -> Vec3 {
    match k {
        0 => Vec3::Y,
        1 => Vec3::NEG_Y,
        _ => {
            let a = (k - 2) as f32 * std::f32::consts::FRAC_PI_4;
            Vec3::new(a.cos(), 0.0, a.sin())
        }
    }
}

/// The scene's glowing pieces (`Glow`), every emissive material's on every mesh: its map read at
/// points over each triangle, about one a texel.
fn glows(which: Authored) -> Vec<(String, Glow)> {
    let Some(b) = scene(which) else {
        return Vec::new();
    };
    let (_, archive) = which.folders();
    let srgb: [f32; 256] = std::array::from_fn(|v| {
        let c = v as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    });
    // Each emissive map's texels, linear, once.
    let mut planes: HashMap<String, Option<(usize, usize, Vec<Vec3>)>> = HashMap::new();
    let mut out = Vec::new();
    for m in &b.model.meshes {
        if m.mesh.uvs.is_empty() {
            continue;
        }
        for (section, material) in m.materials.iter().enumerate() {
            let Some(path) = b
                .roles
                .get(&material.name)
                .and_then(|r| r.emissive.as_ref())
            else {
                continue;
            };
            let plane = planes.entry(path.clone()).or_insert_with(|| {
                let t = std::fs::read(archive.join(path))
                    .ok()
                    .and_then(|bytes| read_dds(path, &bytes, true).ok())?;
                let texels = color_plane(&t)?
                    .into_iter()
                    .map(|c| Vec3::from(c.map(|v| srgb[usize::from(v)])))
                    .collect();
                Some((t.width as usize, t.height as usize, texels))
            });
            let Some((w, h, texels)) = plane else {
                continue;
            };
            let tris: Vec<[u32; 3]> = m
                .mesh
                .indices
                .as_chunks::<3>()
                .0
                .iter()
                .enumerate()
                .filter(|&(t, _)| usize::from(m.mesh.section(t)) == section)
                .map(|(_, tri)| *tri)
                .collect();
            // The pieces: triangles that share a corner, by a union-find over the welded corners.
            let mut parent: Vec<usize> = (0..tris.len()).collect();
            fn root(parent: &mut [usize], mut i: usize) -> usize {
                while parent[i] != i {
                    parent[i] = parent[parent[i]];
                    i = parent[i];
                }
                i
            }
            let mut first: HashMap<[i32; 3], usize> = HashMap::new();
            for (t, tri) in tris.iter().enumerate() {
                for &v in tri {
                    let key = (Vec3::from(m.mesh.positions[v as usize]) * 1000.0)
                        .round()
                        .as_ivec3()
                        .to_array();
                    let other = *first.entry(key).or_insert(t);
                    let (a, c) = (root(&mut parent, t), root(&mut parent, other));
                    parent[a] = c;
                }
            }
            // By their roots, in order: the lamps' order the same at every run.
            let mut pieces: std::collections::BTreeMap<usize, Glow> = Default::default();
            for (t, tri) in tris.iter().enumerate() {
                let p = tri.map(|v| Vec3::from(m.mesh.positions[v as usize]));
                let uv = tri.map(|v| glam::Vec2::from(m.mesh.uvs[v as usize]));
                let cross = (p[1] - p[0]).cross(p[2] - p[0]);
                let area = 0.5 * cross.length();
                if area <= 0.0 {
                    continue;
                }
                // Points at the middles of the triangle cut into n² (the upright ones): about
                // one a texel of its footprint, 1 to 8,000.
                let footprint =
                    0.5 * (uv[1] - uv[0]).perp_dot(uv[2] - uv[0]).abs() * (*w as f32) * (*h as f32);
                let n = (2.0 * footprint).sqrt().ceil().clamp(1.0, 128.0) as usize;
                let mut sum = Vec3::ZERO;
                let mut count = 0;
                for i in 0..n {
                    for j in 0..n - i {
                        let (b1, b2) = (
                            (i as f32 + 1.0 / 3.0) / n as f32,
                            (j as f32 + 1.0 / 3.0) / n as f32,
                        );
                        let at = uv[0] + (uv[1] - uv[0]) * b1 + (uv[2] - uv[0]) * b2;
                        let x = ((at.x.rem_euclid(1.0) * *w as f32) as usize).min(*w - 1);
                        let y = ((at.y.rem_euclid(1.0) * *h as f32) as usize).min(*h - 1);
                        sum += texels[y * *w + x];
                        count += 1;
                    }
                }
                let g = pieces.entry(root(&mut parent, t)).or_insert(Glow {
                    low: Vec3::INFINITY,
                    high: Vec3::NEG_INFINITY,
                    ..Glow::default()
                });
                g.low = g.low.min(p[0].min(p[1]).min(p[2]));
                g.high = g.high.max(p[0].max(p[1]).max(p[2]));
                g.area += area;
                for (k, b) in g.beams.iter_mut().enumerate() {
                    *b += 0.5 * cross.dot(beam_direction(k)).max(0.0);
                }
                g.middle += (p[0] + p[1] + p[2]) / 3.0 * area;
                g.normal += 0.5 * cross;
                g.luminance += sum / count as f32 * area;
            }
            // How much of each piece faces out from its middle (1) or in (-1): a lantern's inner
            // shell faces in, and Falcor's emitters shine from their front alone.
            let mut outward: HashMap<usize, f32> = HashMap::new();
            for (t, tri) in tris.iter().enumerate() {
                let r = root(&mut parent, t);
                let Some(g) = pieces.get(&r) else {
                    continue;
                };
                let middle = g.middle / g.area;
                let p = tri.map(|v| Vec3::from(m.mesh.positions[v as usize]));
                let cross = (p[1] - p[0]).cross(p[2] - p[0]);
                // The cosine between its normal and the way from the piece's middle: a sheet
                // of two faces back to back, whose normals cancel too, measures about none.
                let away = ((p[0] + p[1] + p[2]) / 3.0 - middle).normalize_or_zero();
                *outward.entry(r).or_default() += 0.5 * cross.dot(away);
            }
            for (r, g) in pieces {
                let g = Glow {
                    outward: outward.get(&r).copied().unwrap_or(0.0) / g.area,
                    ..g
                };
                out.push((
                    material.name.clone(),
                    Glow {
                        middle: g.middle / g.area,
                        luminance: g.luminance / g.area * EMISSIVE_LUMINANCE,
                        ..g
                    },
                ));
            }
        }
    }
    out
}

/// The lamp a glowing piece lights the scene as (D-087): its light π × luminance × area, from its
/// middle; every way where it is closed (a sphere of its area: a quarter of luminance × area
/// candelas), forward where it is flat (a disc of its area, luminance × area candelas along its
/// normal, falling off as `Emission::flush`). Its colour is its tint, balanced as the lamps'.
fn glow_lamp(g: &Glow) -> Option<Lamp> {
    let luminance = g.luminance.dot(Vec3::new(0.2126, 0.7152, 0.0722));
    if luminance < LEAST_GLOW || g.area <= 0.0 {
        return None;
    }
    let flat = g.normal.length() / g.area > FLAT_PIECE;
    // A closed piece facing in (a lantern's inner shell) lights its fitting's inside alone, and
    // is left out; its glow, a lamp row's, is left out of the probes too.
    if !flat && g.outward < -0.5 {
        return None;
    }
    let pi = std::f32::consts::PI;
    // Its glass's area seen from round the horizon, and the shares of it seen from above and
    // below.
    let side = g.beams[2..].iter().sum::<f32>() / 8.0;
    let (up, down) = (g.beams[0] / side.max(1e-9), g.beams[1] / side.max(1e-9));
    let (candela, radius, emission) = if flat {
        (
            luminance * g.area,
            (g.area / pi).sqrt(),
            Emission::flush(g.normal),
        )
    } else if up < LANTERN_UP && down < LANTERN_DOWN {
        // A lantern: glass round its sides under a metal cap, no light straight up
        // (`Emission::LANTERN`); lit every way, it lit what it hangs from (D-087).
        (
            luminance * side,
            (g.area / (4.0 * pi)).sqrt(),
            Emission::LANTERN,
        )
    } else {
        (
            luminance * g.area / 4.0,
            (g.area / (4.0 * pi)).sqrt(),
            Emission::Every,
        )
    };
    Some(Lamp {
        // A flat piece's in front of it by its radius: the shadow rays aim at its disc, and on
        // the piece itself half the disc lay behind its face, which blocked them from aside.
        position: if flat {
            g.middle + g.normal.normalize_or_zero() * radius
        } else {
            g.middle
        },
        radius,
        candela,
        kelvin: WHITE_BALANCE,
        tint: g.luminance / luminance,
        regions: EVERY_REGION,
        emission,
    })
}

/// The scene's glowing pieces as lamps (D-087's prototype), made once; none without
/// [`lamps_on`].
pub(crate) fn lamps(which: Authored) -> Vec<Lamp> {
    static BISTRO: OnceLock<Vec<Lamp>> = OnceLock::new();
    if which != Authored::Bistro || !lamps_on() {
        return Vec::new();
    }
    BISTRO
        .get_or_init(|| {
            let started = std::time::Instant::now();
            let glows = glows(which);
            let lamps: Vec<Lamp> = glows.iter().filter_map(|(_, g)| glow_lamp(g)).collect();
            tracing::info!(
                pieces = glows.len(),
                lamps = lamps.len(),
                lumens = lamps
                    .iter()
                    .map(|l| l.lumens().dot(Vec3::new(0.2126, 0.7152, 0.0722)))
                    .sum::<f32>() as u64,
                ms = started.elapsed().as_millis() as u64,
                "the Bistro's glowing pieces as lamps (D-087)"
            );
            lamps
        })
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// How each material is drawn: its share of open edges (`open_edges`) and whether `rows`
    /// draws it from both sides, and why. A closed surface has no open edge; a sheet seen from
    /// behind would vanish if drawn from one side.
    /// `cargo test --release -p physics-lab bistro_open_sheets -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bistro_open_sheets() {
        let Some(b) = scene(Authored::Bistro) else {
            return;
        };
        let (_, archive) = Authored::Bistro.folders();
        let cut_out = |path: &Option<String>| {
            path.as_ref().is_some_and(|p| {
                std::fs::read(archive.join(p))
                    .ok()
                    .and_then(|bytes| read_dds(p, &bytes, true).ok())
                    .and_then(|t| alpha_plane(&t))
                    .is_some_and(|a| a.iter().any(|&a| a < 255))
            })
        };
        let mut open: Vec<_> = open_edges(&b.model).into_iter().collect();
        open.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (name, share) in open {
            let why = if name.ends_with(".DoubleSided") {
                "both sides: named so"
            } else if cut_out(&b.roles.get(&name).and_then(|r| r.base_color.clone())) {
                "both sides: cut out"
            } else if SHEETS.iter().any(|s| name.contains(s)) {
                "both sides: a sheet by name"
            } else if share >= OPEN_SHEET {
                "both sides: open"
            } else {
                "one side"
            };
            println!("{name}: {:.1} % open, {why}", 100.0 * share);
        }
    }

    /// The glowing pieces by material (D-087): how many, how many become lamps, their area, their
    /// mean luminance and their lumens, and how many shine forward (flat) or every way.
    /// `cargo test --release -p physics-lab bistro_glows -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bistro_glows() {
        let luma = Vec3::new(0.2126, 0.7152, 0.0722);
        let mut by: std::collections::BTreeMap<String, (usize, usize, usize, f32, f32, f32)> =
            Default::default();
        for (name, g) in glows(Authored::Bistro) {
            let e = by.entry(name.clone()).or_default();
            e.0 += 1;
            if let Some(l) = glow_lamp(&g) {
                e.1 += 1;
                e.2 += usize::from(l.emission != Emission::Every);
                e.5 += l.lumens().dot(luma);
            }
            e.3 += g.area;
            e.4 += g.luminance.dot(luma) * g.area;
            if std::env::var("FORGE_GLOW_PIECES").is_ok() {
                let size = g.high - g.low;
                let side = g.beams[2..].iter().sum::<f32>() / 8.0;
                println!(
                    "  {name} at {:.2} {:.2} {:.2}: {:.3} m², {:.2} x {:.2} x {:.2} m, closed {:.2}, outward {:.2}, up {:.2} down {:.2} of the side's",
                    g.middle.x,
                    g.middle.y,
                    g.middle.z,
                    g.area,
                    size.x,
                    size.y,
                    size.z,
                    1.0 - g.normal.length() / g.area,
                    g.outward,
                    g.beams[0] / side.max(1e-9),
                    g.beams[1] / side.max(1e-9)
                );
            }
        }
        let mut lamps = 0;
        let mut lumens = 0.0;
        for (name, (pieces, lit, flat, area, flux, lm)) in by {
            println!(
                "{name}: {pieces} pieces, {lit} lamps ({flat} flat), {area:.3} m², {:.0} cd/m² mean, {lm:.0} lm",
                flux / area.max(1e-9)
            );
            lamps += lit;
            lumens += lm;
        }
        println!("{lamps} lamps, {lumens:.0} lm");
    }

    /// The meshes whose names hold one of `FORGE_OBJECTS`' comma-separated words (all without
    /// it), with their bounds: the places to match a reference picture by.
    /// `cargo test --release -p physics-lab bistro_objects -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bistro_objects() {
        let Some(b) = scene(Authored::Bistro) else {
            return;
        };
        let words: Vec<String> = std::env::var("FORGE_OBJECTS")
            .map(|w| w.split(',').map(str::to_lowercase).collect())
            .unwrap_or_default();
        for (k, m) in b.model.meshes.iter().enumerate() {
            let name = m.name.to_lowercase();
            if !words.is_empty() && !words.iter().any(|w| name.contains(w.as_str())) {
                continue;
            }
            let (lo, hi) = m.mesh.positions.iter().fold(
                (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
                |(lo, hi), p| (lo.min(Vec3::from(*p)), hi.max(Vec3::from(*p))),
            );
            let c = 0.5 * (lo + hi);
            println!(
                "mesh {k} {}: centre ({:.2}, {:.2}, {:.2}) min ({:.2}, {:.2}, {:.2}) max ({:.2}, {:.2}, {:.2}), {} UVs",
                m.name,
                c.x,
                c.y,
                c.z,
                lo.x,
                lo.y,
                lo.z,
                hi.x,
                hi.y,
                hi.z,
                m.mesh.uvs.len()
            );
            for (s, material) in m.materials.iter().enumerate() {
                // Per section: its triangles, and how often the vertex normals disagree with the
                // winding's face normal and with the outward direction from the mesh's axis.
                let (mut count, mut against_face, mut inward) = (0, 0, 0);
                let mesh = &m.mesh;
                for (t, tri) in mesh.indices.as_chunks::<3>().0.iter().enumerate() {
                    if mesh.section(t) as usize != s {
                        continue;
                    }
                    count += 1;
                    let p = tri.map(|i| Vec3::from(mesh.positions[i as usize]));
                    let face = (p[1] - p[0]).cross(p[2] - p[0]);
                    let normal: Vec3 = tri
                        .iter()
                        .map(|&i| Vec3::from(mesh.normals[i as usize]))
                        .sum();
                    against_face += usize::from(normal.dot(face) < 0.0);
                    let centroid = (p[0] + p[1] + p[2]) / 3.0;
                    let out = (centroid - c) * Vec3::new(1.0, 0.0, 1.0);
                    inward += usize::from(normal.dot(out) < 0.0);
                }
                println!(
                    "  section {s}: {count} triangles, {against_face} against their winding, {inward} facing the axis"
                );
                let maps = b.roles.get(&material.name);
                println!(
                    "  {}: maps {}, base {:?}, metallic {}, roughness {}",
                    material.name,
                    maps.map_or("none".into(), |r| format!("{r:?}")),
                    material.base_color,
                    material.metallic,
                    material.roughness
                );
            }
        }
    }

    /// Pixel `(u, v)` of world point `p` seen from camera `c` (`[x, y, z, yaw, pitch, roll,
    /// fov_y]`, angles in radians, the fly camera's yaw and pitch) in an image `size` wide and
    /// high; `None` behind the camera.
    fn project(c: &[f64; 7], p: [f64; 3], size: [f64; 2]) -> Option<[f64; 2]> {
        let (sy, cy) = c[3].sin_cos();
        let (sp, cp) = c[4].sin_cos();
        let f = [-sy * cp, sp, -cy * cp];
        let r0 = [cy, 0.0, -sy];
        let u0 = [
            r0[1] * f[2] - r0[2] * f[1],
            r0[2] * f[0] - r0[0] * f[2],
            r0[0] * f[1] - r0[1] * f[0],
        ];
        let (sr, cr) = c[5].sin_cos();
        let r: [f64; 3] = std::array::from_fn(|k| cr * r0[k] + sr * u0[k]);
        let u: [f64; 3] = std::array::from_fn(|k| cr * u0[k] - sr * r0[k]);
        let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
        let dot = |a: [f64; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        let z = dot(f);
        if z <= 1e-3 {
            return None;
        }
        let focal = 0.5 * size[1] / (0.5 * c[6]).tan();
        Some([
            0.5 * size[0] + focal * dot(r) / z,
            0.5 * size[1] - focal * dot(u) / z,
        ])
    }

    /// A reference picture's camera fitted to points seen in it (Gauss-Newton on the pixel
    /// errors, from `FORGE_SOLVE_START`'s `--view` and 70°): `FORGE_SOLVE` names a file of lines
    /// `x y z u v` (a scene point and its pixel), `FORGE_SOLVE_SIZE` the picture's `width,height`.
    /// Prints the `--view` and `--fov` that frame it, and each point's error.
    #[test]
    #[ignore]
    fn bistro_solve_view() {
        let (Ok(file), Ok(size), Ok(start)) = (
            std::env::var("FORGE_SOLVE"),
            std::env::var("FORGE_SOLVE_SIZE"),
            std::env::var("FORGE_SOLVE_START"),
        ) else {
            return;
        };
        let nums = |s: &str| -> Vec<f64> {
            s.split([',', ' '])
                .filter(|t| !t.is_empty())
                .filter_map(|t| t.parse().ok())
                .collect()
        };
        let size = nums(&size);
        let size = [size[0], size[1]];
        let points: Vec<([f64; 3], [f64; 2])> = std::fs::read_to_string(&file)
            .expect("the points")
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| {
                let v = nums(l);
                ([v[0], v[1], v[2]], [v[3], v[4]])
            })
            .collect();
        let s = nums(&start);
        let mut c = [
            s[0],
            s[1],
            s[2],
            s[3].to_radians(),
            s[4].to_radians(),
            0.0,
            s.get(5).copied().unwrap_or(70.0).to_radians(),
        ];
        let residuals = |c: &[f64; 7]| -> Vec<f64> {
            points
                .iter()
                .flat_map(|&(p, uv)| {
                    project(c, p, size).map_or([1e4, 1e4], |q| [q[0] - uv[0], q[1] - uv[1]])
                })
                .collect()
        };
        let cost = |r: &[f64]| r.iter().map(|x| x * x).sum::<f64>();
        let mut lambda = 1e-2;
        for _ in 0..200 {
            let r = residuals(&c);
            let base = cost(&r);
            // The Jacobian by central differences.
            let mut j = vec![[0.0; 7]; r.len()];
            for k in 0..7 {
                let h = if k < 3 { 1e-4 } else { 1e-6 };
                let (mut a, mut b) = (c, c);
                a[k] += h;
                b[k] -= h;
                let (ra, rb) = (residuals(&a), residuals(&b));
                for i in 0..r.len() {
                    j[i][k] = (ra[i] - rb[i]) / (2.0 * h);
                }
            }
            // (JᵀJ + λ diag) δ = −Jᵀr, Levenberg–Marquardt.
            let mut a = [[0.0; 7]; 7];
            let mut g = [0.0; 7];
            for i in 0..r.len() {
                for x in 0..7 {
                    g[x] -= j[i][x] * r[i];
                    for y in 0..7 {
                        a[x][y] += j[i][x] * j[i][y];
                    }
                }
            }
            for (x, row) in a.iter_mut().enumerate() {
                row[x] *= 1.0 + lambda;
            }
            // Gaussian elimination.
            let mut m = a;
            let mut v = g;
            for col in 0..7 {
                let pivot = (col..7)
                    .max_by(|&p, &q| m[p][col].abs().total_cmp(&m[q][col].abs()))
                    .unwrap();
                m.swap(col, pivot);
                v.swap(col, pivot);
                for row in col + 1..7 {
                    let k = m[row][col] / m[col][col];
                    let pivot_row = m[col];
                    for (to, from) in m[row][col..].iter_mut().zip(&pivot_row[col..]) {
                        *to -= k * from;
                    }
                    v[row] -= k * v[col];
                }
            }
            let mut delta = [0.0; 7];
            for row in (0..7).rev() {
                let s: f64 = (row + 1..7).map(|x| m[row][x] * delta[x]).sum();
                delta[row] = (v[row] - s) / m[row][row];
            }
            let trial: [f64; 7] = std::array::from_fn(|k| c[k] + delta[k]);
            if cost(&residuals(&trial)) < base {
                c = trial;
                lambda = (lambda * 0.3).max(1e-9);
            } else {
                lambda *= 10.0;
            }
        }
        let r = residuals(&c);
        println!(
            "--view={:.3},{:.3},{:.3},{:.2},{:.2} --fov {:.2} (roll {:.2}°), rms {:.1} px",
            c[0],
            c[1],
            c[2],
            c[3].to_degrees(),
            c[4].to_degrees(),
            c[6].to_degrees(),
            c[5].to_degrees(),
            (cost(&r) / r.len() as f64).sqrt()
        );
        for (i, (p, uv)) in points.iter().enumerate() {
            println!(
                "  point {i} {p:?} at {uv:?}: off by ({:.1}, {:.1}) px",
                r[2 * i],
                r[2 * i + 1]
            );
        }
    }

    /// The camera path's frames as `--view` strings, one every `FORGE_PATH_STRIDE` frames (120
    /// by default): to find a place along it.
    /// `cargo test --release -p physics-lab bistro_path_views -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bistro_path_views() {
        let (converted, _) = Authored::Bistro.folders();
        let Ok(bytes) = std::fs::read(converted.join("BistroExterior.camera.json")) else {
            return;
        };
        let path: CameraPath = serde_json::from_slice(&bytes).expect("a camera path");
        let stride = std::env::var("FORGE_PATH_STRIDE")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(120);
        for k in (0..path.frames.len()).step_by(stride) {
            let (p, yaw, pitch) = path.pose(k).expect("a frame");
            println!(
                "frame {k}: --view={:.2},{:.2},{:.2},{:.1},{:.1}",
                p.x,
                p.y,
                p.z,
                yaw.to_degrees(),
                pitch.to_degrees()
            );
        }
    }

    #[test]
    fn the_paths_camera_looks_where_blender_did() {
        // Blender's camera unturned looks down its −Z, straight down Blender's world: down here.
        let path = CameraPath {
            frames: vec![
                [1.0, 2.0, 3.0, 0.0, 0.0, 0.0, 1.0],
                // The path's first frame: down the street, a little left and down.
                [
                    24.82,
                    3.16,
                    -61.65,
                    0.110_894_08,
                    0.717_842_46,
                    -0.677_190_1,
                    0.117_551_17,
                ],
            ],
        };
        let (p, _, pitch) = path.pose(0).expect("a frame");
        assert_eq!(p, Vec3::new(1.0, 2.0, 3.0));
        assert!(
            (pitch + std::f32::consts::FRAC_PI_2).abs() < 1e-3,
            "{pitch}"
        );
        let (_, yaw, pitch) = path.pose(1).expect("a frame");
        // Forward (−0.32, −0.06, 0.95): towards +Z, so yaw near ±180° less a little.
        assert!(
            (yaw.to_degrees() - 161.4).abs() < 1.0,
            "{}",
            yaw.to_degrees()
        );
        assert!(
            (pitch.to_degrees() + 3.3).abs() < 0.5,
            "{}",
            pitch.to_degrees()
        );
        assert!(path.pose(2).is_none());
    }
}
