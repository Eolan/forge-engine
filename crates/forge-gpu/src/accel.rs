//! Acceleration structures for ray queries (issue #45): bottom-level structures over triangle
//! lists and a top-level one over instance records, built once in one-shot submissions (a
//! static scene; rebuilding and refitting come with moving geometry). Shaders reach a
//! structure by its device address (`RaytracingAccelerationStructure(address)` in Slang), so
//! no descriptor is involved.

use std::sync::Arc;

use ash::vk;
use gpu_allocator::MemoryLocation;

use crate::device::{Device, QueueKind};
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
    /// The opacity micromap its cut-out geometry indexes (#333), kept as long as it is.
    _micromap: Option<Arc<OpacityMicromap>>,
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
    /// How many of the first triangles are opaque; the others make a second geometry, not
    /// opaque, whose triangles a ray's query loop sees as candidates (#307). A ray meets the
    /// second's triangle `p` as the list's `opaque + p`. All of them: one opaque geometry.
    pub opaque: u32,
    /// The not-opaque triangles' opacity at a finer grain (#333): a micromap and, per triangle
    /// after the first `opaque`, its map there or a special index ([`opacity_index`]).
    pub opacity: Option<BlasOpacity<'a>>,
}

/// A bottom-level structure's not-opaque triangles' opacity micromap ([`BlasTriangles::opacity`]).
#[derive(Clone, Copy, Debug)]
pub struct BlasOpacity<'a> {
    /// The micromap.
    pub micromap: &'a Arc<OpacityMicromap>,
    /// Per not-opaque triangle, in order: its triangle in the micromap, or a special index.
    pub indices: &'a [i32],
}

/// The special indices of `VK_EXT_opacity_micromap`: a triangle wholly in one state, without a
/// map of its own.
pub mod opacity_index {
    /// Every ray passes it.
    pub const TRANSPARENT: i32 = -1;
    /// Every ray meets it, without the query loop's test.
    pub const OPAQUE: i32 = -2;
    /// The query loop decides, mostly clear.
    pub const UNKNOWN_TRANSPARENT: i32 = -3;
    /// The query loop decides, mostly solid.
    pub const UNKNOWN_OPAQUE: i32 = -4;
}

/// An opacity micromap (`VK_EXT_opacity_micromap`, #333): for each of its triangles, the state
/// of each of its micro-triangles, which the not-opaque geometries of bottom-level structures
/// index ([`BlasOpacity`]): a ray passes a transparent one and meets an opaque one without the
/// query loop's test; the unknown ones reach the loop. Destroyed on drop, after the structures
/// built over it, which keep it.
pub struct OpacityMicromap {
    device: Arc<Device>,
    raw: vk::MicromapEXT,
    /// Keeps the micromap's memory alive; dropped after `raw` is destroyed.
    buffer: Buffer,
    /// Each triangle's subdivision level.
    levels: Vec<u16>,
}

impl OpacityMicromap {
    /// Bytes of the built micromap.
    pub fn size(&self) -> u64 {
        self.buffer.size()
    }

    /// How many triangles it holds.
    pub fn len(&self) -> usize {
        self.levels.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// The usage counts of `indices` (a geometry's): how many of them reference a map at each
    /// subdivision level, 4-state.
    fn usage(&self, indices: &[i32]) -> Vec<vk::MicromapUsageEXT> {
        let mut counts = [0_u32; 16];
        for &i in indices.iter().filter(|&&i| i >= 0) {
            counts[usize::from(self.levels[i as usize])] += 1;
        }
        usage_counts(&counts)
    }
}

impl std::fmt::Debug for OpacityMicromap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpacityMicromap")
            .field("triangles", &self.levels.len())
            .field("bytes", &self.buffer.size())
            .finish()
    }
}

impl Drop for OpacityMicromap {
    fn drop(&mut self) {
        if let Some((loader, _)) = self.device.micromap_loader() {
            // SAFETY: the micromap was created by this loader's device and the device is done
            // with it (resources are dropped after the device idles).
            unsafe {
                (loader.fp().destroy_micromap_ext)(loader.device(), self.raw, std::ptr::null())
            };
        }
    }
}

/// The 4-state usage counts of `counts` (triangles per subdivision level), the empty levels left
/// out.
fn usage_counts(counts: &[u32; 16]) -> Vec<vk::MicromapUsageEXT> {
    counts
        .iter()
        .enumerate()
        .filter(|(_, c)| **c > 0)
        .map(|(level, &count)| vk::MicromapUsageEXT {
            count,
            subdivision_level: level as u32,
            format: vk::OpacityMicromapFormatEXT::TYPE_4_STATE.as_raw() as u32,
        })
        .collect()
}

/// One triangle of an opacity micromap: its subdivision level (4^level micro-triangles) and the
/// byte of the data its states start at, 2 bits a micro-triangle in the order of the spec's
/// space-filling curve, the first in a byte's low bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicromapTriangle {
    /// The byte its states start at.
    pub offset: u32,
    /// Its subdivision level.
    pub level: u16,
}

/// A not-opaque geometry's link to an opacity micromap, where the geometry's triangles' `pNext`
/// points. Kept in a `Vec` that grows no more once that pointer is taken (moving a `Vec` moves
/// none of its elements), so its address and its usage counts' hold until the build is done.
struct OpacityLink {
    usage: Vec<vk::MicromapUsageEXT>,
    info: vk::AccelerationStructureTrianglesOpacityMicromapEXT<'static>,
}

/// One build's parts: its geometries with their primitive counts, the structure it builds into
/// and the scratch memory it needs.
struct Build {
    geometries: Vec<(vk::AccelerationStructureGeometryKHR<'static>, u32)>,
    ty: vk::AccelerationStructureTypeKHR,
    flags: vk::BuildAccelerationStructureFlagsKHR,
    structure: AccelerationStructure,
    scratch: Buffer,
    /// What the geometries' `pNext` point at, alive until the build is done.
    _links: Vec<OpacityLink>,
}

impl Device {
    fn acceleration(&self) -> Result<&ash::khr::acceleration_structure::Device> {
        self.acceleration_loader()
            .ok_or_else(|| GpuError::Unsupported("acceleration structures (no ray queries)".into()))
    }

    /// Sizes a build of `geometries` (each with its primitive count), creates its structure and
    /// scratch. `opacity`: the micromap its geometries' `links` index, which the structure keeps.
    fn prepare_build(
        self: &Arc<Self>,
        geometries: Vec<(vk::AccelerationStructureGeometryKHR<'static>, u32)>,
        ty: vk::AccelerationStructureTypeKHR,
        opacity: Option<(Arc<OpacityMicromap>, Vec<OpacityLink>)>,
        name: &str,
    ) -> Result<Build> {
        let loader = self.acceleration()?;
        let descriptions: Vec<_> = geometries.iter().map(|g| g.0).collect();
        let primitives: Vec<u32> = geometries.iter().map(|g| g.1).collect();
        // An instance may turn the micromaps off (one whose rows the maps were not made from).
        let flags = if opacity.is_some() {
            vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
                | vk::BuildAccelerationStructureFlagsKHR::ALLOW_DISABLE_OPACITY_MICROMAPS_EXT
        } else {
            vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE
        };
        let (micromap, links) = opacity.map_or((None, Vec::new()), |(m, l)| (Some(m), l));
        let info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
            .ty(ty)
            .flags(flags)
            .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
            .geometries(&descriptions);
        let mut sizes = vk::AccelerationStructureBuildSizesInfoKHR::default();
        // SAFETY: a size query on valid build info, a primitive count per geometry.
        unsafe {
            loader.get_acceleration_structure_build_sizes(
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &primitives,
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
            geometries,
            ty,
            flags,
            structure: AccelerationStructure {
                device: Arc::clone(self),
                raw,
                buffer,
                address,
                _micromap: micromap,
            },
            scratch,
            _links: links,
        })
    }

    /// Records every build of `builds` into one submission and waits for it.
    fn run_builds(&self, builds: &[Build]) -> Result<()> {
        let loader = self.acceleration()?;
        let geometries: Vec<Vec<vk::AccelerationStructureGeometryKHR<'static>>> = builds
            .iter()
            .map(|b| b.geometries.iter().map(|g| g.0).collect())
            .collect();
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
                    .flags(b.flags)
                    .mode(vk::BuildAccelerationStructureModeKHR::BUILD)
                    .dst_acceleration_structure(b.structure.raw)
                    .geometries(g)
                    .scratch_data(vk::DeviceOrHostAddressKHR {
                        device_address: scratch,
                    })
            })
            .collect();
        let ranges: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>> = builds
            .iter()
            .map(|b| {
                b.geometries
                    .iter()
                    .map(|g| {
                        vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(g.1)
                    })
                    .collect()
            })
            .collect();
        let range_refs: Vec<&[vk::AccelerationStructureBuildRangeInfoKHR]> =
            ranges.iter().map(|r| r.as_slice()).collect();
        // Earlier writes (uploads, the instance pass, a micromap's build) before the builds read
        // them.
        let micromaps = if self.micromap_loader().is_some() {
            vk::AccessFlags2::MICROMAP_READ_EXT
        } else {
            vk::AccessFlags2::empty()
        };
        let before = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
            .dst_access_mask(
                vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
                    | vk::AccessFlags2::SHADER_READ
                    | micromaps,
            )];
        // The built structures before anything traces or builds against them.
        let after = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR)
            .src_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR)];
        // On the compute queue: beside the frames when a worker builds a scene (#220). What the
        // builds read was written by submissions already waited for.
        self.execute_transient_on(QueueKind::Compute, |raw, cb| {
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

    /// Builds a bottom-level structure over each of `meshes` (its opaque triangles, then the
    /// others, [`BlasTriangles::opaque`]), in one submission; waits for it.
    pub fn build_blases(
        self: &Arc<Self>,
        meshes: &[BlasTriangles<'_>],
        name: &str,
    ) -> Result<Vec<AccelerationStructure>> {
        let input = vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR;
        // Every mesh's positions in one buffer and its indices in another: two uploads, not
        // two a mesh. Each upload waits for its submission, which queues behind the frames in
        // flight when a worker builds a scene while they run (#220: 400 waits took half a
        // second for the planet's 200 tiles).
        let positions: Vec<[f32; 3]> = meshes
            .iter()
            .flat_map(|m| m.positions.iter().copied())
            .collect();
        let indices: Vec<u32> = meshes
            .iter()
            .flat_map(|m| m.indices.iter().copied())
            .collect();
        let position_buffer =
            self.create_buffer_with_data(&positions, input, MemoryCategory::Transfer, name)?;
        let index_buffer =
            self.create_buffer_with_data(&indices, input, MemoryCategory::Transfer, name)?;
        // The not-opaque triangles' micromap indices, every mesh's in one buffer too (#333).
        let map_indices: Vec<i32> = meshes
            .iter()
            .filter_map(|m| m.opacity.map(|o| o.indices))
            .flatten()
            .copied()
            .collect();
        let map_buffer = (!map_indices.is_empty())
            .then(|| {
                self.create_buffer_with_data(&map_indices, input, MemoryCategory::Transfer, name)
            })
            .transpose()?;
        let mut builds = Vec::with_capacity(meshes.len());
        let (mut first_position, mut first_index, mut first_map) = (0_u64, 0_u64, 0_u64);
        for mesh in meshes {
            let triangles = (mesh.indices.len() / 3) as u32;
            let opaque = mesh.opaque.min(triangles);
            let positions = position_buffer.address() + first_position * 12;
            let vertices = mesh.positions.len() as u32;
            let first = index_buffer.address() + first_index * 4;
            // The not-opaque triangles' link to their micromap, when they have one.
            let mut links = Vec::new();
            let mut micromap = None;
            if let (Some(o), Some(maps)) =
                (mesh.opacity.filter(|_| opaque < triangles), &map_buffer)
            {
                assert_eq!(o.indices.len() as u32, triangles - opaque);
                let mut link = OpacityLink {
                    usage: o.micromap.usage(o.indices),
                    info: vk::AccelerationStructureTrianglesOpacityMicromapEXT::default()
                        .index_type(vk::IndexType::UINT32)
                        .index_buffer(vk::DeviceOrHostAddressConstKHR {
                            device_address: maps.address() + first_map * 4,
                        })
                        .index_stride(4)
                        .micromap(o.micromap.raw),
                };
                link.info.usage_counts_count = link.usage.len() as u32;
                link.info.p_usage_counts = link.usage.as_ptr();
                links.push(link);
                micromap = Some(Arc::clone(o.micromap));
            }
            let next = links.first().map_or(std::ptr::null(), |l| {
                std::ptr::from_ref(&l.info).cast::<std::ffi::c_void>()
            });
            // One geometry when the triangles are all opaque or none is; else the opaque ones,
            // then the others.
            let geometries = if opaque == triangles || opaque == 0 {
                let opaque = opaque == triangles;
                let next = if opaque { std::ptr::null() } else { next };
                vec![(
                    triangle_geometry(positions, vertices, first, opaque, next),
                    triangles,
                )]
            } else {
                let rest = first + u64::from(opaque) * 12;
                vec![
                    (
                        triangle_geometry(positions, vertices, first, true, std::ptr::null()),
                        opaque,
                    ),
                    (
                        triangle_geometry(positions, vertices, rest, false, next),
                        triangles - opaque,
                    ),
                ]
            };
            builds.push(self.prepare_build(
                geometries,
                vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
                micromap.map(|m| (m, links)),
                name,
            )?);
            first_position += mesh.positions.len() as u64;
            first_index += mesh.indices.len() as u64;
            first_map += mesh.opacity.map_or(0, |o| o.indices.len() as u64);
        }
        self.run_builds(&builds)?;
        drop((position_buffer, index_buffer, map_buffer));
        Ok(builds.into_iter().map(|b| b.structure).collect())
    }

    /// Builds an opacity micromap over `triangles`, whose states `data` holds (#333); waits for
    /// it. The device must have them ([`crate::DeviceFeatures::opacity_micromap`]) and every
    /// level be at most [`Device::micromap_max_level`].
    pub fn build_opacity_micromap(
        self: &Arc<Self>,
        triangles: &[MicromapTriangle],
        data: &[u8],
        name: &str,
    ) -> Result<Arc<OpacityMicromap>> {
        let Some((loader, max_level)) = self.micromap_loader() else {
            return Err(GpuError::Unsupported("opacity micromaps".into()));
        };
        let mut counts = [0_u32; 16];
        for t in triangles {
            assert!(
                u32::from(t.level) <= max_level.min(15),
                "a micromap's level {} past the device's {max_level}",
                t.level
            );
            counts[usize::from(t.level)] += 1;
        }
        let usage = usage_counts(&counts);
        let info = vk::MicromapBuildInfoEXT::default()
            .ty(vk::MicromapTypeEXT::OPACITY_MICROMAP)
            .flags(vk::BuildMicromapFlagsEXT::PREFER_FAST_TRACE)
            .mode(vk::BuildMicromapModeEXT::BUILD)
            .usage_counts(&usage);
        let mut sizes = vk::MicromapBuildSizesInfoEXT::default();
        // SAFETY: a size query on valid build info.
        unsafe {
            (loader.fp().get_micromap_build_sizes_ext)(
                loader.device(),
                vk::AccelerationStructureBuildTypeKHR::DEVICE,
                &info,
                &mut sizes,
            )
        };
        let buffer = self.create_buffer(BufferDesc {
            size: sizes.micromap_size.max(4),
            usage: vk::BufferUsageFlags::MICROMAP_STORAGE_EXT,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Geometry,
            name,
        })?;
        let create = vk::MicromapCreateInfoEXT::default()
            .buffer(buffer.raw())
            .size(sizes.micromap_size)
            .ty(vk::MicromapTypeEXT::OPACITY_MICROMAP);
        let mut raw = vk::MicromapEXT::null();
        // SAFETY: the buffer is live, large enough and has the micromap storage usage.
        unsafe {
            (loader.fp().create_micromap_ext)(loader.device(), &create, std::ptr::null(), &mut raw)
        }
        .result()?;
        let micromap = Arc::new(OpacityMicromap {
            device: Arc::clone(self),
            raw,
            buffer,
            levels: triangles.iter().map(|t| t.level).collect(),
        });
        // The triangles (`VkMicromapTriangleEXT`: the offset, the level, the format), then the
        // states, each at a multiple of 256 bytes as the build wants.
        let format = vk::OpacityMicromapFormatEXT::TYPE_4_STATE.as_raw() as u16;
        let records: Vec<u8> = triangles
            .iter()
            .flat_map(|t| {
                let mut r = [0_u8; 8];
                r[..4].copy_from_slice(&t.offset.to_le_bytes());
                r[4..6].copy_from_slice(&t.level.to_le_bytes());
                r[6..].copy_from_slice(&format.to_le_bytes());
                r
            })
            .collect();
        let records = records.as_slice();
        let data_at = (records.len() as u64).next_multiple_of(256);
        let input = self.create_buffer(BufferDesc {
            size: 256 + data_at + data.len().max(4) as u64,
            usage: vk::BufferUsageFlags::MICROMAP_BUILD_INPUT_READ_ONLY_EXT
                | vk::BufferUsageFlags::TRANSFER_DST,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Transfer,
            name: "micromap input",
        })?;
        let base = input.address().next_multiple_of(256);
        let skip = base - input.address();
        self.write_buffers_staged(&[(&input, skip, records), (&input, skip + data_at, data)])?;
        let scratch = self.create_buffer(BufferDesc {
            size: sizes.build_scratch_size.max(4) + self.scratch_alignment(),
            usage: vk::BufferUsageFlags::STORAGE_BUFFER,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Transfer,
            name: "micromap scratch",
        })?;
        let info = info
            .dst_micromap(raw)
            .data(vk::DeviceOrHostAddressConstKHR {
                device_address: base + data_at,
            })
            .triangle_array(vk::DeviceOrHostAddressConstKHR {
                device_address: base,
            })
            .triangle_array_stride(std::mem::size_of::<vk::MicromapTriangleEXT>() as u64)
            .scratch_data(vk::DeviceOrHostAddressKHR {
                device_address: scratch.address().next_multiple_of(self.scratch_alignment()),
            });
        let before = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::MICROMAP_BUILD_EXT)
            .dst_access_mask(vk::AccessFlags2::SHADER_READ | vk::AccessFlags2::MICROMAP_READ_EXT)];
        let after = [vk::MemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::MICROMAP_BUILD_EXT)
            .src_access_mask(vk::AccessFlags2::MICROMAP_WRITE_EXT)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::MICROMAP_READ_EXT)];
        self.execute_transient_on(QueueKind::Compute, |raw, cb| {
            // SAFETY: recorded into the transient command buffer; the input, scratch and
            // micromap outlive the call (`execute_transient` waits for the fence).
            unsafe {
                raw.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().memory_barriers(&before),
                );
                (loader.fp().cmd_build_micromaps_ext)(cb, 1, &info);
                raw.cmd_pipeline_barrier2(
                    cb,
                    &vk::DependencyInfo::default().memory_barriers(&after),
                );
            }
        })?;
        drop((input, scratch));
        self.set_name(raw, name);
        Ok(micromap)
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
            vec![(geometry, count)],
            vk::AccelerationStructureTypeKHR::TOP_LEVEL,
            None,
            name,
        )?;
        self.run_builds(std::slice::from_ref(&build))?;
        Ok(build.structure)
    }
}

/// The geometry of triangles, `opaque` or not: `vertices` positions (three floats each, packed)
/// at device address `positions`, three `u32` indices a triangle at `indices`; `next` its
/// triangles' `pNext` (an [`OpacityLink`]'s, or null).
fn triangle_geometry(
    positions: u64,
    vertices: u32,
    indices: u64,
    opaque: bool,
    next: *const std::ffi::c_void,
) -> vk::AccelerationStructureGeometryKHR<'static> {
    let mut triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
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
    triangles.p_next = next;
    vk::AccelerationStructureGeometryKHR::default()
        .geometry_type(vk::GeometryTypeKHR::TRIANGLES)
        .geometry(vk::AccelerationStructureGeometryDataKHR { triangles })
        .flags(if opaque {
            vk::GeometryFlagsKHR::OPAQUE
        } else {
            vk::GeometryFlagsKHR::empty()
        })
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
        let geometry = triangle_geometry(positions, vertices, indices, true, std::ptr::null());
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

    /// Makes the next build a full one rather than an update in place: an updated structure's
    /// boxes grow as its instances move, and a rebuild now and then fits them again (#323).
    pub fn rebuild_next(&self) {
        self.built.set(None);
    }

    /// Bytes of its storage.
    pub fn size(&self) -> u64 {
        self.storage.size()
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
        let flag = |name: &str| std::env::var_os(name).is_some_and(|v| v != "0");
        let refit = flag("FORGE_TLAS_REFIT");
        let flags = dynamic_tlas_flags(refit, flag("FORGE_TLAS_FAST_TRACE"));
        self.create_tlas_with(capacity, name, refit, flags)
    }

    /// Creates a top-level structure built once, fast to trace, then updated in place when a
    /// few of its `capacity` instance records change (#323: a scene's static instances shown,
    /// hidden or moved at run time). Its first [`crate::Commands::build_dynamic_tlas`] builds it;
    /// the next ones over the same count update it, until [`DynamicTlas::rebuild_next`].
    pub fn create_updatable_tlas(
        self: &Arc<Self>,
        capacity: u32,
        name: &str,
    ) -> Result<DynamicTlas> {
        use vk::BuildAccelerationStructureFlagsKHR as F;
        self.create_tlas_with(capacity, name, true, F::PREFER_FAST_TRACE | F::ALLOW_UPDATE)
    }

    fn create_tlas_with(
        self: &Arc<Self>,
        capacity: u32,
        name: &str,
        refit: bool,
        flags: vk::BuildAccelerationStructureFlagsKHR,
    ) -> Result<DynamicTlas> {
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
