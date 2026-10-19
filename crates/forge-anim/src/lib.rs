//! `forge-anim` — D-012's clip layer, in-house (its amendment of 2026-10-04; issue #165).
//!
//! A [`Skeleton`] (joints, parents, rest pose, inverse bind matrices) and its [`Clip`]s (keys
//! of translation, rotation and scale per joint) come from a glTF file's skin and animations
//! ([`load_rig`]). A frame then goes:
//!
//! 1. [`Clip::sample`] writes a clip's [`Pose`] at a time: each joint's local transform, kept
//!    as three arrays (translations, rotations, scales);
//! 2. [`Pose::blend`] mixes two poses by a weight (a walk into a run, an idle into a walk);
//!    [`Pose::mix_masked`] on some joints only ([`Skeleton::mask_below`]: an upper-body action
//!    over walking), and [`Pose::add`] adds a layer's difference from its reference (a lean, a
//!    recoil; #313);
//! 3. [`Skeleton::model_space`] chains the local transforms from the roots down;
//! 4. [`Skeleton::skinning_matrices`] multiplies each by its inverse bind matrix: what the
//!    GPU's skinning moves the vertices by.
//!
//! Above the clips (#167): a [`Player`] runs a clip or a [`BlendSpace`] on (or once, holding its
//! end: [`Player::play_once`]), its switches
//! inertialized ([`Inertializer`]); a [`PackedClip`] holds a clip on as few bits a track as keep
//! the skeleton within a tolerance, a quarter to a third of its bytes. [`Clip::root_motion`] gives
//! a clip's travel and turn for a character controller, and [`lock_root`] holds the pose in place
//! while it applies them (#311). Above the player, a data-driven state machine ([`machine`],
//! #312): action states the game's variables and events move between, the animation node whose
//! conditions hold most specifically playing in each, and events at a node's start, end or a
//! share of it that set variables or tell the host.
//!
//! **Determinism** (D-016): nothing here calls a transcendental function. Keys are
//! interpolated linearly (normalised for rotations, as ozz does between dense keys), by steps
//! or by glTF's cubic splines, all with `+ − × ÷` and `sqrt`, which IEEE 754 rounds exactly;
//! so the same inputs give the same bits on every machine of one instruction set. Poses are
//! never sent over the network (D-012 replicates parameters and events), so this is what the
//! foot events and replays need.

#![forbid(unsafe_code)]

mod clip;
mod ik;
mod import;
pub mod machine;
mod pack;
mod play;
mod pose;
mod root;
mod skeleton;

pub use clip::{Clip, Interpolation, JointTracks, Track};
pub use ik::{Chain, FootDown, Footfall, look_at, two_bone, two_bone_toward};
pub use import::{AnimError, Rig, load_rig, load_rigs};
pub use machine::{Animator, Machine, MachineDef, MachineError, Vars};
pub use pack::PackedClip;
pub use play::{BlendSpace, Inertializer, Player, Source, decay};
pub use pose::{Pose, Transform};
pub use root::{RootMotion, lock_root, yaw_of};
pub use skeleton::{Skeleton, SkeletonError};
