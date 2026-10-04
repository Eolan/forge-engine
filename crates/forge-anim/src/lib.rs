//! `forge-anim` — D-012's clip layer, in-house (its amendment of 2026-10-04; issue #165).
//!
//! A [`Skeleton`] (joints, parents, rest pose, inverse bind matrices) and its [`Clip`]s (keys
//! of translation, rotation and scale per joint) come from a glTF file's skin and animations
//! ([`load_rig`]). A frame then goes:
//!
//! 1. [`Clip::sample`] writes a clip's [`Pose`] at a time: each joint's local transform, kept
//!    as three arrays (translations, rotations, scales);
//! 2. [`Pose::blend`] mixes two poses by a weight (a walk into a run, an idle into a walk);
//! 3. [`Skeleton::model_space`] chains the local transforms from the roots down;
//! 4. [`Skeleton::skinning_matrices`] multiplies each by its inverse bind matrix: what the
//!    GPU's skinning moves the vertices by.
//!
//! **Determinism** (D-016): nothing here calls a transcendental function. Keys are
//! interpolated linearly (normalised for rotations, as ozz does between dense keys), by steps
//! or by glTF's cubic splines, all with `+ − × ÷` and `sqrt`, which IEEE 754 rounds exactly;
//! so the same inputs give the same bits on every machine of one instruction set. Poses are
//! never sent over the network (D-012 replicates parameters and events), so this is what the
//! foot events and replays need.

#![forbid(unsafe_code)]

mod clip;
mod import;
mod pose;
mod skeleton;

pub use clip::{Clip, Interpolation, JointTracks, Track};
pub use import::{AnimError, Rig, load_rig};
pub use pose::{Pose, Transform};
pub use skeleton::{Skeleton, SkeletonError};
