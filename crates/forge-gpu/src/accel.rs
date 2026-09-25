//! Acceleration structures for ray queries (issue #45): bottom-level structures over triangle
//! lists and a top-level one over instance records, built once in one-shot submissions (a
//! static scene; rebuilding and refitting come with moving geometry). Shaders reach a
//! structure by its device address (`RaytracingAccelerationStructure(address)` in Slang), so
//! no descriptor is involved.

use std::sync::Arc;

use ash::vk;
use gpu_allocator::MemoryLocation;

use crate::device::Device;
use crate::error::{GpuError, Result};
use crate::memory::{Buffer, BufferDesc};
use crate::memory_report::MemoryCategory;

/// Alignment of a build's scratch address (`minAccelerationStructureScratchOffsetAlignment`
/// is at most 256 on the devices that matter).
const SCRATCH_ALIGNMENT: u64 = 256;

/// An acceleration structure and the buffer that holds it. Destroyed on drop (after the
/// device is idle, like every resource).
pub struct AccelerationStructure {
    device: Arc<Device>,
    raw: vk::AccelerationStructureKHR,
    /// Keeps the structure's memory alive; dropped after `raw` is destroyed.
    buffer: Buffer,
    address: u64,
}

impl AccelerationStructure {
    /// The device address shaders trace against.
    pub fn address(&self) -> u64 {
        self.address
    }

    /// Bytes of the structure.
    pub fn size(&self) -> u64 {
        self.buffer.size()
    }
}

impl Drop for AccelerationStructure {
    fn drop(&mut self) {
        if let Some(loader) = self.device.acceleration_loader() {
            // SAFETY: the structure was created by this loader and the device is done with
            // it (resources are dropped after the device idles).
            unsafe { loader.destroy_acceleration_structure(self.raw, None) };
        }
    }
}

/// A bottom-level structure's triangles: positions and a triangle list into them.
#[derive(Clone, Copy, Debug)]
pub struct BlasTriangles<'a> {
    /// Positions in the structure's space.
    pub positions: &'a [[f32; 3]],
    /// Three indices per triangle.
    pub indices: &'a [u32],
}

/// One build's parts: its geometry description, the structure it builds into and the
/// scratch memory it needs.
struct Build {
    geometry: vk::AccelerationStructureGeometryKHR<'static>,
    ty: vk::AccelerationStructureTypeKHR,
    primitives: u32,
    structure: AccelerationStructure,
    scratch: Buffer,
}

impl Device {
    fn acceleration(&self) -> Result<&ash::khr::acceleration_structure::Device> {
        self.acceleration_loader()
            .ok_or_else(|| GpuError::Unsupported("acceleration structures (no ray queries)".into()))
    }

    /// Sizes a build of `geometry`, creates its structure and scratch.
    fn prepare_build(
        self: &Arc<Self>,
        geometry: vk::AccelerationStructureGeometryKHR<'static>,
        ty: vk::AccelerationStructureTypeKHR,
        primitives: u32,
        name: &str,
    ) -> Result<Build> {
        let loader = self.acceleration()?;
        let geometries = [geometry];
        let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(ty)
            .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        // SAFETY: a size query on valid build info.
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &[primitives],
                &mut sizes,
            )
        };
        let buffer = self.create_buffer(BufferDesc {
            size: sizes.acceleration_structure_size,
            usage: vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Geometry,
            name,
        })?;
        let create = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(buffer.raw())
            .size(sizes.acceleration_structure_size)
            .ty(ty);
        // SAFETY: the buffer is live, large enough and has the storage usage.
        let raw = unsafe { loader.create_acceleration_structure(&create, None)? };
        let address_info =
            vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(raw);
        // SAFETY: `raw` is a live structure.
        let address = unsafe { loader.get_acceleration_structure_device_address(&address_info) };
        self.set_name(raw, name);
        let scratch = self.create_buffer(BufferDesc {
            size: sizes.build_scratch_size + SCRATCH_ALIGNMENT,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Transfer,
            name: "acceleration structure scratch",
        })?;
        Ok(Build {
            geometry,
            ty,
            primitives,
            structure: AccelerationStructure {
                device: Arc::clone(self),
                raw,
                buffer,
                address,
            },
            scratch,
        })
    }

    /// Records every build of `builds` into one submission and waits for it.
    fn run_builds(&self, builds: &[Build]) -> Result<()> {
        let loader = self.acceleration()?;
        let geometries: Vec<[vk::AccelerationStructureGeometryKHR<'static>; 1]> =
            builds.iter().map(|b| [b.geometry]).collect();
        let infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR<'_>> = builds
            .iter()
            .zip(&geometries)
            .map(|(b, g)| {
                let scratch = b.scratch.address().next_multiple_of(SCRATCH_ALIGNMENT);
                vk::AccelerationStructureBuildGeometryInfoKHR::default()
                    .ty(b.ty)
                    .flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .dst_acceleration_structure(b.structure.raw)
                    .geometries(g)
                    .scratch_data(vk::DeviceOrHostAddressKHR {
                        device_address: scratch,
                    })
            })
            .collect();
        let ranges: Vec<[vk::AccelerationStructureBuildRangeInfoKHR; 1]> = builds
            .iter()
            .map(|b| {
                [vk::AccelerationStructureBuildRangeInfoKHR::default()
                    .primitive_count(b.primitives)]
            })
            .collect();
        let range_refs: Vec<&[vk::AccelerationStructureBuildRangeInfoKHR]> =
            ranges.iter().map(|r| r.as_slice()).collect();
        // Earlier writes (uploads, the instance pass) before the builds read them.
        let before = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
            .dst_access_mask(
                vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags2::SHADER_READ,
            )];
        // The built structures before anything traces or builds against them.
        let after = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
            .src_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR)];
        self.execute_transient(|raw, cb| {
            // SAFETY: recorded into the transient command buffer; every buffer the builds
            // read or write outlives the call (`execute_transient` waits for the fence).
            unsafe {
                raw.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().memory_barriers(&before),
                );
                loader.cmd_build_acceleration_structures(cb, &infos, &range_refs);
                raw.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().memory_barriers(&after),
                );
            }
        })
    }

    /// Builds a bottom-level structure over each of `meshes` (opaque triangles), in one
    /// submission; waits for it.
    pub fn build_blases(
        self: &Arc<Self>,
        meshes: &[BlasTriangles<'_>],
        name: &str,
    ) -> Result<Vec<AccelerationStructure>> {
        let input = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        let mut inputs = Vec::with_capacity(meshes.len());
        let mut builds = Vec::with_capacity(meshes.len());
        for mesh in meshes {
            let positions = self.create_buffer_with_data(
                mesh.positions,
                input,
                MemoryCategory::Transfer,
                name,
            )?;
            let indices =
                self.create_buffer_with_data(mesh.indices, input, MemoryCategory::Transfer, name)?;
            let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
                .vertex_format(vk::Format::R32G32B32_SFLOAT)
                .vertex_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: positions.address(),
                })
                .vertex_stride(12)
                .max_vertex(mesh.positions.len().saturating_sub(1) as u32)
                .index_type(vk::IndexType::UINT32)
                .index_data(vk::DeviceOrHostAddressConstKHR {
                    device_address: indices.address(),
                });
            let geometry = vk::AccelerationStructureGeometryKHR::default()
                .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
                .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
                .flags(vk::GeometryFlagsKHR::OPAQUE);
            builds.push(self.prepare_build(
                geometry,
                vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
                (mesh.indices.len() / 3) as u32,
                name,
            )?);
            inputs.push((positions, indices));
        }
        self.run_builds(&builds)?;
        drop(inputs);
        Ok(builds.into_iter().map(|b| b.structure).collect())
    }

    /// Builds a top-level structure over `count` instance records
    /// (`VkAccelerationStructureInstanceKHR`, 64 bytes each) at device address `instances`,
    /// written before this call; waits for it.
    pub fn build_tlas(
        self: &Arc<Self>,
        instances: u64,
        count: u32,
        name: &str,
    ) -> Result<AccelerationStructure> {
        let data = vk::AccelerationStructureGeometryInstancesDataKHR::default()
            .array_of_pointers(false)
            .data(vk::DeviceOrHostAddressConstKHR {
                device_address: instances,
            });
        let geometry = vk::AccelerationStructureGeometryKHR::default()
            .geometry_type(vk::GeometryTypeKHR::INSTANCES)
            .geometry(vk::AccelerationStructureGeometryDataKHR { instances: data })
            .flags(vk::GeometryFlagsKHR::OPAQUE);
        let build = self.prepare_build(
            geometry,
            vk::AccelerationStructureTypeKHR::TOP_LEVEL,
            count,
            name,
        )?;
        self.run_builds(std::slice::from_ref(&build))?;
        Ok(build.structure)
    }
}
