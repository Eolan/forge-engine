//! Positions as an integer cell and an `f32` offset inside it (issue #93, D-004's amendment):
//! what the GPU instance table and the frame block store. The type lives in `forge_world`
//! ([`forge_world::cells`]), where the frame tree produces it from `f64` positions; the renderer
//! re-exports it, and its shaders mirror `CELL_SIZE`.
//!
//! The camera-relative position of an instance is its cell difference to the camera's, taken
//! in integers (exact), times the cell size (a power of two, so the product is exact), plus the
//! difference of the two offsets: exact near the camera wherever the scene stands in the world,
//! so the table is never rewritten when the camera moves and holds no `f64`.
//!
//! The renderer also keeps a **scene frame**: the frame of the top-level acceleration structure,
//! the probes and the dust's noise, anchored at the scene's origin
//! ([`crate::MeshletScene::origin`]). Rays start from `camera-relative position +
//! Frame::camera_in_scene`, which the renderer computes each frame with [`CellPos::relative_to`].

pub use forge_world::cells::{CELL_SIZE, CellPos};
