//! A planet's atmosphere (`shaders/atmosphere.slang`), after Hillaire, "A Scalable and
//! Production Ready Sky and Atmosphere Rendering Technique" (EGSR 2020), lifted from the
//! `world` project onto the render graph.
//!
//! Two tables are built by compute passes when the atmosphere changes and sampled by later
//! passes: **transmittance** to the top of the atmosphere (256 × 64, height × zenith angle,
//! Bruneton's mapping) and the **multiple-scattering** transfer (32 × 32, sun zenith angle ×
//! height: second-order light from 64 directions, then the geometric series of the higher
//! orders). Seen from space, as in the ballad, the sky pass marches each pixel's ray through
//! the atmosphere with both tables: the ground lit through the air, the blue limb, the
//! reddened terminator and the sun dimmed and reddened behind the limb all come from the same
//! integral. The sky-view and aerial-perspective tables of the paper serve cameras inside the
//! atmosphere and come with the first ground demo.
//!
//! Distances are in kilometres and coefficients per kilometre; luminance is per unit of sun
//! illuminance, so the sky pass multiplies it by the pre-exposed illuminance.

use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph, FrameSlot,
    GraphImage, ImageAccess, ImageDesc, ImageHandle, MemoryCategory, MemoryLocation, Pipeline,
    Result, ShaderCompiler, ShaderStage, vk,
};
use glam::Vec3;

/// Transmittance table: view height × zenith angle.
pub const TRANSMITTANCE_SIZE: [u32; 2] = [256, 64];
/// Multiple-scattering table: sun zenith angle × height.
pub const MULTIPLE_SCATTERING_SIZE: [u32; 2] = [32, 32];
const LUT_FORMAT: vk::Format = vk::Format::R16G16B16A16_SFLOAT;
const GROUP_SIZE: u32 = 8;
/// Profiler zone of the table passes (they run only when the atmosphere changes).
const LABEL: &str = "sky/atmosphere tables";
/// The planet-view table (issue #26, `PLANET_VIEW_*` in `atmosphere.slang`): the light scattered
/// towards a camera outside the atmosphere, over the rays' closest approach to the planet ×
/// their azimuth from the sun's side.
pub const PLANET_VIEW_SIZE: [u32; 2] = [512, 256];
/// Profiler zone of its pass, which runs when the camera, the sun or the atmosphere moves.
const PLANET_VIEW_LABEL: &str = "sky/planet-view table";
/// Default segments of the march behind each texel of the planet-view table: within 1/255 of a
/// 512-segment march everywhere the ballad was checked, where 16 were 4/255 off at the limb
/// (issue #73). 0.08 ms to build, once.
pub const PLANET_VIEW_STEPS: u32 = 64;
/// Default segments of the per-pixel march from space (#8: within 4/255 of 128).
pub const MARCH_STEPS: u32 = 16;

/// An atmosphere: a Rayleigh layer, an aerosol (Mie) layer and an absorbing ozone layer over a
/// spherical ground. Distances in kilometres, coefficients per kilometre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AtmosphereParams {
    /// The ground's radius.
    pub bottom_radius: f32,
    /// Where the atmosphere is considered to end.
    pub top_radius: f32,
    /// Rayleigh scattering at the ground (RGB).
    pub rayleigh_scattering: [f32; 3],
    /// Rayleigh scale height.
    pub rayleigh_scale_height: f32,
    /// Aerosol scattering at the ground (RGB).
    pub mie_scattering: [f32; 3],
    /// Aerosol absorption at the ground (RGB).
    pub mie_absorption: [f32; 3],
    /// Aerosol scale height.
    pub mie_scale_height: f32,
    /// Cornette–Shanks asymmetry of the aerosols (forward scattering towards 1).
    pub mie_asymmetry: f32,
    /// Absorption at the peak of the tent-shaped ozone layer (RGB).
    pub ozone_absorption: [f32; 3],
    /// Altitude of the ozone layer's peak.
    pub ozone_peak_altitude: f32,
    /// Half-width of the ozone layer.
    pub ozone_half_width: f32,
    /// Grey ground albedo for the light the ground bounces back into the air (the tables).
    pub ground_albedo: f32,
}

impl AtmosphereParams {
    /// The Earth of Hillaire 2020 (and Bruneton 2017): Rayleigh coefficients for 680, 550 and
    /// 440 nm, a continental aerosol, the ozone layer between 10 and 40 km.
    pub fn earth() -> Self {
        Self {
            bottom_radius: 6360.0,
            top_radius: 6460.0,
            rayleigh_scattering: [5.802e-3, 13.558e-3, 33.1e-3],
            rayleigh_scale_height: 8.0,
            mie_scattering: [3.996e-3; 3],
            mie_absorption: [0.444e-3; 3],
            mie_scale_height: 1.2,
            mie_asymmetry: 0.8,
            ozone_absorption: [0.650e-3, 1.881e-3, 0.085e-3],
            ozone_peak_altitude: 25.0,
            ozone_half_width: 15.0,
            ground_albedo: 0.3,
        }
    }

    /// Extinction (scattering plus absorption) at `altitude` km (RGB, per km); the CPU mirror of
    /// `sample_medium` in `atmosphere.slang`.
    pub fn extinction(&self, altitude: f32) -> [f32; 3] {
        let h = altitude.max(0.0);
        let rayleigh = (-h / self.rayleigh_scale_height).exp();
        let mie = (-h / self.mie_scale_height).exp();
        let ozone =
            (1.0 - (altitude - self.ozone_peak_altitude).abs() / self.ozone_half_width).max(0.0);
        std::array::from_fn(|c| {
            self.rayleigh_scattering[c] * rayleigh
                + (self.mie_scattering[c] + self.mie_absorption[c]) * mie
                + self.ozone_absorption[c] * ozone
        })
    }

    /// Transmittance from `origin` (km, relative to the planet's centre, inside or outside the
    /// atmosphere) along the unit `direction` to where the ray leaves the atmosphere; zero when
    /// the ground blocks it. A midpoint integral in `steps` segments: the CPU mirror of the
    /// transmittance table, for tests and for CPU-side light estimates.
    pub fn transmittance(&self, origin: Vec3, direction: Vec3, steps: u32) -> [f32; 3] {
        let Some((near, far)) = sphere_span(origin, direction, self.top_radius) else {
            return [1.0; 3];
        };
        if sphere_span(origin, direction, self.bottom_radius).is_some_and(|(n, _)| n > 0.0) {
            return [0.0; 3];
        }
        let start = near.max(0.0);
        let dt = (far - start) / steps as f32;
        let mut depth = [0.0_f32; 3];
        for i in 0..steps {
            let position = origin + direction * (start + (i as f32 + 0.5) * dt);
            let extinction = self.extinction(position.length() - self.bottom_radius);
            for (d, e) in depth.iter_mut().zip(extinction) {
                *d += e * dt;
            }
        }
        depth.map(|d| (-d).exp())
    }

    /// Where a camera sits (km, relative to the planet's centre) when the planet's ground fills
    /// a disc of `angular_radius` radians around `direction` (from the camera to the planet).
    pub fn view_from_space(&self, direction: Vec3, angular_radius: f32) -> Vec3 {
        let distance = self.bottom_radius / angular_radius.clamp(1e-4, 1.5).sin();
        -direction.normalize_or(Vec3::NEG_Z) * distance
    }

    fn gpu(&self, view_position: Vec3) -> GpuPlanet {
        let extend = |rgb: [f32; 3], w: f32| [rgb[0], rgb[1], rgb[2], w];
        GpuPlanet {
            radii: [
                self.bottom_radius,
                self.top_radius.max(self.bottom_radius + 1.0),
                self.ground_albedo,
                0.0,
            ],
            rayleigh: extend(
                self.rayleigh_scattering,
                self.rayleigh_scale_height.max(0.01),
            ),
            mie_scattering: extend(self.mie_scattering, self.mie_scale_height.max(0.01)),
            mie_absorption: extend(self.mie_absorption, self.mie_asymmetry.clamp(-0.99, 0.99)),
            ozone: extend(self.ozone_absorption, self.ozone_peak_altitude),
            ozone_shape: [self.ozone_half_width.max(0.01), 0.0, 0.0, 0.0],
            view_position: view_position.extend(0.0).to_array(),
        }
    }
}

/// Where the ray crosses the sphere of `radius` (distances in and out), from the point of
/// closest approach like `sphere_span` in `atmosphere.slang`.
fn sphere_span(origin: Vec3, direction: Vec3, radius: f32) -> Option<(f32, f32)> {
    let along = -origin.dot(direction);
    let h = (origin + direction * along).length();
    if h > radius {
        return None;
    }
    let half_chord = ((radius - h) * (radius + h)).sqrt();
    (along + half_chord > 0.0).then_some((along - half_chord, along + half_chord))
}

/// `Planet` in `atmosphere.slang` (float4 fields only).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct GpuPlanet {
    radii: [f32; 4],
    rayleigh: [f32; 4],
    mie_scattering: [f32; 4],
    mie_absorption: [f32; 4],
    ozone: [f32; 4],
    ozone_shape: [f32; 4],
    view_position: [f32; 4],
}

/// `LutPush` in `atmosphere_luts.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct LutPush {
    planet: u64,
    output: u32,
    transmittance: u32,
    width: u32,
    height: u32,
}

/// `PlanetViewPush` in `atmosphere_luts.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PlanetViewPush {
    /// xyz: towards the sun.
    sun: [f32; 4],
    planet: u64,
    transmittance: u32,
    multiple_scattering: u32,
    luminance_out: u32,
    transmittance_out: u32,
    steps: u32,
    pad: u32,
}

/// What the planet-view table was built for: the atmosphere, the camera in the planet's frame,
/// the direction to the sun and the segments of its march.
type PlanetViewKey = (AtmosphereParams, [f32; 3], [f32; 3], u32);

/// What the passes of a frame that draw the atmosphere need: the planet's data by device
/// address and the two tables, to declare as sampled.
#[derive(Clone, Copy, Debug)]
pub struct AtmosphereFrame {
    /// Device address of this frame's `Planet` (the atmosphere and the camera in its frame).
    pub planet: u64,
    /// The transmittance table.
    pub transmittance: ImageHandle,
    /// The multiple-scattering table.
    pub multiple_scattering: ImageHandle,
    /// From the camera to the planet's centre.
    pub direction: Vec3,
    /// Cosine of the angular radius of the top of the atmosphere seen from the camera (−1
    /// from inside it): view rays outside that cone miss the atmosphere.
    pub cos_top: f32,
    /// From a camera outside the atmosphere, the planet-view table and its transmittance row
    /// (issue #26); `None` from inside it or with [`Atmosphere::planet_view`] off.
    pub planet_view: Option<(ImageHandle, ImageHandle)>,
    /// Segments of the per-pixel march where there is no planet-view table.
    pub march_steps: u32,
}

/// The atmosphere's tables, their passes and the per-frame planet data.
pub struct Atmosphere {
    transmittance_pipeline: Pipeline,
    multiple_scattering_pipeline: Pipeline,
    transmittance: GraphImage,
    multiple_scattering: GraphImage,
    planet_view_pipeline: Pipeline,
    /// The planet-view table and its transmittance row (issue #26).
    view_table: GraphImage,
    view_transmittance: GraphImage,
    /// What the planet-view table holds.
    view_built: Option<PlanetViewKey>,
    /// Draw a planet seen from space through the planet-view table (the default), else by
    /// marching every pixel's ray (the reference the table is checked against).
    pub planet_view: bool,
    /// Segments of the march behind each texel of the planet-view table (issue #73).
    pub planet_view_steps: u32,
    /// Segments of the per-pixel march, where the planet-view table is not used.
    pub march_steps: u32,
    planets: Vec<Buffer>,
    /// The atmosphere drawn; changing it rebuilds the tables.
    pub params: AtmosphereParams,
    /// The atmosphere the tables hold.
    built: Option<AtmosphereParams>,
}

impl Atmosphere {
    /// Compiles the table passes and creates the tables; they are built by the first frame.
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        params: AtmosphereParams,
    ) -> Result<Self> {
        let compute = |entry: &str, name: &str, push_bytes: usize| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("atmosphere_luts.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: push_bytes as u32,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let table = |size: [u32; 2], name: &str| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: size[0],
                    height: size[1],
                    format: LUT_FORMAT,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
            )
        };
        let planets = (0..FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuPlanet>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("planet {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            transmittance_pipeline: compute(
                "transmittance_main",
                "atmosphere transmittance",
                std::mem::size_of::<LutPush>(),
            )?,
            multiple_scattering_pipeline: compute(
                "multiple_scattering_main",
                "atmosphere multiple scattering",
                std::mem::size_of::<LutPush>(),
            )?,
            planet_view_pipeline: compute(
                "planet_view_main",
                "atmosphere planet view",
                std::mem::size_of::<PlanetViewPush>(),
            )?,
            view_table: table(PLANET_VIEW_SIZE, "atmosphere planet view")?,
            view_transmittance: table(
                [PLANET_VIEW_SIZE[0], 1],
                "atmosphere planet view transmittance",
            )?,
            view_built: None,
            planet_view: true,
            planet_view_steps: PLANET_VIEW_STEPS,
            march_steps: MARCH_STEPS,
            transmittance: table(TRANSMITTANCE_SIZE, "atmosphere transmittance")?,
            multiple_scattering: table(MULTIPLE_SCATTERING_SIZE, "atmosphere multiple scattering")?,
            planets,
            params,
            built: None,
        })
    }

    /// Writes this frame's planet data with the camera at `view_position` (km, relative to the
    /// planet's centre, world axes), imports the tables and, when the atmosphere changed since
    /// they were built, declares the passes "sky/atmosphere tables" that rebuild them. From
    /// outside the atmosphere, with [`Atmosphere::planet_view`], it also imports the planet-view
    /// table, rebuilt ("sky/planet-view table") when the atmosphere, the camera or `sun` (the
    /// direction to the sun) changed since it was built.
    pub fn frame<'f>(
        &'f mut self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        view_position: Vec3,
        sun: Vec3,
    ) -> AtmosphereFrame {
        self.planets[slot.index].write(0, &[self.params.gpu(view_position)]);
        let rebuild = self.built != Some(self.params);
        self.built = Some(self.params);
        let outside = view_position.length() > self.params.top_radius;
        let view_key = (
            self.params,
            view_position.to_array(),
            sun.to_array(),
            self.planet_view_steps,
        );
        let view_rebuild = self.view_built != Some(view_key);
        let use_view = outside && self.planet_view;
        if use_view {
            self.view_built = Some(view_key);
        }
        let this: &'f Self = self;
        let planet = this.planets[slot.index].address();
        let transmittance = graph.import(&this.transmittance);
        let multiple_scattering = graph.import(&this.multiple_scattering);
        if rebuild {
            let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
            let pipeline = &this.transmittance_pipeline;
            graph
                .pass(LABEL)
                .image(transmittance, ImageAccess::StorageWrite(compute))
                .run(move |resources, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &lut_push(
                            planet,
                            resources.storage(transmittance, 0).0,
                            0,
                            TRANSMITTANCE_SIZE,
                        ),
                    );
                    let [w, h] = TRANSMITTANCE_SIZE;
                    commands.dispatch(w.div_ceil(GROUP_SIZE), h.div_ceil(GROUP_SIZE), 1);
                    Ok(())
                });
            let pipeline = &this.multiple_scattering_pipeline;
            graph
                .pass(LABEL)
                .image(transmittance, ImageAccess::Sampled(compute))
                .image(multiple_scattering, ImageAccess::StorageWrite(compute))
                .run(move |resources, commands| {
                    commands.bind_pipeline(pipeline);
                    commands.push_constants(
                        pipeline,
                        &lut_push(
                            planet,
                            resources.storage(multiple_scattering, 0).0,
                            resources.sampled(transmittance).0,
                            MULTIPLE_SCATTERING_SIZE,
                        ),
                    );
                    let [w, h] = MULTIPLE_SCATTERING_SIZE;
                    commands.dispatch(w.div_ceil(GROUP_SIZE), h.div_ceil(GROUP_SIZE), 1);
                    Ok(())
                });
        }
        let planet_view = use_view.then(|| {
            let table = graph.import(&this.view_table);
            let table_transmittance = graph.import(&this.view_transmittance);
            if view_rebuild {
                let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
                let pipeline = &this.planet_view_pipeline;
                graph
                    .pass(PLANET_VIEW_LABEL)
                    .image(transmittance, ImageAccess::Sampled(compute))
                    .image(multiple_scattering, ImageAccess::Sampled(compute))
                    .image(table, ImageAccess::StorageWrite(compute))
                    .image(table_transmittance, ImageAccess::StorageWrite(compute))
                    .run(move |resources, commands| {
                        commands.bind_pipeline(pipeline);
                        commands.push_constants(
                            pipeline,
                            &PlanetViewPush {
                                sun: sun.extend(0.0).to_array(),
                                planet,
                                transmittance: resources.sampled(transmittance).0,
                                multiple_scattering: resources.sampled(multiple_scattering).0,
                                luminance_out: resources.storage(table, 0).0,
                                transmittance_out: resources.storage(table_transmittance, 0).0,
                                steps: this.planet_view_steps,
                                pad: 0,
                            },
                        );
                        let [w, h] = PLANET_VIEW_SIZE;
                        commands.dispatch(w.div_ceil(GROUP_SIZE), h.div_ceil(GROUP_SIZE), 1);
                        Ok(())
                    });
            }
            (table, table_transmittance)
        });
        let distance = view_position.length();
        let top = this.params.top_radius;
        let cos_top = if distance > top {
            // A hair wider than the tangent cone: the march decides the edge.
            (1.0 - (top / distance).powi(2)).sqrt() - 1e-5
        } else {
            -1.0
        };
        AtmosphereFrame {
            planet,
            transmittance,
            multiple_scattering,
            direction: -view_position.normalize_or(Vec3::NEG_Z),
            cos_top,
            planet_view,
            march_steps: this.march_steps,
        }
    }
}

fn lut_push(planet: u64, output: u32, transmittance: u32, size: [u32; 2]) -> LutPush {
    LutPush {
        planet,
        output,
        transmittance,
        width: size[0],
        height: size[1],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GREEN: usize = 1;

    #[test]
    fn the_layouts_match_the_shaders() {
        assert_eq!(std::mem::size_of::<GpuPlanet>(), 7 * 16);
        assert_eq!(std::mem::size_of::<LutPush>(), 24);
    }

    #[test]
    fn earth_lets_most_of_the_noon_sun_through_and_reddens_the_sunset() {
        let earth = AtmosphereParams::earth();
        let ground = Vec3::new(0.0, earth.bottom_radius + 0.01, 0.0);
        let noon = earth.transmittance(ground, Vec3::Y, 400);
        // Rayleigh 0.108 + aerosols 0.005 + ozone 0.028 of optical depth in green.
        assert!((0.86..0.88).contains(&noon[GREEN]), "{noon:?}");
        assert!(noon[2] < noon[GREEN] && noon[GREEN] < noon[0] + 0.02);
        // The sun on the horizon crosses about 38 air masses: red survives, blue does not.
        let horizon = earth.transmittance(ground, Vec3::X, 4000);
        assert!(horizon[0] > 20.0 * horizon[2], "{horizon:?}");
        assert!(horizon[GREEN] < 0.05 && horizon[0] > 0.02, "{horizon:?}");
    }

    #[test]
    fn a_grazing_ray_from_space_is_reddened_by_the_limb_and_the_ground_blocks_lower_ones() {
        let earth = AtmosphereParams::earth();
        let camera = earth.view_from_space(Vec3::NEG_Z, 18_f32.to_radians());
        assert!((camera.length() - 20_581.0).abs() < 5.0, "{camera}");
        // Aim just past the ground's edge, at a tangent altitude of `h` km.
        let aim = |h: f32| {
            let r = earth.bottom_radius + h;
            let angle = (r / camera.length()).asin();
            let to_centre = -camera.normalize();
            let side = Vec3::Y;
            (to_centre * angle.cos() + side * angle.sin()).normalize()
        };
        let high = earth.transmittance(camera, aim(60.0), 4000);
        let low = earth.transmittance(camera, aim(10.0), 4000);
        assert!(high[GREEN] > 0.9, "{high:?}");
        assert!(low[0] > low[2] * 3.0 && low[GREEN] < 0.5, "{low:?}");
        assert_eq!(earth.transmittance(camera, aim(-5.0), 100), [0.0; 3]);
        // A ray that misses the atmosphere is clear.
        assert_eq!(earth.transmittance(camera, aim(150.0), 100), [1.0; 3]);
    }

    #[test]
    fn the_span_keeps_its_precision_twenty_thousand_kilometres_out() {
        let origin = Vec3::new(0.0, 0.0, 20_581.0);
        // A ray passing 100 m inside the sphere of 6460 km: a chord of 2 sqrt(2 r dh), 72 km.
        // Differences of squares (|o|² − r², 4e8 km² in f32) would be off by tens of km².
        let radius = 6460.0_f32;
        let h = radius - 0.1;
        let angle = (h / origin.length()).asin();
        let direction = Vec3::new(angle.sin(), 0.0, -angle.cos());
        let (near, far) = sphere_span(origin, direction, radius).expect("hits");
        let chord = far - near;
        let expected = 2.0 * (2.0 * radius * 0.1_f32).sqrt();
        assert!(
            (chord / expected - 1.0).abs() < 0.02,
            "{chord} vs {expected}"
        );
    }
}
