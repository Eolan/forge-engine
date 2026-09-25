//! A procedural starfield background (`shaders/starfield.slang`): one full-screen triangle,
//! stars and a faint nebula hashed from the view direction. Drawn *after* the geometry with a
//! depth test and no depth write: in reversed-Z the triangle sits at depth 0, so only the
//! pixels nothing was drawn to run the sky shader.
//!
//! Units: the sun is `sun_illuminance` lux arriving from a disc of `sun_angular_radius`, so
//! the disc's luminance is the illuminance over its solid angle (1.9 · 10⁹ cd/m² for the Sun
//! seen from 1 AU). A planet's ground is lit by that sun through its atmosphere
//! ([`crate::atmosphere`]), which also dims and reddens what is seen through it. Stars,
//! nebula, the glow around the sun and city lights are *authored* emissives, in units of the
//! luminance of a white Lambertian surface facing the sun: a real starfield is eight orders
//! of magnitude below a sunlit rock and would not show at the same exposure (the "no stars
//! in the Moon photos" effect), so this sky is art-directed to read next to the rocks, as
//! every space game's is. Everything is written pre-exposed ([`crate::exposure`]).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::Mat4;

use crate::atmosphere::AtmosphereFrame;

/// Mirrors `Push` in `starfield.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    inv_view_proj: [f32; 16],
    sun_dir: [f32; 3],
    /// Pre-exposed luminance of a white Lambertian surface facing the sun: the unit of the
    /// authored sky and of the planet's albedo.
    luminance_unit: f32,
    /// From the camera to the planet's centre.
    planet_dir: [f32; 3],
    /// Cosine of the angular radius of the top of the planet's atmosphere: pixels outside
    /// that cone skip it.
    planet_cos: f32,
    /// Pre-exposed luminance of the sun's disc.
    sun_disc: f32,
    /// Angular radius of the sun's disc (radians).
    sun_angle: f32,
    /// Device address of the planet and its atmosphere; 0 = no planet.
    planet: u64,
    /// The atmosphere's tables (sampled images).
    transmittance: u32,
    multiple_scattering: u32,
    /// The planet-view table (issue #26); `u32::MAX`: every pixel marches its ray instead.
    planet_view: u32,
    /// The table's transmittance row; when marching, the segments per ray (issue #73).
    planet_view_row: u32,
}

/// Illuminance of the Sun at 1 AU outside an atmosphere, in lux.
pub const SUN_ILLUMINANCE_1AU: f32 = 128_000.0;
/// Angular radius of the Sun seen from 1 AU (0.267°), in radians.
pub const SUN_ANGULAR_RADIUS_1AU: f32 = 0.004_65;

/// Luminance in cd/m² of a uniform disc of angular radius `angle` delivering `illuminance`
/// lux: the illuminance over the disc's solid angle `2π (1 − cos angle)`, written as
/// `4π sin²(angle / 2)` so it keeps its precision for small discs.
pub fn disc_luminance(illuminance: f32, angle: f32) -> f32 {
    let half = (0.5 * angle).sin();
    illuminance / (2.0 * std::f32::consts::TAU * half * half).max(1e-20)
}

/// The starfield pass.
pub struct Starfield {
    pipeline: Pipeline,
    /// Illuminance of the sun at the scene, in lux.
    pub sun_illuminance: f32,
    /// Angular radius of the sun's disc, in radians.
    pub sun_angular_radius: f32,
}

impl Starfield {
    /// Compiles the pipeline for `color_format`, tested against a `D32_SFLOAT` depth buffer.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        color_format: vk::Format,
    ) -> Result<Self> {
        let vertex = device.create_shader_module(
            &shaders.compile("starfield.slang", "vert_main", ShaderStage::Vertex)?,
            "starfield vs",
        )?;
        let fragment = device.create_shader_module(
            &shaders.compile("starfield.slang", "frag_main", ShaderStage::Fragment)?,
            "starfield fs",
        )?;
        let pipeline = device.create_fullscreen_pipeline(&FullscreenPipelineDesc {
            vertex: (vertex, "vert_main"),
            fragment: (fragment, "frag_main"),
            color_formats: &[color_format],
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            alpha_blend: false,
            depth_test: Some(vk::Format::D32_SFLOAT),
            depth_write: false,
            name: "starfield",
        })?;
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        Ok(Self {
            pipeline,
            sun_illuminance: SUN_ILLUMINANCE_1AU,
            sun_angular_radius: SUN_ANGULAR_RADIUS_1AU,
        })
    }

    /// Declares the pass that draws the background into `color` wherever `depth` (the
    /// buffer the geometry was drawn with) is still clear, pre-exposed by `exposure`, with
    /// `planet` and its atmosphere in front of the stars when given.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
        view_proj: Mat4,
        sun_dir: glam::Vec3,
        exposure: f32,
        planet: Option<AtmosphereFrame>,
    ) {
        let push = Push {
            inv_view_proj: view_proj.inverse().to_cols_array(),
            sun_dir: sun_dir.to_array(),
            luminance_unit: crate::exposure::lambertian_luminance(self.sun_illuminance) * exposure,
            sun_disc: disc_luminance(self.sun_illuminance, self.sun_angular_radius) * exposure,
            sun_angle: self.sun_angular_radius,
            planet_dir: planet.map_or([0.0; 3], |p| p.direction.to_array()),
            planet_cos: planet.map_or(2.0, |p| p.cos_top),
            planet: planet.map_or(0, |p| p.planet),
            transmittance: 0,
            multiple_scattering: 0,
            planet_view: u32::MAX,
            planet_view_row: planet.map_or(0, |p| p.march_steps),
        };
        let pipeline = &self.pipeline;
        let mut pass = graph
            .pass("sky/starfield + planet")
            .image(color, ImageAccess::ColorAttachment)
            .image(depth, ImageAccess::DepthRead);
        if let Some(p) = planet {
            let fragment = vk::PipelineStageFlags2::FRAGMENT_SHADER;
            pass = pass
                .image(p.transmittance, ImageAccess::Sampled(fragment))
                .image(p.multiple_scattering, ImageAccess::Sampled(fragment));
            if let Some((table, table_transmittance)) = p.planet_view {
                pass = pass
                    .image(table, ImageAccess::Sampled(fragment))
                    .image(table_transmittance, ImageAccess::Sampled(fragment));
            }
        }
        pass.run(move |resources, commands| {
            let mut push = push;
            if let Some(p) = planet {
                push.transmittance = resources.sampled(p.transmittance).0;
                push.multiple_scattering = resources.sampled(p.multiple_scattering).0;
                if let Some((table, table_transmittance)) = p.planet_view {
                    push.planet_view = resources.sampled(table).0;
                    push.planet_view_row = resources.sampled(table_transmittance).0;
                }
            }
            let attachments = [vk::RenderingAttachmentInfo::default()
                .image_view(resources.view(color))
                .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::STORE)];
            let depth_attachment = vk::RenderingAttachmentInfo::default()
                .image_view(resources.view(depth))
                .image_layout(vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL)
                .load_op(vk::AttachmentLoadOp::LOAD)
                .store_op(vk::AttachmentStoreOp::NONE);
            let info = vk::RenderingInfo::default()
                .render_area(vk::Rect2D {
                    offset: vk::Offset2D::default(),
                    extent,
                })
                .layer_count(1)
                .color_attachments(&attachments)
                .depth_attachment(&depth_attachment);
            commands.begin_rendering(&info);
            commands.bind_pipeline(pipeline);
            commands.set_viewport_full(extent);
            commands.push_constants(pipeline, &push);
            commands.draw(3, 1);
            commands.end_rendering();
            Ok(())
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_push_constants_fit_the_guaranteed_128_bytes() {
        assert_eq!(std::mem::size_of::<Push>(), 128);
    }

    #[test]
    fn the_sun_disc_carries_its_illuminance() {
        // The Sun outside the atmosphere: about 2 x 10^9 cd/m^2.
        let luminance = disc_luminance(SUN_ILLUMINANCE_1AU, SUN_ANGULAR_RADIUS_1AU);
        assert!((1.7e9..2.1e9).contains(&luminance), "{luminance}");
        // Twice the angle, a quarter of the luminance for the same illuminance.
        let wide = disc_luminance(SUN_ILLUMINANCE_1AU, 2.0 * SUN_ANGULAR_RADIUS_1AU);
        assert!((luminance / wide - 4.0).abs() < 0.01);
    }
}
