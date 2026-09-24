//! A procedural starfield background (`shaders/starfield.slang`): one full-screen triangle,
//! stars and a faint nebula hashed from the view direction. Drawn *after* the geometry with a
//! depth test and no depth write: in reversed-Z the triangle sits at depth 0, so only the
//! pixels nothing was drawn to run the sky shader.
//!
//! Units: the sun is `sun_illuminance` lux arriving from a disc of `sun_angular_radius`, so
//! the disc's luminance is the illuminance over its solid angle (1.9 · 10⁹ cd/m² for the Sun
//! seen from 1 AU). The planet is lit by that sun like the rocks (albedo × E / π). Stars,
//! nebula and the glow around the sun are *authored* emissives, in units of the luminance of a
//! white Lambertian surface facing the sun: a real starfield is eight orders of magnitude
//! below a sunlit rock and would not show at the same exposure (the "no stars in the Moon
//! photos" effect), so this sky is art-directed to read next to the rocks, as every space
//! game's is. Everything is written pre-exposed ([`crate::exposure`]).

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Device, FrameGraph, FullscreenPipelineDesc, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::Mat4;

/// Mirrors `Push` in `starfield.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    inv_view_proj: [f32; 16],
    sun_dir: [f32; 3],
    /// Pre-exposed luminance of a white Lambertian surface facing the sun: the unit of the
    /// authored sky and of the planet's albedo.
    luminance_unit: f32,
    planet_dir: [f32; 3],
    planet_angle: f32,
    /// Pre-exposed luminance of the sun's disc.
    sun_disc: f32,
    /// Angular radius of the sun's disc (radians).
    sun_angle: f32,
    pad: [f32; 2],
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
    /// Direction to a distant planet drawn in the sky.
    pub planet_dir: glam::Vec3,
    /// Angular radius of the planet in radians; 0 hides it.
    pub planet_angle: f32,
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
            name: "starfield",
        })?;
        device.destroy_shader_module(vertex);
        device.destroy_shader_module(fragment);
        Ok(Self {
            pipeline,
            sun_illuminance: SUN_ILLUMINANCE_1AU,
            sun_angular_radius: SUN_ANGULAR_RADIUS_1AU,
            planet_dir: glam::Vec3::new(-0.35, 0.12, -1.0).normalize(),
            planet_angle: 0.0,
        })
    }

    /// Declares the pass that draws the background into `color` wherever `depth` (the
    /// buffer the geometry was drawn with) is still clear, pre-exposed by `exposure`.
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
    ) {
        let push = Push {
            inv_view_proj: view_proj.inverse().to_cols_array(),
            sun_dir: sun_dir.to_array(),
            luminance_unit: crate::exposure::lambertian_luminance(self.sun_illuminance) * exposure,
            planet_dir: self.planet_dir.to_array(),
            planet_angle: self.planet_angle,
            sun_disc: disc_luminance(self.sun_illuminance, self.sun_angular_radius) * exposure,
            sun_angle: self.sun_angular_radius,
            pad: [0.0; 2],
        };
        let pipeline = &self.pipeline;
        graph
            .pass("sky/starfield + planet")
            .image(color, ImageAccess::ColorAttachment)
            .image(depth, ImageAccess::DepthRead)
            .run(move |resources, commands| {
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
    fn the_sun_disc_carries_its_illuminance() {
        // The Sun outside the atmosphere: about 2 x 10^9 cd/m^2.
        let luminance = disc_luminance(SUN_ILLUMINANCE_1AU, SUN_ANGULAR_RADIUS_1AU);
        assert!((1.7e9..2.1e9).contains(&luminance), "{luminance}");
        // Twice the angle, a quarter of the luminance for the same illuminance.
        let wide = disc_luminance(SUN_ILLUMINANCE_1AU, 2.0 * SUN_ANGULAR_RADIUS_1AU);
        assert!((luminance / wide - 4.0).abs() < 0.01);
    }
}
