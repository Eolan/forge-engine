//! Geometry processing for the GPU-driven pipeline: meshlets and procedural test meshes.

#![forbid(unsafe_code)]

pub mod meshlet;
pub mod procedural;

pub use meshlet::{
    GpuMeshlet, GpuVertex, MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES, MeshletMesh,
};
pub use procedural::TriMesh;
