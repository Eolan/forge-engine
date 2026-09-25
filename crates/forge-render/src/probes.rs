//! Diffuse light from probes updated by ray queries (issue #53, D-008's T1 tier,
//! `shaders/probes.slang`): DDGI after Majercik et al. 2019 and 2021, written from the papers.
//!
//! Cascades of probes follow the camera, each twice as coarse as the one before. Every probe
//! keeps two octahedral maps in an atlas: the irradiance its rays measured (6 × 6 texels) and
//! the mean and mean square of the distance to what they met (14 × 14), each with a
//! one-texel border for bilinear filtering. Three graph passes a frame, after the sky's tables
//! and before the resolve:
//! - `gi/probe rays`: every probe's rays against the scene's TLAS, a thread per ray. Hits are
//!   lit by the sun (a shadow ray) and by the probes as the frame before left them, so the
//!   light bounces once more every frame; misses take the sky-view table.
//! - `gi/probe state`: from the 32 rays whose directions never change, each probe moves out
//!   of walls and away from surfaces it nearly touches, and turns inactive where it is inside
//!   something or has nothing within a cell of it.
//! - `gi/probe blend`: the other rays blended into the maps, keeping `hysteresis` of what
//!   they held; a probe new to its cascade (it scrolled in) starts from its first rays.
//!
//! The resolve then lights every pixel's diffuse side with the probes around it
//! ([`crate::AmbientLight::probes`]) instead of the open sky's irradiance.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, BufferHandle, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT,
    FrameGraph, FrameSlot, GraphBuffer, GraphImage, ImageAccess, ImageDesc, ImageHandle,
    MemoryCategory, MemoryLocation, Pipeline, Result, ShaderCompiler, ShaderStage, vk,
};
use glam::{IVec3, Mat3, Quat, Vec3};

use crate::sky::SkyLight;

/// Most cascades (`PROBE_MAX_CASCADES` in `probes.slang`).
pub const MAX_CASCADES: usize = 6;
/// Rays per probe whose directions never change (`PROBE_FIXED_RAYS`).
const FIXED_RAYS: u32 = 32;
/// Most rays per probe (`PROBE_MAX_RAYS`).
const MAX_RAYS: u32 = 256;
/// Texels a side of a probe's maps with their borders (`IRRADIANCE_TILE`, `DISTANCE_TILE`).
const IRRADIANCE_TILE: u32 = 8;
const DISTANCE_TILE: u32 = 16;
/// Rays a workgroup of `gi/probe rays` (`PROBE_TRACE_GROUP` in `meshlet.slang`).
const TRACE_GROUP: u32 = 64;
/// Workgroups along one dimension of a dispatch (the shaders spread probes over two).
const MAX_GROUPS: u32 = 65535;
/// `probe_data`'s w for an inactive probe (`PROBE_INACTIVE`).
const PROBE_INACTIVE: f32 = 1.0;

/// Mirrors `ProbeCascade` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuCascade {
    origin: [i32; 3],
    spacing: f32,
    previous: [i32; 3],
    pad: u32,
}

/// Mirrors `ProbeField` in `probes.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuProbeField {
    rotation: [[f32; 4]; 3],
    cascades: [GpuCascade; MAX_CASCADES],
    counts: [u32; 3],
    cascade_count: u32,
    camera: [f32; 3],
    rays: u32,
    ray_data: u64,
    probe_data: u64,
    irradiance_image: u32,
    distance_image: u32,
    irradiance_storage: u32,
    distance_storage: u32,
    irradiance_texel: [f32; 2],
    distance_texel: [f32; 2],
    hysteresis: f32,
    pad0: u32,
    pad1: [u32; 2],
}

const _: () = assert!(std::mem::size_of::<GpuProbeField>() == 336);

/// Mirrors `ProbeTracePush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct TracePush {
    frame: u64,
    field: u64,
    sky: u64,
    probe_count: u32,
    pad: u32,
}

/// Mirrors `Push` in `probe_update.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UpdatePush {
    field: u64,
    probe_count: u32,
    pad: u32,
}

/// The probes' layout and update.
#[derive(Clone, Copy, Debug)]
pub struct ProbeParams {
    /// Probes per cascade along x, y (up) and z, each even.
    pub counts: [u32; 3],
    /// Cascades, each twice as coarse as the one before (at most [`MAX_CASCADES`]).
    pub cascades: u32,
    /// Metres between the finest cascade's probes.
    pub spacing: f32,
    /// Rays per probe and frame, the first 32 with fixed directions (64 to 256).
    pub rays: u32,
    /// Share of the maps an update keeps: 0.97 lets a change settle in about 30 updates.
    pub hysteresis: f32,
}

impl Default for ProbeParams {
    /// The city's: 24 × 12 × 24 probes in five cascades 4, 8, 16, 32 and 64 m apart (the
    /// finest reaches 40 m around the camera, the coarsest 640 m), 128 rays each.
    fn default() -> Self {
        Self {
            counts: [24, 12, 24],
            cascades: 5,
            spacing: 4.0,
            rays: 128,
            hysteresis: 0.97,
        }
    }
}

impl ProbeParams {
    /// Probes in all the cascades.
    pub fn probe_count(&self) -> u32 {
        self.counts.iter().product::<u32>() * self.cascades
    }

    /// The atlas of maps `tile` texels a side: a row of x × y tiles per z of every cascade.
    fn atlas_size(&self, tile: u32) -> [u32; 2] {
        [
            self.counts[0] * self.counts[1] * tile,
            self.counts[2] * self.cascades * tile,
        ]
    }
}

/// What the resolve reads of this frame's probes (see [`crate::AmbientLight`]).
#[derive(Clone, Copy, Debug)]
pub struct ProbeLight {
    /// The frame's `ProbeField` (`probes.slang`).
    pub(crate) address: u64,
    pub(crate) data: BufferHandle,
    pub(crate) irradiance: ImageHandle,
    pub(crate) distance: ImageHandle,
}

/// The world cell of a cascade's first probe: the camera rounded to the cascade's cells, less
/// half the counts, so the probes always reach (counts / 2 − 1) cells from the camera
/// whichever way it sits in its cell (`cascade_weight` in `probes.slang` relies on it).
fn cascade_origin(camera: Vec3, spacing: f32, counts: [u32; 3]) -> IVec3 {
    let cell = (camera / spacing).round();
    IVec3::new(cell.x as i32, cell.y as i32, cell.z as i32)
        - IVec3::new(counts[0] as i32, counts[1] as i32, counts[2] as i32) / 2
}

/// A uniformly random rotation from `index` alone (reproducible captures): three
/// numbers from a hash of the index, turned into a unit quaternion (Shoemake, "Uniform Random
/// Rotations", Graphics Gems III, 1992).
fn frame_rotation(index: u64) -> Mat3 {
    let mut state = index.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut next = || {
        // splitmix64
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    };
    let (u1, u2, u3) = (next(), next(), next());
    let tau = std::f32::consts::TAU;
    let (a, b) = ((1.0 - u1).sqrt(), u1.sqrt());
    Mat3::from_quat(
        Quat::from_xyzw(
            a * (tau * u2).sin(),
            a * (tau * u2).cos(),
            b * (tau * u3).sin(),
            b * (tau * u3).cos(),
        )
        .normalize(),
    )
}

/// The probes: their maps, their rays and state, and the passes that update them.
pub struct Probes {
    params: ProbeParams,
    trace: Pipeline,
    state: Pipeline,
    blend: Pipeline,
    irradiance: GraphImage,
    distance: GraphImage,
    rays: GraphBuffer,
    data: GraphBuffer,
    fields: Vec<Buffer>,
    /// Per cascade, the origin of the last update.
    origins: [IVec3; MAX_CASCADES],
    /// Every probe starts over at the next update.
    reset: bool,
}

impl Probes {
    /// Compiles the passes (`gi/probe rays` needs ray queries) and creates the atlases, the
    /// rays' buffer and the probes' state, every probe inactive until its first update.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        params: ProbeParams,
    ) -> Result<Self> {
        assert!(
            params.cascades >= 1 && params.cascades as usize <= MAX_CASCADES,
            "1 to {MAX_CASCADES} cascades"
        );
        assert!(
            params.counts.iter().all(|&n| n >= 4 && n % 2 == 0),
            "even counts of at least 4"
        );
        assert!(
            (FIXED_RAYS + 32..=MAX_RAYS).contains(&params.rays),
            "64 to 256 rays"
        );
        let compute = |file: &str, entry: &str, push: usize, name: &str| -> Result<Pipeline> {
            let module = device
                .create_shader_module(&shaders.compile(file, entry, ShaderStage::Compute)?, name)?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: push as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let atlas = |tile: u32, format: vk::Format, name: &str| {
            let [width, height] = params.atlas_size(tile);
            GraphImage::new(
                device,
                ImageDesc {
                    width,
                    height,
                    format,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let probes = u64::from(params.probe_count());
        let rays = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: probes * u64::from(params.rays) * 8,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "probe rays",
        })?);
        let data = device.create_buffer(BufferDesc {
            size: probes * 16,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER
                | vk::BufferUsageFlags::TRANSFER_DST
                | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "probe state",
        })?;
        let inactive = vec![[0.0, 0.0, 0.0, PROBE_INACTIVE]; probes as usize];
        device.write_buffer_staged(&data, 0, bytemuck::cast_slice(&inactive))?;
        let fields = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuProbeField>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("probe field {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            params,
            trace: compute(
                "meshlet.slang",
                "probe_trace_main",
                std::mem::size_of::<TracePush>(),
                "probe rays",
            )?,
            state: compute(
                "probe_update.slang",
                "probe_state_main",
                std::mem::size_of::<UpdatePush>(),
                "probe state",
            )?,
            blend: compute(
                "probe_update.slang",
                "probe_blend_main",
                std::mem::size_of::<UpdatePush>(),
                "probe blend",
            )?,
            irradiance: atlas(
                IRRADIANCE_TILE,
                vk::Format::R16G16B16A16_SFLOAT,
                "probe irradiance",
            )?,
            distance: atlas(DISTANCE_TILE, vk::Format::R16G16_SFLOAT, "probe distance")?,
            rays,
            data: GraphBuffer::new(data),
            fields,
            origins: [IVec3::ZERO; MAX_CASCADES],
            reset: true,
        })
    }

    /// The layout and update this was made with.
    pub fn params(&self) -> &ProbeParams {
        &self.params
    }

    /// Bytes of the atlases, the rays and the probes' state.
    pub fn bytes(&self) -> u64 {
        let texels = |tile: u32| {
            let [w, h] = self.params.atlas_size(tile);
            u64::from(w) * u64::from(h)
        };
        texels(IRRADIANCE_TILE) * 8
            + texels(DISTANCE_TILE) * 4
            + self.rays.size()
            + self.data.size()
    }

    /// Every probe's offset from its cell's centre (xyz, metres) and state and age (w: the
    /// state, 0 active, 1 inactive, 2 active and starting over, plus 4 × the updates it has
    /// had, up to 8, `probe_state` in `probes.slang`) as the last submitted frame left them, in probe
    /// order (`probe_index` in `probes.slang`). Waits for the device: for logs and tests.
    pub fn read_states(&self, device: &Arc<Device>) -> Result<Vec<[f32; 4]>> {
        let bytes = device.read_back(&self.data, 0, self.data.size())?;
        Ok(bytemuck::cast_slice(&bytes).to_vec())
    }

    /// Starts every probe over at the next update (after the probes were off for a while, or
    /// the camera jumped).
    pub fn reset(&mut self) {
        self.reset = true;
    }

    /// Declares this frame's three passes around the camera at `camera` (world metres), with
    /// the scene of the renderer's frame block at `frame` (its TLAS, instances and materials,
    /// [`crate::MeshletRenderer::frame_address`]) and the sky's light and table, and returns
    /// what the resolve reads.
    ///
    /// `noise_frame` picks the rays' rotation. The city passes TAA's frame modulo its jitter's
    /// period, as for the ambient occlusion (issue #48): the rotations then repeat with the
    /// jitter, the maps of a still scene settle into the same cycle and TAA averages it, where
    /// a new rotation every frame kept them wandering (measured: the static view's slow change
    /// 0.27 % of pixels, 0.09 % without probes).
    pub fn update<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        sky: SkyLight,
        camera: Vec3,
        noise_frame: u64,
    ) -> ProbeLight {
        let p = self.params;
        let mut cascades = [GpuCascade::zeroed(); MAX_CASCADES];
        for (c, cascade) in cascades.iter_mut().enumerate().take(p.cascades as usize) {
            let spacing = p.spacing * (1u32 << c) as f32;
            let origin = cascade_origin(camera, spacing, p.counts);
            // After a reset, a previous origin two volumes away: every probe is new.
            let previous = if self.reset {
                origin + 2 * IVec3::new(p.counts[0] as i32, p.counts[1] as i32, p.counts[2] as i32)
            } else {
                self.origins[c]
            };
            self.origins[c] = origin;
            *cascade = GpuCascade {
                origin: origin.to_array(),
                spacing,
                previous: previous.to_array(),
                pad: 0,
            };
        }
        self.reset = false;
        let rotation = frame_rotation(noise_frame);
        let row = |r: usize| {
            let v = rotation.row(r);
            [v.x, v.y, v.z, 0.0]
        };
        let texel = |tile: u32| {
            let [w, h] = p.atlas_size(tile);
            [1.0 / w as f32, 1.0 / h as f32]
        };
        let field = GpuProbeField {
            rotation: [row(0), row(1), row(2)],
            cascades,
            counts: p.counts,
            cascade_count: p.cascades,
            camera: camera.to_array(),
            rays: p.rays,
            ray_data: self.rays.address(),
            probe_data: self.data.address(),
            irradiance_image: self.irradiance.sampled().0,
            distance_image: self.distance.sampled().0,
            irradiance_storage: self.irradiance.storage(0).0,
            distance_storage: self.distance.storage(0).0,
            irradiance_texel: texel(IRRADIANCE_TILE),
            distance_texel: texel(DISTANCE_TILE),
            hysteresis: p.hysteresis,
            pad0: 0,
            pad1: [0; 2],
        };
        self.fields[slot.index].write(0, &[field]);

        let this: &'f Self = self;
        let address = this.fields[slot.index].address();
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let rays = graph.import_buffer(&this.rays);
        let data = graph.import_buffer(&this.data);
        let irradiance = graph.import(&this.irradiance);
        let distance = graph.import(&this.distance);
        let count = p.probe_count();
        let spread = [count.min(MAX_GROUPS), count.div_ceil(MAX_GROUPS)];
        let trace = &this.trace;
        graph
            .pass("gi/probe rays")
            .buffer(sky.buffer, BufferAccess::ShaderRead(compute))
            .image(sky.table, ImageAccess::Sampled(compute))
            .image(irradiance, ImageAccess::Sampled(compute))
            .image(distance, ImageAccess::Sampled(compute))
            .buffer(data, BufferAccess::ShaderRead(compute))
            .buffer(rays, BufferAccess::ShaderWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(trace);
                commands.push_constants(
                    trace,
                    &TracePush {
                        frame,
                        field: address,
                        sky: sky.address,
                        probe_count: count,
                        pad: 0,
                    },
                );
                commands.dispatch(p.rays.div_ceil(TRACE_GROUP), spread[0], spread[1]);
                Ok(())
            });
        let push = UpdatePush {
            field: address,
            probe_count: count,
            pad: 0,
        };
        let state = &this.state;
        graph
            .pass("gi/probe state")
            .buffer(rays, BufferAccess::ShaderRead(compute))
            .buffer(data, BufferAccess::ShaderReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(state);
                commands.push_constants(state, &push);
                commands.dispatch(count.div_ceil(64), 1, 1);
                Ok(())
            });
        let blend = &this.blend;
        graph
            .pass("gi/probe blend")
            .buffer(rays, BufferAccess::ShaderRead(compute))
            .buffer(data, BufferAccess::ShaderRead(compute))
            .image(irradiance, ImageAccess::StorageReadWrite(compute))
            .image(distance, ImageAccess::StorageReadWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(blend);
                commands.push_constants(blend, &push);
                commands.dispatch(spread[0], spread[1], 1);
                Ok(())
            });
        ProbeLight {
            address,
            data,
            irradiance,
            distance,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `oct_encode` in `probes.slang`.
    fn oct_encode(d: Vec3) -> [f32; 2] {
        let l1 = d.x.abs() + d.y.abs() + d.z.abs();
        let (x, z) = (d.x / l1, d.z / l1);
        let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        if d.y >= 0.0 {
            [x, z]
        } else {
            [(1.0 - z.abs()) * sign(x), (1.0 - x.abs()) * sign(z)]
        }
    }

    /// `oct_decode` in `probes.slang`.
    fn oct_decode(p: [f32; 2]) -> Vec3 {
        let h = 1.0 - p[0].abs() - p[1].abs();
        let sign = |v: f32| if v >= 0.0 { 1.0 } else { -1.0 };
        let q = if h >= 0.0 {
            p
        } else {
            [
                (1.0 - p[1].abs()) * sign(p[0]),
                (1.0 - p[0].abs()) * sign(p[1]),
            ]
        };
        Vec3::new(q[0], h, q[1]).normalize()
    }

    /// `border_source` in `probes.slang`.
    fn border_source(t: [u32; 2], texels: u32) -> [u32; 2] {
        let last = texels + 1;
        let (left, right, top, bottom) = (t[0] == 0, t[0] == last, t[1] == 0, t[1] == last);
        let c = if (left || right) && (top || bottom) {
            [if left { texels } else { 1 }, if top { texels } else { 1 }]
        } else if top || bottom {
            [last - t[0], if top { 1 } else { texels }]
        } else if left || right {
            [if left { 1 } else { texels }, last - t[1]]
        } else {
            t
        };
        [c[0] - 1, c[1] - 1]
    }

    #[test]
    fn the_octahedral_map_round_trips() {
        for i in 0..200 {
            let d = Vec3::new(
                (i as f32 * 0.37).sin(),
                (i as f32 * 0.91).cos(),
                (i as f32 * 1.73).sin(),
            )
            .normalize();
            let back = oct_decode(oct_encode(d));
            assert!((back - d).length() < 1e-5, "{d} → {back}");
        }
        assert!((oct_decode([0.0, 0.0]) - Vec3::Y).length() < 1e-6);
        assert!((oct_decode([1.0, 1.0]) + Vec3::Y).length() < 1e-6);
    }

    /// A border texel holds the interior texel nearest to where the map continues across its
    /// edge: across an edge the octahedron folds, (u, v) beyond u = 1 being (2 − u, −v).
    #[test]
    fn a_border_texel_holds_the_direction_just_across_its_edge() {
        for texels in [6u32, 14] {
            let tile = texels + 2;
            let to_p = |t: f32| (t - 1.0) / texels as f32 * 2.0 - 1.0; // tile coordinate → [-1, 1]
            let wrap = |v: f32, w: f32| -> [f32; 2] {
                // Fold a coordinate beyond ±1 back: the other coordinate changes sign.
                if v > 1.0 {
                    [2.0 - v, -w]
                } else if v < -1.0 {
                    [-2.0 - v, -w]
                } else {
                    [v, w]
                }
            };
            for y in 0..tile {
                for x in 0..tile {
                    let src = border_source([x, y], texels);
                    let (px, py) = (to_p(x as f32 + 0.5), to_p(y as f32 + 0.5));
                    let [px, py] = wrap(px, py);
                    let [py, px] = wrap(py, px);
                    let want = oct_decode([px, py]);
                    let got = oct_decode([
                        (src[0] as f32 + 0.5) / texels as f32 * 2.0 - 1.0,
                        (src[1] as f32 + 0.5) / texels as f32 * 2.0 - 1.0,
                    ]);
                    assert!(
                        (want - got).length() < 1e-4,
                        "tile {tile}: ({x}, {y}) holds {src:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_cascade_reaches_past_the_fade_whatever_the_camera() {
        let counts = [24, 12, 24];
        for i in 0..1000 {
            let camera = Vec3::new(
                i as f32 * 1.37 - 700.0,
                i as f32 * 0.29 - 60.0,
                i as f32 * -2.11,
            );
            for spacing in [4.0, 8.0, 64.0] {
                let origin = cascade_origin(camera, spacing, counts);
                for a in 0..3 {
                    let n = counts[a] as f32;
                    let first = (origin[a] as f32 + 0.5) * spacing;
                    let last = (origin[a] as f32 + n - 0.5) * spacing;
                    let reach = (n * 0.5 - 1.0) * spacing;
                    assert!(first <= camera[a] - reach + 1e-3 && last >= camera[a] + reach - 1e-3);
                }
            }
        }
    }

    #[test]
    fn the_frame_rotation_is_a_rotation_and_changes() {
        let a = frame_rotation(0);
        let b = frame_rotation(1);
        assert!((a.determinant() - 1.0).abs() < 1e-5);
        assert!((a * a.transpose() - Mat3::IDENTITY).abs_diff_eq(Mat3::ZERO, 1e-5));
        assert!(!a.abs_diff_eq(b, 1e-3));
        assert!(a.abs_diff_eq(frame_rotation(0), 0.0));
    }
}
