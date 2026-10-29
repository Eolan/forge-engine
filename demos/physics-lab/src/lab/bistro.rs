//! `physics-lab --lab bistro` (D-083 step 1): the Amazon Lumberyard Bistro's exterior, an
//! authored scene (ORCA, CC BY 4.0) to measure Forge's image, load and frame against, by day
//! (Forge's sun and sky) and by night (its emissive surfaces alone, `--time-of-day 1.5`).
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
use forge_render::dds::{BlockFormat, alpha_plane, flip_green, read_dds};
use forge_render::material::{ExternalMaps, external_layer, upload_compressed};
use glam::{Mat4, Quat, Vec3};

use city_blocks::CityMaterials;

/// The luminance of an emissive map's full colour, cd/m² (a lit shop sign or lamp shade): the
/// night is lit by these alone. To be set against the path-traced reference (D-083 step 2).
const EMISSIVE_LUMINANCE: f32 = 1000.0;
/// Cut out under this base colour alpha where the map has any (the archive's alpha is opacity,
/// tested at a half as its own renderer does).
const ALPHA_CUTOFF: f32 = 0.5;

/// The converted exterior's folder, and the archive's (the maps' paths are relative to it).
fn folders() -> (PathBuf, PathBuf) {
    let root =
        forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR")).join("assets/external/bistro");
    (root.join("converted"), root.join("Bistro_v5_2"))
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

/// The exterior, read once; `None` (logged) where the converted files are missing.
pub(super) fn bistro() -> Option<&'static Bistro> {
    static BISTRO: OnceLock<Option<Bistro>> = OnceLock::new();
    BISTRO
        .get_or_init(|| match read() {
            Ok(b) => Some(b),
            Err(e) => {
                tracing::warn!(
                    "no Bistro: {e:#} (fetch the archive into assets/external/bistro/ and run \
                     tools/fbx-to-gltf.py, D-083)"
                );
                None
            }
        })
        .as_ref()
}

fn read() -> Result<Bistro> {
    let (converted, _) = folders();
    let gltf = converted.join("BistroExterior.gltf");
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
        "bistro-exterior#{}#{}",
        stamp("BistroExterior.gltf"),
        stamp("BistroExterior.bin")
    );
    let roles: HashMap<String, Roles> = serde_json::from_slice(
        &std::fs::read(converted.join("BistroExterior.textures.json"))
            .context("BistroExterior.textures.json")?,
    )
    .context("BistroExterior.textures.json")?;
    let camera = std::fs::read(converted.join("BistroExterior.camera.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CameraPath>(&bytes).ok());
    let props = (0..model.meshes.len())
        .map(|k| &*Box::leak(format!("lab-bistro-{k}").into_boxed_str()))
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
        "the Bistro's exterior"
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
pub(super) fn props() -> Vec<PropSpec> {
    let Some(b) = bistro() else {
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
pub(super) fn build(first: usize) -> Vec<(usize, Mat4)> {
    bistro().map_or_else(Vec::new, |b| {
        (0..b.model.meshes.len())
            .map(|k| (first + k, Mat4::IDENTITY))
            .collect()
    })
}

/// Where the camera starts: the path's first frame (the street before the café), else above
/// the scene's middle.
pub(crate) fn camera() -> (Vec3, f32, f32) {
    bistro()
        .and_then(|b| b.camera.as_ref())
        .and_then(|c| c.pose(0))
        .unwrap_or((Vec3::new(0.0, 3.0, 0.0), 0.0, 0.0))
}

/// The rows of every mesh's materials, their maps read from the archive and uploaded once each:
/// the base colour and emission sRGB, the normal maps' green turned over, a material cut out
/// where its base colour's map has any alpha.
pub(super) fn rows(device: &Arc<forge_gpu::Device>, materials: &mut CityMaterials) -> Result<()> {
    let Some(b) = bistro() else {
        return Ok(());
    };
    let started = std::time::Instant::now();
    let (_, archive) = folders();
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
    let uploaded = upload_compressed(device, &read)?;
    let mut ids = HashMap::new();
    let mut cut_out = HashMap::new();
    let mut bytes = 0_u64;
    for ((image, size), texture) in uploaded.into_iter().zip(&read) {
        bytes += size;
        let id = materials.textures.add_compressed(image, size, texture);
        let alpha = alpha_plane(texture).is_some_and(|a| a.iter().any(|&a| a < 255));
        cut_out.insert(texture.name.clone(), alpha);
        ids.insert(texture.name.clone(), id);
    }
    let map = |p: &Option<String>| p.as_ref().and_then(|p| ids.get(p).copied());
    let mut rows_made = 0;
    for (k, mesh) in b.model.meshes.iter().enumerate() {
        let uvs = !mesh.mesh.uvs.is_empty();
        let rows = mesh
            .materials
            .iter()
            .map(|m| {
                let r = b.roles.get(&m.name).cloned().unwrap_or_default();
                let maps = if uvs {
                    ExternalMaps {
                        base_color: map(&r.base_color),
                        metal_rough: map(&r.specular),
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
                (m.name.clone(), external_layer(m, maps))
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
        total_ms = started.elapsed().as_millis() as u64,
        "the Bistro's maps"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The camera path's frames as `--view` strings, one every `FORGE_PATH_STRIDE` frames (120
    /// by default): to find a place along it.
    /// `cargo test --release -p physics-lab bistro_path_views -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bistro_path_views() {
        let (converted, _) = folders();
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
