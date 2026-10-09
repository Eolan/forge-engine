//! A sky box (#220): an equirectangular map of the whole sky drawn behind everything, the real
//! stars and Milky Way where the procedural starfield (`crate::Starfield`) invents them. The
//! `planet` demo takes NASA's Deep Star Maps 2020 (from ESA's Gaia and Hipparcos).
//!
//! - **The map:** right ascension 0 at its middle, increasing to the left as the sky is seen from
//!   inside; declination +90° on its first row. It is turned into the world by a rotation from the
//!   world's axes to the sky's (equatorial: x towards RA 0, y towards RA 90°, z the north pole), so
//!   a body's own pole and meridian place its sky.
//! - **Brightness:** the map is made for display, its brightest stars clipped at 1, so it carries
//!   no physical calibration. A map value of 1 is drawn at [`SkyBox::luminance`] cd/m² (the demo's
//!   `--stars` stops): next to a sunlit body an eye would see no star, but the owner wants them shown.
//! - **Over an atmosphere,** the sky is added to what the sky's compose drew, and fades out where
//!   the ray passes through the lowest 40 km of air; the air's sky draws the sun. With no air, the
//!   pass draws the sun's disc too.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    ComputePipelineDesc, Device, FrameGraph, ImageAccess, ImageHandle, Pipeline, Result,
    ShaderCompiler, ShaderStage, vk,
};
use glam::{Mat3, Mat4, Vec3};

use crate::material::TextureSet;
use crate::starfield::{SUN_ANGULAR_RADIUS_1AU, SUN_ILLUMINANCE_1AU, disc_luminance};
use crate::textures::TextureData;

/// Threads a side of the pass's workgroups.
const GROUP: u32 = 8;

/// Mirrors `Push` in `skybox.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Push {
    sky_from_clip: [f32; 16],
    sun: [f32; 4],
    planet: [f32; 4],
    top: f32,
    luminance: f32,
    sun_luminance: f32,
    image: u32,
    color: u32,
    depth: u32,
    size: u32,
    pad: u32,
}

/// The body the camera is near, for the sky box to fade behind its air and stop at its ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyBoxPlanet {
    /// From the camera to the body's centre, world axes, km.
    pub centre_km: Vec3,
    /// Its ground's radius, km.
    pub radius_km: f32,
    /// Its atmosphere's top, km; 0 for none (the pass then draws the sun's disc).
    pub top_km: f32,
}

/// The sky box pass and its map.
pub struct SkyBox {
    pipeline: Pipeline,
    /// Keeps the map's image alive.
    _textures: TextureSet,
    image: u32,
    /// Luminance of a map value of 1, cd/m².
    pub luminance: f32,
    /// Illuminance of the sun at the scene, lux.
    pub sun_illuminance: f32,
    /// Angular radius of the sun's disc, radians.
    pub sun_angular_radius: f32,
}

impl SkyBox {
    /// Compiles the pass and uploads `map`, the sky's equirectangular image with its mips.
    pub fn new(device: &Arc<Device>, shaders: &ShaderCompiler, map: &TextureData) -> Result<Self> {
        let module = device.create_shader_module(
            &shaders.compile("skybox.slang", "skybox_main", ShaderStage::Compute)?,
            "sky box",
        )?;
        let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
            shader: (module, "skybox_main"),
            push_constant_bytes: std::mem::size_of::<Push>() as u32,
            name: "sky box",
        });
        device.destroy_shader_module(module);
        let mut textures = TextureSet::new(device);
        let id = textures.add(map)?;
        let image = textures.sampled(id);
        Ok(Self {
            pipeline: pipeline?,
            _textures: textures,
            image,
            luminance: 4096.0,
            sun_illuminance: SUN_ILLUMINANCE_1AU,
            sun_angular_radius: SUN_ANGULAR_RADIUS_1AU,
        })
    }

    /// Declares `sky/box`: the map added into `color` (the resolved HDR image) wherever `depth`
    /// is clear, pre-exposed by `exposure`. `view_proj` is the drawing camera's (camera-relative),
    /// `sky_from_world` turns the world's axes into the sky's, `sun_dir` points to the sun.
    #[allow(clippy::too_many_arguments)]
    pub fn draw<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        color: ImageHandle,
        depth: ImageHandle,
        extent: vk::Extent2D,
        view_proj: Mat4,
        sky_from_world: Mat3,
        sun_dir: Vec3,
        exposure: f32,
        planet: Option<SkyBoxPlanet>,
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let sky_from_clip = Mat4::from_mat3(sky_from_world) * view_proj.inverse();
        let planet = planet.unwrap_or(SkyBoxPlanet {
            centre_km: Vec3::ZERO,
            radius_km: 0.0,
            top_km: 0.0,
        });
        let push = Push {
            sky_from_clip: sky_from_clip.to_cols_array(),
            sun: (sky_from_world * sun_dir)
                .extend(self.sun_angular_radius.cos())
                .to_array(),
            planet: (sky_from_world * planet.centre_km)
                .extend(planet.radius_km)
                .to_array(),
            top: planet.top_km,
            luminance: self.luminance * exposure,
            sun_luminance: disc_luminance(self.sun_illuminance, self.sun_angular_radius) * exposure,
            image: self.image,
            color: 0,
            depth: 0,
            size: extent.width | extent.height << 16,
            pad: 0,
        };
        let pipeline = &self.pipeline;
        graph
            .pass("sky/box")
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

/// The rotation from a body's frame (+Y its north pole, +Z its prime meridian, +X 90° east) to
/// the sky's equatorial axes, for a pole at right ascension `pole_ra` and declination `pole_dec`
/// and a prime meridian `meridian` degrees from the node of the body's equator on the sky's (the
/// IAU's α₀, δ₀ and W).
pub fn sky_from_body(pole_ra: f64, pole_dec: f64, meridian: f64) -> glam::DMat3 {
    use glam::{DMat3, DVec3};
    // Into the IAU's order (x the prime meridian, y 90° east, z the pole): the body's +X (90° east)
    // goes to y, its +Y (the pole) to z, its +Z (the meridian) to x.
    let iau_from_body = DMat3::from_cols(DVec3::Y, DVec3::Z, DVec3::X);
    let r = DMat3::from_rotation_z((90.0 + pole_ra).to_radians())
        * DMat3::from_rotation_x((90.0 - pole_dec).to_radians())
        * DMat3::from_rotation_z(meridian.to_radians());
    r * iau_from_body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_push_constants_fit_the_guaranteed_128_bytes() {
        assert_eq!(std::mem::size_of::<Push>(), 128);
    }

    #[test]
    fn a_body_s_pole_and_meridian_land_where_the_iau_puts_them() {
        use glam::DVec3;
        let (ra, dec) = (266.86_f64, 65.64_f64);
        let m = sky_from_body(ra, dec, 38.32);
        let pole = m * DVec3::Y;
        let (ra_r, dec_r) = (ra.to_radians(), dec.to_radians());
        let expected = DVec3::new(
            dec_r.cos() * ra_r.cos(),
            dec_r.cos() * ra_r.sin(),
            dec_r.sin(),
        );
        assert!((pole - expected).length() < 1e-9, "{pole} {expected}");
        // The Earth's north is the sky's: its pole at declination 90°.
        assert!((sky_from_body(0.0, 90.0, 0.0) * DVec3::Y - DVec3::Z).length() < 1e-9);
        // A right-handed rotation.
        assert!((m.determinant() - 1.0).abs() < 1e-9);
        // The Earth's prime meridian at W = 0 lies towards the node, right ascension 90°.
        let meridian = sky_from_body(0.0, 90.0, 0.0) * DVec3::Z;
        assert!((meridian - DVec3::Y).length() < 1e-9, "{meridian}");
    }
}
