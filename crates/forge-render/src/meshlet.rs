//! GPU-driven meshlet rendering (`shaders/meshlet.slang`, `shaders/hzb.slang`).
//!
//! [`MeshletSceneBuilder`] concatenates any number of meshlet meshes and instances into the
//! GPU tables; [`MeshletRenderer`] owns the hierarchical-Z pyramid, the pipelines and the
//! per-slot frame blocks, and declares the passes of the two-pass occluded draw into the
//! render graph (the depth buffer is a transient of the frame).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_geom::{GpuMeshlet, GpuVertex, MeshletMesh};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory,
    MemoryLocation, MeshPipelineDesc, Pipeline, Result, ShaderCompiler, ShaderStage, TransientDesc,
    vk,
};
use glam::{Mat4, Vec2, Vec3, Vec4};

/// Culling flags, mirrored in `meshlet.slang`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CullFlags(pub u32);

impl CullFlags {
    /// Keep the culling camera fixed while the view moves.
    pub const FREEZE: u32 = 1;
    /// Normal-cone backface culling.
    pub const CONE: u32 = 2;
    /// Frustum culling of instances and meshlets.
    pub const FRUSTUM: u32 = 4;
    /// Two-pass hierarchical-Z occlusion culling.
    pub const OCCLUSION: u32 = 8;
    /// Colour every meshlet differently (debug view).
    pub const MESHLET_COLORS: u32 = 16;
    /// Draw the meshlets that cone or occlusion culling rejected, tinted red, instead of
    /// skipping them: a correctly culled meshlet is back-facing or hidden and leaves no
    /// pixel, so every red pixel is a culling error (debug view).
    pub const SHOW_CULLED: u32 = 32;
    /// Select clusters by projected LOD error; off draws level 0 only (full detail).
    pub const LOD: u32 = 64;
    /// Colour every cluster by its LOD level (debug view).
    pub const LOD_COLORS: u32 = 128;
    /// Disable the per-group LOD window (debug: every group tests its clusters).
    pub const GROUP_WINDOW_OFF: u32 = 256;
    /// Everything on except the debug view.
    pub const DEFAULT: Self = Self(Self::CONE | Self::FRUSTUM | Self::OCCLUSION | Self::LOD);

    /// Whether a bit is set.
    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
    /// Toggles a bit.
    pub fn toggle(&mut self, bit: u32) {
        self.0 ^= bit;
    }
}

const PASS_PREVIOUSLY_VISIBLE: u32 = 1;
const PASS_REMAINDER: u32 = 2;
const PASS_SINGLE: u32 = 3;
const TASK_GROUP_SIZE: u32 = 32;
const STAT_COUNT: usize = 8;
/// LOD levels a mesh may have on the GPU (mirrors `forge_geom::MAX_LEVELS`).
const LOD_LEVELS: usize = forge_geom::MAX_LEVELS as usize;
const FRAME_BLOCK_STRIDE: u64 = 512;

/// Mirrors `Mesh` in `meshlet.slang` (32 bytes).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuMesh {
    meshlet_offset: u32,
    meshlet_count: u32,
    triangle_count: u32,
    level_count: u32,
    center: [f32; 3],
    radius: f32,
    /// Cluster offsets per LOD level relative to `meshlet_offset`, plus the end.
    level_offset: [u32; LOD_LEVELS + 1],
    pad: [u32; 3],
    /// Per level: the smallest `self_error` of its clusters.
    self_error_min: [f32; LOD_LEVELS],
    /// Per level: the largest `parent_error` (infinite when the level holds a root).
    parent_error_max: [f32; LOD_LEVELS],
    /// Per level: how far a cluster's `self` sphere reaches from the mesh centre.
    self_reach_max: [f32; LOD_LEVELS],
    /// Per level: the same for the `parent` spheres.
    parent_reach_max: [f32; LOD_LEVELS],
}

/// Mirrors `Instance` in `meshlet.slang` (96 bytes).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuInstance {
    model: [f32; 16],
    center: [f32; 3],
    radius: f32,
    mesh: u32,
    id: u32,
    /// First task group of this instance (`ceil(meshlet_count / 32)` groups follow).
    group_offset: u32,
    /// First visibility bit of this instance (`meshlet_count` bits follow).
    bit_offset: u32,
}

/// Mirrors `Frame` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuFrame {
    view_proj: [f32; 16],
    cull_view_proj: [f32; 16],
    cull_view: [f32; 16],
    planes: [[f32; 4]; 6],
    camera_pos: [f32; 3],
    instance_count: u32,
    max_meshlets: u32,
    flags: u32,
    pass: u32,
    hzb_image: u32,
    hzb_size: [u32; 2],
    p00: f32,
    p11: f32,
    znear: f32,
    sun_dir: [f32; 3],
    draw_jitter: [f32; 2],
    lod_threshold: f32,
    viewport_height: f32,
    vertices: u64,
    meshlets: u64,
    meshlet_vertices: u64,
    meshlet_triangles: u64,
    meshes: u64,
    instances: u64,
    stats: u64,
    visibility: u64,
    group_table: u64,
    total_groups: u32,
    pad_end: u32,
    work: u64,
    indirect: u64,
    /// The visible-cluster list: slot 0 holds the count, then (instance, meshlet | flags).
    visible: u64,
    visible_capacity: u32,
    /// Pre-exposure of the frame (see `crate::exposure`).
    exposure: f32,
    /// Illuminance of the sun in lux.
    sun_illuminance: f32,
    pad_exposure: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    frame: u64,
}

/// Mirrors `ResolvePush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ResolvePush {
    frame: u64,
    vis_image: u32,
    color_image: u32,
    width: u32,
    height: u32,
    use_background: u32,
    pad: u32,
    background: [f32; 4],
}

/// Clusters the visible-cluster list can hold per frame (8 MB per frame slot); the task
/// shader drops and counts what does not fit (`FrameStats::visible_overflow`).
pub const VISIBLE_CAPACITY: u32 = 1 << 20;

/// What a frame's draw leaves behind for the passes after it.
#[derive(Clone, Copy, Debug)]
pub struct DrawTargets {
    /// The depth buffer (a transient), `D32_SFLOAT`.
    pub depth: ImageHandle,
    /// The visibility buffer (a transient), `R32_UINT`: `visible_slot << 7 | triangle`, or
    /// `u32::MAX` where nothing was drawn (see `forge_render::visibility`).
    pub visibility: ImageHandle,
    /// The visible-cluster list the ids index (this frame slot's).
    pub visible_list: forge_gpu::BufferHandle,
}

/// Mirrors `Push` in `hzb.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct HzbPush {
    src_image: u32,
    src_level: u32,
    dst_image: u32,
    dst_width: u32,
    dst_height: u32,
    pad: [u32; 3],
}

/// Handle of a mesh added to a [`MeshletSceneBuilder`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MeshId(u32);

/// Concatenates meshes and instances into the GPU tables.
#[derive(Default)]
pub struct MeshletSceneBuilder {
    vertices: Vec<GpuVertex>,
    meshlets: Vec<GpuMeshlet>,
    meshlet_vertices: Vec<u32>,
    meshlet_triangles: Vec<u8>,
    meshes: Vec<GpuMesh>,
    instances: Vec<GpuInstance>,
    total_triangles: u64,
    /// Instance index of every task group.
    group_table: Vec<u32>,
    /// Visibility bits over all instances (one per cluster).
    total_bits: u32,
}

impl MeshletSceneBuilder {
    /// An empty builder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a mesh; offsets are rebased into the shared tables.
    pub fn add_mesh(&mut self, mesh: &MeshletMesh) -> MeshId {
        let vertex_base = self.vertices.len() as u32;
        let meshlet_vertex_base = self.meshlet_vertices.len() as u32;
        while !self.meshlet_triangles.len().is_multiple_of(4) {
            self.meshlet_triangles.push(0);
        }
        let triangle_base = self.meshlet_triangles.len() as u32;
        let meshlet_offset = self.meshlets.len() as u32;
        self.vertices.extend_from_slice(&mesh.vertices);
        self.meshlet_vertices
            .extend(mesh.meshlet_vertices.iter().map(|v| v + vertex_base));
        self.meshlet_triangles
            .extend_from_slice(&mesh.meshlet_triangles);
        self.meshlets
            .extend(mesh.meshlets.iter().map(|m| GpuMeshlet {
                vertex_offset: m.vertex_offset + meshlet_vertex_base,
                triangle_offset: m.triangle_offset + triangle_base,
                ..*m
            }));
        let id = MeshId(self.meshes.len() as u32);
        // Per-level tables for the task shader's group window: clusters are stored level by level.
        let level_count = mesh.clusters_per_level.len().min(LOD_LEVELS);
        let mut level_offset = [0_u32; LOD_LEVELS + 1];
        let mut self_error_min = [f32::INFINITY; LOD_LEVELS];
        let mut parent_error_max = [0.0_f32; LOD_LEVELS];
        let mut self_reach_max = [0.0_f32; LOD_LEVELS];
        let mut parent_reach_max = [0.0_f32; LOD_LEVELS];
        for level in 0..level_count {
            level_offset[level + 1] = level_offset[level] + mesh.clusters_per_level[level];
        }
        let reach = |c: [f32; 3], r: f32| (Vec3::from(c) - Vec3::from(mesh.center)).length() + r;
        for m in &mesh.meshlets {
            let level = (m.lod_level as usize).min(level_count.saturating_sub(1));
            self_error_min[level] = self_error_min[level].min(m.self_error);
            parent_error_max[level] = parent_error_max[level].max(m.parent_error);
            self_reach_max[level] = self_reach_max[level].max(reach(m.self_center, m.self_radius));
            parent_reach_max[level] =
                parent_reach_max[level].max(reach(m.parent_center, m.parent_radius));
        }
        for value in self_error_min.iter_mut().skip(level_count) {
            *value = 0.0;
        }
        self.meshes.push(GpuMesh {
            meshlet_offset,
            meshlet_count: mesh.meshlets.len() as u32,
            triangle_count: mesh.triangle_count as u32,
            level_count: level_count as u32,
            center: mesh.center,
            radius: mesh.radius,
            level_offset,
            pad: [0; 3],
            self_error_min,
            parent_error_max,
            self_reach_max,
            parent_reach_max,
        });
        id
    }

    /// Adds an instance of `mesh` with a uniform-scale transform.
    pub fn add_instance(&mut self, mesh: MeshId, model: Mat4) {
        let info = self.meshes[mesh.0 as usize];
        let scale = model.x_axis.truncate().length();
        let center = model.transform_point3(Vec3::from(info.center));
        self.total_triangles += u64::from(info.triangle_count);
        let groups = info.meshlet_count.div_ceil(TASK_GROUP_SIZE);
        let group_offset = self.group_table.len() as u32;
        self.group_table.extend(std::iter::repeat_n(
            self.instances.len() as u32,
            groups as usize,
        ));
        let bit_offset = self.total_bits;
        self.total_bits += info.meshlet_count;
        self.instances.push(GpuInstance {
            model: model.to_cols_array(),
            center: center.to_array(),
            radius: info.radius * scale,
            mesh: mesh.0,
            id: self.instances.len() as u32,
            group_offset,
            bit_offset,
        });
    }

    /// Number of instances so far.
    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Uploads everything.
    pub fn build(mut self, device: &Arc<Device>) -> Result<MeshletScene> {
        while !self.meshlet_triangles.len().is_multiple_of(4) {
            self.meshlet_triangles.push(0);
        }
        let max_meshlets = self
            .meshes
            .iter()
            .map(|m| m.meshlet_count)
            .max()
            .unwrap_or(0);
        let usage = vk::BufferUsageFlags::STORAGE_BUFFER;
        let visibility_words = (self.total_bits as usize).div_ceil(32).max(1);
        if self.group_table.is_empty() {
            self.group_table.push(0);
        }
        Ok(MeshletScene {
            vertices: device.create_buffer_with_data(
                &self.vertices,
                usage,
                MemoryCategory::Geometry,
                "meshlet vertices",
            )?,
            meshlets: device.create_buffer_with_data(
                &self.meshlets,
                usage,
                MemoryCategory::Geometry,
                "meshlets",
            )?,
            meshlet_vertices: device.create_buffer_with_data(
                &self.meshlet_vertices,
                usage,
                MemoryCategory::Geometry,
                "meshlet vertex indices",
            )?,
            meshlet_triangles: device.create_buffer_with_data(
                &self.meshlet_triangles,
                usage,
                MemoryCategory::Geometry,
                "meshlet triangles",
            )?,
            meshes: device.create_buffer_with_data(
                &self.meshes,
                usage,
                MemoryCategory::Geometry,
                "meshes",
            )?,
            instances: device.create_buffer_with_data(
                &self.instances,
                usage,
                MemoryCategory::Geometry,
                "instances",
            )?,
            visibility: GraphBuffer::new(device.create_buffer_with_data(
                &vec![0_u32; visibility_words],
                usage,
                MemoryCategory::Work,
                "visibility bits",
            )?),
            group_table: device.create_buffer_with_data(
                &self.group_table,
                usage,
                MemoryCategory::Geometry,
                "task group table",
            )?,
            work: (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    device
                        .create_buffer(BufferDesc {
                            size: u64::from(self.group_table.len() as u32) * 8,
                            usage,
                            location: MemoryLocation::GpuOnly,
                            category: MemoryCategory::Work,
                            name: &format!("task work list {i}"),
                        })
                        .map(GraphBuffer::new)
                })
                .collect::<Result<Vec<_>>>()?,
            indirect: (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    device
                        .create_buffer(BufferDesc {
                            size: 16,
                            usage: usage | vk::BufferUsageFlags::INDIRECT_BUFFER,
                            location: MemoryLocation::CpuToGpu,
                            category: MemoryCategory::Frame,
                            name: &format!("task indirect {i}"),
                        })
                        .map(GraphBuffer::new)
                })
                .collect::<Result<Vec<_>>>()?,
            visible: (0..FRAMES_IN_FLIGHT)
                .map(|i| {
                    device
                        .create_buffer(BufferDesc {
                            size: u64::from(VISIBLE_CAPACITY + 1) * 8,
                            usage,
                            location: MemoryLocation::GpuOnly,
                            category: MemoryCategory::Work,
                            name: &format!("visible clusters {i}"),
                        })
                        .map(GraphBuffer::new)
                })
                .collect::<Result<Vec<_>>>()?,
            instance_count: self.instances.len() as u32,
            total_groups: self.group_table.len() as u32,
            total_bits: self.total_bits,
            max_meshlets,
            mesh_count: self.meshes.len() as u32,
            meshlet_count: self.meshlets.len() as u32,
            total_triangles: self.total_triangles,
        })
    }
}

/// The uploaded scene tables.
pub struct MeshletScene {
    vertices: Buffer,
    meshlets: Buffer,
    meshlet_vertices: Buffer,
    meshlet_triangles: Buffer,
    meshes: Buffer,
    instances: Buffer,
    /// One bit per (instance, cluster): visible last frame. Read and rewritten by the task
    /// shader every frame, so the graph tracks it.
    visibility: GraphBuffer,
    group_table: Buffer,
    /// Per frame slot: the task-group work list built by the cull pass.
    work: Vec<GraphBuffer>,
    /// Per frame slot: the indirect mesh-task command (x = work count).
    indirect: Vec<GraphBuffer>,
    /// Per frame slot: the visible-cluster list the task shader appends to (slot 0 counts).
    visible: Vec<GraphBuffer>,
    /// Instances.
    pub instance_count: u32,
    /// Task groups per pass (every instance's clusters in groups of 32).
    pub total_groups: u32,
    /// Clusters over all instances (one visibility bit each).
    pub total_bits: u32,
    /// Largest meshlet count of any mesh (task groups per instance derive from it).
    pub max_meshlets: u32,
    /// Distinct meshes.
    pub mesh_count: u32,
    /// Meshlets over all meshes.
    pub meshlet_count: u32,
    /// Triangles over all instances.
    pub total_triangles: u64,
}

impl MeshletScene {
    /// Meshlets over all instances (the culling universe).
    pub fn instance_meshlets(&self) -> u64 {
        u64::from(self.total_bits)
    }
}

/// The culling camera, frozen as a unit by the freeze flag.
#[derive(Clone, Copy, Debug)]
pub struct CullCamera {
    /// World → view.
    pub view: Mat4,
    /// World → clip.
    pub view_proj: Mat4,
    /// Camera position.
    pub position: Vec3,
    /// Projection scale x.
    pub p00: f32,
    /// Projection scale y.
    pub p11: f32,
    /// Near plane distance (reversed-Z infinite projection).
    pub near: f32,
}

impl CullCamera {
    /// From a view matrix, a projection matrix and the camera position.
    pub fn new(view: Mat4, proj: Mat4, position: Vec3, near: f32) -> Self {
        Self {
            view,
            view_proj: proj * view,
            position,
            p00: proj.x_axis.x,
            p11: proj.y_axis.y,
            near,
        }
    }
}

/// Frustum planes (inward normals), Gribb–Hartmann; the far plane is replaced by an
/// always-pass plane because the projection is infinite.
fn frustum_planes(view_proj: Mat4) -> [[f32; 4]; 6] {
    let m = view_proj.transpose();
    let rows = [m.x_axis, m.y_axis, m.z_axis, m.w_axis];
    let plane = |p: Vec4| {
        let len = p.truncate().length().max(1e-6);
        (p / len).to_array()
    };
    [
        plane(rows[3] + rows[0]),
        plane(rows[3] - rows[0]),
        plane(rows[3] + rows[1]),
        plane(rows[3] - rows[1]),
        plane(rows[2]),
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// Per-frame counters read back two frames later.
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    /// Meshlets drawn in pass 1 (previously visible) or in the single pass.
    pub meshlets_pass1: u32,
    /// Meshlets drawn in pass 2 (newly visible).
    pub meshlets_pass2: u32,
    /// Triangles emitted.
    pub triangles: u32,
    /// Meshlets rejected by the depth pyramid.
    pub occluded: u32,
    /// Instances that passed the frustum test.
    pub instances_visible: u32,
    /// Sum of the LOD level of every drawn meshlet (mean = sum / drawn).
    pub lod_level_sum: u32,
    /// Clusters dropped because the visible-cluster list was full (must stay 0).
    pub visible_overflow: u32,
}

/// The hierarchical-Z pyramid: power-of-two, one storage view per level, sampled in
/// `GENERAL`. Persistent (a frozen culling camera keeps using the last one built).
fn create_pyramid(device: &Arc<Device>, extent: vk::Extent2D) -> Result<GraphImage> {
    let prev_pow2 = |v: u32| 1_u32 << (31 - v.max(1).leading_zeros());
    let (w, h) = (prev_pow2(extent.width), prev_pow2(extent.height));
    let mips = 32 - w.max(h).leading_zeros();
    let hzb = GraphImage::new(
        device,
        ImageDesc {
            width: w,
            height: h,
            format: vk::Format::R32_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: mips,
            name: "hzb",
        },
    )?;
    tracing::info!(hzb_width = w, hzb_height = h, mips, "depth pyramid created");
    Ok(hzb)
}

/// Declares the meshlet passes for one scene.
pub struct MeshletRenderer {
    device: Arc<Device>,
    pipeline_solid: Pipeline,
    pipeline_wire: Pipeline,
    pipeline_hzb: Pipeline,
    pipeline_cull: Pipeline,
    pipeline_resolve: Pipeline,
    hzb: GraphImage,
    frame_buffers: Vec<Buffer>,
    stats_buffers: Vec<GraphBuffer>,
    /// Direction *to* the sun (world space), used by the visibility resolve.
    pub sun_dir: Vec3,
    /// Illuminance of the sun at the scene, in lux (the rocks return albedo × E / π).
    pub sun_illuminance: f32,
}

/// What to draw this frame.
#[derive(Clone, Copy)]
pub struct DrawParams<'a> {
    /// The scene tables.
    pub scene: &'a MeshletScene,
    /// The drawing camera.
    pub view_proj: Mat4,
    /// The culling camera (equal to the drawing one unless frozen).
    pub cull: CullCamera,
    /// Where the drawn image sits relative to the culling camera's, in uv units (x right,
    /// y down): the temporal anti-aliasing jitter divided by the extent, or zero. The
    /// occlusion test reads the depth pyramid there.
    pub draw_jitter: Vec2,
    /// Projected LOD error a drawn cluster may have, in pixels (when `CullFlags::LOD` is set).
    pub lod_threshold_px: f32,
    /// Culling flags.
    pub flags: CullFlags,
    /// Target size.
    pub extent: vk::Extent2D,
    /// Wireframe.
    pub wireframe: bool,
    /// Pre-exposure the resolve multiplies the shaded luminance by (see `crate::exposure`).
    pub exposure: f32,
}

/// The graph handles one mesh pass touches.
#[derive(Clone, Copy)]
struct MeshPassIo {
    /// The visibility buffer (colour attachment of the mesh passes).
    visibility: ImageHandle,
    depth: ImageHandle,
    hzb: ImageHandle,
    work: forge_gpu::BufferHandle,
    indirect: forge_gpu::BufferHandle,
    /// The per-cluster "visible last frame" bits.
    visibility_bits: forge_gpu::BufferHandle,
    /// The visible-cluster list of this frame slot.
    visible: forge_gpu::BufferHandle,
    stats: forge_gpu::BufferHandle,
}

impl MeshletRenderer {
    /// Compiles the pipelines and creates the depth resources for `extent`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let task = device.create_shader_module(
            &shaders.compile("meshlet.slang", "task_main", ShaderStage::Task)?,
            "task",
        )?;
        let mesh = device.create_shader_module(
            &shaders.compile("meshlet.slang", "mesh_main", ShaderStage::Mesh)?,
            "mesh",
        )?;
        let frag = device.create_shader_module(
            &shaders.compile("meshlet.slang", "frag_main", ShaderStage::Fragment)?,
            "frag",
        )?;
        let hzb = device.create_shader_module(
            &shaders.compile("hzb.slang", "hzb_main", ShaderStage::Compute)?,
            "hzb",
        )?;
        let cull = device.create_shader_module(
            &shaders.compile("meshlet.slang", "cull_main", ShaderStage::Compute)?,
            "instance cull",
        )?;
        let make_pipeline = |wireframe: bool| {
            device.create_mesh_pipeline(&MeshPipelineDesc {
                task: Some((task, "task_main")),
                mesh: (mesh, "mesh_main"),
                fragment: (frag, "frag_main"),
                color_formats: &[vk::Format::R32_UINT],
                depth_format: Some(vk::Format::D32_SFLOAT),
                push_constant_bytes: std::mem::size_of::<Push>() as u32,
                cull_mode: vk::CullModeFlags::BACK,
                wireframe,
                depth_test: true,
                name: if wireframe {
                    "meshlets wire"
                } else {
                    "meshlets"
                },
            })
        };
        let pipeline_solid = make_pipeline(false)?;
        let pipeline_wire = make_pipeline(true)?;
        let pipeline_hzb = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (hzb, "hzb_main"),
            push_constant_bytes: std::mem::size_of::<HzbPush>() as u32,
            name: "hzb build",
        })?;
        let pipeline_cull = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (cull, "cull_main"),
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            name: "instance cull + LOD window",
        })?;
        let resolve = device.create_shader_module(
            &shaders.compile("meshlet.slang", "resolve_main", ShaderStage::Compute)?,
            "visibility resolve",
        )?;
        let pipeline_resolve = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (resolve, "resolve_main"),
            push_constant_bytes: std::mem::size_of::<ResolvePush>() as u32,
            name: "visibility resolve",
        })?;
        for module in [task, mesh, frag, hzb, cull, resolve] {
            device.destroy_shader_module(module);
        }
        let hzb = create_pyramid(device, extent)?;
        let frame_buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: FRAME_BLOCK_STRIDE * 2,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("meshlet frame {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let stats_buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                let b = device.create_buffer(BufferDesc {
                    size: (STAT_COUNT * 4) as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("meshlet stats {i}"),
                })?;
                b.write(0, &[0_u32; STAT_COUNT]);
                Ok(GraphBuffer::new(b))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            device: Arc::clone(device),
            pipeline_solid,
            pipeline_wire,
            pipeline_hzb,
            pipeline_cull,
            pipeline_resolve,
            hzb,
            frame_buffers,
            stats_buffers,
            sun_dir: Vec3::new(0.4, 1.0, 0.3).normalize(),
            sun_illuminance: crate::starfield::SUN_ILLUMINANCE_1AU,
        })
    }

    /// Recreates the depth pyramid. The device must be idle.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        self.hzb = create_pyramid(&self.device, extent)?;
        Ok(())
    }

    /// Size of the depth pyramid's level 0.
    pub fn hzb_extent(&self) -> vk::Extent2D {
        self.hzb.extent()
    }

    /// Reads and clears the statistics of the frame that last used `slot`. Returns `None`
    /// for the first frames in flight.
    pub fn take_stats(&self, slot: FrameSlot) -> Option<FrameStats> {
        let mut raw = [0_u32; STAT_COUNT];
        self.stats_buffers[slot.index].read(0, &mut raw);
        self.stats_buffers[slot.index].write(0, &[0_u32; STAT_COUNT]);
        (slot.frame_number >= FRAMES_IN_FLIGHT as u64).then_some(FrameStats {
            meshlets_pass1: raw[0],
            meshlets_pass2: raw[1],
            triangles: raw[2],
            occluded: raw[3],
            instances_visible: raw[4],
            lod_level_sum: raw[5],
            visible_overflow: raw[6],
        })
    }

    fn frame_block(&self, slot: FrameSlot, params: &DrawParams<'_>, pass: u32) -> GpuFrame {
        let scene = params.scene;
        let hzb = self.hzb.extent();
        GpuFrame {
            view_proj: params.view_proj.to_cols_array(),
            cull_view_proj: params.cull.view_proj.to_cols_array(),
            cull_view: params.cull.view.to_cols_array(),
            planes: frustum_planes(params.cull.view_proj),
            camera_pos: params.cull.position.to_array(),
            instance_count: scene.instance_count,
            max_meshlets: scene.max_meshlets,
            flags: params.flags.0,
            pass,
            hzb_image: self.hzb.sampled().0,
            hzb_size: [hzb.width, hzb.height],
            p00: params.cull.p00,
            p11: params.cull.p11,
            znear: params.cull.near,
            sun_dir: self.sun_dir.to_array(),
            draw_jitter: params.draw_jitter.to_array(),
            lod_threshold: params.lod_threshold_px,
            viewport_height: params.extent.height as f32,
            vertices: scene.vertices.address(),
            meshlets: scene.meshlets.address(),
            meshlet_vertices: scene.meshlet_vertices.address(),
            meshlet_triangles: scene.meshlet_triangles.address(),
            meshes: scene.meshes.address(),
            instances: scene.instances.address(),
            stats: self.stats_buffers[slot.index].address(),
            visibility: scene.visibility.address(),
            group_table: scene.group_table.address(),
            total_groups: scene.total_groups,
            pad_end: 0,
            work: scene.work[slot.index].address(),
            indirect: scene.indirect[slot.index].address(),
            visible: scene.visible[slot.index].address(),
            visible_capacity: VISIBLE_CAPACITY,
            exposure: params.exposure,
            sun_illuminance: self.sun_illuminance,
            pad_exposure: 0,
        }
    }

    /// Declares the passes of this frame's draw: instance cull, the first mesh pass, the
    /// depth pyramid and the second mesh pass (with occlusion), all reading and writing
    /// through declared graph accesses. The mesh passes write the visibility buffer and the
    /// depth (both transients); [`MeshletRenderer::resolve`] shades the result.
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        params: DrawParams<'f>,
    ) -> Result<DrawTargets> {
        let occlusion = params.flags.has(CullFlags::OCCLUSION);
        let frozen = params.flags.has(CullFlags::FREEZE);
        let first_pass = if occlusion {
            PASS_PREVIOUSLY_VISIBLE
        } else {
            PASS_SINGLE
        };
        let block1 = self.frame_block(slot, &params, first_pass);
        let block2 = self.frame_block(slot, &params, PASS_REMAINDER);
        self.frame_buffers[slot.index].write(0, &[block1]);
        self.frame_buffers[slot.index].write(FRAME_BLOCK_STRIDE, &[block2]);
        // The indirect command of both passes (x = group count, reset here every frame).
        let scene = params.scene;
        scene.indirect[slot.index].write(0, &[0_u32, 1, 1, 0]);

        let extent = params.extent;
        let depth = graph.transient(TransientDesc {
            name: "depth",
            width: extent.width,
            height: extent.height,
            format: vk::Format::D32_SFLOAT,
            usage: vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::DEPTH,
            mip_levels: 1,
        });
        let visibility = graph.transient(TransientDesc {
            name: "visibility",
            width: extent.width,
            height: extent.height,
            format: vk::Format::R32_UINT,
            usage: vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let io = MeshPassIo {
            visibility,
            depth,
            hzb: graph.import(&self.hzb),
            work: graph.import_buffer(&scene.work[slot.index]),
            indirect: graph.import_buffer(&scene.indirect[slot.index]),
            visibility_bits: graph.import_buffer(&scene.visibility),
            visible: graph.import_buffer(&scene.visible[slot.index]),
            stats: graph.import_buffer(&self.stats_buffers[slot.index]),
        };
        let frame_address = self.frame_buffers[slot.index].address();

        // Instance culling and LOD level windows in compute: the task-group work list. It
        // also resets the visible-cluster counter.
        let cull_pipeline = &self.pipeline_cull;
        let instance_count = scene.instance_count;
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        graph
            .pass("geometry/instance cull")
            .buffer(io.work, BufferAccess::ShaderWrite(compute))
            .buffer(io.indirect, BufferAccess::ShaderWrite(compute))
            .buffer(io.visible, BufferAccess::ShaderWrite(compute))
            .buffer(io.stats, BufferAccess::ShaderWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(cull_pipeline);
                commands.push_constants(
                    cull_pipeline,
                    &Push {
                        frame: frame_address,
                    },
                );
                commands.dispatch(instance_count.div_ceil(64).max(1), 1, 1);
                Ok(())
            });

        let first = MeshPass {
            label: if occlusion {
                "geometry/meshlet pass 1 (visible last frame)"
            } else {
                "geometry/meshlets (single pass)"
            },
            io,
            frame_address,
            first: true,
            reads_hzb: false,
        };
        self.mesh_pass(graph, first, &params, slot);

        if occlusion {
            if !frozen {
                self.pyramid_passes(graph, io);
            }
            let second = MeshPass {
                label: "geometry/meshlet pass 2 (newly visible)",
                io,
                frame_address: frame_address + FRAME_BLOCK_STRIDE,
                first: false,
                reads_hzb: true,
            };
            self.mesh_pass(graph, second, &params, slot);
        }
        Ok(DrawTargets {
            depth,
            visibility,
            visible_list: io.visible,
        })
    }

    /// Declares the pass that shades the visibility buffer once per pixel into `color` (a
    /// storage-capable colour image of `extent`): the triangle behind each pixel is fetched,
    /// its attributes reconstructed at the pixel centre from analytic barycentrics, and lit.
    /// Empty pixels get `background`, or are left for a later pass (the sky) when `None`.
    pub fn resolve<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        targets: DrawTargets,
        color: ImageHandle,
        extent: vk::Extent2D,
        background: Option<[f32; 4]>,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let pipeline = &self.pipeline_resolve;
        let frame_address = self.frame_buffers[slot.index].address();
        graph
            .pass("shading/visibility resolve")
            .image(targets.visibility, ImageAccess::Sampled(compute))
            .image(color, ImageAccess::StorageWrite(compute))
            .buffer(targets.visible_list, BufferAccess::ShaderRead(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &ResolvePush {
                        frame: frame_address,
                        vis_image: resources.sampled(targets.visibility).0,
                        color_image: resources.storage(color, 0).0,
                        width: extent.width,
                        height: extent.height,
                        use_background: u32::from(background.is_some()),
                        pad: 0,
                        background: background.unwrap_or([0.0; 4]),
                    },
                );
                commands.dispatch(extent.width.div_ceil(8), extent.height.div_ceil(8), 1);
                Ok(())
            });
    }

    /// One mesh-shader pass over the work list: clears the visibility buffer and the depth
    /// when `first`, tests the depth pyramid when `reads_hzb`.
    fn mesh_pass<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        pass: MeshPass,
        params: &DrawParams<'f>,
        slot: FrameSlot,
    ) {
        use vk::PipelineStageFlags2 as S;
        let pipeline = if params.wireframe {
            &self.pipeline_wire
        } else {
            &self.pipeline_solid
        };
        let indirect: &'f GraphBuffer = &params.scene.indirect[slot.index];
        let extent = params.extent;
        let MeshPass {
            label,
            io,
            frame_address,
            first,
            reads_hzb,
        } = pass;
        let mut builder = graph
            .pass(label)
            .buffer(io.work, BufferAccess::ShaderRead(S::TASK_SHADER_EXT))
            .buffer(io.indirect, BufferAccess::IndirectArgs)
            .buffer(
                io.visibility_bits,
                BufferAccess::ShaderReadWrite(S::TASK_SHADER_EXT),
            )
            .buffer(
                io.visible,
                BufferAccess::ShaderReadWrite(S::TASK_SHADER_EXT),
            )
            .buffer(
                io.stats,
                BufferAccess::ShaderWrite(S::TASK_SHADER_EXT | S::MESH_SHADER_EXT),
            )
            .image(io.visibility, ImageAccess::ColorAttachment)
            .image(io.depth, ImageAccess::DepthAttachment);
        if reads_hzb {
            builder = builder.image(io.hzb, ImageAccess::Sampled(S::TASK_SHADER_EXT));
        }
        builder.run(move |resources, commands| {
            let load = if first {
                vk::AttachmentLoadOp::CLEAR
            } else {
                vk::AttachmentLoadOp::LOAD
            };
            let color = [vk::RenderingAttachmentInfo::default()
                .image_view(resources.view(io.visibility))
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(load)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    color: vk::ClearColorValue {
                        uint32: [crate::visibility::EMPTY; 4],
                    },
                })];
            let depth = vk::RenderingAttachmentInfo::default()
                .image_view(resources.view(io.depth))
                .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .load_op(load)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 0.0,
                        stencil: 0,
                    },
                });
            let info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                })
                .layer_count(1)
                .color_attachments(&color)
                .depth_attachment(&depth);
            commands.begin_rendering(&info);
            commands.bind_pipeline(pipeline);
            commands.set_viewport_full(extent);
            commands.push_constants(
                pipeline,
                &Push {
                    frame: frame_address,
                },
            );
            let result = commands.draw_mesh_tasks_indirect(indirect, 0);
            commands.end_rendering();
            result
        });
    }

    /// Builds the depth pyramid level by level: level 0 from the depth buffer, each next
    /// level from the previous one (one pass per level, one profiler zone for all).
    fn pyramid_passes<'f>(&'f self, graph: &mut FrameGraph<'f>, io: MeshPassIo) {
        use vk::PipelineStageFlags2 as S;
        let pipeline = &self.pipeline_hzb;
        let hzb = &self.hzb;
        for level in 0..hzb.mip_levels() {
            let dst = hzb.mip_extent(level);
            let pass = graph.pass("geometry/depth pyramid");
            let pass = if level == 0 {
                pass.image(io.depth, ImageAccess::Sampled(S::COMPUTE_SHADER))
            } else {
                pass.image_mip(io.hzb, level - 1, ImageAccess::Sampled(S::COMPUTE_SHADER))
            };
            pass.image_mip(io.hzb, level, ImageAccess::StorageWrite(S::COMPUTE_SHADER))
                .run(move |resources, commands| {
                    let (src_image, src_level) = if level == 0 {
                        (resources.sampled(io.depth).0, 0)
                    } else {
                        (hzb.sampled().0, level - 1)
                    };
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &HzbPush {
                            src_image,
                            src_level,
                            dst_image: hzb.storage(level).0,
                            dst_width: dst.width,
                            dst_height: dst.height,
                            pad: [0; 3],
                        },
                    );
                    commands.dispatch(dst.width.div_ceil(8), dst.height.div_ceil(8), 1);
                    Ok(())
                });
        }
    }
}

/// One mesh-shader pass to declare.
#[derive(Clone, Copy)]
struct MeshPass {
    label: &'static str,
    io: MeshPassIo,
    frame_address: u64,
    /// Clears the visibility buffer and the depth; the second pass loads both.
    first: bool,
    /// Tests clusters against the depth pyramid.
    reads_hzb: bool,
}
