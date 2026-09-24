//! Geometry processing for the GPU-driven pipeline: meshlets, the cluster LOD DAG and
//! procedural test meshes.

#![forbid(unsafe_code)]

pub mod cache;
pub mod city;
pub mod lod;
pub mod meshlet;
pub mod page;
pub mod procedural;

pub use lod::{ClusterDag, GROUP_SIZE, MAX_LEVELS, build_dag};
pub use meshlet::{
    CookOptions, DagStats, GpuMeshlet, GpuVertex, MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES,
    MeshletMesh,
};
pub use page::{PAGE_NONE, PAGE_SIZE, PagedVertex};
pub use procedural::TriMesh;
