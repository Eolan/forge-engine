//! GPU-driven meshlet rendering (`shaders/meshlet.slang`, `shaders/hzb.slang`).
//!
//! [`MeshletSceneBuilder`] concatenates any number of meshlet meshes and instances into the
//! GPU tables; [`MeshletRenderer`] owns the depth buffer, the hierarchical-Z pyramid, the
//! pipelines and the per-slot frame blocks, and records the two-pass occluded draw.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_geom::{GpuMeshlet, GpuVertex, MeshletMesh};
use forge_gpu::{
    Buffer, BufferDesc, Commands, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameSlot, Image,
    ImageDesc, MemoryLocation, MeshPipelineDesc, Pipeline, Result, SampledImageId, ShaderCompiler,
    ShaderStage, StorageImageId, vk,
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
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    frame: u64,
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
            vertices: device.create_buffer_with_data(&self.vertices, usage, "meshlet vertices")?,
            meshlets: device.create_buffer_with_data(&self.meshlets, usage, "meshlets")?,
            meshlet_vertices: device.create_buffer_with_data(
                &self.meshlet_vertices,
                usage,
                "meshlet vertex indices",
            )?,
            meshlet_triangles: device.create_buffer_with_data(
                &self.meshlet_triangles,
                usage,
                "meshlet triangles",
            )?,
            meshes: device.create_buffer_with_data(&self.meshes, usage, "meshes")?,
            instances: device.create_buffer_with_data(&self.instances, usage, "instances")?,
            visibility: device.create_buffer_with_data(
                &vec![0_u32; visibility_words],
                usage,
                "visibility bits",
            )?,
            group_table: device.create_buffer_with_data(
                &self.group_table,
                usage,
                "task group table",
            )?,
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
    visibility: Buffer,
    group_table: Buffer,
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
}

struct DepthResources {
    depth: Image,
    depth_sampled: SampledImageId,
    hzb: Image,
    hzb_sampled: SampledImageId,
    hzb_storage: Vec<StorageImageId>,
}

impl DepthResources {
    fn new(device: &Arc<Device>, extent: vk::Extent2D) -> Result<Self> {
        let depth = device.create_image(ImageDesc {
            width: extent.width,
            height: extent.height,
            format: vk::Format::D32_SFLOAT,
            usage: vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::DEPTH,
            mip_levels: 1,
            name: "depth",
        })?;
        let prev_pow2 = |v: u32| 1_u32 << (31 - v.max(1).leading_zeros());
        let (w, h) = (prev_pow2(extent.width), prev_pow2(extent.height));
        let mips = 32 - w.max(h).leading_zeros();
        let hzb = device.create_image(ImageDesc {
            width: w,
            height: h,
            format: vk::Format::R32_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: mips,
            name: "hzb",
        })?;
        device.initialize_image_layout(
            &hzb,
            vk::ImageAspectFlags::COLOR,
            vk::ImageLayout::GENERAL,
        )?;
        let depth_sampled =
            device.register_sampled_image(depth.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
        let hzb_sampled = device.register_sampled_image(hzb.view(), vk::ImageLayout::GENERAL);
        let hzb_storage = (0..mips)
            .map(|level| device.register_storage_image(hzb.mip_view(level)))
            .collect();
        tracing::info!(hzb_width = w, hzb_height = h, mips, "depth pyramid created");
        Ok(Self {
            depth,
            depth_sampled,
            hzb,
            hzb_sampled,
            hzb_storage,
        })
    }

    fn release(&self, device: &Device) {
        device.release_sampled_image(self.depth_sampled);
        device.release_sampled_image(self.hzb_sampled);
        for &id in &self.hzb_storage {
            device.release_storage_image(id);
        }
    }
}

/// Records the meshlet passes for one scene.
pub struct MeshletRenderer {
    device: Arc<Device>,
    pipeline_solid: Pipeline,
    pipeline_wire: Pipeline,
    pipeline_hzb: Pipeline,
    depth: DepthResources,
    frame_buffers: Vec<Buffer>,
    stats_buffers: Vec<Buffer>,
    /// Direction *to* the sun (world space), used by the fragment shader.
    pub sun_dir: Vec3,
}

/// What to draw this frame.
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
    /// Colour target, already in `COLOR_ATTACHMENT_OPTIMAL`.
    pub color_view: vk::ImageView,
    /// Target size.
    pub extent: vk::Extent2D,
    /// Clear the colour target first, or load what a previous pass drew.
    pub clear_color: Option<[f32; 4]>,
    /// Wireframe.
    pub wireframe: bool,
}

impl MeshletRenderer {
    /// Compiles the pipelines and creates the depth resources for `extent`.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        color_format: vk::Format,
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
        let make_pipeline = |wireframe: bool| {
            device.create_mesh_pipeline(&MeshPipelineDesc {
                task: Some((task, "task_main")),
                mesh: (mesh, "mesh_main"),
                fragment: (frag, "frag_main"),
                color_formats: &[color_format],
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
        for module in [task, mesh, frag, hzb] {
            device.destroy_shader_module(module);
        }
        let depth = DepthResources::new(device, extent)?;
        let frame_buffers = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: FRAME_BLOCK_STRIDE * 2,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
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
                    name: &format!("meshlet stats {i}"),
                })?;
                b.write(0, &[0_u32; STAT_COUNT]);
                Ok(b)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            device: Arc::clone(device),
            pipeline_solid,
            pipeline_wire,
            pipeline_hzb,
            depth,
            frame_buffers,
            stats_buffers,
            sun_dir: Vec3::new(0.4, 1.0, 0.3).normalize(),
        })
    }

    /// Recreates the depth resources. The device must be idle.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        self.depth.release(&self.device);
        self.depth = DepthResources::new(&self.device, extent)?;
        Ok(())
    }

    /// Size of the depth pyramid's level 0.
    pub fn hzb_extent(&self) -> vk::Extent2D {
        self.depth.hzb.extent()
    }

    /// The depth buffer and its sampled-image handle (`DEPTH_ATTACHMENT_OPTIMAL` after `draw`).
    pub fn depth(&self) -> (vk::Image, SampledImageId) {
        (self.depth.depth.raw(), self.depth.depth_sampled)
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
        })
    }

    fn frame_block(&self, slot: FrameSlot, params: &DrawParams<'_>, pass: u32) -> GpuFrame {
        let scene = params.scene;
        let hzb = self.depth.hzb.extent();
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
            hzb_image: self.depth.hzb_sampled.0,
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
        }
    }

    /// Records the passes into `commands`. The colour target must be in
    /// `COLOR_ATTACHMENT_OPTIMAL` and stays there.
    pub fn draw(
        &mut self,
        commands: &Commands<'_>,
        slot: FrameSlot,
        params: &DrawParams<'_>,
    ) -> Result<()> {
        let occlusion = params.flags.has(CullFlags::OCCLUSION);
        let frozen = params.flags.has(CullFlags::FREEZE);
        let first_pass = if occlusion {
            PASS_PREVIOUSLY_VISIBLE
        } else {
            PASS_SINGLE
        };
        let block1 = self.frame_block(slot, params, first_pass);
        let block2 = self.frame_block(slot, params, PASS_REMAINDER);
        self.frame_buffers[slot.index].write(0, &[block1]);
        self.frame_buffers[slot.index].write(FRAME_BLOCK_STRIDE, &[block2]);

        let extent = params.extent;
        let depth_range = vk::ImageSubresourceRange {
            aspect_mask: vk::ImageAspectFlags::DEPTH,
            base_mip_level: 0,
            level_count: 1,
            base_array_layer: 0,
            layer_count: 1,
        };
        let depth_image = self.depth.depth.raw();
        let depth_barrier = || {
            vk::ImageMemoryBarrier2::default()
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .image(depth_image)
                .subresource_range(depth_range)
        };
        let attachment_stages = vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS
            | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
        let attachment_access = vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
            | vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ;

        let color_attachment = |load: vk::AttachmentLoadOp| {
            vk::RenderingAttachmentInfo::default()
                .image_view(params.color_view)
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(load)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    color: vk::ClearColorValue {
                        float32: params.clear_color.unwrap_or([0.0; 4]),
                    },
                })
        };
        let depth_attachment = |load: vk::AttachmentLoadOp| {
            vk::RenderingAttachmentInfo::default()
                .image_view(self.depth.depth.view())
                .image_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                .load_op(load)
                .store_op(vk::AttachmentStoreOp::STORE)
                .clear_value(vk::ClearValue {
                    depth_stencil: vk::ClearDepthStencilValue {
                        depth: 0.0,
                        stencil: 0,
                    },
                })
        };
        let first_color = [color_attachment(if params.clear_color.is_some() {
            vk::AttachmentLoadOp::CLEAR
        } else {
            vk::AttachmentLoadOp::LOAD
        })];
        let second_color = [color_attachment(vk::AttachmentLoadOp::LOAD)];
        let first_depth = depth_attachment(vk::AttachmentLoadOp::CLEAR);
        let second_depth = depth_attachment(vk::AttachmentLoadOp::LOAD);
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };
        let rendering_pass1 = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&first_color)
            .depth_attachment(&first_depth);
        let rendering_pass2 = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&second_color)
            .depth_attachment(&second_depth);

        let pipeline = if params.wireframe {
            &self.pipeline_wire
        } else {
            &self.pipeline_solid
        };
        let frame_address = self.frame_buffers[slot.index].address();
        let task_groups = params.scene.total_groups;

        // Visibility bits written by the previous frame must be visible to this frame's task shader.
        commands.memory_barrier(
            vk::PipelineStageFlags2::TASK_SHADER_EXT,
            vk::AccessFlags2::SHADER_STORAGE_WRITE,
            vk::PipelineStageFlags2::TASK_SHADER_EXT,
            vk::AccessFlags2::SHADER_STORAGE_READ,
        );
        // The previous frame may have sampled the depth (pyramid build, TAA): wait for everything.
        commands.image_barriers(&[depth_barrier()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(attachment_stages)
            .dst_access_mask(attachment_access)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)]);

        commands.begin_rendering(&rendering_pass1);
        commands.bind_pipeline(pipeline);
        commands.set_viewport_full(extent);
        commands.push_constants(
            pipeline,
            &Push {
                frame: frame_address,
            },
        );
        if task_groups > 0 {
            commands.draw_mesh_tasks(task_groups, 1, 1)?;
        }
        commands.end_rendering();
        commands.mark(if occlusion {
            "geometry/meshlet pass 1 (visible last frame)"
        } else {
            "geometry/meshlets (single pass)"
        });

        if occlusion {
            if !frozen {
                commands.image_barriers(&[depth_barrier()
                    .src_stage_mask(attachment_stages)
                    .src_access_mask(vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE)
                    .dst_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                    .dst_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .old_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)]);
                commands.bind_pipeline(&self.pipeline_hzb);
                for level in 0..self.depth.hzb.mip_levels() {
                    let dst = self.depth.hzb.mip_extent(level);
                    let (src_image, src_level) = if level == 0 {
                        (self.depth.depth_sampled.0, 0)
                    } else {
                        (self.depth.hzb_sampled.0, level - 1)
                    };
                    commands.push_constants(
                        &self.pipeline_hzb,
                        &HzbPush {
                            src_image,
                            src_level,
                            dst_image: self.depth.hzb_storage[level as usize].0,
                            dst_width: dst.width,
                            dst_height: dst.height,
                            pad: [0; 3],
                        },
                    );
                    commands.dispatch(dst.width.div_ceil(8), dst.height.div_ceil(8), 1);
                    commands.memory_barrier(
                        vk::PipelineStageFlags2::COMPUTE_SHADER,
                        vk::AccessFlags2::SHADER_STORAGE_WRITE,
                        vk::PipelineStageFlags2::COMPUTE_SHADER
                            | vk::PipelineStageFlags2::TASK_SHADER_EXT,
                        vk::AccessFlags2::SHADER_SAMPLED_READ,
                    );
                }
                commands.mark("geometry/depth pyramid");
                commands.image_barriers(&[depth_barrier()
                    .src_stage_mask(vk::PipelineStageFlags2::COMPUTE_SHADER)
                    .src_access_mask(vk::AccessFlags2::SHADER_SAMPLED_READ)
                    .dst_stage_mask(attachment_stages)
                    .dst_access_mask(attachment_access)
                    .old_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .new_layout(vk::ImageLayout::DEPTH_ATTACHMENT_OPTIMAL)]);
            }
            commands.begin_rendering(&rendering_pass2);
            commands.bind_pipeline(pipeline);
            commands.set_viewport_full(extent);
            commands.push_constants(
                pipeline,
                &Push {
                    frame: frame_address + FRAME_BLOCK_STRIDE,
                },
            );
            if task_groups > 0 {
                commands.draw_mesh_tasks(task_groups, 1, 1)?;
            }
            commands.end_rendering();
            commands.mark("geometry/meshlet pass 2 (newly visible)");
        }
        Ok(())
    }
}

impl Drop for MeshletRenderer {
    fn drop(&mut self) {
        self.depth.release(&self.device);
    }
}
