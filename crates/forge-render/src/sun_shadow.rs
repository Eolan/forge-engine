//! The sun's soft shadows denoised (issues #172 and #173, D-049): by NVIDIA's SIGMA where NRD is
//! installed, else by AMD's FidelityFX shadow denoiser, ported to Slang.
//!
//! [`crate::MeshletRenderer::trace_sun_shadow`] traces one ray a pixel to a point of the sun's
//! disc: for SIGMA it keeps the closest occluder, whose distance sizes the penumbra; for FFX the
//! first hit is enough, its filter's width following the variation instead. The denoiser blurs
//! the binary visibility and stabilises it over a few frames; the resolve multiplies the result
//! (the visibility's square root, from either) into the sun's light in place of its own ray
//! ([`crate::AmbientLight::sun_shadow`]). Without a denoiser the resolve keeps its ray, pixel for
//! pixel as before #172.
//!
//! SIGMA comes from NVIDIA's NRD, loaded at run time by [`forge_gpu::nrd`]. A game that ships NRD
//! ships it under NVIDIA's RTX SDKs License: object code only, under terms at least as protective
//! of NVIDIA as its own (`CREDITS.md`, D-049). FFX is MIT ([`crate::ffx_shadows`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use forge_gpu::nrd::{Sigma, SigmaFrame, SigmaImages};
use forge_gpu::{Device, FrameGraph, FrameSlot, ImageHandle, Result, ShaderCompiler, vk};
use glam::{DMat4, Mat4, Vec2, Vec3, Vec4};

use crate::CellPos;
use crate::ffx_shadows::{FfxFrame, FfxImages, FfxShadows};
use crate::meshlet::SunShadowRays;

/// The sun's apparent radius for the soft shadows when a denoiser smooths them, radians: wider
/// than the real 0.27° (`starfield::SUN_ANGULAR_RADIUS_1AU`) for art's sake (D-049), so a
/// penumbra reads under an exposure metered for the shade. Judged in the models lab (#172).
pub const DENOISED_SUN_RADIUS: f32 = 1.0 * std::f32::consts::PI / 180.0;

/// Which denoiser smooths the sun's shadows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowDenoiserKind {
    /// NVIDIA's SIGMA (NRD, #172).
    Sigma,
    /// AMD's FidelityFX shadow denoiser (#173).
    Ffx,
}

impl ShadowDenoiserKind {
    /// Its name in the logs and the overlay.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sigma => "SIGMA",
            Self::Ffx => "FFX",
        }
    }
}

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

/// One frame's settings, from [`SunShadowDenoiser::frame`].
#[derive(Clone, Copy, Debug)]
pub struct SunShadowFrame {
    sigma: SigmaFrame,
    ffx: FfxFrame,
}

enum Backend {
    Sigma(Box<Sigma>, PathBuf),
    Ffx(Box<FfxShadows>),
}

/// A denoiser and the previous frame's camera (see the module's documentation).
pub struct SunShadowDenoiser {
    device: Arc<Device>,
    backend: Backend,
    previous: Option<SunShadowCamera>,
    reset: bool,
}

impl SunShadowDenoiser {
    /// Loads NRD's SIGMA from `dir` (`nrd-sdk/bin`, or wherever `FORGE_NRD_DIR` points) for
    /// frames of `extent`. Fails without the library, or built without the `nrd` feature.
    pub fn sigma(device: &Arc<Device>, dir: &Path, extent: vk::Extent2D) -> Result<Self> {
        Ok(Self::with(
            device,
            Backend::Sigma(Box::new(Sigma::load(device, dir, extent)?), dir.to_owned()),
        ))
    }

    /// Builds AMD's FidelityFX shadow denoiser for frames of `extent`.
    pub fn ffx(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        Ok(Self::with(
            device,
            Backend::Ffx(Box::new(FfxShadows::new(device, shaders, extent)?)),
        ))
    }

    fn with(device: &Arc<Device>, backend: Backend) -> Self {
        Self {
            device: Arc::clone(device),
            backend,
            previous: None,
            reset: true,
        }
    }

    /// Which denoiser this is.
    pub fn kind(&self) -> ShadowDenoiserKind {
        match self.backend {
            Backend::Sigma(..) => ShadowDenoiserKind::Sigma,
            Backend::Ffx(_) => ShadowDenoiserKind::Ffx,
        }
    }

    /// The denoiser's version: NRD's, or the FFX port's source.
    pub fn version(&self) -> String {
        match &self.backend {
            Backend::Sigma(sigma, _) => sigma.version(),
            Backend::Ffx(_) => "FidelityFX-Denoiser d7dfecb (2021), ported".to_owned(),
        }
    }

    /// Whether the trace must find the closest occluder (SIGMA sizes the penumbra from its
    /// distance) rather than any.
    pub fn needs_closest_hit(&self) -> bool {
        self.kind() == ShadowDenoiserKind::Sigma
    }

    /// Recreates the denoiser for a new frame size. The caller makes sure the GPU is done with
    /// the old one's frames.
    pub fn resize(&mut self, extent: vk::Extent2D) -> Result<()> {
        match &mut self.backend {
            Backend::Sigma(sigma, dir) => {
                if sigma.extent() != extent {
                    **sigma = Sigma::load(&self.device, dir, extent)?;
                }
            }
            Backend::Ffx(ffx) => ffx.resize(extent)?,
        }
        self.reset = true;
        Ok(())
    }

    /// Discards the history at the next frame (a cut).
    pub fn reset_history(&mut self) {
        self.reset = true;
    }

    /// The third row of the view matrix the denoiser is given this frame, for the trace pass's
    /// view depth.
    pub fn view_z_row(camera: &SunShadowCamera) -> Vec4 {
        camera.view.row(2)
    }

    /// The frame's settings: this camera against the previous one, which it then replaces.
    /// `frame_index` is TAA's jitter phase (SIGMA's taps then repeat with its cycle),
    /// `time_delta_ms` the frame's step.
    pub fn frame(
        &mut self,
        camera: SunShadowCamera,
        sun_dir: Vec3,
        frame_index: u32,
        time_delta_ms: f32,
    ) -> SunShadowFrame {
        let reset = self.reset || self.previous.is_none();
        let previous = self.previous.filter(|_| !reset).unwrap_or(camera);
        // A point `x` relative to this camera was `x + (camera − previous camera)` relative to
        // the previous one (as TAA's reprojection, issue #93).
        let step = camera.position.relative_to(previous.position).as_dvec3();
        let world_to_view_prev =
            (previous.view.as_dmat4() * DMat4::from_translation(step)).as_mat4();
        self.previous = Some(camera);
        self.reset = false;
        if let Backend::Ffx(ffx) = &mut self.backend {
            ffx.advance();
        }
        // NRD's jitter is where the pixel's sample lies from its centre: against the scene's
        // move.
        let jitter = |j: Vec2| (-j).to_array();
        SunShadowFrame {
            sigma: SigmaFrame {
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
                // NRD's most (5 by default): on the models lab's curtains at noon, the per-pixel range
                // over the 16-frame cycle fell from 1.67 to 1.51 codes (99th percentile 17 to 14).
                stabilized_frames: 7,
            },
            ffx: FfxFrame {
                view: camera.view,
                world_to_view_prev,
                projection: camera.projection,
                jitter: camera.jitter,
                reset,
            },
        }
    }

    /// Declares the denoiser's passes over `rays` (this frame's, from
    /// [`crate::MeshletRenderer::trace_sun_shadow`]) and `motion` (TAA's), and returns the
    /// denoised shadow for [`crate::AmbientLight::sun_shadow`].
    pub fn denoise<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        rays: SunShadowRays,
        motion: ImageHandle,
        frame: &SunShadowFrame,
    ) -> Result<ImageHandle> {
        match &self.backend {
            Backend::Sigma(sigma, _) => sigma.denoise(
                graph,
                slot,
                SigmaImages {
                    penumbra: rays.penumbra,
                    normal_roughness: rays.normal_roughness,
                    view_z: rays.view_z,
                    motion,
                },
                &frame.sigma,
            ),
            Backend::Ffx(ffx) => Ok(ffx.denoise(
                graph,
                slot,
                FfxImages {
                    penumbra: rays.penumbra,
                    normal: rays.normal_roughness,
                    view_z: rays.view_z,
                    motion,
                },
                &frame.ffx,
            )),
        }
    }
}
