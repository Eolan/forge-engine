//! Geometry processing for the GPU-driven pipeline: meshlets, the cluster LOD DAG and
//! procedural test meshes.

#![forbid(unsafe_code)]

pub mod cache;
pub mod city;
pub mod fracture;
pub mod lod;
pub mod meshlet;
pub mod model;
pub mod page;
pub mod procedural;
pub mod skin;
pub mod stone;

pub use lod::{ClusterDag, GROUP_SIZE, MAX_LEVELS, build_dag, build_dag_clustered};
pub use meshlet::{
    CookOptions, DagStats, GpuMeshlet, GpuVertex, MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES,
    MeshletMesh, PackedPage, PageFile,
};
pub use page::{PAGE_NONE, PAGE_SIZE, PagedVertex};
pub use procedural::TriMesh;
pub use skin::{MorphDelta, MorphTarget, Morphs, SkinMore, SkinVertex, SkinnedMesh, VertexSkin};
