//! The mirror rays' history (#176, D-050): what the glass's rays add, settled over the frames
//! before TAA sees it.
//!
//! One exact mirror ray a pixel ([`crate::MeshletRenderer::resolve`]'s "shading/reflections")
//! lands somewhere else in the reflected scene with every jitter of TAA. TAA follows the glass,
//! not its reflection, and cannot settle it: the towers' glass shimmered. Here the rays' light
//! is kept apart, per pixel, and each frame's ray is blended into it. The history is reprojected
//! as the *reflection* moves: a flat mirror shows what the ray met as a virtual point behind it,
//! on the pixel's view ray, as far from the eye as the surface plus the ray's length. NVIDIA's NRD
//! and AMD's FidelityFX reflection denoiser reproject mirrors the same way.
//!
//! The work is in `reflections_main` (`shaders/meshlet.slang`); this module owns the two images
//! written and read in turn, and the pass that clears the one about to be written
//! ("shading/reflection history"), since the rays only write the tiles that hold glass.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot, GraphImage, ImageAccess,
    ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Result, vk,
};
use glam::{Mat4, Vec2};

/// The new ray's share of a pixel whose history is whole: about ten frames' average, close to
/// TAA's own 16-frame cycle.
pub const REFLECTION_BLEND: f32 = 0.1;

/// Mirrors `ReflectionHistory` in `meshlet.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ReflectionHistoryBlock {
    previous_from_current: [f32; 16],
    jitter: [f32; 2],
    previous: u32,
    next: u32,
    first_mover: u32,
    blend: f32,
    reset: u32,
    near: f32,
}

const _: () = assert!(std::mem::size_of::<ReflectionHistoryBlock>() == 96);

/// One frame of the history, for [`crate::AmbientLight::reflection_history`].
#[derive(Clone, Copy, Debug)]
pub struct ReflectionHistoryFrame {
    /// The address of the frame's `ReflectionHistory` block.
    pub address: u64,
    /// The previous frame's history.
    pub previous: ImageHandle,
    /// This frame's, cleared.
    pub next: ImageHandle,
}

/// The history's images and per-frame blocks (see the module's documentation).
pub struct ReflectionHistory {
    device: Arc<Device>,
    extent: vk::Extent2D,
    /// What the rays added so far (rgb, before the exposure) and the view depth of the virtual
    /// point they showed (a): written and read in turn.
    images: [GraphImage; 2],
    /// The per-frame blocks, one per frame slot.
    frames: Vec<Buffer>,
    /// The image this frame writes.
    written: usize,
    /// Whether the other image holds a frame's history.
    primed: bool,
    /// The frame about to be declared starts without history.
    reset: bool,
}

fn images(device: &Arc<Device>, extent: vk::Extent2D) -> Result<[GraphImage; 2]> {
    let image = |name: &'static str| {
        GraphImage::new(
            device,
            ImageDesc {
                width: extent.width,
                height: extent.height,
                format: vk::Format::R16G16B16A16_SFLOAT,
                usage: vk::ImageUsageFlags::STORAGE
                    | vk::ImageUsageFlags::SAMPLED
                    | vk::ImageUsageFlags::TRANSFER_DST,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name,
            },
        )
    };
    Ok([
        image("reflection history 0")?,
        image("reflection history 1")?,
    ])
}

impl ReflectionHistory {
    /// Builds the history for frames of `extent`.
    pub fn new(device: &Arc<Device>, extent: vk::Extent2D) -> Result<Self> {
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<ReflectionHistoryBlock>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("reflection history frame {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            device: Arc::clone(device),
            extent,
            images: images(device, extent)?,
            frames,
            written: 0,
            primed: false,
            reset: true,
        })
    }

    /// Recreates the images for a new frame size (the GPU done with the old one's frames).
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        if self.extent != extent {
            self.images = images(&self.device, extent)?;
            self.extent = extent;
            self.primed = false;
        }
        Ok(())
    }

    /// Drops the history (a cut, a frame drawn without it): the next frame starts afresh.
    pub fn reset_history(&mut self) {
        self.primed = false;
    }

    /// Turns the history over: call once a frame, before [`ReflectionHistory::frame`].
    pub fn advance(&mut self) {
        self.written = 1 - self.written;
        self.reset = !self.primed;
        self.primed = true;
    }

    /// Declares "shading/reflection history", which clears the image this frame's rays write,
    /// and returns the frame for the resolve. `previous_from_current` and `jitter` are TAA's
    /// ([`crate::TaaFrame`]), `near` the reversed-Z projection's near plane (depth = `near` / view
    /// depth), `first_mover` the scene's first mover's instance ([`crate::MeshletScene::first_mover`]):
    /// glass on a mover keeps each frame's ray.
    pub fn frame<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        previous_from_current: Mat4,
        jitter: Vec2,
        near: f32,
        first_mover: Option<u32>,
    ) -> ReflectionHistoryFrame {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let next = graph.import(&self.images[self.written]);
        let previous = graph.import(&self.images[1 - self.written]);
        let block = &self.frames[slot.index];
        let reset = self.reset;
        graph
            .pass("shading/reflection history")
            .image(next, ImageAccess::TransferDst)
            .image(previous, ImageAccess::Sampled(compute))
            .run(move |resources, commands| {
                block.write(
                    0,
                    &[ReflectionHistoryBlock {
                        previous_from_current: previous_from_current.to_cols_array(),
                        jitter: jitter.to_array(),
                        previous: resources.sampled(previous).0,
                        next: resources.storage(next, 0).0,
                        first_mover: first_mover.unwrap_or(u32::MAX),
                        blend: REFLECTION_BLEND,
                        reset: reset as u32,
                        near,
                    }],
                );
                commands.clear_color_image(resources.image(next).raw);
                Ok(())
            });
        ReflectionHistoryFrame {
            address: block.address(),
            previous,
            next,
        }
    }
}
