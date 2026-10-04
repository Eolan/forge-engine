//! A skeleton: its joints, their parents, the rest pose and the bind.

use glam::Mat4;

use crate::pose::{Pose, Transform};

/// Why joints do not make a skeleton.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum SkeletonError {
    /// The arrays do not all have one entry per joint.
    #[error("{0} joints, but not one entry per joint in every array")]
    Mismatch(usize),
    /// More joints than a vertex's joint index holds.
    #[error("{0} joints: at most 65 536")]
    TooManyJoints(usize),
    /// A joint names a parent that does not exist, or is its own ancestor.
    #[error("joint {0}'s parent is missing or makes a loop")]
    BadParent(usize),
}

/// Joints in the skin's order (the order the vertices' joint indices count in), each with its
/// parent, its rest transform and its inverse bind matrix.
#[derive(Clone, Debug)]
pub struct Skeleton {
    names: Vec<String>,
    parents: Vec<Option<u16>>,
    /// What stands between a joint and its parent joint, or the scene for a root: the
    /// transforms of the nodes that are not joints (Blender's armature object). Identity for
    /// most joints.
    frames: Vec<Mat4>,
    rest: Pose,
    inverse_bind: Vec<Mat4>,
    /// Every joint after its parent: the order the model-space pass walks.
    order: Vec<u16>,
}

impl Skeleton {
    /// A skeleton of `names.len()` joints. `parents[j]` is joint `j`'s parent (none for a
    /// root); `frames[j]` stands between it and its parent (identity but for a root under a
    /// node that is not a joint).
    pub fn new(
        names: Vec<String>,
        parents: Vec<Option<u16>>,
        frames: Vec<Mat4>,
        rest: &[Transform],
        inverse_bind: Vec<Mat4>,
    ) -> Result<Self, SkeletonError> {
        let n = names.len();
        if parents.len() != n || frames.len() != n || rest.len() != n || inverse_bind.len() != n {
            return Err(SkeletonError::Mismatch(n));
        }
        if n > usize::from(u16::MAX) + 1 {
            return Err(SkeletonError::TooManyJoints(n));
        }
        // Depth by walking up; a walk longer than the skeleton is a loop.
        let mut depth = vec![0usize; n];
        for (j, d) in depth.iter_mut().enumerate() {
            let mut at = j;
            while let Some(p) = parents[at] {
                let p = usize::from(p);
                if p >= n || *d >= n {
                    return Err(SkeletonError::BadParent(j));
                }
                *d += 1;
                at = p;
            }
        }
        let mut order: Vec<u16> = (0..n as u32).map(|j| j as u16).collect();
        order.sort_by_key(|&j| depth[usize::from(j)]);
        Ok(Self {
            names,
            parents,
            frames,
            rest: Pose::from_transforms(rest),
            inverse_bind,
            order,
        })
    }

    /// The number of joints.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// No joints.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// The joints' names ("" when unnamed).
    pub fn names(&self) -> &[String] {
        &self.names
    }

    /// The joint named `name`.
    pub fn joint(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n == name)
    }

    /// Joint `joint`'s parent, none for a root.
    pub fn parent(&self, joint: usize) -> Option<usize> {
        self.parents[joint].map(usize::from)
    }

    /// The rest pose (the nodes' own transforms in the file).
    pub fn rest(&self) -> &Pose {
        &self.rest
    }

    /// The inverse bind matrices: from the mesh's frame to each joint's at bind time.
    pub fn inverse_bind(&self) -> &[Mat4] {
        &self.inverse_bind
    }

    /// Writes each joint's transform in the model's frame into `out`: its local transform
    /// from `pose` after its parent's.
    ///
    /// # Panics
    ///
    /// When `pose` or `out` does not have one entry per joint.
    pub fn model_space(&self, pose: &Pose, out: &mut [Mat4]) {
        assert!(pose.len() == self.len() && out.len() == self.len());
        for &j in &self.order {
            let j = usize::from(j);
            let local = self.frames[j] * pose.transform(j).matrix();
            out[j] = match self.parents[j] {
                Some(p) => out[usize::from(p)] * local,
                None => local,
            };
        }
    }

    /// Writes the matrices that take a vertex from its bind position to where joint `j` now
    /// carries it: `model[j]` times the inverse bind matrix.
    ///
    /// # Panics
    ///
    /// When `model` or `out` does not have one entry per joint.
    pub fn skinning_matrices(&self, model: &[Mat4], out: &mut [Mat4]) {
        assert!(model.len() == self.len() && out.len() == self.len());
        for ((out, model), inverse_bind) in out.iter_mut().zip(model).zip(&self.inverse_bind) {
            *out = *model * *inverse_bind;
        }
    }
}

#[cfg(test)]
mod tests {
    use glam::{Quat, Vec3};

    use super::*;

    /// A chain of three joints listed child first, each 1 m above its parent, with the bind
    /// at rest.
    fn chain() -> Skeleton {
        let up = Transform {
            translation: Vec3::Y,
            ..Transform::IDENTITY
        };
        let rest = [up, up, Transform::IDENTITY];
        let inverse_bind = [2.0, 1.0, 0.0]
            .map(|y| Mat4::from_translation(Vec3::new(0.0, -y, 0.0)))
            .to_vec();
        Skeleton::new(
            vec!["tip".into(), "mid".into(), "root".into()],
            vec![Some(1), Some(2), None],
            vec![Mat4::IDENTITY; 3],
            &rest,
            inverse_bind,
        )
        .unwrap()
    }

    #[test]
    fn the_rest_pose_skins_to_identity() {
        let skeleton = chain();
        let mut model = vec![Mat4::IDENTITY; 3];
        skeleton.model_space(skeleton.rest(), &mut model);
        assert!(
            model[0]
                .transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(0.0, 2.0, 0.0), 1e-6)
        );
        let mut skin = vec![Mat4::ZERO; 3];
        skeleton.skinning_matrices(&model, &mut skin);
        for m in skin {
            assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6), "{m}");
        }
    }

    #[test]
    fn a_turned_root_carries_its_children() {
        let skeleton = chain();
        let mut pose = skeleton.rest().clone();
        pose.rotations[2] = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let mut model = vec![Mat4::IDENTITY; 3];
        skeleton.model_space(&pose, &mut model);
        // A quarter turn about +z takes +y to −x.
        let tip = model[0].transform_point3(Vec3::ZERO);
        assert!(tip.abs_diff_eq(Vec3::new(-2.0, 0.0, 0.0), 1e-6), "{tip}");
    }

    #[test]
    fn a_loop_is_not_a_skeleton() {
        let error = Skeleton::new(
            vec!["a".into(), "b".into()],
            vec![Some(1), Some(0)],
            vec![Mat4::IDENTITY; 2],
            &[Transform::IDENTITY; 2],
            vec![Mat4::IDENTITY; 2],
        );
        assert!(matches!(error, Err(SkeletonError::BadParent(_))));
    }
}
