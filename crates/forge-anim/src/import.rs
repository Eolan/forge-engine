//! Skeletons and clips from glTF 2.0's skins and animations (the `.glb` Blender exports).

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use gltf::animation::util::ReadOutputs;

use crate::clip::{Clip, Interpolation, JointTracks, Track};
use crate::pose::Transform;
use crate::skeleton::{Skeleton, SkeletonError};

/// Why a file gave no rig.
#[derive(Debug, thiserror::Error)]
pub enum AnimError {
    /// The bytes are not a glTF file the reader understands.
    #[error("not a glTF model: {0}")]
    Format(#[from] gltf::Error),
    /// The file refers to data outside itself (Forge reads `.glb` with its buffer inside).
    #[error("the model's buffer {0} is not in the file")]
    ExternalBuffer(usize),
    /// No skin in the file.
    #[error("the model has no skin")]
    NoSkin,
    /// The skin's joints do not make a skeleton.
    #[error("the skin's joints: {0}")]
    Skeleton(#[from] SkeletonError),
    /// A channel's keys cannot be read.
    #[error("animation {animation:?}: {reason}")]
    Channel {
        /// The animation's name.
        animation: String,
        /// What is wrong.
        reason: &'static str,
    },
}

/// A skin's skeleton and the clips that move it.
#[derive(Clone, Debug)]
pub struct Rig {
    /// The skin's name (Blender's armature's).
    pub name: String,
    /// The skin's joints.
    pub skeleton: Skeleton,
    /// The animations that move some of its joints, keeping their channels on them.
    pub clips: Vec<Clip>,
}

impl Rig {
    /// The clip named `name`.
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }
}

/// Reads a binary glTF's first skin as a skeleton (joints in the skin's order, which the
/// vertices' joint indices count in) and the animations that move it as clips.
pub fn load_rig(bytes: &[u8]) -> Result<Rig, AnimError> {
    load_rigs(bytes)?
        .into_iter()
        .next()
        .ok_or(AnimError::NoSkin)
}

/// Reads every skin of a binary glTF as [`load_rig`] reads the first, in the file's order.
pub fn load_rigs(bytes: &[u8]) -> Result<Vec<Rig>, AnimError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();
    for buffer in gltf.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(AnimError::ExternalBuffer(buffer.index()));
        }
    }
    let mut parent_of_node = vec![None; gltf.nodes().len()];
    for node in gltf.nodes() {
        for child in node.children() {
            parent_of_node[child.index()] = Some(node.index());
        }
    }
    gltf.skins()
        .map(|skin| read_rig(&gltf, &skin, blob, &parent_of_node))
        .collect()
}

fn read_rig(
    gltf: &gltf::Document,
    skin: &gltf::Skin,
    blob: Option<&[u8]>,
    parent_of_node: &[Option<usize>],
) -> Result<Rig, AnimError> {
    let joints: Vec<gltf::Node> = skin.joints().collect();
    let mut joint_of_node = vec![None; parent_of_node.len()];
    for (j, node) in joints.iter().enumerate() {
        joint_of_node[node.index()] = Some(j);
    }
    let local = |node: usize| {
        let node = gltf.nodes().nth(node).expect("a node the file lists");
        Mat4::from_cols_array_2d(&node.transform().matrix())
    };
    // Each joint's parent is its nearest ancestor that is a joint; the nodes in between (or
    // above a root) make its frame, and `chains` keeps them, the topmost first.
    let mut parents = Vec::with_capacity(joints.len());
    let mut frames = Vec::with_capacity(joints.len());
    let mut chains: Vec<Vec<usize>> = Vec::with_capacity(joints.len());
    for node in &joints {
        let mut frame = Mat4::IDENTITY;
        let mut parent = None;
        let mut chain = Vec::new();
        let mut at = parent_of_node[node.index()];
        while let Some(n) = at {
            if let Some(j) = joint_of_node[n] {
                parent = Some(j as u16);
                break;
            }
            frame = local(n) * frame;
            chain.insert(0, n);
            at = parent_of_node[n];
        }
        parents.push(parent);
        frames.push(frame);
        chains.push(chain);
    }
    let rest: Vec<Transform> = joints
        .iter()
        .map(|node| {
            let (t, r, s) = node.transform().decomposed();
            Transform {
                translation: Vec3::from(t),
                rotation: Quat::from_array(r).normalize(),
                scale: Vec3::from(s),
            }
        })
        .collect();
    let inverse_bind: Vec<Mat4> = match skin.reader(|_| blob).read_inverse_bind_matrices() {
        Some(matrices) => matrices.map(|m| Mat4::from_cols_array_2d(&m)).collect(),
        None => vec![Mat4::IDENTITY; joints.len()],
    };
    let names = joints
        .iter()
        .map(|n| n.name().unwrap_or_default().to_owned())
        .collect();
    let skeleton = Skeleton::new(names, parents, frames.clone(), &rest, inverse_bind)?;

    let mut clips = Vec::new();
    for animation in gltf.animations() {
        let name = animation.name().unwrap_or_default().to_owned();
        let bad = |reason| AnimError::Channel {
            animation: name.clone(),
            reason,
        };
        let mut tracks = vec![JointTracks::default(); joints.len()];
        // The tracks of the nodes between a joint and its parent (#310): a root frame above the
        // pelvis that carries the clip's travel and turn.
        let mut between: HashMap<usize, JointTracks> = HashMap::new();
        let (mut first, mut last) = (f32::INFINITY, f32::NEG_INFINITY);
        for channel in animation.channels() {
            let node = channel.target().node().index();
            let target = match joint_of_node[node] {
                Some(j) => &mut tracks[j],
                None if chains.iter().any(|c| c.contains(&node)) => {
                    between.entry(node).or_default()
                }
                None => continue,
            };
            let reader = channel.reader(|_| blob);
            let times: Vec<f32> = reader
                .read_inputs()
                .ok_or_else(|| bad("no key times"))?
                .collect();
            if times.is_empty() || times.windows(2).any(|w| w[0] >= w[1]) {
                return Err(bad("key times not increasing"));
            }
            let interpolation = match channel.sampler().interpolation() {
                gltf::animation::Interpolation::Step => Interpolation::Step,
                gltf::animation::Interpolation::Linear => Interpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => Interpolation::CubicSpline,
            };
            let per_key = if interpolation == Interpolation::CubicSpline {
                3
            } else {
                1
            };
            let expected = per_key * times.len();
            first = first.min(times[0]);
            last = last.max(times[times.len() - 1]);
            match reader.read_outputs().ok_or_else(|| bad("no key values"))? {
                ReadOutputs::Translations(values) => {
                    let values: Vec<Vec3> = values.map(Vec3::from).collect();
                    if values.len() != expected {
                        return Err(bad("not one value per key"));
                    }
                    target.translation = Some(Track {
                        times,
                        values,
                        interpolation,
                    });
                }
                ReadOutputs::Rotations(values) => {
                    let values: Vec<Quat> = values.into_f32().map(Quat::from_array).collect();
                    if values.len() != expected {
                        return Err(bad("not one value per key"));
                    }
                    target.rotation = Some(Track {
                        times,
                        values,
                        interpolation,
                    });
                }
                ReadOutputs::Scales(values) => {
                    let values: Vec<Vec3> = values.map(Vec3::from).collect();
                    if values.len() != expected {
                        return Err(bad("not one value per key"));
                    }
                    target.scale = Some(Track {
                        times,
                        values,
                        interpolation,
                    });
                }
                // Morph target weights (#169): not read from clips yet; the scene gives them
                // every frame (`forge_render::MeshletScene::set_morphs`).
                ReadOutputs::MorphTargetWeights(_) => {}
            }
        }
        if first > last {
            // Nothing on this skin's joints.
            continue;
        }
        // A joint under animated nodes that are not joints takes their motion (#310): its
        // track becomes what its frame and its own transform make together, over the frame
        // the skeleton keeps still, sampled at BAKE_RATE.
        for (j, chain) in chains.iter().enumerate() {
            if !chain.iter().any(|n| between.contains_key(n)) {
                continue;
            }
            let node_at = |n: usize, t: f32| {
                let (tr, r, s) = gltf
                    .nodes()
                    .nth(n)
                    .expect("a listed node")
                    .transform()
                    .decomposed();
                let at = between.get(&n);
                transform_at(
                    at,
                    t,
                    Vec3::from(tr),
                    Quat::from_array(r).normalize(),
                    Vec3::from(s),
                )
            };
            let own_rest = &rest[j];
            let still = frames[j].inverse();
            let steps = ((last - first) * BAKE_RATE).ceil().max(1.0) as usize;
            let times: Vec<f32> = (0..=steps)
                .map(|k| first + (last - first) * k as f32 / steps as f32)
                .collect();
            let (mut translations, mut rotations, mut scales) =
                (Vec::new(), Vec::new(), Vec::new());
            for &t in &times {
                let above = chain.iter().fold(Mat4::IDENTITY, |m, &n| m * node_at(n, t));
                let own = transform_at(
                    Some(&tracks[j]),
                    t,
                    own_rest.translation,
                    own_rest.rotation,
                    own_rest.scale,
                );
                let (s, r, tr) = (still * above * own).to_scale_rotation_translation();
                // Each rotation on the side of the one before, so the keys blend the short way.
                let r = match rotations.last() {
                    Some(&before) if r.dot(before) < 0.0 => -r,
                    _ => r,
                };
                translations.push(tr);
                rotations.push(r);
                scales.push(s);
            }
            let track = |values| Track {
                times: times.clone(),
                values,
                interpolation: Interpolation::Linear,
            };
            tracks[j] = JointTracks {
                translation: Some(track(translations)),
                rotation: Some(Track {
                    times: times.clone(),
                    values: rotations,
                    interpolation: Interpolation::Linear,
                }),
                scale: Some(track(scales)),
            };
        }
        clips.push(Clip {
            name,
            start: first,
            duration: last - first,
            joints: tracks,
        });
    }
    Ok(Rig {
        name: skin.name().unwrap_or_default().to_owned(),
        skeleton,
        clips,
    })
}

/// Keys a second of a joint's track baked from the animated nodes above it (#310).
const BAKE_RATE: f32 = 60.0;

/// A node's or a joint's local matrix at `time`: its tracks where it has them, else its rest
/// translation `t`, rotation `r` and scale `s`.
fn transform_at(tracks: Option<&JointTracks>, time: f32, t: Vec3, r: Quat, s: Vec3) -> Mat4 {
    let t = tracks
        .and_then(|k| k.translation.as_ref())
        .map_or(t, |k| k.sample(time));
    let r = tracks
        .and_then(|k| k.rotation.as_ref())
        .map_or(r, |k| k.sample(time));
    let s = tracks
        .and_then(|k| k.scale.as_ref())
        .map_or(s, |k| k.sample(time));
    Mat4::from_scale_rotation_translation(s, r, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pose::Pose;

    /// Packs `json` and `bin` as a `.glb`.
    fn glb(json: String, mut bin: Vec<u8>) -> Vec<u8> {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let json = json.replace("BIN_LENGTH", &bin.len().to_string());
        let mut json = json.into_bytes();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let total = 12 + 8 + json.len() + 8 + bin.len();
        let mut out = Vec::new();
        out.extend_from_slice(b"glTF");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&(total as u32).to_le_bytes());
        out.extend_from_slice(&(json.len() as u32).to_le_bytes());
        out.extend_from_slice(b"JSON");
        out.extend_from_slice(&json);
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(b"BIN\0");
        out.extend_from_slice(&bin);
        out
    }

    /// An armature node 3 m up holding a two-joint arm along +y (`shoulder`, then `elbow` 1 m
    /// above it), its inverse bind matrices, and a clip "raise" turning the shoulder a quarter
    /// round +z over one second; Blender's layout (the armature object above the root joint).
    fn arm_glb() -> Vec<u8> {
        arm_glb_with(false)
    }

    /// [`arm_glb`]; with `travel`, the clip also carries the armature node (not a joint) 2 m
    /// along +z over its second, as a root frame above the pelvis carries a walk's travel (#310).
    fn arm_glb_with(travel: bool) -> Vec<u8> {
        let mut bin: Vec<u8> = Vec::new();
        let mut put = |values: &[f32]| {
            let offset = bin.len();
            bin.extend(values.iter().flat_map(|v| v.to_le_bytes()));
            offset
        };
        // Bound where the joints rest in the model's frame, in the skin's order: the elbow at
        // y = 4, the shoulder at 3.
        let inverse_bind: Vec<f32> = [4.0_f32, 3.0]
            .iter()
            .flat_map(|&y| Mat4::from_translation(Vec3::new(0.0, -y, 0.0)).to_cols_array())
            .collect();
        let ibm = put(&inverse_bind);
        let times = put(&[0.0, 1.0]);
        let s = std::f32::consts::FRAC_1_SQRT_2;
        let rotations = put(&[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, s, s]);
        let moves = put(&[0.0, 3.0, 0.0, 0.0, 3.0, 2.0]);
        let (sampler, channel, accessor, view) = if travel {
            (
                r#",{"input":1,"output":3,"interpolation":"LINEAR"}"#,
                r#",{"sampler":1,"target":{"node":0,"path":"translation"}}"#,
                r#",{"bufferView":3,"componentType":5126,"count":2,"type":"VEC3"}"#,
                format!(r#",{{"buffer":0,"byteOffset":{moves},"byteLength":24}}"#),
            )
        } else {
            ("", "", "", String::new())
        };
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[
              {{"name":"Armature","translation":[0,3,0],"children":[1]}},
              {{"name":"shoulder","children":[2]}},
              {{"name":"elbow","translation":[0,1,0]}}],
            "skins":[{{"joints":[2,1],"inverseBindMatrices":0}}],
            "animations":[{{"name":"raise",
              "samplers":[{{"input":1,"output":2,"interpolation":"LINEAR"}}{sampler}],
              "channels":[{{"sampler":0,"target":{{"node":1,"path":"rotation"}}}}{channel}]}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":2,"type":"MAT4"}},
              {{"bufferView":1,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},
              {{"bufferView":2,"componentType":5126,"count":2,"type":"VEC4"}}{accessor}],
            "bufferViews":[
              {{"buffer":0,"byteOffset":{ibm},"byteLength":128}},
              {{"buffer":0,"byteOffset":{times},"byteLength":8}},
              {{"buffer":0,"byteOffset":{rotations},"byteLength":32}}{view}],
            "buffers":[{{"byteLength":BIN_LENGTH}}]}}"#
        );
        glb(json, bin)
    }

    /// A clip that moves a node above the root joint which is not a joint (a root frame over the
    /// pelvis) moves the skeleton with it (#310): its motion baked into the root's track.
    #[test]
    fn an_animated_frame_above_the_root_carries_the_skeleton() {
        let rig = load_rig(&arm_glb_with(true)).unwrap();
        let skeleton = &rig.skeleton;
        let clip = rig.clip("raise").unwrap();
        assert_eq!((clip.start, clip.duration), (0.0, 1.0));
        let mut model = vec![Mat4::IDENTITY; 2];
        let mut pose = Pose::clone(skeleton.rest());
        // At rest the skeleton stands as without the channel.
        skeleton.model_space(skeleton.rest(), &mut model);
        let elbow = model[0].transform_point3(Vec3::ZERO);
        assert!(elbow.abs_diff_eq(Vec3::new(0.0, 4.0, 0.0), 1e-6), "{elbow}");
        // Halfway, the frame has carried the arm 1 m along +z while the shoulder turns.
        clip.sample(skeleton, 0.5, &mut pose);
        skeleton.model_space(&pose, &mut model);
        let shoulder = model[1].transform_point3(Vec3::ZERO);
        assert!(
            shoulder.abs_diff_eq(Vec3::new(0.0, 3.0, 1.0), 1e-4),
            "{shoulder}"
        );
        // At the end, 2 m along with the quarter turn: the elbow at the shoulder's −x side.
        clip.sample(skeleton, 1.0, &mut pose);
        skeleton.model_space(&pose, &mut model);
        let elbow = model[0].transform_point3(Vec3::ZERO);
        assert!(
            elbow.abs_diff_eq(Vec3::new(-1.0, 3.0, 2.0), 1e-4),
            "{elbow}"
        );
        // Without the channel, the clip turns the shoulder where it stands.
        let still = load_rig(&arm_glb()).unwrap();
        let mut pose = Pose::clone(still.skeleton.rest());
        still
            .clip("raise")
            .unwrap()
            .sample(&still.skeleton, 1.0, &mut pose);
        still.skeleton.model_space(&pose, &mut model);
        let elbow = model[0].transform_point3(Vec3::ZERO);
        assert!(
            elbow.abs_diff_eq(Vec3::new(-1.0, 3.0, 0.0), 1e-6),
            "{elbow}"
        );
    }

    #[test]
    fn a_skin_and_its_clip_come_through() {
        let rig = load_rig(&arm_glb()).unwrap();
        let skeleton = &rig.skeleton;
        // The skin's order, child first, as the file lists it.
        assert_eq!(skeleton.names(), ["elbow", "shoulder"]);
        assert_eq!(skeleton.parent(0), Some(1));
        assert_eq!(skeleton.parent(1), None);

        let mut model = vec![Mat4::IDENTITY; 2];
        let mut skin = vec![Mat4::IDENTITY; 2];
        // At rest the armature's 3 m are under the root, and the bind matches the rest.
        skeleton.model_space(skeleton.rest(), &mut model);
        let elbow = model[0].transform_point3(Vec3::ZERO);
        assert!(elbow.abs_diff_eq(Vec3::new(0.0, 4.0, 0.0), 1e-6), "{elbow}");
        skeleton.skinning_matrices(&model, &mut skin);
        assert!(skin.iter().all(|m| m.abs_diff_eq(Mat4::IDENTITY, 1e-6)));

        let clip = rig.clip("raise").unwrap();
        assert_eq!((clip.start, clip.duration), (0.0, 1.0));
        let mut pose = Pose::clone(skeleton.rest());
        clip.sample(skeleton, 1.0, &mut pose);
        skeleton.model_space(&pose, &mut model);
        skeleton.skinning_matrices(&model, &mut skin);
        // A quarter turn about +z at the shoulder swings the elbow from above it to its −x
        // side, and a vertex bound at the elbow with it.
        let elbow = model[0].transform_point3(Vec3::ZERO);
        assert!(
            elbow.abs_diff_eq(Vec3::new(-1.0, 3.0, 0.0), 1e-6),
            "{elbow}"
        );
        let vertex = skin[0].transform_point3(Vec3::new(0.0, 4.0, 0.0));
        assert!(
            vertex.abs_diff_eq(Vec3::new(-1.0, 3.0, 0.0), 1e-6),
            "{vertex}"
        );
    }

    /// The lab's creatures as Blender exports them (`assets/blender/skinned_creatures.py`).
    #[test]
    fn the_lab_s_creatures_come_through() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/models/skinned-creatures.glb"
        );
        let rigs = load_rigs(&std::fs::read(path).unwrap()).unwrap();
        let mut names: Vec<&str> = rigs.iter().map(|r| r.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["dog", "mannequin"]);
        for rig in &rigs {
            let skeleton = &rig.skeleton;
            assert_eq!(skeleton.len(), 11, "{}", rig.name);
            let root = if rig.name == "dog" { "torso" } else { "pelvis" };
            assert_eq!(skeleton.parent(skeleton.joint(root).unwrap()), None);
            // Bound at rest: every skinning matrix of the rest pose is the identity.
            let mut model = vec![Mat4::IDENTITY; 11];
            let mut skin = vec![Mat4::IDENTITY; 11];
            skeleton.model_space(skeleton.rest(), &mut model);
            skeleton.skinning_matrices(&model, &mut skin);
            for (j, m) in skin.iter().enumerate() {
                assert!(
                    m.abs_diff_eq(Mat4::IDENTITY, 1e-4),
                    "{} joint {j}: {m}",
                    rig.name
                );
            }
            // Its walk and its idle, which loop: their last pose is their first.
            let seconds = if rig.name == "dog" {
                [0.75, 2.0]
            } else {
                [1.0, 4.0]
            };
            for (clip, seconds) in ["walk", "idle"].into_iter().zip(seconds) {
                let clip = rig.clip(&format!("{}-{clip}", rig.name)).unwrap();
                assert!((clip.duration - seconds).abs() < 1e-4, "{}", clip.name);
                let (mut first, mut last) = (skeleton.rest().clone(), skeleton.rest().clone());
                clip.sample(skeleton, 0.0, &mut first);
                clip.sample(skeleton, clip.duration, &mut last);
                for j in 0..11 {
                    let (a, b) = (first.transform(j), last.transform(j));
                    assert!(
                        a.translation.abs_diff_eq(b.translation, 1e-5),
                        "{}",
                        clip.name
                    );
                    assert!(a.rotation.abs_diff_eq(b.rotation, 1e-5), "{}", clip.name);
                }
            }
        }
    }

    #[test]
    fn a_model_without_a_skin_has_no_rig() {
        let json = r#"{"asset":{"version":"2.0"},"nodes":[{"name":"x"}]}"#;
        let bytes = glb(json.to_owned(), Vec::new());
        assert!(matches!(load_rig(&bytes), Err(AnimError::NoSkin)));
    }
}
