//! A skeleton's local transforms, one per joint, and blending them.

use glam::{Mat4, Quat, Vec3};

/// A joint's transform relative to its parent: scale, then rotation, then translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Where it sits in its parent's frame.
    pub translation: Vec3,
    /// A unit quaternion.
    pub rotation: Quat,
    /// Per axis.
    pub scale: Vec3,
}

impl Transform {
    /// No move, no turn, scale 1.
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// As a matrix.
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Every joint's local transform, as three arrays indexed by joint (the skin's order).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Pose {
    /// Translations.
    pub translations: Vec<Vec3>,
    /// Unit quaternions.
    pub rotations: Vec<Quat>,
    /// Scales.
    pub scales: Vec<Vec3>,
}

impl Pose {
    /// `transforms` as a pose.
    pub fn from_transforms(transforms: &[Transform]) -> Self {
        Self {
            translations: transforms.iter().map(|t| t.translation).collect(),
            rotations: transforms.iter().map(|t| t.rotation).collect(),
            scales: transforms.iter().map(|t| t.scale).collect(),
        }
    }

    /// The number of joints.
    pub fn len(&self) -> usize {
        self.translations.len()
    }

    /// No joints.
    pub fn is_empty(&self) -> bool {
        self.translations.is_empty()
    }

    /// Joint `joint`'s transform.
    pub fn transform(&self, joint: usize) -> Transform {
        Transform {
            translation: self.translations[joint],
            rotation: self.rotations[joint],
            scale: self.scales[joint],
        }
    }

    /// Mixes `other` into this pose by `weight`: [`Pose::blend`] of this pose and `other`, in
    /// place.
    ///
    /// # Panics
    ///
    /// When the two poses do not have the same number of joints.
    pub fn mix(&mut self, other: &Pose, weight: f32) {
        assert!(self.len() == other.len());
        for j in 0..self.len() {
            self.translations[j] = lerp3(self.translations[j], other.translations[j], weight);
            self.rotations[j] = nlerp(self.rotations[j], other.rotations[j], weight);
            self.scales[j] = lerp3(self.scales[j], other.scales[j], weight);
        }
    }

    /// Mixes `other` into this pose joint by joint (#313): joint `j` by `weight · mask[j]`, so a
    /// clip plays over another on some joints only (an upper-body action over walking, the mask
    /// from [`crate::Skeleton::mask_below`]). A mask of ones is [`Pose::mix`].
    ///
    /// # Panics
    ///
    /// When the poses and the mask do not have the same number of joints.
    pub fn mix_masked(&mut self, other: &Pose, weight: f32, mask: &[f32]) {
        assert!(self.len() == other.len() && mask.len() == self.len());
        for (j, &m) in mask.iter().enumerate() {
            let w = weight * m;
            if w == 0.0 {
                continue;
            }
            self.translations[j] = lerp3(self.translations[j], other.translations[j], w);
            self.rotations[j] = nlerp(self.rotations[j], other.rotations[j], w);
            self.scales[j] = lerp3(self.scales[j], other.scales[j], w);
        }
    }

    /// Adds `additive`'s difference from `reference` to this pose by `weight` (#313): a lean, a
    /// recoil or a breath over whatever plays. Per joint, the translation moves by `weight` of
    /// `additive − reference`; the rotation turns by `weight` of `reference⁻¹ · additive` in its
    /// own frame (local space, as engines' additive layers do; the short way, normalised); the
    /// scale multiplies by `weight` of `additive / reference`. Added at weight 0, or with
    /// `additive` equal to `reference`, it leaves the pose as it was.
    ///
    /// # Panics
    ///
    /// When the three poses do not have the same number of joints.
    pub fn add(&mut self, additive: &Pose, reference: &Pose, weight: f32) {
        assert!(self.len() == additive.len() && self.len() == reference.len());
        for j in 0..self.len() {
            let moved = additive.translations[j] - reference.translations[j];
            self.translations[j] += moved * weight;
            let turn = reference.rotations[j].inverse() * additive.rotations[j];
            let turn = nlerp(Quat::IDENTITY, turn, weight);
            self.rotations[j] = (self.rotations[j] * turn).normalize();
            let grown = additive.scales[j] / reference.scales[j];
            self.scales[j] *= lerp3(Vec3::ONE, grown, weight);
        }
    }

    /// Writes `a` mixed with `b` into `out`: `a` at weight 0, `b` at 1. Translations and scales
    /// are lerped, rotations taken the short way round and normalised; a pose mixed with
    /// itself is that pose.
    ///
    /// # Panics
    ///
    /// When the three poses do not have the same number of joints.
    pub fn blend(a: &Pose, b: &Pose, weight: f32, out: &mut Pose) {
        assert!(a.len() == b.len() && a.len() == out.len());
        for j in 0..a.len() {
            out.translations[j] = lerp3(a.translations[j], b.translations[j], weight);
            out.rotations[j] = nlerp(a.rotations[j], b.rotations[j], weight);
            out.scales[j] = lerp3(a.scales[j], b.scales[j], weight);
        }
    }
}

/// `a + (b − a)·s`: exactly `a` at 0, and exactly `a` whatever `s` when `b` is `a`.
pub(crate) fn lerp3(a: Vec3, b: Vec3, s: f32) -> Vec3 {
    a + (b - a) * s
}

/// The normalised lerp between two unit quaternions, the short way round.
pub(crate) fn nlerp(a: Quat, b: Quat, s: f32) -> Quat {
    let b = if a.dot(b) < 0.0 { -b } else { b };
    (a + (b - a) * s).normalize()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pose(angle: f32, x: f32) -> Pose {
        Pose::from_transforms(&[
            Transform {
                translation: Vec3::new(x, 1.0, -0.3),
                rotation: Quat::from_rotation_z(angle),
                scale: Vec3::ONE,
            },
            Transform {
                translation: Vec3::new(0.1, 0.7, 0.2),
                rotation: Quat::from_rotation_x(-angle) * Quat::from_rotation_y(0.4),
                scale: Vec3::splat(1.2),
            },
        ])
    }

    #[test]
    fn a_pose_blended_with_itself_is_that_pose() {
        let a = pose(0.8, 0.25);
        let mut out = a.clone();
        for w in [0.0, 0.3, 0.5, 0.71, 1.0] {
            Pose::blend(&a, &a, w, &mut out);
            assert_eq!(out.translations, a.translations, "weight {w}");
            assert_eq!(out.scales, a.scales, "weight {w}");
            for (q, r) in out.rotations.iter().zip(&a.rotations) {
                assert!(q.abs_diff_eq(*r, 1e-7), "weight {w}: {q} against {r}");
            }
        }
    }

    #[test]
    fn a_blend_goes_from_one_pose_to_the_other() {
        let (a, b) = (pose(0.0, 0.0), pose(1.0, 2.0));
        let mut out = a.clone();
        Pose::blend(&a, &b, 0.0, &mut out);
        assert_eq!(out.translations, a.translations);
        Pose::blend(&a, &b, 1.0, &mut out);
        assert_eq!(out.translations, b.translations);
        for (q, r) in out.rotations.iter().zip(&b.rotations) {
            assert!(q.abs_diff_eq(*r, 1e-6));
        }
        // Half way: the translation half way, the turn about half (nlerp is not uniform in
        // angle, but symmetric about the middle).
        Pose::blend(&a, &b, 0.5, &mut out);
        assert!((out.translations[0].x - 1.0).abs() < 1e-6);
        let (axis, angle) = out.rotations[0].to_axis_angle();
        assert!(axis.abs_diff_eq(Vec3::Z, 1e-5) && (angle - 0.5).abs() < 1e-5);
    }

    #[test]
    fn a_blend_takes_the_short_way_round() {
        let a = Pose::from_transforms(&[Transform {
            rotation: Quat::from_rotation_y(0.1),
            ..Transform::IDENTITY
        }]);
        // The same turn written with the opposite sign.
        let b = Pose::from_transforms(&[Transform {
            rotation: -Quat::from_rotation_y(0.3),
            ..Transform::IDENTITY
        }]);
        let mut out = a.clone();
        Pose::blend(&a, &b, 0.5, &mut out);
        let (_, angle) = out.rotations[0].to_axis_angle();
        assert!((angle - 0.2).abs() < 1e-4, "{angle}");
    }

    #[test]
    fn a_masked_mix_moves_only_the_masked_joints() {
        let (a, b) = (pose(0.2, 0.0), pose(0.9, 1.0));
        let mut out = a.clone();
        out.mix_masked(&b, 1.0, &[0.0, 1.0]);
        assert_eq!(out.translations[0], a.translations[0]);
        assert_eq!(out.rotations[0], a.rotations[0]);
        assert!(out.rotations[1].abs_diff_eq(b.rotations[1], 1e-6));
        // A mask of ones is a plain mix.
        let (mut masked, mut plain) = (a.clone(), a.clone());
        masked.mix_masked(&b, 0.4, &[1.0, 1.0]);
        plain.mix(&b, 0.4);
        assert_eq!(masked, plain);
    }

    #[test]
    fn an_additive_layer_adds_its_difference_from_its_reference() {
        let base = pose(0.3, 0.5);
        let reference = pose(0.0, 0.0);
        let lean = pose(0.25, 0.1);
        // Its own reference adds nothing, and nothing is added at weight 0.
        let mut out = base.clone();
        out.add(&reference, &reference, 1.0);
        for (q, r) in out.rotations.iter().zip(&base.rotations) {
            assert!(q.abs_diff_eq(*r, 1e-6));
        }
        let mut out = base.clone();
        out.add(&lean, &reference, 0.0);
        assert_eq!(out.translations, base.translations);
        // Whole: the first joint turned 0.25 more about z, moved 0.1 along x.
        let mut out = base.clone();
        out.add(&lean, &reference, 1.0);
        assert!(out.rotations[0].abs_diff_eq(Quat::from_rotation_z(0.55), 1e-5));
        assert!((out.translations[0].x - 0.6).abs() < 1e-6);
    }
}
