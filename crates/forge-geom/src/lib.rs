//! Geometry processing for the GPU-driven pipeline: meshlets, the cluster LOD DAG and
//! procedural test meshes.

#![forbid(unsafe_code)]

pub mod cache;
pub mod check;
pub mod city;
pub mod depot;
pub mod figures;
pub mod flora;
pub mod fracture;
pub mod furniture;
pub mod landmarks;
pub mod lod;
pub mod meshlet;
pub mod model;
pub mod modules;
pub mod page;
pub mod procedural;
pub mod skin;
pub mod stone;
pub mod street;
pub mod vehicles;

pub use lod::{ClusterDag, GROUP_SIZE, MAX_LEVELS, build_dag, build_dag_clustered};
pub use meshlet::{
    CookOptions, DagStats, GpuMeshlet, GpuVertex, MESHLET_MAX_TRIANGLES, MESHLET_MAX_VERTICES,
    MeshletMesh, PackedPage, PageFile,
};
pub use page::{PAGE_NONE, PAGE_SIZE, PagedVertex};
pub use procedural::TriMesh;
pub use skin::{MorphDelta, MorphTarget, Morphs, SkinMore, SkinVertex, SkinnedMesh, VertexSkin};
