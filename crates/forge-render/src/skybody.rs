//! A distant body in the sky (#220): a sphere far enough to be a disc, the Moon from the Earth
//! or the Earth from the Moon, drawn where the depth is clear (`skybody.slang`).
//!
//! - **Its face:** its colour map, the same projection as a planet's (`forge_terrain::planet`),
//!   shaded by the sun with the map as its albedo, scaled by [`SkyBodyView::albedo`].
//! - **Its air** (the Earth's): a rim of scattered light over the disc towards its limb and beyond
//!   it, fading over the air's depth. Not an atmosphere's tables: a disc a degree or two across.
//! - **Seen through an air** (the Moon from the Earth's ground), its light is added to the sky's,
//!   which lies in front of it, and dimmed by the air's transmittance towards it (the caller's,
//!   from `AtmosphereParams::transmittance`). In space it replaces the stars behind it, so it is
//!   drawn after the sky box.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::{Mat3, Mat4, Vec3};

use crate::material::TextureSet;
use crate::textures::TextureData;

/// Threads a side of the pass's workgroups.
const GROUP: u32 = 8;

/// Mirrors `Push` in `skybody.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    body_from_clip: [f32; 16],
    centre: [f32; 4],
    sun: [f32; 4],
    rim: [f32; 4],
    image: u32,
    color: u32,
    depth: u32,
    size: u32,
}

/// Where a body stands in the sky this frame and how it is lit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyBodyView {
    /// From the camera towards its centre, world axes, unit.
    pub direction: Vec3,
    /// Its angular radius, radians.
    pub angular_radius: f32,
    /// From the world's axes to its frame (+Y its north, +Z its prime meridian).
    pub body_from_world: Mat3,
    /// Towards the sun, world axes.
    pub sun_dir: Vec3,
    /// Pre-exposed luminance of a white Lambertian surface facing the sun, dimmed by any air
    /// between the camera and the body.
    pub sun_luminance: f32,
    /// Its map's scale to an albedo (0.2 for the Moon's map, brightened for show; 1 for the Blue
    /// Marble).
    pub albedo: f32,
    /// Its air's light at the limb, a share of the sun's term, and the air's depth, a share of its
    /// radius; zero depth for no air.
    pub rim: [f32; 4],
    /// Seen through an air: its light is added to the sky's.
    pub through_air: bool,
}

/// The pass and the body's map.
pub struct SkyBody {
    pipeline: Pipeline,
    /// Keeps the map's image alive.
    _textures: TextureSet,
    image: u32,
}

impl SkyBody {
    /// Compiles the pass and uploads `map`, the body's equirectangular colour map with its mips.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler, map: &TextureData) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("skybody.slang", "skybody_main", ShaderStage::Compute)?,
            "sky body",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "skybody_main"),
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            name: "sky body",
        });
        device.destroy_shader_module(module);
        let mut textures = TextureSet::new(device);
        let id = textures.add(map)?;
        let image = textures.sampled(id);
        Ok(Self {
            pipeline: pipeline?,
            _textures: textures,
            image,
        })
    }

    /// Declares `sky/body`: the body drawn into `color` where `depth` is clear. `view_proj` is
    /// the drawing camera's (camera-relative).
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
        view_proj: Mat4,
        body: &SkyBodyView,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let to_body = body.body_from_world;
        let push = Push {
            body_from_clip: (Mat4::from_mat3(to_body) * view_proj.inverse()).to_cols_array(),
            centre: (to_body * body.direction)
                .normalize()
                .extend(body.angular_radius.sin())
                .to_array(),
            sun: (to_body * body.sun_dir)
                .normalize()
                .extend(body.sun_luminance * body.albedo)
                .to_array(),
            rim: body.rim,
            image: self.image,
            color: 0,
            depth: 0,
            size: extent.width | extent.height << 16 | u32::from(body.through_air) << 31,
        };
        let pipeline = &self.pipeline;
        graph
            .pass("sky/body")
            .image(depth, ImageAccess::Sampled(compute))
            .image(color, ImageAccess::StorageReadWrite(compute))
            .run(move |resources, commands| {
                let mut push = push;
                push.color = resources.storage(color, 0).0;
                push.depth = resources.sampled(depth).0;
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &push);
                commands.dispatch(
                    extent.width.div_ceil(GROUP),
                    extent.height.div_ceil(GROUP),
                    1,
                );
                Ok(())
            });
    }
}

/// The rotation from the world's axes to a body's frame that turns the body's point at latitude
/// `lat` and longitude `lon` (degrees) towards the camera, seen in `direction` (camera to body,
/// world axes), its north as near the world's up as that allows.
pub fn facing(direction: Vec3, lat: f32, lon: f32) -> Mat3 {
    let (lat, lon) = (lat.to_radians(), lon.to_radians());
    // In the body's frame: the point towards the camera and its north.
    let point = Vec3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos());
    let north = (Vec3::Y - point * point.y)
        .try_normalize()
        .unwrap_or(Vec3::Z);
    // In the world: from the body towards the camera, and up as seen across.
    let toward = -direction.normalize();
    let up = (Vec3::Y - toward * toward.dot(Vec3::Y))
        .try_normalize()
        .unwrap_or(Vec3::Z);
    let world = Mat3::from_cols(toward, up, toward.cross(up));
    let body = Mat3::from_cols(point, north, point.cross(north));
    body * world.transpose()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_push_constants_fit_the_guaranteed_128_bytes() {
        assert_eq!(std::mem::size_of::<Push>(), 128);
    }

    #[test]
    fn the_facing_point_looks_at_the_camera() {
        let direction = Vec3::new(0.3, 0.8, -0.5).normalize();
        let m = facing(direction, 20.0, 10.0);
        // The world's direction back to the camera is the body's point at (20°, 10°).
        let back = m * -direction;
        let (lat, lon) = (20f32.to_radians(), 10f32.to_radians());
        let point = Vec3::new(lat.cos() * lon.sin(), lat.sin(), lat.cos() * lon.cos());
        assert!((back - point).length() < 1e-5, "{back} {point}");
        assert!((m.determinant() - 1.0).abs() < 1e-5);
    }
}
