//! `physics-lab --lab models` (#170, D-048): models made by others, the Khronos glTF sample
//! assets, to compare Forge's renderings with Khronos's and to test the importer and the
//! materials on them. `tools/fetch-assets.sh` fetches them into `assets/external/`, which git
//! ignores. The scene shows what is there and says what is missing.
//!
//! Each model stands on a plinth in a row, scaled to fit [`FIT`] metres, its meshes as its
//! file places them. `--model NAME` shows one model alone at the origin, scaled the same way,
//! for a capture beside Khronos's screenshot; Sponza only shows that way, at its own size.
//! Skinned models (Fox, CesiumMan) stand in their bind pose.
//!
//! These models are for the labs and the tests only. Nothing that ships reads
//! `assets/external/`, and the reference models (restricted licences, `--reference-only`) are
//! never in the engine or a game.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use forge_geom::city::{Block, Imported, PropKind, PropSpec};
use forge_geom::model::{Model, load_gltf};
use glam::{Mat4, Vec3};

/// The size a model is scaled to fit (its bounds' largest side), metres.
const FIT: f32 = 1.0;
/// The models' spacing in the row, metres.
const SPACING: f32 = 1.6;
/// The plinth: half its side and its height.
const PLINTH_HALF: f32 = 0.6;
const PLINTH_HEIGHT: f32 = 0.3;
/// Models drawn at their own size, only alone (`--model`): scenes the camera stands in.
const ROOMS: [&str; 1] = ["Sponza"];

/// The model `--model` shows alone, set once from the arguments.
pub(crate) static FOCUS: OnceLock<Option<String>> = OnceLock::new();

/// A model found in `assets/external/`.
pub(super) struct External {
    /// Its folder's name (the Khronos name).
    pub name: String,
    /// The model.
    pub model: Model,
    /// The cache key of its meshes: its file and a hash of its bytes.
    key: String,
    /// Its bounds over every mesh: min and max.
    bounds: [Vec3; 2],
    /// Each mesh's prop name (`lab-model-<name>-<mesh>`), leaked: the material rows keep them.
    pub props: Vec<&'static str>,
}

impl External {
    /// Where it stands and how large: scaled to fit [`FIT`] (rooms at their own size), its
    /// bounds' bottom middle at `at`.
    fn placed(&self, at: Vec3) -> Mat4 {
        let [min, max] = self.bounds;
        let scale = if ROOMS.contains(&self.name.as_str()) {
            1.0
        } else {
            FIT / (max - min).max_element().max(1e-6)
        };
        let foot = Vec3::new(0.5 * (min.x + max.x), min.y, 0.5 * (min.z + max.z));
        Mat4::from_translation(at)
            * Mat4::from_scale(Vec3::splat(scale))
            * Mat4::from_translation(-foot)
    }
}

/// The models' folder.
fn folder() -> PathBuf {
    forge_app::workspace_root_from(env!("CARGO_MANIFEST_DIR")).join("assets/external")
}

/// The model file in `dir` (a Khronos model's folder): its `.glb`, else its `.gltf`.
fn model_file(dir: &Path) -> Option<PathBuf> {
    for (kind, extension) in [("glTF-Binary", "glb"), ("glTF", "gltf")] {
        let Ok(files) = std::fs::read_dir(dir.join(kind)) else {
            continue;
        };
        let mut found: Vec<PathBuf> = files
            .flatten()
            .map(|f| f.path())
            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some(extension))
            .collect();
        found.sort();
        if let Some(path) = found.into_iter().next() {
            return Some(path);
        }
    }
    None
}

/// The models in `assets/external/`, read once, by name; those that do not load are logged and
/// left out. With `--model`, that model alone.
pub(super) fn externals() -> &'static [External] {
    static MODELS: OnceLock<Vec<External>> = OnceLock::new();
    MODELS.get_or_init(|| {
        let focus = FOCUS.get().cloned().flatten();
        let Ok(dirs) = std::fs::read_dir(folder()) else {
            tracing::warn!(
                "no external models in {}: tools/fetch-assets.sh fetches them (D-048)",
                folder().display()
            );
            return Vec::new();
        };
        let mut dirs: Vec<PathBuf> = dirs.flatten().map(|d| d.path()).collect();
        dirs.sort();
        let mut out = Vec::new();
        for dir in dirs {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            match &focus {
                Some(f) if *f != name => continue,
                None if ROOMS.contains(&name.as_str()) => continue,
                _ => {}
            }
            let Some(path) = model_file(&dir) else {
                continue;
            };
            let mut model = match load_gltf(&path) {
                Ok(model) => model,
                Err(e) => {
                    tracing::warn!(model = name, "an external model does not load: {e}");
                    continue;
                }
            };
            let bytes = std::fs::read(&path).unwrap_or_default();
            rest_pose(&mut model, &bytes, &name);
            let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |h, &b| {
                (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
            });
            let mut bounds = [Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)];
            for m in &model.meshes {
                for p in &m.mesh.positions {
                    bounds[0] = bounds[0].min(Vec3::from(*p));
                    bounds[1] = bounds[1].max(Vec3::from(*p));
                }
            }
            let props = (0..model.meshes.len())
                .map(|k| &*Box::leak(format!("lab-model-{name}-{k}").into_boxed_str()))
                .collect();
            tracing::info!(
                model = name,
                meshes = model.meshes.len(),
                triangles = model.meshes.iter().map(|m| m.mesh.triangle_count()).sum::<usize>(),
                images = model.images.len(),
                min = ?bounds[0],
                max = ?bounds[1],
                "external model"
            );
            out.push(External {
                key: format!("{}#{digest:016x}", path.display()),
                name,
                model,
                bounds,
                props,
            });
        }
        if let Some(f) = &focus
            && out.is_empty()
        {
            tracing::warn!(
                model = f,
                "no such external model: tools/fetch-assets.sh lists them"
            );
        }
        out
    })
}

/// Stands `model`'s skinned meshes in their rest pose: glTF keeps a skinned mesh in its bind
/// pose and places it by its joints, so a figure whose root joint turns it upright (CesiumMan)
/// lies on its back until it is skinned. Each vertex is bent by its joints' skinning matrices at
/// the rest pose (`forge-anim`, from the file's first skin), and the mesh is then drawn as a
/// rigid one. A file `forge-anim` cannot read keeps its bind pose (logged).
fn rest_pose(model: &mut Model, bytes: &[u8], name: &str) {
    if model.meshes.iter().all(|m| m.skin.is_none()) {
        return;
    }
    let rig = match forge_anim::load_rigs(bytes) {
        Ok(rigs) if !rigs.is_empty() => rigs.into_iter().next().expect("a rig"),
        Ok(_) | Err(_) => {
            tracing::warn!(
                model = name,
                "its skin does not read: drawn in its bind pose"
            );
            return;
        }
    };
    let skeleton = &rig.skeleton;
    let mut world = vec![Mat4::IDENTITY; skeleton.len()];
    let mut skinning = vec![Mat4::IDENTITY; skeleton.len()];
    skeleton.model_space(skeleton.rest(), &mut world);
    skeleton.skinning_matrices(&world, &mut skinning);
    for mesh in &mut model.meshes {
        let Some(skin) = mesh.skin.take() else {
            continue;
        };
        for (v, s) in skin.iter().enumerate() {
            let mut bent = Mat4::ZERO;
            for k in 0..4 {
                if let Some(m) = skinning.get(usize::from(s.joints[k])) {
                    bent += *m * s.weights[k];
                }
            }
            if bent == Mat4::ZERO {
                continue;
            }
            let p = Vec3::from(mesh.mesh.positions[v]);
            mesh.mesh.positions[v] = bent.transform_point3(p).to_array();
            let n = Vec3::from(mesh.mesh.normals[v]);
            let n = bent.inverse().transpose().transform_vector3(n);
            mesh.mesh.normals[v] = n.normalize_or(Vec3::Y).to_array();
        }
    }
}

/// The scene's props: the plinth, then every model's meshes in order.
pub(super) fn props() -> Vec<PropSpec> {
    let mut props = vec![PropSpec {
        name: "lab-model-plinth".to_owned(),
        kind: PropKind::Block(Block {
            half: [PLINTH_HALF, 0.5 * PLINTH_HEIGHT, PLINTH_HALF],
            radius: 0.01,
            segments: 2,
        }),
    }];
    for e in externals() {
        for (k, mesh) in e.model.meshes.iter().enumerate() {
            props.push(PropSpec {
                name: e.props[k].to_owned(),
                kind: PropKind::Imported(Imported {
                    // "posed": skinned meshes stand in their rest pose (`rest_pose`).
                    key: format!("{} posed {k}", e.key),
                    mesh: Arc::new(mesh.mesh.clone()),
                    normal_weight: None,
                }),
            });
        }
    }
    props
}

/// What the scene draws, with the plinth prop at `first` and the models' after it: the models
/// in a row along x centred on the origin, each on a plinth, or the one `--model` names alone
/// at the origin.
pub(super) fn build(first: usize) -> Vec<(usize, Mat4)> {
    let models = externals();
    let alone = FOCUS.get().cloned().flatten().is_some();
    let mut statics = Vec::new();
    let mut prop = first + 1;
    let row = models.len() as f32;
    for (i, e) in models.iter().enumerate() {
        let at = if alone {
            Vec3::ZERO
        } else {
            let x = (i as f32 - 0.5 * (row - 1.0)) * SPACING;
            statics.push((
                first,
                Mat4::from_translation(Vec3::new(x, 0.5 * PLINTH_HEIGHT, 0.0)),
            ));
            Vec3::new(x, PLINTH_HEIGHT, 0.0)
        };
        let placed = e.placed(at);
        for _ in &e.model.meshes {
            statics.push((prop, placed));
            prop += 1;
        }
    }
    statics
}

/// Where the camera starts: before the row; before the one model, framing its bounds as the
/// Khronos screenshots do (the camera's 70° field, a little margin); in a room, at a person's
/// height near one end of its longer side, looking along it.
pub(crate) fn camera() -> (Vec3, f32, f32) {
    let models = externals();
    let alone = FOCUS.get().cloned().flatten().is_some();
    let Some(e) = models.first().filter(|_| alone) else {
        let half = 0.5 * models.len().max(1) as f32 * SPACING;
        return (Vec3::new(0.0, 1.1, 0.8 * half.max(2.0) + 1.0), 0.0, -0.1);
    };
    // The bounds where the model stands.
    let placed = e.placed(Vec3::ZERO);
    let [min, max] = [
        placed.transform_point3(e.bounds[0]),
        placed.transform_point3(e.bounds[1]),
    ];
    let (min, max) = (min.min(max), min.max(max));
    let middle = 0.5 * (min + max);
    if ROOMS.contains(&e.name.as_str()) {
        let along_x = max.x - min.x > max.z - min.z;
        return if along_x {
            let x = max.x - 0.15 * (max.x - min.x);
            (Vec3::new(x, min.y + 1.7, middle.z), 1.571, 0.1)
        } else {
            let z = max.z - 0.15 * (max.z - min.z);
            (Vec3::new(middle.x, min.y + 1.7, z), 0.0, 0.1)
        };
    }
    let radius = 0.5 * (max - min).length();
    let distance = 1.1 * radius / 35_f32.to_radians().sin();
    (
        Vec3::new(middle.x, middle.y, max.z + distance - (max.z - middle.z)),
        0.0,
        0.0,
    )
}
