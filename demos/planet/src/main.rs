//! `planet` — a planet of Earth's size from orbit to its ground (D-056, #220): its ground is
//! cube-sphere tiles of cluster DAGs that `forge-terrain` makes from `assets/worlds/earth.toml`
//! and cooks through the cache, finer around a coast the camera descends to. Each tile's DAG
//! coarsens itself with distance, so the whole sphere draws from orbit and the coast's 2.4 m
//! samples near the ground. The sky is the island's (Hillaire's atmosphere at the planet's
//! radius), the shading the layered ground's with its layers by height, slope and latitude.
//!
//! The first step draws a fixed set of tiles chosen at start; the tiles that come and go as the
//! camera flies follow (#220).
//!
//! Controls: P pause the descent and fly (right mouse look, WASD, Shift faster), T TAA, O
//! occlusion, C cone culling, X show culled, K LOD colours, M meshlet colours, J shadows, N
//! ambient occlusion, G tone curve, - / = exposure compensation, [ ] LOD error, Tab wireframe.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::time::Instant;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use forge_app::{AppConfig, Context, Demo, Finish, FlyCamera, FrameInfo, HdrMode, Input, Setup};
use forge_core::MaterialTable;
use forge_core::material::{Material, RenderLayer, ShadingClass, TextureId};
use forge_geom::{CookOptions, MeshletMesh};
use forge_render::material::TextureSet;
use forge_render::meshlet::DrawParams;
use forge_render::textures::{self, TextureData};
use forge_render::{
    AmbientLight, Atmosphere, AtmosphereParams, AutoExposure, Bloom, CellPos, CullCamera,
    CullFlags, FrameStats, GroundSky, Gtao, GtaoParams, HdrOutput, LuminanceMeter, MeshletRenderer,
    MeshletScene, MeshletSceneBuilder, PlanetView, Residency, SkyBody, SkyBodyView, SkyBox,
    SkyBoxPlanet, SkyParams, Starfield, StartView, StreamingConfig, SwRaster, Taa, Tonemap,
    sky_from_body,
};
use forge_task::TaskPool;
use forge_terrain::planet::{SkyBodyParams, TourStop, tile_mesh, tile_name, tile_origin};
use forge_terrain::{Planet, PlanetWorld};
use forge_world::CellId;
use glam::{DQuat, DVec3, Vec3};
use winit::keyboard::KeyCode;

#[derive(Parser, Debug, Clone)]
#[command(about = "A planet from orbit to its ground (D-056)")]
struct Args {
    /// The world file (`assets/worlds/earth.toml` by default; `assets/worlds/moon.toml` the Moon).
    #[arg(long)]
    world: Option<PathBuf>,
    /// Print the world the run uses, every setting named, and exit.
    #[arg(long)]
    print_world: bool,
    /// The planet's radius, km, over the world file's (6 371 by default).
    #[arg(long)]
    radius: Option<f64>,
    /// The planet's seed, over the world file's.
    #[arg(long)]
    seed: Option<u64>,
    /// Hold the descent at one of its golden shots: orbit (the world's, 400 km on the Earth),
    /// high (10 km), ground (the
    /// coast) or top (400 km straight over the target, looking down).
    #[arg(long)]
    shot: Option<Shot>,
    /// The point the descent ends over, "LAT,LON" in degrees, over the world file's.
    #[arg(long, value_parser = parse_lat_lon, allow_hyphen_values = true)]
    target: Option<(f64, f64)>,
    /// The way the descent looks, degrees from north towards east, over the world file's.
    #[arg(long, allow_hyphen_values = true)]
    heading: Option<f64>,
    /// Fly the world's tour (`[[view.tour]]` in its file) instead of the descent.
    #[arg(long)]
    tour: bool,
    /// Hold the tour at its stop N (from 0), for a capture.
    #[arg(long, value_name = "N")]
    tour_stop: Option<usize>,
    /// Seconds from orbit to the ground.
    #[arg(long, default_value_t = 60.0)]
    duration: f32,
    /// Advance the descent by 1/120 s per frame instead of wall time (deterministic captures).
    #[arg(long)]
    fixed_step: bool,
    /// The sun's elevation over the target, degrees, over the world's.
    #[arg(long, allow_hyphen_values = true)]
    sun_elevation: Option<f64>,
    /// The sun's azimuth over the target, degrees from north towards east, over the world's.
    #[arg(long)]
    sun_azimuth: Option<f64>,
    /// Draw without the sun's ray-traced shadows (J toggles them).
    #[arg(long)]
    no_shadows: bool,
    /// Leave the sky's light unoccluded: no ambient occlusion (N toggles it).
    #[arg(long)]
    no_ao: bool,
    /// Every page resident instead of a streamed pool (the A/B against streaming).
    #[arg(long)]
    resident: bool,
    /// The streamed pool, MiB.
    #[arg(long, default_value_t = 512)]
    stream_pool: u32,
    /// Pages uploaded per frame at most, MiB.
    #[arg(long, default_value_t = 16)]
    stream_upload: u32,
    /// Window width in pixels.
    #[arg(long, default_value_t = 1600)]
    width: u32,
    /// Window height in pixels.
    #[arg(long, default_value_t = 900)]
    height: u32,
    /// Vertical sync.
    #[arg(long)]
    vsync: bool,
    /// Vulkan validation layer.
    #[arg(long)]
    validate: bool,
    /// Exit after this many frames.
    #[arg(long)]
    frames: Option<u64>,
    /// Write a PNG of frame `--capture-frame` to this path.
    #[arg(long)]
    capture: Option<PathBuf>,
    /// Which frame to capture.
    #[arg(long, default_value_t = 240)]
    capture_frame: u64,
    /// Also capture every N-th frame (`<capture stem>-NNNNN.png`).
    #[arg(long)]
    capture_every: Option<u64>,
    /// Start with TAA off (T toggles it).
    #[arg(long)]
    no_taa: bool,
    /// How strongly TAA's image is sharpened (FidelityFX RCAS, D-045), in stops below its
    /// strongest.
    #[arg(long, default_value_t = 0.5)]
    rcas: f32,
    /// Start with occlusion culling off (O toggles it).
    #[arg(long)]
    no_occlusion: bool,
    /// Start with cone culling off (C toggles it).
    #[arg(long)]
    no_cone: bool,
    /// Start with the culling-error view on (X toggles it): culled clusters drawn in red.
    #[arg(long)]
    show_culled: bool,
    /// Start with clusters coloured by LOD level (K toggles it).
    #[arg(long)]
    lod_colors: bool,
    /// Projected LOD error a drawn cluster may have, in pixels.
    #[arg(long, default_value_t = 1.0)]
    lod_error: f32,
    /// Draw through the indirect-count fallback: the device is created without mesh shaders.
    #[arg(long)]
    force_fallback: bool,
    /// When the software rasteriser draws the dense clusters: auto, on or off.
    #[arg(long, default_value = "auto")]
    sw_raster: SwRaster,
    /// Bloom strength, the share of the shown image that is bloom (0 for none).
    #[arg(long, default_value_t = 0.04)]
    bloom: f32,
    /// Tone curve: agx, agx-punchy, aces, aces2 or neutral (G cycles them).
    #[arg(long, default_value = "agx")]
    tonemap: Tonemap,
    /// The exposure value at ISO 100: 15 by default, sunny 16, as a camera takes anything the sun
    /// lights, in space as on the ground (a metered exposure, over a frame of black sky, burns a
    /// sunlit body white).
    #[arg(long, default_value_t = 15.0)]
    ev100: f32,
    /// Meter the exposure from the frame instead, starting from `--ev100`.
    #[arg(long)]
    auto_exposure: bool,
    /// How bright the sky box's stars are: a map value of 1 at 2^STOPS cd/m² (12: the bright
    /// stars faint beside a sunlit body, where an eye would see none).
    #[arg(long, default_value_t = 12.0, allow_hyphen_values = true)]
    stars: f32,
    /// The automatic exposure's compensation in stops (- / =).
    #[arg(long, default_value_t = 0.0, allow_hyphen_values = true)]
    exposure_compensation: f32,
    /// Force the profiling overlay on.
    #[arg(long)]
    overlay: bool,
    /// Force the profiling overlay off (F1 still toggles it).
    #[arg(long)]
    no_overlay: bool,
}

/// "LAT,LON" in degrees.
fn parse_lat_lon(text: &str) -> std::result::Result<(f64, f64), String> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().map_err(|e| e.to_string()))
        .collect::<std::result::Result<_, _>>()?;
    match parts.as_slice() {
        [lat, lon] => Ok((*lat, *lon)),
        _ => Err("expected two numbers: latitude,longitude".to_owned()),
    }
}

/// The descent's golden shots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
enum Shot {
    /// 400 km up, the coast ahead under the limb.
    Orbit,
    /// 10 km up.
    High,
    /// Over the coast.
    Ground,
    /// 400 km straight over the target, looking down: as the space station sees it.
    Top,
}

/// How far over the target's ground it ends, metres.
const LOW: f64 = 60.0;

/// The planet's tiles, cooked behind the loading screen.
struct Tiles {
    planet: Planet,
    /// The point the cut is finest around (a direction from the planet's centre).
    target: DVec3,
    cells: Vec<CellId>,
    meshes: Vec<MeshletMesh>,
    cooked: usize,
    ms: f64,
}

/// Cooks the tiles of the cut around the world's target in parallel; a tile in the cache under
/// the planet's key loads instead. `body` names them in the cache.
fn cook_tiles(planet: Planet, body: &str, streamed: bool) -> Result<Tiles> {
    let start = Instant::now();
    let world = &planet.world;
    let fine = world.min_wavelength(world.tiles.finest);
    let Some(target) = planet.target(50.0) else {
        bail!("no coast within a quarter of the planet from the equator: name a [view] target");
    };
    let cells = world.tile_cut(target);
    let mut per_level = [0u32; 21];
    for c in &cells {
        per_level[usize::from(c.level())] += 1;
    }
    tracing::info!(
        tiles = cells.len(),
        per_level = ?&per_level[..=usize::from(world.tiles.finest)],
        target_height_m = %format_args!("{:.1}", planet.height(target, fine)),
        "the planet's cut"
    );
    let cache = forge_core::derived::cache_dir(forge_core::derived::CacheKind::Meshes);
    let key = planet.tile_key();
    let pool = TaskPool::client();
    let mut slots: Vec<Option<(MeshletMesh, bool)>> = cells.iter().map(|_| None).collect();
    pool.scope(|s| {
        for (&cell, slot) in cells.iter().zip(slots.iter_mut()) {
            let (cache, key, planet) = (&cache, &key, &planet);
            s.spawn(move |_| {
                let name = tile_name(body, cell);
                let (done, stored) = forge_geom::cache::cook_cached(
                    cache,
                    &name,
                    key,
                    CookOptions { normal_weight: 0.0 },
                    !streamed,
                    || tile_mesh(planet, cell),
                );
                if let Err(error) = stored {
                    tracing::warn!(tile = %name, %error, "cooked tile not cached");
                }
                *slot = Some((done.mesh, done.from_cache));
            });
        }
    });
    let mut cooked = 0;
    let meshes = slots
        .into_iter()
        .map(|slot| {
            let (mesh, from_cache) = slot.expect("every tile cooked");
            cooked += usize::from(!from_cache);
            mesh
        })
        .collect();
    let ms = start.elapsed().as_secs_f64() * 1e3;
    tracing::info!(
        tiles = cells.len(),
        cooked,
        loaded = cells.len() - cooked,
        ms = %format_args!("{ms:.0}"),
        "the planet's tiles"
    );
    Ok(Tiles {
        planet,
        target,
        cells,
        meshes,
        cooked,
        ms,
    })
}

/// A plain or textured standard row for one of the planet's layers.
fn layer(
    texture: Option<(TextureId, TextureId)>,
    color: [f32; 3],
    scale: f32,
    power: f32,
    specular: f32,
) -> RenderLayer {
    RenderLayer {
        color_a: color,
        color_b: color,
        albedo_texture: texture.map(|t| t.0),
        normal_texture: texture.map(|t| t.1),
        texture_scale: scale,
        roughness: RenderLayer::roughness_for_power(power),
        specular,
        cavity: 0.0,
        ..RenderLayer::default()
    }
}

/// The planet's material: the layered ground (its layers from the frame's planet), then the
/// rows of `meshlet.slang`'s `PLANET_*` order: on the Earth the sea, sand, grass, rock and snow;
/// on an airless body regolith of a few kinds under its colour map. Also the colour map's
/// sampled index, when the world has one.
struct PlanetMaterials {
    table: MaterialTable,
    set: TextureSet,
    ground: forge_core::MaterialId,
    colour_map: Option<u32>,
    sea_mask: Option<u32>,
}

fn planet_materials(ctx: &Setup, world: &PlanetWorld) -> Result<PlanetMaterials> {
    let mut table = MaterialTable::new();
    let mut set = TextureSet::new(&ctx.device);
    let mut add = |[albedo, normal]: [TextureData; 2]| -> Result<_> {
        Ok((set.add(&albedo)?, set.add(&normal)?))
    };
    let sand = add(textures::grus(31, 512))?;
    let grass = add(textures::grass(32, 512))?;
    let rock = add(textures::granite(33, 512))?;
    let ground = table.add(Material::new(
        "planet ground",
        RenderLayer {
            class: ShadingClass::Layered,
            planet_layers: true,
            ..RenderLayer::default()
        },
    ));
    let rows = if world.view.atmosphere {
        [
            (
                "planet: sea",
                layer(None, [0.015, 0.035, 0.055], 1.0, 400.0, 0.5),
            ),
            (
                "planet: sand",
                layer(Some(sand), [1.25, 1.12, 0.92], 3.0, 10.0, 0.03),
            ),
            (
                "planet: grass",
                layer(Some(grass), [0.85, 0.95, 0.75], 12.0, 6.0, 0.02),
            ),
            (
                "planet: rock",
                layer(Some(rock), [1.0, 0.97, 0.94], 8.0, 20.0, 0.04),
            ),
            (
                "planet: snow",
                layer(None, [0.82, 0.86, 0.9], 1.0, 30.0, 0.2),
            ),
        ]
    } else {
        // Regolith: the colour map (about 0.6 in linear light, brightened for show) times these
        // makes the Moon's albedo of about 0.12; the textures only break it up.
        let regolith =
            |texture, grey: f32, scale| layer(Some(texture), [grey; 3], scale, 4.0, 0.01);
        [
            ("planet: regolith", regolith(sand, 0.22, 3.0)),
            ("planet: fine regolith", regolith(sand, 0.24, 2.0)),
            ("planet: regolith", regolith(sand, 0.22, 3.0)),
            ("planet: rock", regolith(rock, 0.3, 6.0)),
            ("planet: bright regolith", regolith(sand, 0.28, 3.0)),
        ]
    };
    for (name, row) in rows {
        table.add(Material::new(name, row));
    }
    // A map of the whole body, decoded with its mips: its sampled index.
    let mut image = |file: &String, use_: textures::ImageUse| -> Result<u32> {
        let path = forge_terrain::workspace_path(file);
        let bytes = std::fs::read(&path).map_err(|e| {
            anyhow::anyhow!(
                "the map {} ({e}; tools/fetch-planets.sh fetches and converts it)",
                path.display()
            )
        })?;
        let data = textures::decode_image(file, &bytes, use_)?;
        let id = set.add(&data)?;
        Ok(set.sampled(id))
    };
    let map = world.map.as_ref();
    let colour_map = map
        .and_then(|m| m.colour.as_ref())
        .map(|f| image(f, textures::ImageUse::Color))
        .transpose()?;
    let sea_mask = map
        .and_then(|m| m.sea_mask.as_ref())
        .map(|f| image(f, textures::ImageUse::Data))
        .transpose()?;
    Ok(PlanetMaterials {
        table,
        set,
        ground,
        colour_map,
        sea_mask,
    })
}

/// The planet's frame in the world: its centre straight under the target, which stands at the
/// world's origin with +Y its up, so the camera's yaw and pitch and the sky's up are the
/// target's.
#[derive(Clone, Copy, Debug)]
struct Placement {
    /// From the planet's axes to the world's.
    rotation: DQuat,
    /// The planet's centre in the world, metres.
    centre: DVec3,
    radius: f64,
    /// North and east at the target, in the world.
    north: DVec3,
    east: DVec3,
}

impl Placement {
    fn new(world: &PlanetWorld, target: DVec3) -> Self {
        let radius = world.planet.radius;
        let up = target.normalize();
        // North and east at the target (east along +X at a pole), then the way the descent looks.
        let north = (DVec3::Y - up * up.y).try_normalize().unwrap_or(DVec3::Z);
        let east = north.cross(up);
        let (sin, cos) = world.view.heading.to_radians().sin_cos();
        let heading = north * cos + east * sin;
        // The target up, then turned about it so the descent looks along −Z.
        let upright = DQuat::from_rotation_arc(up, DVec3::Y);
        let turn = DQuat::from_rotation_arc((upright * heading).normalize(), DVec3::NEG_Z);
        let rotation = turn * upright;
        Self {
            rotation,
            centre: DVec3::new(0.0, -radius, 0.0),
            radius,
            north: rotation * north,
            east: rotation * east,
        }
    }

    /// A point of the planet's frame (metres from its centre) in the world.
    fn world_point(&self, p: DVec3) -> DVec3 {
        self.centre + self.rotation * p
    }

    /// A world point as a direction from the planet's centre, in the planet's axes.
    fn direction(&self, p: DVec3) -> DVec3 {
        (self.rotation.inverse() * (p - self.centre)).normalize()
    }
}

struct PlanetDemo {
    args: Args,
    planet: Planet,
    placement: Placement,
    /// The target's ground, metres over the sea.
    target_height: f64,
    /// The body's colour map's and sea mask's sampled indices, if any.
    colour_map: Option<u32>,
    sea_mask: Option<u32>,
    tiles: usize,
    renderer: MeshletRenderer,
    scene: MeshletScene,
    /// The air and its sky (the Earth's), or none (the Moon's: space's black and the stars).
    air: Option<(Atmosphere, GroundSky)>,
    /// The same air seen from the ground under the camera, whose sky lights and is reflected by
    /// the ground while the camera is high: from orbit the camera's own sky is space's black.
    ground_air: Option<(Atmosphere, GroundSky)>,
    starfield: Starfield,
    /// The real sky (NASA's Deep Star Maps) when the world names it, and the rotation from the
    /// world's axes to the sky's.
    sky_box: Option<SkyBox>,
    /// The bodies in the sky, their passes and their settings.
    bodies: Vec<(SkyBody, SkyBodyParams)>,
    /// The tour's stop the camera holds at, for the log.
    tour_stop: Option<String>,
    /// The air's settings, for the light reaching a body through it.
    air_params: Option<AtmosphereParams>,
    sky_from_world: glam::Mat3,
    taa: Taa,
    taa_enabled: bool,
    bloom: Bloom,
    gtao: Gtao,
    ao_on: bool,
    meter: LuminanceMeter,
    exposure: AutoExposure,
    tonemap: Tonemap,
    camera: FlyCamera,
    flags: CullFlags,
    wireframe: bool,
    paused: bool,
    /// Seconds into the descent.
    time: f32,
    step: f32,
    stats: Vec<FrameStats>,
    gpu_ms: Vec<f64>,
}

impl PlanetDemo {
    fn new(ctx: &Setup, args: Args, tiles: Tiles) -> Result<Self> {
        let Tiles {
            planet,
            target,
            cells,
            meshes,
            cooked,
            ms,
        } = tiles;
        let start = Instant::now();
        let world = planet.world.clone();
        let placement = Placement::new(&world, target);
        let mut renderer = MeshletRenderer::new(&ctx.device, &ctx.shaders, ctx.extent())?;
        // The sun over the coast, through the air (the coast's sunlight lights every tile).
        let elevation = world.view.sun.1.to_radians();
        let azimuth = world.view.sun.0.to_radians();
        renderer.sun_dir = ((placement.north * azimuth.cos() + placement.east * azimuth.sin())
            * elevation.cos()
            + DVec3::Y * elevation.sin())
        .as_vec3();
        renderer.sun_illuminance = forge_render::starfield::SUN_ILLUMINANCE_1AU;
        let mut starfield = Starfield::new(&ctx.device, &ctx.shaders, forge_render::HDR_FORMAT)?;
        starfield.sun_illuminance = renderer.sun_illuminance;
        let mut air_params = None;
        let air = if world.view.atmosphere {
            // The Earth's air over a ground of the planet's radius; the sunlight through it at the
            // target lights every tile.
            let mut params = AtmosphereParams::earth();
            params.bottom_radius = (world.planet.radius * 1e-3) as f32;
            params.top_radius = params.bottom_radius + 100.0;
            renderer.sun_color = Vec3::from(params.transmittance(
                Vec3::new(0.0, params.bottom_radius + 0.05, 0.0),
                renderer.sun_dir,
                64,
            ));
            air_params = Some(params);
            let pair = || -> Result<_> {
                Ok((
                    Atmosphere::new(&ctx.device, &ctx.shaders, params)?,
                    GroundSky::new(&ctx.device, &ctx.shaders)?,
                ))
            };
            Some((pair()?, pair()?))
        } else {
            renderer.sun_color = Vec3::ONE;
            None
        };
        let (air, ground_air) = match air {
            Some((camera, ground)) => (Some(camera), Some(ground)),
            None => (None, None),
        };
        // The real sky, turned by the body's pole and meridian and by the placement.
        let sky_box = world
            .view
            .stars
            .as_ref()
            .map(|file| -> Result<SkyBox> {
                let path = forge_terrain::workspace_path(file);
                let bytes = std::fs::read(&path).map_err(|e| {
                    anyhow::anyhow!(
                        "the sky's map {} ({e}; tools/fetch-planets.sh sky fetches it)",
                        path.display()
                    )
                })?;
                let map = textures::decode_image(file, &bytes, textures::ImageUse::Color)?;
                let mut sky_box = SkyBox::new(&ctx.device, &ctx.shaders, &map)?;
                sky_box.luminance = 2f32.powf(args.stars);
                sky_box.sun_illuminance = renderer.sun_illuminance;
                Ok(sky_box)
            })
            .transpose()?;
        // The bodies in the sky, each with its map.
        let bodies = world
            .view
            .bodies
            .iter()
            .map(|body| -> Result<(SkyBody, SkyBodyParams)> {
                let path = forge_terrain::workspace_path(&body.map);
                let bytes = std::fs::read(&path).map_err(|e| {
                    anyhow::anyhow!(
                        "{}'s map {} ({e}; tools/fetch-planets.sh fetches it)",
                        body.name,
                        path.display()
                    )
                })?;
                let map = textures::decode_image(&body.map, &bytes, textures::ImageUse::Color)?;
                Ok((SkyBody::new(&ctx.device, &ctx.shaders, &map)?, body.clone()))
            })
            .collect::<Result<Vec<_>>>()?;
        let (pole_ra, pole_dec) = world.view.sky_pole;
        let sky_from_world = (sky_from_body(pole_ra, pole_dec, world.view.sky_meridian)
            * glam::DMat3::from_quat(placement.rotation.inverse()))
        .as_mat3();

        let mut builder = MeshletSceneBuilder::new();
        let PlanetMaterials {
            table,
            set,
            ground,
            colour_map,
            sea_mask,
        } = planet_materials(ctx, &world)?;
        builder.set_materials(&table, Some(set));
        builder.set_ray_traced(!args.no_shadows);
        let ids: Vec<_> = meshes.iter().map(|m| builder.add_mesh(m)).collect();
        // Every tile is ground to the rays, the finest under a kilometre across too.
        builder.set_ray_terrain(&ids);
        // The rays cut each level's tiles as one surface: a tile far from the coast is coarse
        // anyway, and its level's budget keeps the structures bounded.
        for level in 0..=world.tiles.finest {
            let members: Vec<_> = cells
                .iter()
                .zip(&ids)
                .filter(|(c, _)| c.level() == level)
                .map(|(_, &id)| id)
                .collect();
            if !members.is_empty() {
                builder.set_ray_group(&members, 120_000);
            }
        }
        let rotation = placement.rotation.as_quat();
        for (&cell, &id) in cells.iter().zip(&ids) {
            let at = placement.world_point(tile_origin(&world, cell));
            builder.add_instance_at(id, CellPos::from_f64(at), rotation, 1.0, ground);
        }
        let residency = if args.resident {
            Residency::All
        } else {
            Residency::Streamed(StreamingConfig::from_mib(
                args.stream_pool,
                args.stream_upload,
            ))
        };
        // The descent's first view: a streamed scene loads its pages before the first frame (#121),
        // so a shot's frames are the same every run.
        let fine = world.min_wavelength(world.tiles.finest);
        let target_height = planet.ground(target, fine);
        let mut camera = FlyCamera::default();
        place(
            args.shot,
            0.0,
            args.duration,
            &planet,
            &placement,
            target_height,
            &mut camera,
        );
        builder.set_start_view(StartView {
            position: CellPos::from_f64(camera.position.as_dvec3()),
            p11: camera.projection(ctx.aspect()).y_axis.y,
            near: camera.near,
            viewport_height: ctx.extent().height,
            lod_threshold_px: args.lod_error,
        });
        let mut scene = builder.build_with(&ctx.device, residency)?;
        scene.build_tlas(&ctx.device, &ctx.shaders)?;
        scene.load_start_view(&ctx.device)?;
        if let Some(rays) = scene.rays() {
            tracing::info!(
                blas_triangles = rays.triangles,
                mib = rays.bytes() >> 20,
                "acceleration structures"
            );
        }
        tracing::info!(
            tiles = cells.len(),
            cooked,
            cook_ms = %format_args!("{ms:.0}"),
            pages = scene.page_count,
            triangles = scene.total_triangles,
            build_ms = start.elapsed().as_millis(),
            "the planet's scene"
        );
        let mut flags = CullFlags::DEFAULT;
        if !args.no_shadows {
            flags.0 |= CullFlags::SHADOWS;
        }
        for (off, flag) in [
            (args.no_occlusion, CullFlags::OCCLUSION),
            (args.no_cone, CullFlags::CONE),
        ] {
            if off {
                flags.toggle(flag);
            }
        }
        for (on, flag) in [
            (args.show_culled, CullFlags::SHOW_CULLED),
            (args.lod_colors, CullFlags::LOD_COLORS),
        ] {
            if on {
                flags.toggle(flag);
            }
        }
        let mut taa = Taa::new(&ctx.device, &ctx.shaders, ctx.extent(), ctx.output.format)?;
        taa.bloom_strength = args.bloom;
        taa.sharpen = Some(args.rcas);
        let mut exposure = if args.auto_exposure {
            AutoExposure::new(args.ev100)
        } else {
            AutoExposure::fixed(args.ev100)
        };
        exposure.compensation = args.exposure_compensation;
        let mut demo = Self {
            taa_enabled: !args.no_taa,
            ao_on: !args.no_ao,
            tonemap: args.tonemap,
            planet,
            placement,
            target_height,
            colour_map,
            sea_mask,
            tiles: cells.len(),
            renderer,
            scene,
            air,
            ground_air,
            starfield,
            sky_box,
            sky_from_world,
            bodies,
            air_params,
            tour_stop: None,
            taa,
            bloom: Bloom::new(&ctx.device, &ctx.shaders)?,
            gtao: Gtao::new(&ctx.device, &ctx.shaders)?,
            meter: LuminanceMeter::new(&ctx.device, &ctx.shaders)?,
            exposure,
            camera,
            flags,
            wireframe: false,
            paused: false,
            time: 0.0,
            step: 0.0,
            stats: Vec::new(),
            gpu_ms: Vec::new(),
            args,
        };
        demo.place_camera();
        Ok(demo)
    }

    /// Places the camera on the tour where it is now (`--tour`, `--tour-stop`), or on the descent.
    fn place_camera(&mut self) {
        let stops = &self.planet.world.view.tour;
        if (self.args.tour || self.args.tour_stop.is_some()) && !stops.is_empty() {
            let time = match self.args.tour_stop {
                // Held a second into its stay, as a capture wants.
                Some(n) => {
                    let n = n.min(stops.len() - 1);
                    stops[..=n].iter().map(|s| s.travel).sum::<f64>()
                        + stops[..n].iter().map(|s| s.hold).sum::<f64>()
                        + 1.0
                }
                None => f64::from(self.time),
            };
            if let Some(name) = tour(stops, time, &self.planet, &self.placement, &mut self.camera)
                && self.tour_stop.as_deref() != Some(name)
            {
                tracing::info!(stop = name, time = %format_args!("{time:.1}"), "tour");
                self.tour_stop = Some(name.to_owned());
            }
            return;
        }
        place(
            self.args.shot,
            self.time,
            self.args.duration,
            &self.planet,
            &self.placement,
            self.target_height,
            &mut self.camera,
        );
    }

    /// The camera's height over the sea and over the ground under it, metres.
    fn heights(&self) -> (f64, f64) {
        let p = self.camera.position.as_dvec3();
        let over_sea = (p - self.placement.centre).length() - self.placement.radius;
        let fine = self
            .planet
            .world
            .min_wavelength(self.planet.world.tiles.finest);
        let ground = self.planet.ground(self.placement.direction(p), fine);
        (over_sea, over_sea - ground)
    }
}

/// `camera` `time` seconds into the tour of `stops`: travelling from one stop to the next along
/// the great circle between them, its height eased in its logarithm and raised over long hops (a
/// flight's arc, so it clears what lies between), its heading, pitch and field of view eased; or
/// holding at a stop. Returns the stop it holds at.
fn tour<'a>(
    stops: &'a [TourStop],
    time: f64,
    planet: &Planet,
    placement: &Placement,
    camera: &mut FlyCamera,
) -> Option<&'a str> {
    // A stop's pose: its direction (the planet's frame), height, heading, pitch, field of view.
    let pose = |s: &TourStop| {
        (
            forge_terrain::planet::direction(s.at.0, s.at.1),
            s.height,
            s.heading,
            s.pitch,
            s.fov,
        )
    };
    let mut start = 0.0;
    let mut held = None;
    let mut at = pose(&stops[0]);
    for (i, stop) in stops.iter().enumerate() {
        let arrive = start + stop.travel;
        let leave = arrive + stop.hold;
        if time < arrive && i > 0 {
            let from = pose(&stops[i - 1]);
            let to = pose(stop);
            let t = ((time - start) / stop.travel.max(1e-3)).clamp(0.0, 1.0);
            let s = t * t * (3.0 - 2.0 * t);
            let angle = from.0.angle_between(to.0);
            let direction = if angle < 1e-9 {
                to.0
            } else {
                // Slerp.
                (from.0 * ((1.0 - s) * angle).sin() + to.0 * (s * angle).sin()) / angle.sin()
            };
            let arc = 0.15 * angle * placement.radius * 4.0 * s * (1.0 - s);
            let height = (from.1.ln() + (to.1.ln() - from.1.ln()) * s).exp() + arc;
            let mut turn = (to.2 - from.2).rem_euclid(360.0);
            if turn > 180.0 {
                turn -= 360.0;
            }
            at = (
                direction,
                height,
                from.2 + turn * s,
                from.3 + (to.3 - from.3) * s,
                from.4 + (to.4 - from.4) * s,
            );
            break;
        }
        at = pose(stop);
        if time < leave {
            held = Some(stop.name.as_str());
            break;
        }
        start = leave;
    }
    let (direction, height, heading, pitch, fov) = at;
    let fine = planet.world.min_wavelength(planet.world.tiles.finest);
    let ground = planet.ground(direction, fine);
    let position =
        placement.world_point(direction * (placement.radius + ground + height.max(30.0)));
    // The view in the place's own frame: north, east and up there, turned into the world.
    let up = direction.normalize();
    let north = (DVec3::Y - up * up.y).try_normalize().unwrap_or(DVec3::Z);
    let east = north.cross(up);
    let (heading, pitch) = (heading.to_radians(), pitch.to_radians());
    let forward = placement.rotation
        * ((north * heading.cos() + east * heading.sin()) * pitch.cos() + up * pitch.sin());
    let forward = forward.as_vec3();
    camera.position = position.as_vec3();
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
    camera.yaw = -flat.x.atan2(-flat.z);
    camera.pitch = forward.y.asin().clamp(-1.55, 1.55);
    camera.fov_y = (fov as f32).to_radians();
    camera.near = (height as f32 * 0.1).clamp(0.05, 100.0);
    held
}

/// How far along the descent, 0 (orbit) to 1 (the ground): eased at both ends `time` seconds
/// into a descent of `duration`, or `shot`'s.
fn progress(shot: Option<Shot>, time: f32, duration: f32, orbit: f64) -> f64 {
    let at = |over: f64| ((orbit / over).ln() / (orbit / LOW).ln()).clamp(0.0, 1.0);
    match shot {
        Some(Shot::Orbit) => 0.0,
        Some(Shot::High) => at(10_000.0),
        Some(Shot::Ground) => 1.0,
        Some(Shot::Top) => 0.0,
        None => {
            let t = f64::from(time / duration.max(1e-3)).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        }
    }
}

/// `camera` on the descent at `s` (0 orbit, 1 the ground): its height over the target's ground
/// falls from orbit to the ground evenly in its logarithm, coming in over the sphere along the
/// world's heading to a point twice and a half that height short of the target.
fn descend(
    planet: &Planet,
    placement: &Placement,
    target_height: f64,
    s: f64,
    camera: &mut FlyCamera,
) {
    // Over the target's ground, not the sea's level: the Moon's ground lies kilometres under
    // its reference in places.
    let orbit = planet.world.view.orbit;
    let over = (orbit.ln() + (LOW.ln() - orbit.ln()) * s).exp();
    let altitude = target_height + over;
    // At most a tenth of the radius from the target: on a small body, farther would view its
    // curve from the side, stretched by the lens at the frame's edge.
    let distance = (2.5 * over + 150.0).min(0.1 * placement.radius);
    let angle = distance / placement.radius;
    let (sin, cos) = angle.sin_cos();
    let along = DVec3::new(0.0, cos, sin);
    // Kept 30 m over the ground under it at least: on the way in it may cross a mountain.
    let fine = planet.world.min_wavelength(planet.world.tiles.finest);
    let under = planet.ground(placement.rotation.inverse() * along, fine);
    let r = placement.radius + altitude.max(under + 30.0);
    let position = placement.centre + along * r;
    // It looks at the target from orbit, and further along its way as it comes down, 3 km past
    // it at the end: the coast ahead, the horizon in view.
    let target = DVec3::new(0.0, target_height, -3000.0 * s);
    let forward = (target - position).normalize().as_vec3();
    camera.position = position.as_vec3();
    let flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or(Vec3::NEG_Z);
    camera.yaw = -flat.x.atan2(-flat.z);
    camera.pitch = forward.y.asin().clamp(-1.5, 1.5);
    // The near plane follows the height over the ground: a tenth of it, 5 cm to 100 m.
    let over_ground = (r - placement.radius) - planet.ground(placement.direction(position), fine);
    camera.near = (over_ground as f32 * 0.1).clamp(0.05, 100.0);
}

/// `camera` at `shot` or `time` seconds into the descent: the top shot straight over the target from
/// orbit, looking down, the others on the descent ([`descend`]).
fn place(
    shot: Option<Shot>,
    time: f32,
    duration: f32,
    planet: &Planet,
    placement: &Placement,
    target_height: f64,
    camera: &mut FlyCamera,
) {
    if shot == Some(Shot::Top) {
        camera.position = DVec3::new(0.0, target_height + planet.world.view.orbit, 0.0).as_vec3();
        camera.yaw = 0.0;
        camera.pitch = -1.5;
        camera.near = 100.0;
        return;
    }
    let s = progress(shot, time, duration, planet.world.view.orbit);
    descend(planet, placement, target_height, s, camera);
}

impl Demo for PlanetDemo {
    fn key_pressed(&mut self, _ctx: &mut Context, code: KeyCode) {
        match code {
            KeyCode::KeyP => self.paused = !self.paused,
            KeyCode::KeyT => {
                self.taa_enabled = !self.taa_enabled;
                self.taa.reset_history();
            }
            KeyCode::KeyM => self.flags.toggle(CullFlags::MESHLET_COLORS),
            KeyCode::KeyO => self.flags.toggle(CullFlags::OCCLUSION),
            KeyCode::KeyC => self.flags.toggle(CullFlags::CONE),
            KeyCode::KeyX => self.flags.toggle(CullFlags::SHOW_CULLED),
            KeyCode::KeyK => self.flags.toggle(CullFlags::LOD_COLORS),
            KeyCode::KeyJ => self.flags.toggle(CullFlags::SHADOWS),
            KeyCode::KeyN => self.ao_on = !self.ao_on,
            KeyCode::KeyG => self.tonemap = self.tonemap.next(),
            KeyCode::Tab => self.wireframe = !self.wireframe,
            KeyCode::BracketLeft => self.args.lod_error = (self.args.lod_error * 0.5).max(0.125),
            KeyCode::BracketRight => self.args.lod_error = (self.args.lod_error * 2.0).min(16.0),
            KeyCode::Minus => self.exposure.compensation -= 0.5,
            KeyCode::Equal => self.exposure.compensation += 0.5,
            _ => {}
        }
    }

    fn update(&mut self, _ctx: &mut Context, input: &Input, dt: f32) {
        self.step = if self.args.fixed_step {
            1.0 / 120.0
        } else {
            dt
        };
        if self.paused {
            // Flying: faster the higher.
            let (_, over_ground) = self.heights();
            self.camera.speed = (over_ground as f32 * 0.5).clamp(10.0, 200_000.0);
            self.camera.update(input, dt);
            return;
        }
        self.time += self.step;
        self.place_camera();
    }

    fn render<'f>(&'f mut self, ctx: &mut Context, frame: &mut FrameInfo<'f>) -> Result<()> {
        let hdr = HdrOutput::new(ctx.output.peak, ctx.output.scene_stops, ctx.output.ui_white);
        self.taa.set_output(&ctx.shaders, ctx.output.format, hdr)?;
        if let Some(stats) = self.renderer.begin_frame(frame.slot, &mut self.scene)? {
            self.stats.push(stats);
            if let Some(ms) = frame.slot.previous_gpu_ms {
                self.gpu_ms.push(ms);
            }
        }
        let (over_sea, over_ground) = self.heights();
        // The near plane follows the height over the ground: a tenth of it, 5 cm to 100 m.
        self.camera.near = (over_ground as f32 * 0.1).clamp(0.05, 100.0);
        ctx.profile.counter(format!(
            "planet: radius {:.0} km, {} tiles, {:.1} M triangles; camera {:.1} km over the sea, {:.0} m over the ground",
            self.placement.radius * 1e-3,
            self.tiles,
            self.scene.total_triangles as f64 / 1e6,
            over_sea * 1e-3,
            over_ground
        ));
        if let Some(last) = self.stats.last() {
            ctx.profile.counter(format!(
                "drawn through {}: {} tiles, {:.0} k + {:.0} k clusters, {:.2} M triangles",
                self.renderer.path().name(),
                last.instances_visible,
                f64::from(last.meshlets_pass1) / 1e3,
                f64::from(last.meshlets_pass2) / 1e3,
                f64::from(last.triangles) / 1e6,
            ));
        }
        let histogram = self.meter.take(frame.slot);
        self.exposure.update(histogram.as_ref(), self.step);
        let exposure = self.exposure.exposure();
        let aspect = ctx.aspect();
        let cull = CullCamera::new(
            self.camera.view_rotation(),
            self.camera.projection(aspect),
            self.scene.origin().offset(self.camera.position),
            self.camera.near,
        );
        // The planet under the camera, for the ground's layers.
        let from_centre = self.camera.position.as_dvec3() - self.placement.centre;
        self.scene.set_planet(&PlanetView {
            camera: from_centre,
            radius: self.placement.radius,
            axis: (self.placement.rotation * DVec3::Y).as_vec3(),
            meridian: (self.placement.rotation * DVec3::Z).as_vec3(),
            sea: self.planet.world.map.as_ref().is_none_or(|m| m.sea),
            colour_map: self.colour_map,
            colour_albedo: self
                .planet
                .world
                .map
                .as_ref()
                .is_some_and(|m| m.colour_albedo),
            sea_mask: self.sea_mask,
        });
        let extent = ctx.extent();
        self.taa.enabled = self.taa_enabled;
        self.renderer.noise_frame =
            (self.taa.frame_index() % u64::from(self.taa.jitter_phases)) as u32;
        let taa_frame = self.taa.begin(
            &mut frame.graph,
            self.camera.projection(aspect),
            cull.view_proj,
            cull.position,
            exposure,
        );
        let draw_view_proj = taa_frame.jittered_projection * self.camera.view_rotation();
        let targets = self.renderer.draw(
            &mut frame.graph,
            frame.slot,
            DrawParams {
                scene: &self.scene,
                view_proj: draw_view_proj,
                cull,
                lod_threshold_px: self.args.lod_error,
                draw_jitter: taa_frame.jitter
                    / glam::Vec2::new(extent.width as f32, extent.height as f32),
                flags: self.flags,
                extent,
                wireframe: self.wireframe,
                exposure,
                sw_raster: self.args.sw_raster,
                instance_occlusion: forge_render::InstanceOcclusion::Auto,
                instance_cells: true,
                sw_raster_area: forge_render::meshlet::SW_RASTER_DEFAULT_AREA,
            },
        )?;
        // The sky: the camera from the planet's centre in km, world axes; the aerial
        // perspective reaching the horizon. Without air, space's black and the stars.
        let view_km = (from_centre * 1e-3).as_vec3();
        let horizon_km = ((2.0 * self.placement.radius * over_sea.max(1.0)).sqrt() * 1e-3) as f32;
        let sun_dir = self.renderer.sun_dir;
        let luminance_scale = self.renderer.sun_illuminance * exposure;
        let sky = self.air.as_mut().map(|(atmosphere, ground)| {
            let air = atmosphere.frame(&mut frame.graph, frame.slot, view_km, sun_dir);
            let ground: &'f GroundSky = ground;
            let tables = ground.tables(
                &mut frame.graph,
                frame.slot,
                &air,
                SkyParams {
                    view_proj: draw_view_proj,
                    camera: Vec3::ZERO,
                    sun_dir,
                    sun_angular_radius: forge_render::starfield::SUN_ANGULAR_RADIUS_1AU,
                    luminance_scale,
                    aerial_far_km: horizon_km.clamp(8.0, 64.0),
                    march_beyond: true,
                    night: None,
                },
                targets.depth,
                taa_frame.color,
                None,
                extent,
            );
            (ground, tables)
        });
        // High up, the ground is lit by the sky over it, not by the camera's: the air's tables
        // again from 2 m over the ground under the camera.
        let ground_light = if over_ground > 1500.0 {
            let under_km = (from_centre.normalize()
                * (self.placement.radius + over_sea - over_ground + 2.0)
                * 1e-3)
                .as_vec3();
            self.ground_air.as_mut().map(|(atmosphere, ground)| {
                let air = atmosphere.frame(&mut frame.graph, frame.slot, under_km, sun_dir);
                let ground: &'f GroundSky = ground;
                ground
                    .tables(
                        &mut frame.graph,
                        frame.slot,
                        &air,
                        SkyParams {
                            view_proj: draw_view_proj,
                            camera: Vec3::ZERO,
                            sun_dir,
                            sun_angular_radius: forge_render::starfield::SUN_ANGULAR_RADIUS_1AU,
                            luminance_scale,
                            aerial_far_km: 8.0,
                            march_beyond: false,
                            night: None,
                        },
                        targets.depth,
                        taa_frame.color,
                        None,
                        extent,
                    )
                    .light
            })
        } else {
            None
        };
        let occlusion = self.ao_on.then(|| {
            self.gtao.draw(
                &mut frame.graph,
                targets.depth,
                extent,
                GtaoParams {
                    projection: taa_frame.jittered_projection,
                    frame: self.taa.frame_index() % u64::from(self.taa.jitter_phases),
                    radius: 2.0,
                },
            )
        });
        self.renderer.resolve(
            &mut frame.graph,
            frame.slot,
            targets,
            taa_frame.color,
            extent,
            // With no air and a sky box, the sky starts black (the box adds to it).
            (sky.is_none() && self.sky_box.is_some()).then_some([0.0; 4]),
            AmbientLight {
                sky: ground_light.or(sky.as_ref().map(|(_, tables)| tables.light)),
                occlusion,
                probes: None,
                wet_ground: None,
                movers: None,
                clouds: None,
                sun_shadow: None,
                reflection_history: None,
            },
        );
        match (&sky, &self.sky_box) {
            (Some((ground, tables)), _) => ground.compose(
                &mut frame.graph,
                tables,
                targets.depth,
                taa_frame.color,
                extent,
            ),
            (None, Some(_)) => {}
            (None, None) => self.starfield.draw(
                &mut frame.graph,
                taa_frame.color,
                targets.depth,
                extent,
                draw_view_proj,
                sun_dir,
                exposure,
                None,
            ),
        }
        // The real sky (#220): added over the air's sky, fading through its lowest layers, or the
        // whole sky with the sun on an airless body.
        if let Some(sky_box) = &self.sky_box {
            let radius_km = (self.placement.radius * 1e-3) as f32;
            sky_box.draw(
                &mut frame.graph,
                taa_frame.color,
                targets.depth,
                extent,
                draw_view_proj,
                self.sky_from_world,
                sun_dir,
                exposure,
                Some(SkyBoxPlanet {
                    centre_km: (-from_centre * 1e-3).as_vec3(),
                    radius_km,
                    top_km: if sky.is_some() {
                        radius_km + 100.0
                    } else {
                        0.0
                    },
                }),
            );
        }
        // The bodies in the sky (#220): placed by their azimuth and elevation over the target
        // (far enough that the camera's moves don't shift them), lit by the sun through the air
        // towards them.
        for (body, params) in &self.bodies {
            let (azimuth, elevation) = (params.azimuth.to_radians(), params.elevation.to_radians());
            let direction = ((self.placement.north * azimuth.cos()
                + self.placement.east * azimuth.sin())
                * elevation.cos()
                + DVec3::Y * elevation.sin())
            .as_vec3();
            let through = self.air_params.map_or(1.0, |air| {
                let t = air.transmittance(view_km, direction, 32);
                0.2126 * t[0] + 0.7152 * t[1] + 0.0722 * t[2]
            });
            let air = params.air.map_or([0.0; 4], |(r, g, b, depth)| {
                [r as f32, g as f32, b as f32, depth as f32]
            });
            body.draw(
                &mut frame.graph,
                taa_frame.color,
                targets.depth,
                extent,
                draw_view_proj,
                &SkyBodyView {
                    direction,
                    angular_radius: (params.radius / params.distance).asin() as f32,
                    body_from_world: forge_render::skybody::facing(
                        direction,
                        params.facing.0 as f32,
                        params.facing.1 as f32,
                    ),
                    sun_dir,
                    sun_luminance: forge_render::exposure::lambertian_luminance(
                        self.renderer.sun_illuminance,
                    ) * exposure
                        * through,
                    albedo: params.albedo as f32,
                    rim: air,
                    through_air: sky.is_some(),
                },
            );
        }
        self.meter.measure(
            &mut frame.graph,
            frame.slot,
            taa_frame.color,
            extent,
            exposure,
        );
        let motion = self
            .taa
            .motion_vectors(&mut frame.graph, &taa_frame, targets.depth);
        let bloom = (self.args.bloom > 0.0).then(|| {
            self.bloom
                .draw(&mut frame.graph, taa_frame.color, taa_frame.extent)
        });
        self.taa.resolve(
            &mut frame.graph,
            &taa_frame,
            targets.depth,
            motion,
            frame.target,
            self.tonemap,
            bloom,
            None,
        );
        Ok(())
    }

    fn title(&mut self, _ctx: &Context) -> Option<String> {
        if self.stats.is_empty() {
            return None;
        }
        let gpu = self.gpu_ms.iter().sum::<f64>() / self.gpu_ms.len().max(1) as f64;
        let n = self.stats.len() as f64;
        let triangles = self
            .stats
            .iter()
            .map(|s| f64::from(s.triangles))
            .sum::<f64>()
            / n;
        let (over_sea, _) = self.heights();
        let title = format!(
            "forge planet | {} tiles | {:.1} km up | {:.2} M tris | GPU {:.2} ms | EV100 {:.1}{}",
            self.tiles,
            over_sea * 1e-3,
            triangles / 1e6,
            gpu,
            self.exposure.ev100,
            if self.paused { " [P flying]" } else { "" }
        );
        self.stats.clear();
        self.gpu_ms.clear();
        Some(title)
    }
}

fn main() -> Result<()> {
    let args = Args::parse();
    let path = args
        .world
        .clone()
        .unwrap_or_else(forge_terrain::planet_file);
    let mut world = PlanetWorld::load(&path)?;
    if let Some(radius) = args.radius {
        tracing::info!(radius_km = radius, "--radius over the world file");
        world.planet.radius = radius * 1e3;
    }
    if let Some(seed) = args.seed {
        world.planet.seed = seed;
    }
    if let Some(target) = args.target {
        world.view.target = Some(target);
    }
    if let Some(heading) = args.heading {
        world.view.heading = heading;
    }
    if let Some(azimuth) = args.sun_azimuth {
        world.view.sun.0 = azimuth;
    }
    if let Some(elevation) = args.sun_elevation {
        world.view.sun.1 = elevation;
    }
    if args.print_world {
        print!("{}", world.to_toml()?);
        return Ok(());
    }
    let config = AppConfig {
        title: "forge planet".into(),
        vsync: args.vsync,
        validate: args.validate,
        frame_limit: args.frames,
        capture: args.capture.clone().map(|p| (p, args.capture_frame)),
        capture_every: args.capture_every,
        overlay: if args.overlay {
            Some(true)
        } else if args.no_overlay {
            Some(false)
        } else {
            None
        },
        force_fallback: args.force_fallback,
        hdr: HdrMode::Off,
        width: args.width,
        height: args.height,
        ..AppConfig::default()
    };
    // The body's name in the cache: the world file's (`earth`, `moon`).
    let body = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("planet")
        .to_owned();
    // The map loads and the tiles cook behind the loading screen (or load from the cache); the
    // uploads follow.
    forge_app::run_loading(config, move || {
        let tiles = cook_tiles(Planet::new(world)?, &body, !args.resident)?;
        let finish: Finish<PlanetDemo> = Box::new(move |ctx| PlanetDemo::new(ctx, args, tiles));
        Ok(finish)
    })
}
