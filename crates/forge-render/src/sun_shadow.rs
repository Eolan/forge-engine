//! The sun's soft shadows denoised by NVIDIA's SIGMA (issue #172, D-049).
//!
//! [`crate::MeshletRenderer::trace_sun_shadow`] traces one ray a pixel to a point of the sun's
//! disc and keeps the closest occluder; SIGMA (NVIDIA's NRD, loaded at run time by
//! [`forge_gpu::nrd`]) blurs the binary visibility over the penumbra that distance gives and
//! stabilises it over a few frames; the resolve multiplies the result into the sun's light in
//! place of its own ray ([`crate::AmbientLight::sun_shadow`]). Without NRD (a fresh clone, CI)
//! nothing here is created and the resolve keeps its ray, pixel for pixel as before.
//!
//! A game that ships NRD ships it under NVIDIA's RTX SDKs License: object code only, under
//! terms at least as protective of NVIDIA as its own (`CREDITS.md`, D-049).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_gpu::nrd::{Sigma, SigmaFrame, SigmaImages};
use forge_gpu::{Device, FrameGraph, FrameSlot, ImageHandle, Result, vk};
use glam::{DMat4, Mat4, Vec2, Vec3, Vec4};

use crate::CellPos;
use crate::meshlet::SunShadowRays;

/// The sun's apparent radius for the soft shadows when SIGMA denoises them, radians: wider
/// than the real 0.27° (`starfield::SUN_ANGULAR_RADIUS_1AU`) for art's sake (D-049), so a
/// penumbra reads under an exposure metered for the shade. Judged in the models lab (#172).
pub const DENOISED_SUN_RADIUS: f32 = 1.0 * std::f32::consts::PI / 180.0;

/// The camera of one frame, as the denoiser needs it.
#[derive(Clone, Copy, Debug)]
pub struct SunShadowCamera {
    /// Camera-relative world to view: the camera's rotation alone.
    pub view: Mat4,
    /// View to clip, without jitter.
    pub projection: Mat4,
    /// Where the camera stands.
    pub position: CellPos,
    /// The frame's jitter in pixels, as the projection moved the scene (x right, y down).
    pub jitter: Vec2,
}

/// SIGMA and the previous frame's camera (see the module's documentation).
pub struct SunShadowDenoiser {
    device: Arc<Device>,
    dir: PathBuf,
    sigma: Sigma,
    previous: Option<SunShadowCamera>,
    reset: bool,
}

impl SunShadowDenoiser {
    /// Loads NRD from `dir` (`nrd-sdk/bin`, or wherever `FORGE_NRD_DIR` points) for frames of
    /// `extent`. Fails without the library, or built without the `nrd` feature.
    pub fn load(device: &Arc<Device>, dir: &Path, extent: vk::Extent2D) -> Result<Self> {
        let sigma = Sigma::load(device, dir, extent)?;
        Ok(Self {
            device: Arc::clone(device),
            dir: dir.to_owned(),
            sigma,
            previous: None,
            reset: true,
        })
    }

    /// NRD's version.
    pub fn version(&self) -> String {
        self.sigma.version()
    }

    /// Recreates SIGMA for a new frame size (NRD cannot resize). The caller makes sure the GPU
    /// is done with the old one's frames.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        if self.sigma.extent() != extent {
            self.sigma = Sigma::load(&self.device, &self.dir, extent)?;
        }
        self.reset = true;
        Ok(())
    }

    /// Discards the history at the next frame (a cut).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// The third row of the view matrix SIGMA is given this frame, for the trace pass's view
    /// depth.
    pub fn view_z_row(camera: &SunShadowCamera) -> Vec4 {
        camera.view.row(2)
    }

    /// The frame's settings: this camera against the previous one, which it then replaces.
    /// `frame_index` is TAA's jitter phase (the blur's taps then repeat with its cycle),
    /// `time_delta_ms` the frame's step.
    pub fn frame(
        &mut self,
        camera: SunShadowCamera,
        sun_dir: Vec3,
        frame_index: u32,
        time_delta_ms: f32,
    ) -> SigmaFrame {
        let reset = self.reset || self.previous.is_none();
        let previous = self.previous.filter(|_| !reset).unwrap_or(camera);
        // A point `x` relative to this camera was `x + (camera − previous camera)` relative to
        // the previous one (as TAA's reprojection, issue #93).
        let step = camera.position.relative_to(previous.position).as_dvec3();
        let world_to_view_prev =
            (previous.view.as_dmat4() * DMat4::from_translation(step)).as_mat4();
        self.previous = Some(camera);
        self.reset = false;
        // NRD's jitter is where the pixel's sample lies from its centre: against the scene's
        // move.
        let jitter = |j: Vec2| (-j).to_array();
        SigmaFrame {
            world_to_view: camera.view.to_cols_array(),
            world_to_view_prev: world_to_view_prev.to_cols_array(),
            view_to_clip: camera.projection.to_cols_array(),
            view_to_clip_prev: previous.projection.to_cols_array(),
            jitter: jitter(camera.jitter),
            jitter_prev: jitter(previous.jitter),
            light_direction: sun_dir.normalize_or(Vec3::Y).to_array(),
            frame_index,
            time_delta_ms,
            reset,
            // The shadow rays' reach (`sun_shadow`'s 50 km); the sky is written far beyond.
            denoising_range: 50_000.0,
            // NRD's most (5 by default): on the models lab's curtains at noon, the per-pixel
            // range over the 16-frame cycle fell from 1.67 to 1.51 codes (99th percentile 17 to
            // 14).
            stabilized_frames: 7,
        }
    }

    /// Declares SIGMA's passes over `rays` (this frame's, from
    /// [`crate::MeshletRenderer::trace_sun_shadow`]) and `motion` (TAA's), and returns the
    /// denoised shadow for [`crate::AmbientLight::sun_shadow`].
    pub fn denoise<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        rays: SunShadowRays,
        motion: ImageHandle,
        frame: &SigmaFrame,
    ) -> Result<ImageHandle> {
        self.sigma.denoise(
            graph,
            slot,
            SigmaImages {
                penumbra: rays.penumbra,
                normal_roughness: rays.normal_roughness,
                view_z: rays.view_z,
                motion,
            },
            frame,
        )
    }
}
