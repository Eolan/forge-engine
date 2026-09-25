//! The scene's acceleration structures for ray-traced sun shadows (issue #45, D-008's first
//! tier).
//!
//! One bottom-level structure per mesh, over a cut of its cluster DAG: the finest cut whose
//! triangles fit a budget (every cluster whose `self_error` is at most the cut's error and
//! whose `parent_error` is above it, which is one watertight surface). A coarse surface
//! stands for the full one, so shadow rays start a little off the surface
//! (`SHADOW_BIAS` in `meshlet.slang`). One top-level structure over every instance, its
//! records written from the scene's instance table by a compute pass
//! (`tlas_instances_main`), since the city places its instances on the GPU.

use std::collections::HashMap;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_geom::{GpuMeshlet, PAGE_SIZE};
use forge_gpu::{
    AccelerationStructure, BlasTriangles, Buffer, BufferDesc, ComputePipelineDesc, Device,
    MemoryCategory, MemoryLocation, Result, ShaderCompiler, ShaderStage, vk,
};

use crate::streaming::PageStore;

/// Most triangles of a mesh's bottom-level structure (the terrain may take more, see
/// [`TERRAIN_BUDGET`]).
pub const TRIANGLE_BUDGET: u32 = 40_000;
/// Budget of a mesh whose sphere is wider than a kilometre (terrain).
pub const TERRAIN_BUDGET: u32 = 600_000;

/// A mesh's DAG cut: the positions and triangle list of its clusters.
pub(crate) struct Cut {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// The cut's object-space error.
    pub error: f32,
}

/// Triangles of the cut of `meshlets` (one mesh's clusters) at error `e`.
fn cut_triangles(meshlets: &[GpuMeshlet], e: f32) -> u64 {
    meshlets
        .iter()
        .filter(|m| m.self_error <= e && e < m.parent_error)
        .map(|m| u64::from(m.triangle_count))
        .sum()
}

/// The finest cut of `meshlets` with at most `budget` triangles, read from `store`.
pub(crate) fn mesh_cut(meshlets: &[GpuMeshlet], store: &PageStore, budget: u32) -> Result<Cut> {
    let mut errors: Vec<f32> = meshlets.iter().map(|m| m.self_error).collect();
    errors.push(0.0);
    errors.sort_by(f32::total_cmp);
    errors.dedup();
    // Coarser cuts have fewer triangles: the first error whose cut fits (the roots at worst).
    let fit = errors.partition_point(|&e| cut_triangles(meshlets, e) > u64::from(budget));
    let error = errors[fit.min(errors.len() - 1)];
    let clusters: Vec<&GpuMeshlet> = meshlets
        .iter()
        .filter(|m| m.self_error <= error && error < m.parent_error)
        .collect();
    let mut pages: Vec<u32> = clusters.iter().map(|m| m.page).collect();
    pages.sort_unstable();
    pages.dedup();
    let bytes = store.read_pages(pages.iter().copied())?;
    let at: HashMap<u32, usize> = pages
        .iter()
        .enumerate()
        .map(|(i, &p)| (p, i * PAGE_SIZE))
        .collect();
    tracing::debug!(
        triangles = cut_triangles(meshlets, error),
        error,
        budget,
        "mesh cut for ray tracing"
    );
    let mut cut = Cut {
        positions: Vec::new(),
        indices: Vec::new(),
        error,
    };
    for m in clusters {
        let base = at[&m.page] + m.payload as usize;
        let first = cut.positions.len() as u32;
        for v in 0..m.vertex_count as usize {
            let p = base + v * 16;
            cut.positions.push([0, 1, 2].map(|k| {
                f32::from_le_bytes(bytes[p + 4 * k..p + 4 * k + 4].try_into().expect("4 bytes"))
            }));
        }
        let triangles = base + m.vertex_count as usize * 16;
        cut.indices.extend(
            bytes[triangles..triangles + m.triangle_count as usize * 3]
                .iter()
                .map(|&l| first + u32::from(l)),
        );
    }
    Ok(cut)
}

/// Mirrors `TlasPush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TlasPush {
    instances: u64,
    blas: u64,
    records: u64,
    count: u32,
    pad: u32,
}

/// The scene's structures.
pub struct SceneRays {
    blases: Vec<AccelerationStructure>,
    /// Per mesh, its bottom-level structure's address (read by the instance pass).
    blas_addresses: Buffer,
    tlas: Option<AccelerationStructure>,
    /// Triangles over the bottom-level structures.
    pub triangles: u64,
    /// The largest object-space error of the meshes' cuts: how far the traced surfaces may
    /// stand from the drawn ones.
    pub max_cut_error: f32,
    /// Milliseconds the bottom-level builds took (with the cuts' reads).
    pub blas_ms: f64,
    /// Milliseconds the top-level build took (with its instance pass), once built.
    pub tlas_ms: f64,
}

impl SceneRays {
    /// Builds one bottom-level structure per cut.
    pub(crate) fn new(device: &Arc<Device>, cuts: &[Cut], ms: f64) -> Result<Self> {
        let start = std::time::Instant::now();
        let meshes: Vec<BlasTriangles<'_>> = cuts
            .iter()
            .map(|c| BlasTriangles {
                positions: &c.positions,
                indices: &c.indices,
            })
            .collect();
        let blases = device.build_blases(&meshes, "mesh BLAS")?;
        let addresses: Vec<u64> = blases.iter().map(AccelerationStructure::address).collect();
        let blas_addresses = device.create_buffer_with_data(
            &addresses,
            vk::BufferUsageFlags::STORAGE_BUFFER,
            MemoryCategory::Geometry,
            "BLAS addresses",
        )?;
        Ok(Self {
            triangles: cuts.iter().map(|c| c.indices.len() as u64 / 3).sum(),
            max_cut_error: cuts.iter().map(|c| c.error).fold(0.0, f32::max),
            blases,
            blas_addresses,
            tlas: None,
            blas_ms: ms + start.elapsed().as_secs_f64() * 1e3,
            tlas_ms: 0.0,
        })
    }

    /// Builds the top-level structure over the `count` instances of `instances` (the
    /// scene's table, written): their records by a compute pass, then the build.
    pub(crate) fn build_tlas(
        &mut self,
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        instances: &Buffer,
        count: u32,
    ) -> Result<()> {
        let start = std::time::Instant::now();
        let module = device.create_shader_module(
            &shaders.compile("meshlet.slang", "tlas_instances_main", ShaderStage::Compute)?,
            "TLAS instances",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "tlas_instances_main"),
            push_constant_bytes: std::mem::size_of::<TlasPush>() as u32,
            name: "TLAS instances",
        });
        device.destroy_shader_module(module);
        let pipeline = pipeline?;
        let records = device.create_buffer(BufferDesc {
            size: u64::from(count.max(1)) * 64,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Transfer,
            name: "TLAS instance records",
        })?;
        let push = TlasPush {
            instances: instances.address(),
            blas: self.blas_addresses.address(),
            records: records.address(),
            count,
            pad: 0,
        };
        device.execute_compute_once(|commands| {
            commands.bind_pipeline(&pipeline);
            commands.push_constants(&pipeline, &push);
            commands.dispatch(count.div_ceil(64), 1, 1);
        })?;
        self.tlas = Some(device.build_tlas(records.address(), count, "scene TLAS")?);
        self.tlas_ms = start.elapsed().as_secs_f64() * 1e3;
        Ok(())
    }

    /// The top-level structure's address, 0 before it is built.
    pub fn tlas_address(&self) -> u64 {
        self.tlas.as_ref().map_or(0, AccelerationStructure::address)
    }

    /// Bytes of every structure.
    pub fn bytes(&self) -> u64 {
        self.blases
            .iter()
            .map(AccelerationStructure::size)
            .sum::<u64>()
            + self.tlas.as_ref().map_or(0, AccelerationStructure::size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cluster(self_error: f32, parent_error: f32, triangles: u32) -> GpuMeshlet {
        GpuMeshlet {
            self_error,
            parent_error,
            triangle_count: triangles,
            ..GpuMeshlet::default()
        }
    }

    #[test]
    fn a_cut_takes_the_clusters_whose_errors_straddle_it() {
        // Two leaves (error 0) under a parent (error 1) under a root (error 2).
        let dag = [
            cluster(0.0, 1.0, 100),
            cluster(0.0, 1.0, 100),
            cluster(1.0, 2.0, 60),
            cluster(2.0, f32::INFINITY, 20),
        ];
        assert_eq!(cut_triangles(&dag, 0.0), 200);
        assert_eq!(cut_triangles(&dag, 1.0), 60);
        assert_eq!(cut_triangles(&dag, 2.0), 20);
        assert_eq!(cut_triangles(&dag, 5.0), 20);
    }
}
