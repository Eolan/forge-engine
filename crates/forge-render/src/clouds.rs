//! The cloud layer (Phase 4, issue #145, `shaders/clouds.slang`), after Schneider's 2015
//! cloudscapes (docs/research/lighting-gi.md §6, D-034's "Nubis-style clouds"): a layer between
//! two shells over the planet, ray-marched at half resolution in `sky/clouds` and laid over the
//! sky by `sky/compose`.
//!
//! Its noises are baked once on the CPU and uploaded: a 64³ shape volume (a Perlin–Worley noise
//! and three Worley octaves) and a 32³ detail volume (three Worley octaves), both tiling and laid
//! out as 2-D atlases of their slices with a texel of wrapped border round each, and a 256²
//! weather map (coverage and the clouds' height), tiling too. The march's result is kept from a
//! frame to the next and blended with it, so the jitter of each pixel's start settles.

use std::cell::Cell;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use forge_gpu::{
    Buffer, BufferAccess, BufferDesc, ComputePipelineDesc, Device, FRAMES_IN_FLIGHT, FrameGraph,
    FrameSlot, GraphImage, Image, ImageAccess, ImageDesc, ImageHandle, MemoryCategory,
    MemoryLocation, Pipeline, QueueKind, Result, SampledImageId, ShaderCompiler, ShaderStage,
    TransientDesc, vk,
};
use glam::Mat4;

use crate::sky::{SKY_VIEW_SIZE, SkyFrame};

/// The noises' atlases (`SHAPE_*` and `DETAIL_*` in `clouds.slang`): texels a side of the
/// volume, tiles a side of the atlas.
const SHAPE: (u32, u32) = (64, 8);
const DETAIL: (u32, u32) = (32, 8);
/// The weather map's texels a side.
const WEATHER: u32 = 256;
const GROUP: u32 = 8;

/// Mirrors `Clouds` in `clouds.slang`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuClouds {
    previous: [f32; 16],
    camera: [f32; 4],
    layer: [f32; 4],
    shape: u32,
    detail: u32,
    weather: u32,
    history: u32,
    target: u32,
    width: u32,
    height: u32,
    frame: u32,
    sky: u64,
    /// The sky-view table with the clouds over it (storage; #163).
    table: u32,
    pad: u32,
    /// The shadow map's first corner (world x, z), metres a texel, 0 (its pass only).
    shadow: [f32; 4],
}

const _: () = assert!(std::mem::size_of::<GpuClouds>() == 160);

/// What a frame's clouds need besides the sky.
#[derive(Clone, Copy, Debug)]
pub struct CloudParams {
    /// The share of the sky they cover, 0 to 1.
    pub coverage: f32,
    /// Their extinction at full density, m⁻¹.
    pub density: f32,
    /// The layer's bottom and top over the ground, metres.
    pub layer: [f32; 2],
    /// The camera's world x and z, metres (the layer drifts past it, it does not follow it).
    pub camera: [f32; 2],
    /// How far the wind has carried the weather, world x and z, metres.
    pub drift: [f32; 2],
    /// Last frame's camera-relative view-projection, for the reprojection.
    pub previous: Mat4,
}

impl CloudParams {
    /// Fair-weather cumulus over half the sky, 1.5 to 4 km up.
    pub fn fair(coverage: f32) -> Self {
        Self {
            coverage,
            density: 0.06,
            layer: [1500.0, 4000.0],
            camera: [0.0; 2],
            drift: [0.0; 2],
            previous: Mat4::IDENTITY,
        }
    }
}

/// The clouds' shadow on the sun's light this frame ([`Clouds::shadow`]), for the resolve.
#[derive(Clone, Copy, Debug)]
pub struct CloudShadow {
    /// `r32f`: the sun's share through the layer for the ground under each texel (along the
    /// sun: a point at height y reads it `y / sun.y` towards the sun's foot).
    pub image: ImageHandle,
    /// Its first corner (world x, z), 1 / the metres it spans, 0.
    pub frame: [f32; 4],
}

/// The shadow map: texels a side, metres it spans round the camera.
const SHADOW_TEXELS: u32 = 384;
const SHADOW_SPAN: f32 = 30_000.0;

/// The cloud layer's noises, its pass and its two images, this frame's and last.
pub struct Clouds {
    pipeline: Pipeline,
    shadow_pipeline: Pipeline,
    _noises: [Image; 3],
    shape: SampledImageId,
    detail: SampledImageId,
    weather: SampledImageId,
    history: [GraphImage; 2],
    size: [u32; 2],
    /// The sky-view table with the clouds laid over it (#163), and its pass.
    table: GraphImage,
    table_pipeline: Pipeline,
    frame: Cell<u32>,
    blocks: Vec<Buffer>,
}

impl Clouds {
    /// Bakes the noises (about 0.1 s), compiles the march and makes its images, half of
    /// `extent` (a frame of another size samples them all the same).
    pub fn new(
        device: &Arc<Device>,
        shaders: &ShaderCompiler,
        extent: vk::Extent2D,
    ) -> Result<Self> {
        let compute = |entry: &str, name: &str| -> Result<Pipeline> {
            let module = device.create_shader_module(
                &shaders.compile("clouds.slang", entry, ShaderStage::Compute)?,
                name,
            )?;
            let pipeline = device.create_compute_pipeline(&ComputePipelineDesc {
                shader: (module, entry),
                push_constant_bytes: 8,
                name,
            });
            device.destroy_shader_module(module);
            pipeline
        };
        let pipeline = compute("march_main", "clouds");
        let shadow_pipeline = compute("shadow_main", "cloud shadow");
        let table_pipeline = compute("sky_table_main", "clouds over the sky");
        let upload = |side: u32, texels: Vec<u8>, name: &str| -> Result<(Image, SampledImageId)> {
            let image = device.create_image_with_data(
                ImageDesc {
                    width: side,
                    height: side,
                    format: vk::Format::R8G8B8A8_UNORM,
                    usage: vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name,
                },
                &texels,
            )?;
            let id = device
                .register_sampled_image(image.view(), vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL);
            Ok((image, id))
        };
        let (shape_side, shape_tiles) = SHAPE;
        let (shape_image, shape) = upload(
            (shape_side + 2) * shape_tiles,
            noise_atlas(shape_side, shape_tiles, |p| {
                let w = [worley(p, 4), worley(p, 8), worley(p, 16)];
                let fbm = w[0] * 0.625 + w[1] * 0.25 + w[2] * 0.125;
                let perlin = perlin_fbm(p, 4, 3).mul_add(0.5, 0.5).clamp(0.0, 1.0);
                // Schneider's Perlin–Worley: the Perlin noise remapped onto the Worley's floor.
                let pw = fbm + perlin * (1.0 - fbm);
                [pw, w[0], w[1], w[2]]
            }),
            "cloud shape noise",
        )?;
        let (detail_side, detail_tiles) = DETAIL;
        let (detail_image, detail) = upload(
            (detail_side + 2) * detail_tiles,
            noise_atlas(detail_side, detail_tiles, |p| {
                [worley(p, 4), worley(p, 8), worley(p, 16), 0.0]
            }),
            "cloud detail noise",
        )?;
        let (weather_image, weather) = upload(WEATHER, weather_map(), "cloud weather map")?;
        let size = [extent.width.div_ceil(2), extent.height.div_ceil(2)];
        let history = [0, 1].map(|i| {
            GraphImage::new(
                device,
                ImageDesc {
                    width: size[0],
                    height: size[1],
                    format: vk::Format::R16G16B16A16_SFLOAT,
                    usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                    aspect: vk::ImageAspectFlags::COLOR,
                    mip_levels: 1,
                    name: if i == 0 { "clouds a" } else { "clouds b" },
                },
            )
        });
        let [a, b] = history;
        let table = GraphImage::new(
            device,
            ImageDesc {
                width: SKY_VIEW_SIZE[0],
                height: SKY_VIEW_SIZE[1],
                format: vk::Format::R16G16B16A16_SFLOAT,
                usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
                aspect: vk::ImageAspectFlags::COLOR,
                mip_levels: 1,
                name: "sky-view table with clouds",
            },
        )?;
        // A block per frame slot for the march, and one for the shadow's pass.
        let blocks = (0..2 * FRAMES_IN_FLIGHT)
            .map(|i| {
                device.create_buffer(BufferDesc {
                    size: std::mem::size_of::<GpuClouds>() as u64,
                    usage: vk::BufferUsageFlags::STORAGE_BUFFER,
                    location: MemoryLocation::CpuToGpu,
                    category: MemoryCategory::Frame,
                    name: &format!("clouds {i}"),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            pipeline: pipeline?,
            shadow_pipeline: shadow_pipeline?,
            _noises: [shape_image, detail_image, weather_image],
            shape,
            detail,
            weather,
            history: [a?, b?],
            table,
            table_pipeline: table_pipeline?,
            size,
            frame: Cell::new(0),
            blocks,
        })
    }

    /// Declares `sky/cloud shadow`: the sun's share through the layer for the ground round the
    /// camera (`SHADOW_SPAN` metres, its corner snapped to its texels so the map does not crawl
    /// as the camera moves), for the resolve ([`crate::AmbientLight::clouds`]).
    pub fn shadow<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        sky: &SkyFrame,
        params: CloudParams,
    ) -> CloudShadow {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let image = graph.transient(TransientDesc {
            name: "cloud shadow",
            width: SHADOW_TEXELS,
            height: SHADOW_TEXELS,
            format: vk::Format::R32_SFLOAT,
            usage: vk::ImageUsageFlags::STORAGE | vk::ImageUsageFlags::SAMPLED,
            aspect: vk::ImageAspectFlags::COLOR,
            mip_levels: 1,
        });
        let texel = SHADOW_SPAN / SHADOW_TEXELS as f32;
        let corner = params
            .camera
            .map(|c| ((c - 0.5 * SHADOW_SPAN) / texel).floor() * texel);
        let block: &'f Buffer = &self.blocks[FRAMES_IN_FLIGHT + slot.index];
        let address = block.address();
        let pipeline = &self.shadow_pipeline;
        let sky_address = sky.address();
        let (shape, detail, weather) = (self.shape, self.detail, self.weather);
        graph
            .pass("sky/cloud shadow")
            .image(image, ImageAccess::StorageWrite(compute))
            .run(move |resources, commands| {
                block.write(
                    0,
                    &[GpuClouds {
                        previous: Mat4::IDENTITY.to_cols_array(),
                        camera: [
                            params.camera[0],
                            params.camera[1],
                            params.drift[0],
                            params.drift[1],
                        ],
                        layer: [
                            params.layer[0],
                            params.layer[1],
                            params.coverage,
                            params.density,
                        ],
                        shape: shape.0,
                        detail: detail.0,
                        weather: weather.0,
                        history: u32::MAX,
                        target: resources.storage(image, 0).0,
                        width: SHADOW_TEXELS,
                        height: SHADOW_TEXELS,
                        frame: 0,
                        sky: sky_address,
                        table: 0,
                        pad: 0,
                        shadow: [corner[0], corner[1], texel, 0.0],
                    }],
                );
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                commands.dispatch(
                    SHADOW_TEXELS.div_ceil(GROUP),
                    SHADOW_TEXELS.div_ceil(GROUP),
                    1,
                );
                Ok(())
            });
        CloudShadow {
            image,
            frame: [corner[0], corner[1], 1.0 / SHADOW_SPAN, 0.0],
        }
    }

    /// This frame's image and last frame's (none on the first): the sky's tables are told of
    /// the first ([`crate::GroundSky::tables`]), [`Clouds::march`] writes it from the second.
    pub fn images<'f>(&'f self, graph: &mut FrameGraph<'f>) -> (ImageHandle, Option<ImageHandle>) {
        let frame = self.frame.get();
        let this = graph.import(&self.history[(frame % 2) as usize]);
        let last = (frame > 0).then(|| graph.import(&self.history[((frame + 1) % 2) as usize]));
        (this, last)
    }

    /// Declares `sky/clouds`: the march into `this` (from [`Clouds::images`]), blended with
    /// `last`, after the sky's tables (its transmittance and irradiance).
    pub fn march<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        sky: &SkyFrame,
        params: CloudParams,
        (this, last): (ImageHandle, Option<ImageHandle>),
    ) {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let frame = self.frame.get();
        self.frame.set(frame.wrapping_add(1));
        let block: &'f Buffer = &self.blocks[slot.index];
        let address = block.address();
        let pipeline = &self.pipeline;
        let sky_address = sky.address();
        let (shape, detail, weather, size) = (self.shape, self.detail, self.weather, self.size);
        let table_storage = self.table.storage(0).0;
        let mut pass = graph
            .pass("sky/clouds")
            .image(sky.transmittance(), ImageAccess::Sampled(compute))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(compute))
            .image(this, ImageAccess::StorageWrite(compute));
        if let Some(last) = last {
            pass = pass.image(last, ImageAccess::Sampled(compute));
        }
        pass.run(move |resources, commands| {
            block.write(
                0,
                &[GpuClouds {
                    previous: params.previous.to_cols_array(),
                    camera: [
                        params.camera[0],
                        params.camera[1],
                        params.drift[0],
                        params.drift[1],
                    ],
                    layer: [
                        params.layer[0],
                        params.layer[1],
                        params.coverage,
                        params.density,
                    ],
                    shape: shape.0,
                    detail: detail.0,
                    weather: weather.0,
                    history: last.map_or(u32::MAX, |l| resources.sampled(l).0),
                    target: resources.storage(this, 0).0,
                    width: size[0],
                    height: size[1],
                    frame,
                    sky: sky_address,
                    table: table_storage,
                    pad: 0,
                    shadow: [0.0; 4],
                }],
            );
            commands.bind_pipeline(pipeline);
            commands.push_constants(pipeline, &address);
            commands.dispatch(size[0].div_ceil(GROUP), size[1].div_ceil(GROUP), 1);
            Ok(())
        });
    }

    /// Declares `sky/clouds over the sky` (#163), after [`Clouds::march`] (whose block it
    /// reads): the clouds along each of the sky-view table's directions, laid over the clear
    /// table, and returns that table. [`crate::GroundSky::light_with`] projects it into the sky's
    /// light with the clouds in it.
    pub fn sky_table<'f>(
        &'f self,
        graph: &mut FrameGraph<'f>,
        slot: FrameSlot,
        sky: &SkyFrame,
    ) -> ImageHandle {
        let compute = vk::PipelineStageFlags2::COMPUTE_SHADER;
        let address = self.blocks[slot.index].address();
        let pipeline = &self.table_pipeline;
        let table = graph.import(&self.table);
        graph
            .pass("sky/clouds over the sky")
            .queue(QueueKind::Compute)
            .image(sky.transmittance(), ImageAccess::Sampled(compute))
            .image(sky.sky_view(), ImageAccess::Sampled(compute))
            .buffer(sky.light.buffer, BufferAccess::ShaderRead(compute))
            .image(table, ImageAccess::StorageWrite(compute))
            .run(move |_, commands| {
                commands.bind_pipeline(pipeline);
                commands.push_constants(pipeline, &address);
                let [w, h] = SKY_VIEW_SIZE;
                commands.dispatch(w.div_ceil(GROUP), h.div_ceil(GROUP), 1);
                Ok(())
            });
        table
    }
}

/// A number in [0, 1) from integers (SplitMix64's finaliser over their mix).
fn hash(a: u32, b: u32, c: u32, salt: u32) -> f32 {
    let mut z = (u64::from(a) << 42 ^ u64::from(b) << 21 ^ u64::from(c) ^ u64::from(salt) << 58)
        .wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
}

/// A tiling Worley noise at `p` (in [0, 1)³) with `n` cells a side, inverted: 1 on a cell's
/// point, falling to 0 a cell away.
fn worley(p: [f32; 3], n: u32) -> f32 {
    let g = p.map(|v| v * n as f32);
    let cell = g.map(|v| v.floor() as i32);
    let mut nearest = f32::MAX;
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let c = [cell[0] + dx, cell[1] + dy, cell[2] + dz];
                let w = c.map(|v| v.rem_euclid(n as i32) as u32);
                let point = [
                    c[0] as f32 + hash(w[0], w[1], w[2], 1),
                    c[1] as f32 + hash(w[0], w[1], w[2], 2),
                    c[2] as f32 + hash(w[0], w[1], w[2], 3),
                ];
                let d = (point[0] - g[0]).powi(2)
                    + (point[1] - g[1]).powi(2)
                    + (point[2] - g[2]).powi(2);
                nearest = nearest.min(d);
            }
        }
    }
    (1.0 - nearest.sqrt()).clamp(0.0, 1.0)
}

/// A tiling gradient noise at `p` with `n` lattice cells a side, about −1 to 1 (Perlin's
/// improved noise, its twelve gradients, its fade).
fn perlin(p: [f32; 3], n: u32) -> f32 {
    const GRADIENTS: [[f32; 3]; 12] = [
        [1.0, 1.0, 0.0],
        [-1.0, 1.0, 0.0],
        [1.0, -1.0, 0.0],
        [-1.0, -1.0, 0.0],
        [1.0, 0.0, 1.0],
        [-1.0, 0.0, 1.0],
        [1.0, 0.0, -1.0],
        [-1.0, 0.0, -1.0],
        [0.0, 1.0, 1.0],
        [0.0, -1.0, 1.0],
        [0.0, 1.0, -1.0],
        [0.0, -1.0, -1.0],
    ];
    let g = p.map(|v| v * n as f32);
    let cell = g.map(|v| v.floor());
    let f = [g[0] - cell[0], g[1] - cell[1], g[2] - cell[2]];
    let fade = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let corner = |dx: u32, dy: u32, dz: u32| {
        let w = [
            (cell[0] as i32 + dx as i32).rem_euclid(n as i32) as u32,
            (cell[1] as i32 + dy as i32).rem_euclid(n as i32) as u32,
            (cell[2] as i32 + dz as i32).rem_euclid(n as i32) as u32,
        ];
        let k = ((hash(w[0], w[1], w[2], 7) * 12.0) as usize).min(11);
        let q = GRADIENTS[k];
        q[0] * (f[0] - dx as f32) + q[1] * (f[1] - dy as f32) + q[2] * (f[2] - dz as f32)
    };
    let (u, v, w) = (fade(f[0]), fade(f[1]), fade(f[2]));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(corner(0, 0, 0), corner(1, 0, 0), u);
    let x10 = lerp(corner(0, 1, 0), corner(1, 1, 0), u);
    let x01 = lerp(corner(0, 0, 1), corner(1, 0, 1), u);
    let x11 = lerp(corner(0, 1, 1), corner(1, 1, 1), u);
    lerp(lerp(x00, x10, v), lerp(x01, x11, v), w)
}

/// `octaves` of [`perlin`] from `n` cells, each twice as fine and half as strong.
fn perlin_fbm(p: [f32; 3], n: u32, octaves: u32) -> f32 {
    let (mut sum, mut weight, mut total) = (0.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += perlin(p, n << o) * weight;
        total += weight;
        weight *= 0.5;
    }
    sum / total
}

/// A tiling volume of `side` texels a side as an atlas of its slices, `tiles` a side, each
/// slice with a texel of border wrapped from its opposite edge; RGBA8 from `f` at each texel's
/// middle.
fn noise_atlas(side: u32, tiles: u32, f: impl Fn([f32; 3]) -> [f32; 4]) -> Vec<u8> {
    let bordered = side + 2;
    let width = bordered * tiles;
    let mut texels = vec![0u8; (width * width * 4) as usize];
    for z in 0..side {
        let (tx, ty) = (z % tiles, z / tiles);
        for y in 0..bordered {
            for x in 0..bordered {
                let sx = (x + side - 1) % side;
                let sy = (y + side - 1) % side;
                let p = [
                    (sx as f32 + 0.5) / side as f32,
                    (sy as f32 + 0.5) / side as f32,
                    (z as f32 + 0.5) / side as f32,
                ];
                let v = f(p);
                let at = (((ty * bordered + y) * width + tx * bordered + x) * 4) as usize;
                for (k, c) in v.iter().enumerate() {
                    texels[at + k] = (c.clamp(0.0, 1.0) * 255.0).round() as u8;
                }
            }
        }
    }
    texels
}

/// The weather map, tiling: in red where clouds may stand (a few octaves of gradient noise),
/// in green how tall they grow.
fn weather_map() -> Vec<u8> {
    let mut texels = vec![0u8; (WEATHER * WEATHER * 4) as usize];
    for y in 0..WEATHER {
        for x in 0..WEATHER {
            let p = [
                (x as f32 + 0.5) / WEATHER as f32,
                (y as f32 + 0.5) / WEATHER as f32,
                0.37,
            ];
            let coverage = perlin_fbm(p, 6, 4).mul_add(0.9, 0.5).clamp(0.0, 1.0);
            let height = perlin_fbm([p[0], p[1], 0.71], 3, 2)
                .mul_add(0.8, 0.5)
                .clamp(0.0, 1.0);
            let at = ((y * WEATHER + x) * 4) as usize;
            texels[at] = (coverage * 255.0).round() as u8;
            texels[at + 1] = (height * 255.0).round() as u8;
            texels[at + 3] = 255;
        }
    }
    texels
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_noises_tile() {
        // A noise and its value a period on agree to the rounding.
        for n in [4, 8] {
            for p in [[0.1, 0.2, 0.3], [0.77, 0.05, 0.5]] {
                let q = [p[0] + 1.0, p[1], p[2] - 1.0];
                assert!((worley(p, n) - worley(q, n)).abs() < 1e-4);
                assert!((perlin(p, n) - perlin(q, n)).abs() < 1e-4);
            }
        }
        let atlas = noise_atlas(8, 4, |p| [p[0], p[1], p[2], 1.0]);
        assert_eq!(atlas.len(), (10 * 4) * (10 * 4) * 4);
        // A tile's border texel holds its opposite edge's.
        let texel = |x: usize, y: usize| atlas[(y * 40 + x) * 4];
        assert_eq!(texel(0, 1), texel(8, 1));
        assert_eq!(texel(9, 1), texel(1, 1));
    }
}
