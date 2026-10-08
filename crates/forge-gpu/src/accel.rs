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
use crate::graph::GraphBuffer;
use crate::memory::{Buffer, BufferDesc};
use crate::memory_report::MemoryCategory;

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
            size: sizes.build_scratch_size + self.scratch_alignment(),
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
                let scratch = b
                    .scratch
                    .address()
                    .next_multiple_of(self.scratch_alignment());
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
            let geometry = triangle_geometry(
                positions.address(),
                mesh.positions.len() as u32,
                indices.address(),
            );
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

/// The geometry of opaque triangles: `vertices` positions (three floats each, packed) at device
/// address `positions`, three `u32` indices a triangle at `indices`.
fn triangle_geometry(
    positions: u64,
    vertices: u32,
    indices: u64,
) -> vk::AccelerationStructureGeometryKHR<'static> {
    let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
        .vertex_format(vk::Format::R32G32B32_SFLOAT)
        .vertex_data(vk::DeviceOrHostAddressConstKHR {
            device_address: positions,
        })
        .vertex_stride(12)
        .max_vertex(vertices.saturating_sub(1))
        .index_type(vk::IndexType::UINT32)
        .index_data(vk::DeviceOrHostAddressConstKHR {
            device_address: indices,
        });
    vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
        .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
        .flags(vk::GeometryFlagsKHR::OPAQUE)
}

/// A bottom-level structure over triangles whose positions a pass rewrites every frame (a
/// skinned mesh, #165): built once from positions and indices already on the device, then
/// updated in place ([`crate::Commands::update_dynamic_blases`]) after each rewrite. An update
/// keeps the triangles and refits the boxes, which suits a body that bends without tearing; a
/// rebuild now and then fits the tree to the pose the body has come to (#169).
/// The positions and indices belong to the caller and must outlive it.
pub struct DynamicBlas {
    device: Arc<Device>,
    pub(crate) raw: vk::AccelerationStructureKHR,
    address: u64,
    storage: GraphBuffer,
    pub(crate) scratch: GraphBuffer,
    pub(crate) geometry: vk::AccelerationStructureGeometryKHR<'static>,
    pub(crate) triangles: u32,
}

impl DynamicBlas {
    /// The device address an instance record names.
    pub fn address(&self) -> u64 {
        self.address
    }

    /// The structure's storage: [`crate::BufferAccess::BuildWrite`] to its update,
    /// [`crate::BufferAccess::BuildInput`] to a top-level build over it.
    pub fn storage(&self) -> &GraphBuffer {
        &self.storage
    }

    /// The update's scratch ([`crate::BufferAccess::BuildWrite`]).
    pub fn scratch(&self) -> &GraphBuffer {
        &self.scratch
    }

    /// Bytes of the structure.
    pub fn size(&self) -> u64 {
        self.storage.size()
    }
}

impl Drop for DynamicBlas {
    fn drop(&mut self) {
        if let Some(loader) = self.device.acceleration_loader() {
            // SAFETY: the structure was created by this loader and the device is done with
            // it (resources are dropped after the device idles).
            unsafe { loader.destroy_acceleration_structure(self.raw, None) };
        }
    }
}

/// How a [`DynamicBlas`] is built: fast to trace (it is traced every frame and only refitted),
/// and updatable.
const DYNAMIC_BLAS_FLAGS: vk::BuildAccelerationStructureFlagsKHR =
    vk::BuildAccelerationStructureFlagsKHR::from_raw(
        vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE.as_raw()
            | vk::BuildAccelerationStructureFlagsKHR::ALLOW_UPDATE.as_raw(),
    );

impl Device {
    /// Creates and builds a [`DynamicBlas`] over `triangles` triangles: `vertices` positions
    /// (three packed floats each) at device address `positions`, three `u32` indices a
    /// triangle at `indices`, both written before this call (and their buffers created with
    /// `ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR`); waits for the build.
    pub fn create_dynamic_blas(
        self: &Arc<Self>,
        positions: u64,
        vertices: u32,
        indices: u64,
        triangles: u32,
        name: &str,
    ) -> Result<DynamicBlas> {
        let loader = self.acceleration()?;
        let geometry = triangle_geometry(positions, vertices, indices);
        let geometries = [geometry];
        let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .flags(DYNAMIC_BLAS_FLAGS)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        // SAFETY: a size query on valid build info.
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &[triangles],
                &mut sizes,
            )
        };
        let storage = self.create_buffer(BufferDesc {
            size: sizes.acceleration_structure_size,
            usage: vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Geometry,
            name,
        })?;
        let create = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(storage.raw())
            .size(sizes.acceleration_structure_size)
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL);
        // SAFETY: the buffer is live, large enough and has the storage usage.
        let raw = unsafe { loader.create_acceleration_structure(&create, None)? };
        let address_info =
            vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(raw);
        // SAFETY: `raw` is a live structure.
        let address = unsafe { loader.get_acceleration_structure_device_address(&address_info) };
        self.set_name(raw, name);
        let scratch = self.create_buffer(BufferDesc {
            size: sizes.build_scratch_size.max(sizes.update_scratch_size)
                + self.scratch_alignment(),
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: &format!("{name} scratch"),
        })?;
        let blas = DynamicBlas {
            device: Arc::clone(self),
            raw,
            address,
            storage: GraphBuffer::new(storage),
            scratch: GraphBuffer::new(scratch),
            geometry,
            triangles,
        };
        let (info, range) = dynamic_blas_build(self, &blas, false);
        let geometries = [blas.geometry];
        let info = info.geometries(&geometries);
        let before = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ)];
        let after = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
            .src_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR)];
        self.execute_transient(|device, cb| {
            // SAFETY: recorded into the transient command buffer; the structure, its scratch and
            // the caller's positions and indices outlive the call (it waits for the fence).
            unsafe {
                device.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().memory_barriers(&before),
                );
                loader.cmd_build_acceleration_structures(cb, &[info], &[&[range]]);
                device.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().memory_barriers(&after),
                );
            }
        })?;
        Ok(blas)
    }
}

/// A [`DynamicBlas`]'s build (`update`: in place of its last one) without its geometries,
/// which the caller attaches, and its range.
pub(crate) fn dynamic_blas_build<'a>(
    device: &Device,
    blas: &DynamicBlas,
    update: bool,
) -> (
    vk::AccelerationStructureBuildGeometryInfoKHR<'a>,
    vk::AccelerationStructureBuildRangeInfoKHR,
) {
    let scratch = blas
        .scratch
        .address()
        .next_multiple_of(device.scratch_alignment());
    let (mode, source) = if update {
        (vk::BuildAccelerationStructureModeKHR::UPDATE, blas.raw)
    } else {
        (
            vk::BuildAccelerationStructureModeKHR::BUILD,
            vk::AccelerationStructureKHR::null(),
        )
    };
    (
        vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
            .flags(DYNAMIC_BLAS_FLAGS)
            .mode(mode)
            .src_acceleration_structure(source)
            .dst_acceleration_structure(blas.raw)
            .scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: scratch,
            }),
        vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(blas.triangles),
    )
}

/// A top-level structure rebuilt every frame over instance records a pass writes (the movers',
/// issue #79): its records, its storage and its scratch, sized for `capacity` instances, each
/// a buffer the render graph tracks. Its build ([`crate::Commands::build_dynamic_tlas`]) is
/// fast to build rather than fast to trace, the structure being small and new every frame.
pub struct DynamicTlas {
    device: Arc<Device>,
    pub(crate) raw: vk::AccelerationStructureKHR,
    address: u64,
    records: GraphBuffer,
    storage: GraphBuffer,
    pub(crate) scratch: GraphBuffer,
    capacity: u32,
    /// `FORGE_TLAS_REFIT=1` (#79's measure): built once, then updated in place.
    pub(crate) refit: bool,
    /// Its build flags ([`dynamic_tlas_flags`]).
    pub(crate) flags: vk::BuildAccelerationStructureFlagsKHR,
    /// The instance count of the last full build, which an update must keep.
    pub(crate) built: std::cell::Cell<Option<u32>>,
}

impl DynamicTlas {
    /// The device address shaders trace against.
    pub fn address(&self) -> u64 {
        self.address
    }

    /// The instance records (`VkAccelerationStructureInstanceKHR`, 64 bytes each), which a pass
    /// writes before the build ([`crate::BufferAccess::BuildInput`] to the build).
    pub fn records(&self) -> &GraphBuffer {
        &self.records
    }

    /// The structure's storage: [`crate::BufferAccess::BuildWrite`] to the build,
    /// [`crate::BufferAccess::AccelerationStructureRead`] to the passes tracing it.
    pub fn storage(&self) -> &GraphBuffer {
        &self.storage
    }

    /// The build's scratch ([`crate::BufferAccess::BuildWrite`]).
    pub fn scratch(&self) -> &GraphBuffer {
        &self.scratch
    }

    /// The most instances a build takes.
    pub fn capacity(&self) -> u32 {
        self.capacity
    }
}

impl Drop for DynamicTlas {
    fn drop(&mut self) {
        if let Some(loader) = self.device.acceleration_loader() {
            // SAFETY: the structure was created by this loader and the device is done with
            // it (resources are dropped after the device idles).
            unsafe { loader.destroy_acceleration_structure(self.raw, None) };
        }
    }
}

/// How a dynamic top-level structure is built and sized: fast to build, or with `fast_trace`
/// fast to trace (`FORGE_TLAS_FAST_TRACE=1`, the A/B NVIDIA's advice for a structure rebuilt
/// every frame asks for); with `refit` updatable in place (#79's measure of a refit against a
/// rebuild, `FORGE_TLAS_REFIT=1`).
pub(crate) fn dynamic_tlas_flags(
    refit: bool,
    fast_trace: bool,
) -> vk::BuildAccelerationStructureFlagsKHR {
    use vk::BuildAccelerationStructureFlagsKHR as F;
    let update = if refit { F::ALLOW_UPDATE } else { F::empty() };
    let prefer = if fast_trace {
        F::PREFER_FAST_TRACE
    } else {
        F::PREFER_FAST_BUILD
    };
    prefer | update
}

/// The geometry of a top-level build over the instance records at `records`.
fn instance_geometry(records: u64) -> vk::AccelerationStructureGeometryKHR<'static> {
    let data = vk::AccelerationStructureGeometryInstancesDataKHR::default()
        .array_of_pointers(false)
        .data(vk::DeviceOrHostAddressConstKHR {
            device_address: records,
        });
    vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::INSTANCES)
        .geometry(vk::AccelerationStructureGeometryDataKHR { instances: data })
        .flags(vk::GeometryFlagsKHR::OPAQUE)
}

impl Device {
    /// Creates a top-level structure for builds every frame over at most `capacity` instance
    /// records ([`DynamicTlas`]).
    pub fn create_dynamic_tlas(self: &Arc<Self>, capacity: u32, name: &str) -> Result<DynamicTlas> {
        let loader = self.acceleration()?;
        let capacity = capacity.max(1);
        let records = self.create_buffer(BufferDesc {
            size: u64::from(capacity) * 64,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: &format!("{name} records"),
        })?;
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v != "0");
        let refit = flag("FORGE_TLAS_REFIT");
        let flags = dynamic_tlas_flags(refit, flag("FORGE_TLAS_FAST_TRACE"));
        let geometries = [instance_geometry(records.address())];
        let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
            .flags(flags)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&geometries);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        // SAFETY: a size query on valid build info.
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &[capacity],
                &mut sizes,
            )
        };
        let storage = self.create_buffer(BufferDesc {
            size: sizes.acceleration_structure_size,
            usage: vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name,
        })?;
        let create = vk::AccelerationStructureCreateInfoKHR::default()
            .buffer(storage.raw())
            .size(sizes.acceleration_structure_size)
            .ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL);
        // SAFETY: the buffer is live, large enough and has the storage usage.
        let raw = unsafe { loader.create_acceleration_structure(&create, None)? };
        let address_info =
            vk::AccelerationStructureDeviceAddressInfoKHR::default().acceleration_structure(raw);
        // SAFETY: `raw` is a live structure.
        let address = unsafe { loader.get_acceleration_structure_device_address(&address_info) };
        self.set_name(raw, name);
        let scratch = self.create_buffer(BufferDesc {
            size: sizes.build_scratch_size.max(sizes.update_scratch_size)
                + self.scratch_alignment(),
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: &format!("{name} scratch"),
        })?;
        Ok(DynamicTlas {
            device: Arc::clone(self),
            raw,
            address,
            records: GraphBuffer::new(records),
            storage: GraphBuffer::new(storage),
            scratch: GraphBuffer::new(scratch),
            capacity,
            refit,
            flags,
            built: std::cell::Cell::new(None),
        })
    }
}

/// The build of a [`DynamicTlas`] over its first `count` records, as
/// [`crate::Commands::build_dynamic_tlas`] records it.
pub(crate) fn dynamic_tlas_build(
    device: &Device,
    tlas: &DynamicTlas,
    count: u32,
) -> (
    vk::AccelerationStructureGeometryKHR<'static>,
    u64,
    vk::AccelerationStructureBuildRangeInfoKHR,
) {
    let scratch = tlas
        .scratch
        .address()
        .next_multiple_of(device.scratch_alignment());
    (
        instance_geometry(tlas.records.address()),
        scratch,
        vk::AccelerationStructureBuildRangeInfoKHR::default()
            .primitive_count(count.min(tlas.capacity)),
    )
}
