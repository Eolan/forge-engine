//! Picking (#409): what a pixel of the camera's view shows, for the question "what is that?".
//!
//! After the resolve a compute pass (`pick_main` in `meshlet.slang`) reads the visibility buffer
//! at up to [`MAX_PICKS`] pixels and writes, for each, the instance drawn there, its mesh, the
//! material row its triangle takes, how far it is and the detail level of its cluster. The frame
//! slot's copy is read when the slot comes round again, two or three frames later.
//!
//! **What it doesn't see:** what is drawn after the visibility buffer and over it: decals, glass,
//! holograms, particles, water. A pixel one of those covers names the surface behind it.

use std::cell::RefCell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphBuffer, ImageAccess, ImageHandle, MemoryCategory, MemoryLocation, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::Vec2;

/// The pixels one request names at most.
pub const MAX_PICKS: usize = 8;
/// The `u32`s `pick_main` writes a pixel.
const RECORD: usize = 8;
/// `VISIBILITY_EMPTY` in `meshlet.slang`: no surface.
const EMPTY: u32 = u32::MAX;
/// `VISIBILITY_GRASS_BIT` in `meshlet.slang`: a grass blade's id.
const GRASS_BIT: u32 = 0x8000_0000;

/// Must match `PickPush` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
struct PickPush {
    frame: u64,
    out: u64,
    vis_image: u32,
    width: u32,
    height: u32,
    count: u32,
    /// x | y << 16, pixels of the visibility buffer.
    pixels: [u32; MAX_PICKS],
}

/// What a pixel of the camera's view shows ([`super::MeshletRenderer::request_picks`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Picked {
    /// Where it was asked: fractions of the view from its top left corner.
    pub at: Vec2,
    /// What is drawn there.
    pub what: PickedWhat,
}

/// What a picked pixel shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PickedWhat {
    /// No surface: the sky.
    Nothing,
    /// A grass blade.
    Grass {
        /// Metres from the camera.
        distance: f32,
    },
    /// An instance of a mesh.
    Instance(PickedInstance),
}

/// The instance a picked pixel shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickedInstance {
    /// Its index in the scene: the order it was added in, or its slot in a dynamic scene.
    pub instance: u32,
    /// Its mesh's index in the scene.
    pub mesh: u32,
    /// The row of the scene's material table its triangle takes: the instance's own plus the
    /// triangle's section.
    pub material: u32,
    /// The triangle's material section in its mesh.
    pub section: u32,
    /// The detail level of the cluster drawn (0 the finest).
    pub lod_level: u32,
    /// The instance's flags, as the shaders have them.
    pub flags: u32,
    /// Metres from the camera.
    pub distance: f32,
}

/// The requests' way through the frames.
#[derive(Default)]
struct State {
    /// Asked, not recorded yet.
    asked: Option<Vec<Vec2>>,
    /// Recorded by the frame that last used each slot.
    recorded: [Option<Vec<Vec2>>; FRAMES_IN_FLIGHT],
    /// Read back, not taken yet.
    done: Option<Vec<Picked>>,
}

/// The picking pass, its buffers and the requests on their way.
pub(crate) struct Picker {
    pipeline: Pipeline,
    /// What `pick_main` writes (device-local).
    out: GraphBuffer,
    /// Its copy per frame slot, for the host.
    readback: Vec<GraphBuffer>,
    state: RefCell<State>,
}

impl Picker {
    /// The picker, its entry compiled from `shading` (the renderer's shader file).
    pub(crate) fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        shading: &'static str,
    ) -> Result<Self> {
        let entry = "pick_main";
        let module = device.create_shader_module(
            &shaders.compile(shading, entry, ShaderStage::Compute)?,
            "picking",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, entry),
            push_constant_bytes: std::mem::size_of::<PickPush>() as u32,
            name: "picking",
        });
        device.destroy_shader_module(module);
        let pipeline = pipeline?;
        let bytes = (MAX_PICKS * RECORD * 4) as u64;
        let out = GraphBuffer::new(device.create_buffer(BufferDesc {
            size: bytes,
            usage: vk::BufferUsageFlags::STORAGE_BUFFER | vk::BufferUsageFlags::TRANSFER_SRC,
            location: MemoryLocation::GpuOnly,
            category: MemoryCategory::Work,
            name: "picks",
        })?);
        let readback = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                Ok(GraphBuffer::new(device.create_buffer(BufferDesc {
                    size: bytes,
                    usage: vk::BufferUsageFlags::TRANSFER_DST,
                    location: MemoryLocation::GpuToCpu,
                    category: MemoryCategory::Transfer,
                    name: &format!("picks readback {i}"),
                })?))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline,
            out,
            readback,
            state: RefCell::default(),
        })
    }

    /// Asks what the pixels at `at` show (fractions of the view from its top left, the first
    /// [`MAX_PICKS`]), in place of a request not recorded yet.
    pub(crate) fn request(&self, at: &[Vec2]) {
        self.state.borrow_mut().asked = Some(at.iter().take(MAX_PICKS).copied().collect());
    }

    /// The last request's answer, once: none until its frame's slot has come round again.
    pub(crate) fn take_done(&self) -> Option<Vec<Picked>> {
        self.state.borrow_mut().done.take()
    }

    /// The frame's passes, after the resolve, when something was asked: the pixels read from the
    /// visibility buffer `visibility` (the frame block at `frame`) and copied into the slot's
    /// readback.
    pub(crate) fn record<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        frame: u64,
        visibility: ImageHandle,
        extent: vk::Extent2D,
    ) {
        let mut state = self.state.borrow_mut();
        let Some(asked) = state.asked.take() else {
            return;
        };
        let mut pixels = [0u32; MAX_PICKS];
        for (pixel, at) in pixels.iter_mut().zip(&asked) {
            *pixel = pixel_of(*at, extent);
        }
        let count = asked.len() as u32;
        state.recorded[slot.index] = Some(asked);
        drop(state);
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let out_buffer: &'f GraphBuffer = &self.out;
        let readback_buffer: &'f GraphBuffer = &self.readback[slot.index];
        let out = graph.import_buffer(out_buffer);
        let readback = graph.import_buffer(readback_buffer);
        let bytes = out_buffer.size();
        let pipeline = &self.pipeline;
        graph
            .pass("debug/pick")
            .image(visibility, ImageAccess::Sampled(compute))
            .buffer(out, BufferAccess::ShaderReadWrite(compute))
            .run(move |resources, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(
                    pipeline,
                    &PickPush {
                        frame,
                        out: out_buffer.address(),
                        vis_image: resources.sampled(visibility).0,
                        width: extent.width,
                        height: extent.height,
                        count,
                        pixels,
                    },
                );
                // A thread a pixel.
                commands.dispatch(1, 1, 1);
                Ok(())
            });
        graph
            .pass("debug/pick copy")
            .buffer(out, BufferAccess::TransferSrc)
            .buffer(readback, BufferAccess::TransferDst)
            .run(move |_, commands| {
                commands.copy_buffer(out_buffer, readback_buffer, bytes);
                Ok(())
            });
        graph
            .pass("debug/pick copy")
            .buffer(readback, BufferAccess::HostRead)
            .run(|_, _| Ok(()));
    }

    /// Reads what the frame that last used `slot` recorded (done by now), if it recorded.
    pub(crate) fn begin_frame(&self, slot: FrameSlot) {
        let mut state = self.state.borrow_mut();
        let Some(asked) = state.recorded[slot.index].take() else {
            return;
        };
        let mut raw = [0u32; MAX_PICKS * RECORD];
        self.readback[slot.index].read(0, &mut raw);
        state.done = Some(
            asked
                .iter()
                .zip(raw.as_chunks::<RECORD>().0)
                .map(|(&at, r)| Picked {
                    at,
                    what: decode(r),
                })
                .collect(),
        );
    }
}

/// The visibility buffer's pixel at `at` (fractions of the view), x | y << 16.
fn pixel_of(at: Vec2, extent: vk::Extent2D) -> u32 {
    let along = |f: f32, size: u32| {
        ((f.clamp(0.0, 1.0) * size as f32) as u32).min(size.saturating_sub(1)) & 0xFFFF
    };
    along(at.x, extent.width) | along(at.y, extent.height) << 16
}

/// One pixel's record, as `pick_main` writes it.
fn decode(r: &[u32; RECORD]) -> PickedWhat {
    let distance = f32::from_bits(r[5]);
    match r[0] {
        EMPTY => PickedWhat::Nothing,
        vis if vis & GRASS_BIT != 0 => PickedWhat::Grass { distance },
        _ => PickedWhat::Instance(PickedInstance {
            instance: r[1],
            mesh: r[2],
            material: r[3],
            section: r[4],
            lod_level: r[7],
            flags: r[6],
            distance,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fraction_of_the_view_is_a_pixel_inside_it() {
        let extent = vk::Extent2D {
            width: 1707,
            height: 960,
        };
        assert_eq!(pixel_of(Vec2::new(0.0, 0.0), extent), 0);
        assert_eq!(pixel_of(Vec2::new(0.5, 0.5), extent), 853 | 480 << 16);
        // The far edge and past it: the last pixel, never one outside.
        assert_eq!(pixel_of(Vec2::new(1.0, 1.0), extent), 1706 | 959 << 16);
        assert_eq!(pixel_of(Vec2::new(7.0, -3.0), extent), 1706);
    }

    #[test]
    fn a_record_says_sky_grass_or_an_instance() {
        let mut r = [0u32; RECORD];
        r[0] = EMPTY;
        assert_eq!(decode(&r), PickedWhat::Nothing);
        r[0] = GRASS_BIT | 5 << 10;
        r[5] = 12.5f32.to_bits();
        assert_eq!(decode(&r), PickedWhat::Grass { distance: 12.5 });
        let r = [9 << 7 | 3, 41, 7, 108, 2, 30.0f32.to_bits(), 512, 4];
        assert_eq!(
            decode(&r),
            PickedWhat::Instance(PickedInstance {
                instance: 41,
                mesh: 7,
                material: 108,
                section: 2,
                lod_level: 4,
                flags: 512,
                distance: 30.0,
            })
        );
    }
}
