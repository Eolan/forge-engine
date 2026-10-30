//! D-088's diffuse bounce denoised: the rays [`crate::MeshletRenderer::trace_diffuse`] traces, one
//! a pixel, through NVIDIA's REBLUR (NRD's diffuse denoiser), before the resolve reads it in place
//! of the probes' light, under the occlusion's contacts ([`crate::AmbientLight::diffuse_bounce`]).
//! Half the pixels a frame in a checkerboard cost half as much (1.09 ms of rays at the Bistro's
//! Exterior_1, 2.16 every pixel), but left REBLUR's grain on a stairwell's walls (2026-10-30).
//!
//! REBLUR reads the guides the sun's shadow rays already write for SIGMA (the normal, the view
//! depth: [`crate::meshlet::SunShadowRays`]) and TAA's motion vectors, so it runs where the sun's
//! shadows are denoised; elsewhere the resolve reads the rays' light as traced, every pixel's own.
//! It comes from NRD as SIGMA does ([`forge_gpu::nrd`]): under NVIDIA's RTX SDKs License, never in
//! the repository (`CREDITS.md`, D-049).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_gpu::nrd::{NrdCamera, ReblurDiffuse, ReblurFrame, ReblurImages};
use forge_gpu::{Device, FrameGraph, FrameSlot, ImageHandle, Result, vk};

use crate::meshlet::SunShadowRays;

/// REBLUR's normalisation of the rays' lengths (its `hitDistanceParameters`: A, B, C): a length is
/// read as a share of A + B × the view depth, metres. NRD's defaults; the trace pass divides by
/// the same (`diffuse_trace_main`).
pub const HIT_DISTANCE: [f32; 3] = [3.0, 0.1, 20.0];

/// Frames REBLUR's history accumulates at most, and its responsive history: NRD's defaults (half a
/// second at 60 Hz).
const ACCUMULATED_FRAMES: u32 = 30;
const FAST_ACCUMULATED_FRAMES: u32 = 6;

/// REBLUR loaded for one frame size, and whether its history starts over.
pub struct DiffuseDenoiser {
    device: Arc<Device>,
    reblur: ReblurDiffuse,
    dir: PathBuf,
    reset: bool,
}

impl DiffuseDenoiser {
    /// Loads NRD's REBLUR from `dir` (`nrd-sdk/bin`, or wherever `FORGE_NRD_DIR` points) for frames
    /// of `extent`. Fails without the library, or built without the `nrd` feature.
    pub fn reblur(device: &Arc<Device>, dir: &Path, extent: vk::Extent2D) -> Result<Self> {
        Ok(Self {
            device: Arc::clone(device),
            reblur: ReblurDiffuse::load(device, dir, extent)?,
            dir: dir.to_owned(),
            reset: true,
        })
    }

    /// NRD's version, as the library reports it.
    pub fn version(&self) -> String {
        self.reblur.version()
    }

    /// Recreates the denoiser for a new frame size. The caller makes sure the GPU is done with the
    /// old one's frames.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        if self.reblur.extent() != extent {
            self.reblur = ReblurDiffuse::load(&self.device, &self.dir, extent)?;
        }
        self.reset = true;
        Ok(())
    }

    /// Discards the history at the next frame (a cut, or the bounce turned back on).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// The frame's settings for `camera` (the sun's shadows', [`crate::SunShadowFrame::camera`]).
    pub fn frame(&mut self, camera: NrdCamera) -> ReblurFrame {
        let frame = ReblurFrame {
            camera: NrdCamera {
                reset: camera.reset || self.reset,
                ..camera
            },
            hit_distance: HIT_DISTANCE,
            max_accumulated_frames: ACCUMULATED_FRAMES,
            max_fast_accumulated_frames: FAST_ACCUMULATED_FRAMES,
            checkerboard: false,
        };
        self.reset = false;
        frame
    }

    /// Declares REBLUR's passes over `bounce` (this frame's rays, a pixel each, from
    /// [`crate::MeshletRenderer::trace_diffuse`]), the shadow rays' guides and TAA's `motion`, and
    /// returns the denoised light for [`crate::AmbientLight::diffuse_bounce`].
    pub fn denoise<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        bounce: ImageHandle,
        guides: SunShadowRays,
        motion: ImageHandle,
        frame: &ReblurFrame,
    ) -> Result<ImageHandle> {
        self.reblur.denoise(
            graph,
            slot,
            ReblurImages {
                radiance_hit_distance: bounce,
                normal_roughness: guides.normal_roughness,
                view_z: guides.view_z,
                motion,
            },
            frame,
        )
    }
}
