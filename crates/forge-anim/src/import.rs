//! Skeletons and clips from glTF 2.0's skins and animations (the `.glb` Blender exports).

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
    /// The first skin's joints.
    pub skeleton: Skeleton,
    /// Every animation of the file, keeping the channels on the skeleton's joints.
    pub clips: Vec<Clip>,
}

impl Rig {
    /// The clip named `name`.
    pub fn clip(&self, name: &str) -> Option<&Clip> {
        self.clips.iter().find(|c| c.name == name)
    }
}

/// Reads a binary glTF's first skin as a skeleton (joints in the skin's order, which the
/// vertices' joint indices count in) and its animations as clips.
pub fn load_rig(bytes: &[u8]) -> Result<Rig, AnimError> {
    let gltf = gltf::Gltf::from_slice(bytes)?;
    let blob = gltf.blob.as_deref();
    for buffer in gltf.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(AnimError::ExternalBuffer(buffer.index()));
        }
    }
    let skin = gltf.skins().next().ok_or(AnimError::NoSkin)?;
    let joints: Vec<gltf::Node> = skin.joints().collect();
    let node_count = gltf.nodes().len();
    let mut joint_of_node = vec![None; node_count];
    for (j, node) in joints.iter().enumerate() {
        joint_of_node[node.index()] = Some(j);
    }
    let mut parent_of_node = vec![None; node_count];
    for node in gltf.nodes() {
        for child in node.children() {
            parent_of_node[child.index()] = Some(node.index());
        }
    }
    let local = |node: usize| {
        let node = gltf.nodes().nth(node).expect("a node the file lists");
        Mat4::from_cols_array_2d(&node.transform().matrix())
    };
    // Each joint's parent is its nearest ancestor that is a joint; the nodes in between (or
    // above a root) make its frame.
    let mut parents = Vec::with_capacity(joints.len());
    let mut frames = Vec::with_capacity(joints.len());
    for node in &joints {
        let mut frame = Mat4::IDENTITY;
        let mut parent = None;
        let mut at = parent_of_node[node.index()];
        while let Some(n) = at {
            if let Some(j) = joint_of_node[n] {
                parent = Some(j as u16);
                break;
            }
            frame = local(n) * frame;
            at = parent_of_node[n];
        }
        parents.push(parent);
        frames.push(frame);
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
    let skeleton = Skeleton::new(names, parents, frames, &rest, inverse_bind)?;

    let mut clips = Vec::new();
    for animation in gltf.animations() {
        let name = animation.name().unwrap_or_default().to_owned();
        let bad = |reason| AnimError::Channel {
            animation: name.clone(),
            reason,
        };
        let mut tracks = vec![JointTracks::default(); joints.len()];
        let (mut first, mut last) = (f32::INFINITY, f32::NEG_INFINITY);
        for channel in animation.channels() {
            let Some(j) = joint_of_node[channel.target().node().index()] else {
                continue;
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
                    tracks[j].translation = Some(Track {
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
                    tracks[j].rotation = Some(Track {
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
                    tracks[j].scale = Some(Track {
                        times,
                        values,
                        interpolation,
                    });
                }
                // Morph target weights: no morph targets in Forge.
                ReadOutputs::MorphTargetWeights(_) => {}
            }
        }
        if first > last {
            // Nothing on this skin's joints.
            continue;
        }
        clips.push(Clip {
            name,
            start: first,
            duration: last - first,
            joints: tracks,
        });
    }
    Ok(Rig { skeleton, clips })
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
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[
              {{"name":"Armature","translation":[0,3,0],"children":[1]}},
              {{"name":"shoulder","children":[2]}},
              {{"name":"elbow","translation":[0,1,0]}}],
            "skins":[{{"joints":[2,1],"inverseBindMatrices":0}}],
            "animations":[{{"name":"raise",
              "samplers":[{{"input":1,"output":2,"interpolation":"LINEAR"}}],
              "channels":[{{"sampler":0,"target":{{"node":1,"path":"rotation"}}}}]}}],
            "accessors":[
              {{"bufferView":0,"componentType":5126,"count":2,"type":"MAT4"}},
              {{"bufferView":1,"componentType":5126,"count":2,"type":"SCALAR","min":[0],"max":[1]}},
              {{"bufferView":2,"componentType":5126,"count":2,"type":"VEC4"}}],
            "bufferViews":[
              {{"buffer":0,"byteOffset":{ibm},"byteLength":128}},
              {{"buffer":0,"byteOffset":{times},"byteLength":8}},
              {{"buffer":0,"byteOffset":{rotations},"byteLength":32}}],
            "buffers":[{{"byteLength":BIN_LENGTH}}]}}"#
        );
        glb(json, bin)
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

    #[test]
    fn a_model_without_a_skin_has_no_rig() {
        let json = r#"{"asset":{"version":"2.0"},"nodes":[{"name":"x"}]}"#;
        let bytes = glb(json.to_owned(), Vec::new());
        assert!(matches!(load_rig(&bytes), Err(AnimError::NoSkin)));
    }
}
