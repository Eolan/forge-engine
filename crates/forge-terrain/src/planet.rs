//! A planet's ground (#220, D-056): a world's description in ([`PlanetWorld`], read from
//! `assets/worlds/earth.toml` for the Earth or `moon.toml`), its ground out as tiles of D-037's
//! cube sphere, each a mesh for `forge_geom` to cook into a cluster DAG.
//!
//! - **The height** ([`Planet::height`]) comes from an elevation map of the whole body when the
//!   world names one ([`MapParams`]: the Earth's from NOAA's ETOPO 2022, the Moon's from NASA's
//!   CGI Moon Kit, fetched into `assets/planets/`), with seeded noise for the detail under its
//!   resolution, rougher where the map stands high. Without a map it is noise alone ([`PlanetParams`]):
//!   continents, ranges of ridges, hills. The sea is the ground flattened at its level for now
//!   (D-056's step 3 gives it waves).
//! - **Every level is the same function, smoothed:** a tile holds nothing narrower than four of its
//!   samples. The map is read from the level of its mip pyramid that holds no narrower detail,
//!   blended with the next, and the noise's narrower octaves are left out. A coarse tile is then
//!   the finer ones smoothed, not aliased.
//! - **A tile** is a cell of the cube sphere sampled on a grid of [`TileParams::samples`] a side,
//!   in the planet's axes relative to the cell's centre ([`tile_mesh`]). A tile next to a coarser
//!   one meets it with a step its skirt hides: a strip hanging from its edge, on vertices of its
//!   own so the simplifier keeps the edge locked.
//! - **The cut** ([`PlanetWorld::tile_cut`]) is a quadtree of tiles around a point, finer near
//!   it. Each tile's DAG coarsens itself with distance, so a fine tile seen from orbit costs
//!   little.
//!
//! The planet's frame: +Y is its north pole, longitude 0 lies towards +Z and east towards +X.
//! Deterministic (D-016): the directions and angles from `forge_core::dmath`, the noise from
//! integer hashes, the rest plain arithmetic.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, bail};
use forge_core::{Seed, dmath};
use forge_geom::procedural::TriMesh;
use forge_procgen::noise::gradient3;
use forge_world::{CellId, CubeSphere, Face, Partition};
use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::keys::code_digests;
use crate::world::{merge, shorten_floats};

/// A planet's world: its shape, how it is cut into tiles and how it is shown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetWorld {
    /// The planet's size and its noise.
    pub planet: PlanetParams,
    /// The elevation map of the whole body, or `false` for noise alone.
    #[serde(with = "forge_core::switch")]
    pub map: Option<MapParams>,
    /// Its tiles.
    pub tiles: TileParams,
    /// What the demo shows of it.
    pub view: ViewParams,
}

/// A planet's size and its noise, in metres: the whole ground without a map, the detail under a
/// map's resolution with one.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanetParams {
    /// The seed.
    pub seed: u64,
    /// The radius of the sea's level (or the map's reference), metres: the Earth's 6 371 km
    /// (D-056), the Moon's 1 737.4 km.
    pub radius: f64,
    /// The continents: the widest octave's wavelength, metres, and its octaves.
    pub continents: (f64, u32),
    /// Added to the continents' noise (about −1..1) before its sign makes land: more is more
    /// land.
    pub land_bias: f64,
    /// Metres the continents rise inland, and the sea's depth off their shelves.
    pub relief: (f64, f64),
    /// The ranges' ridges: the widest octave's wavelength, metres, its octaves, and their
    /// height, metres.
    pub ranges: (f64, u32, f64),
    /// The hills down to the finest detail: the widest octave's wavelength, metres, its
    /// octaves, their height, metres, and each octave's strength over the one before.
    pub hills: (f64, u32, f64, f64),
}

/// An elevation map of the whole body: an equirectangular grid of `i16` metres over the radius,
/// little-endian, the north row first, longitude from −180° (`assets/blender/planet_elevation.py`
/// makes it from the downloaded TIFF).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapParams {
    /// The grid's file, relative to the workspace.
    pub file: String,
    /// Its samples across (longitude) and down (latitude).
    pub size: (u32, u32),
    /// The noise under its resolution: the widest octave's wavelength, metres, its octaves, its
    /// strength on the highest ground, metres, and each octave's strength over the one before.
    pub detail: (f64, u32, f64, f64),
    /// The heights at which the detail reaches its strength (it is a quarter of it at the sea's
    /// level), metres.
    pub rough_above: f64,
    /// Whether the ground under 0 is sea, flattened at its level (the Earth's), or ground (the
    /// Moon's).
    pub sea: bool,
    /// A colour map of the whole body, an equirectangular PNG relative to the workspace, the north
    /// row first, longitude from −180° (the Moon's, `assets/blender/planet_colour.py`), or
    /// `false`: it tints the ground's layers.
    #[serde(with = "forge_core::switch")]
    pub colour: Option<String>,
    /// Whether the colour map is the ground's albedo seen from afar, in place of its layers (the
    /// Earth's Blue Marble), or a tint of its layers everywhere (the Moon's).
    pub colour_albedo: bool,
    /// A sea mask of the whole body (white over the sea), a PNG in the colour map's projection
    /// (`assets/blender/planet_sea_mask.py`), or `false`: from afar the sea follows it, not a
    /// coarse tile's triangles.
    #[serde(with = "forge_core::switch")]
    pub sea_mask: Option<String>,
}

/// How a planet is cut into tiles.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TileParams {
    /// Samples a side of a tile: 257 is 256 cells.
    pub samples: u32,
    /// The finest level of the cube sphere's cells (14 on Earth: tiles of 611 m, samples 2.4 m
    /// apart).
    pub finest: u8,
    /// A cell splits into its four children while the point the cut is around lies within
    /// this many of its sides of its centre (plus half its diagonal).
    pub rings: f64,
}

/// What the demo shows of a planet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewParams {
    /// The point the descent ends over, latitude and longitude in degrees, or `false` for the
    /// first coast north of the equator at longitude 0.
    #[serde(with = "forge_core::switch")]
    pub target: Option<(f64, f64)>,
    /// The way the descent looks, degrees from north towards east: it comes in from the
    /// opposite side.
    pub heading: f64,
    /// Where the descent starts, metres over the target's ground: 400 km on the Earth, the
    /// space station's height; a few thousand on the Moon, from where it is seen whole.
    pub orbit: f64,
    /// Whether it has an atmosphere (the Earth's), or space's black sky (the Moon's).
    pub atmosphere: bool,
    /// The sun over the target, degrees: its azimuth from north towards east, and its elevation.
    pub sun: (f64, f64),
    /// The sky's map (an equirectangular PNG relative to the workspace: NASA's Deep Star Maps,
    /// `forge_render::SkyBox`), or `false` for the procedural starfield.
    #[serde(with = "forge_core::switch")]
    pub stars: Option<String>,
    /// Where the body's north pole points in the sky, right ascension and declination in degrees
    /// (the IAU's α₀ and δ₀), and its prime meridian's angle W, degrees: they turn the stars.
    pub sky_pole: (f64, f64),
    /// See `sky_pole`.
    pub sky_meridian: f64,
    /// The bodies in its sky (the Moon over the Earth, the Earth over the Moon).
    pub bodies: Vec<SkyBodyParams>,
    /// The tour (`planet --tour`): its stops in turn.
    pub tour: Vec<TourStop>,
}

/// A body in a planet's sky (#220): the Moon over the Earth, the Earth over the Moon.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkyBodyParams {
    /// Its name, for the log.
    pub name: String,
    /// Its colour map, an equirectangular PNG or JPEG relative to the workspace (a planet's).
    pub map: String,
    /// Its map's scale to an albedo.
    pub albedo: f64,
    /// Its radius and its distance from the camera, metres.
    pub radius: f64,
    /// See `radius`.
    pub distance: f64,
    /// Where it stands over the target, degrees: from north towards east, and over the horizon.
    pub azimuth: f64,
    /// See `azimuth`.
    pub elevation: f64,
    /// Its point that faces the camera, latitude and longitude in degrees.
    pub facing: (f64, f64),
    /// Its air: the light at its limb (a share of the sun's term, RGB) and the air's depth (a
    /// share of its radius), or `false` for none.
    #[serde(with = "forge_core::switch")]
    pub air: Option<(f64, f64, f64, f64)>,
}

/// A stop of a planet's tour (#220): where the camera goes, how long it takes to get there and
/// how long it stays.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TourStop {
    /// Its name, for the log and the captures.
    pub name: String,
    /// Latitude and longitude, degrees.
    pub at: (f64, f64),
    /// Metres over the ground there.
    pub height: f64,
    /// The way the camera looks, degrees from north towards east, and over the horizon.
    pub heading: f64,
    /// See `heading`.
    pub pitch: f64,
    /// The vertical field of view, degrees.
    pub fov: f64,
    /// Seconds to get there from the stop before, and to stay.
    pub travel: f64,
    /// See `travel`.
    pub hold: f64,
}

impl Default for PlanetWorld {
    /// The planet as the code makes it: Earth's size, seed 1, noise alone.
    fn default() -> Self {
        Self {
            planet: PlanetParams {
                seed: 1,
                radius: 6_371_000.0,
                continents: (4.0e6, 6),
                land_bias: -0.05,
                relief: (700.0, 4000.0),
                ranges: (8.0e5, 7, 3500.0),
                hills: (4.0e4, 14, 300.0, 0.6),
            },
            map: None,
            tiles: TileParams {
                samples: 257,
                finest: 14,
                rings: 1.0,
            },
            view: ViewParams {
                target: None,
                heading: 0.0,
                orbit: 400_000.0,
                atmosphere: true,
                sun: (240.0, 25.0),
                stars: None,
                sky_pole: (0.0, 90.0),
                sky_meridian: 280.46,
                bodies: Vec::new(),
                tour: Vec::new(),
            },
        }
    }
}

impl PlanetWorld {
    /// Reads `text` over the code's values.
    pub fn parse(text: &str) -> Result<Self> {
        let file: toml::Table = text.parse().context("not TOML")?;
        let mut merged =
            toml::Table::try_from(Self::default()).context("the code's planet as TOML")?;
        merge(&mut merged, file);
        let world: Self = toml::Value::Table(merged).try_into()?;
        world.check()?;
        Ok(world)
    }

    /// Reads the file at `path`.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("the world file {}", path.display()))?;
        Self::parse(&text).with_context(|| format!("the world file {}", path.display()))
    }

    /// The description as TOML, every setting named.
    pub fn to_toml(&self) -> Result<String> {
        let mut table = toml::Table::try_from(self)?;
        shorten_floats(&mut table);
        Ok(toml::to_string(&table)?)
    }

    /// What the makers assume of it.
    fn check(&self) -> Result<()> {
        if self.planet.radius <= 0.0 {
            bail!("planet.radius must be positive");
        }
        if !(3..=1025).contains(&self.tiles.samples) {
            bail!("tiles.samples must be 3 to 1025");
        }
        if self.tiles.finest > 20 {
            bail!("tiles.finest must be 20 or less");
        }
        if let Some(map) = &self.map
            && (map.size.0 < 2 || map.size.1 < 2)
        {
            bail!("map.size must be at least 2 × 2");
        }
        Ok(())
    }

    /// The planet's sphere, cut by the cube's faces.
    pub fn sphere(&self) -> CubeSphere {
        CubeSphere {
            radius: self.planet.radius,
        }
    }

    /// Metres between a tile's samples at `level`.
    pub fn spacing(&self, level: u8) -> f64 {
        self.sphere().cell_size(level) / f64::from(self.tiles.samples - 1)
    }

    /// The narrowest wavelength a tile of `level` holds: four of its samples.
    pub fn min_wavelength(&self, level: u8) -> f64 {
        4.0 * self.spacing(level)
    }

    /// The tiles around `target` (a direction from the planet's centre): the cut around its
    /// point on the sphere ([`PlanetWorld::tile_cut_around`]).
    pub fn tile_cut(&self, target: DVec3) -> Vec<CellId> {
        self.tile_cut_around(&[self.sphere().surface_point(target, 0.0)])
    }

    /// The tiles around `points` (in the planet's frame, metres from its centre: a camera, a
    /// place it heads for): every cell whose centre lies within [`TileParams::rings`] of its
    /// sides (plus half its diagonal) of one of them splits into its four children, down to
    /// [`TileParams::finest`]. A point high over the ground splits no cell much smaller than its
    /// height, whose detail would be under its pixels. The cells that don't split cover the
    /// sphere once, in id order.
    pub fn tile_cut_around(&self, points: &[DVec3]) -> Vec<CellId> {
        let sphere = self.sphere();
        let mut stack: Vec<CellId> = Face::ALL
            .iter()
            .map(|f| CellId::cube(f.index(), 0, 0, 0))
            .collect();
        let mut cut = Vec::new();
        while let Some(cell) = stack.pop() {
            let level = cell.level();
            let reach =
                (self.tiles.rings + std::f64::consts::FRAC_1_SQRT_2) * sphere.cell_size(level);
            let centre = sphere.cell_center(cell);
            let near = points.iter().any(|&p| (centre - p).length() < reach);
            match cell.children() {
                Some(children) if near && level < self.tiles.finest => stack.extend(children),
                _ => cut.push(cell),
            }
        }
        cut.sort_unstable();
        cut
    }
}

/// The default world file: the Earth's, `assets/worlds/earth.toml`.
pub fn planet_file() -> PathBuf {
    crate::workspace_root().join("assets/worlds/earth.toml")
}

/// The direction of latitude `lat` and longitude `lon`, degrees, in the planet's frame.
pub fn direction(lat: f64, lon: f64) -> DVec3 {
    let (lat, lon) = (lat.to_radians(), lon.to_radians());
    let (sin_lat, cos_lat) = dmath::sin_cos(lat);
    let (sin_lon, cos_lon) = dmath::sin_cos(lon);
    DVec3::new(cos_lat * sin_lon, sin_lat, cos_lat * cos_lon)
}

/// The latitude and longitude of `direction` in the planet's frame, radians.
fn lat_lon(direction: DVec3) -> (f64, f64) {
    let d = direction.normalize();
    (dmath::asin(d.y.clamp(-1.0, 1.0)), dmath::atan2(d.x, d.z))
}

/// An elevation map, loaded: its grid and the levels of its mip pyramid, each half the last's
/// samples a side (a box filter), as `i16` metres.
pub struct Elevation {
    /// The levels, the full grid first: (width, height, samples).
    levels: Vec<(u32, u32, Vec<i16>)>,
    /// A digest of the full grid's bytes, for the tiles' keys.
    pub digest: u64,
}

impl Elevation {
    /// Reads `map`'s file and builds its pyramid.
    pub fn load(map: &MapParams) -> Result<Self> {
        let path = crate::workspace_root().join(&map.file);
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "the elevation map {} (tools/fetch-planets.sh fetches and converts it)",
                path.display()
            )
        })?;
        let (width, height) = map.size;
        let count = width as usize * height as usize;
        if bytes.len() != 2 * count {
            bail!(
                "{} holds {} bytes, not {width} × {height} i16 samples",
                path.display(),
                bytes.len()
            );
        }
        let digest = xxhash_rust::xxh3::xxh3_64(&bytes);
        let base: Vec<i16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&b| i16::from_le_bytes(b))
            .collect();
        let mut levels = vec![(width, height, base)];
        loop {
            let (w, h, last) = levels.last().expect("the full grid");
            if *w < 4 || *h < 4 {
                break;
            }
            let (nw, nh) = (w / 2, h / 2);
            let at = |x: u32, y: u32| i32::from(last[(y * w + x) as usize]);
            let next = (0..nh)
                .flat_map(|y| (0..nw).map(move |x| (x, y)))
                .map(|(x, y)| {
                    let sum = at(2 * x, 2 * y)
                        + at(2 * x + 1, 2 * y)
                        + at(2 * x, 2 * y + 1)
                        + at(2 * x + 1, 2 * y + 1);
                    // Rounded to the nearest metre, halves away from zero: the same everywhere.
                    (if sum >= 0 { sum + 2 } else { sum - 2 } / 4) as i16
                })
                .collect();
            levels.push((nw, nh, next));
        }
        Ok(Self { levels, digest })
    }

    /// The height at latitude `lat` and longitude `lon` (radians) on `level`, Catmull-Rom,
    /// wrapping round the longitude.
    fn sample(&self, level: usize, lat: f64, lon: f64) -> f64 {
        let (w, h, grid) = &self.levels[level];
        let (w, h) = (*w as i64, *h as i64);
        let u = (lon / std::f64::consts::TAU + 0.5) * w as f64 - 0.5;
        let v = (0.5 - lat / std::f64::consts::PI) * h as f64 - 0.5;
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let at = |x: i64, y: i64| {
            let x = x.rem_euclid(w);
            let y = y.clamp(0, h - 1);
            f64::from(grid[(y * w + x) as usize])
        };
        // Catmull-Rom across four samples each way (#220): through the samples as bilinear is,
        // but smooth across them, where bilinear left flat facets a map's texel wide (a peak a
        // few texels across stood as a square pyramid, its faces catching the sun in bands).
        let weights = |t: f64| {
            let (t2, t3) = (t * t, t * t * t);
            [
                0.5 * (-t3 + 2.0 * t2 - t),
                0.5 * (3.0 * t3 - 5.0 * t2 + 2.0),
                0.5 * (-3.0 * t3 + 4.0 * t2 + t),
                0.5 * (t3 - t2),
            ]
        };
        let (wx, wy) = (weights(fx), weights(fy));
        let mut sum = 0.0;
        for (j, wy) in wy.iter().enumerate() {
            let row: f64 = wx
                .iter()
                .enumerate()
                .map(|(i, wx)| wx * at(x0 + i as i64 - 1, y0 + j as i64 - 1))
                .sum();
            sum += wy * row;
        }
        sum
    }

    /// The height in `direction` holding no detail narrower than `min_wavelength` metres on a
    /// body of `radius`: the levels whose samples are half that apart, blended.
    pub fn height(&self, direction: DVec3, radius: f64, min_wavelength: f64) -> f64 {
        let (lat, lon) = lat_lon(direction);
        // The full grid's spacing along the equator; each level doubles it.
        let spacing = std::f64::consts::TAU * radius / f64::from(self.levels[0].0);
        let wanted = 0.5 * min_wavelength;
        let mut level = 0;
        let mut at = spacing;
        while at * 2.0 <= wanted && level + 1 < self.levels.len() {
            at *= 2.0;
            level += 1;
        }
        let here = self.sample(level, lat, lon);
        if wanted <= at || level + 1 == self.levels.len() {
            return here;
        }
        // Between this level and the next, by where the wanted spacing falls.
        let t = (wanted - at) / at;
        here + (self.sample(level + 1, lat, lon) - here) * t
    }
}

/// The loaded maps, one each per file, kept for the process.
/// The maps loaded so far, by file.
type Loaded = Mutex<Vec<(String, Arc<Elevation>)>>;

fn elevation(map: &MapParams) -> Result<Arc<Elevation>> {
    static MAPS: OnceLock<Loaded> = OnceLock::new();
    let maps = MAPS.get_or_init(|| Mutex::new(Vec::new()));
    let mut maps = maps.lock().expect("the maps' lock");
    if let Some((_, loaded)) = maps.iter().find(|(file, _)| *file == map.file) {
        return Ok(loaded.clone());
    }
    let start = std::time::Instant::now();
    let loaded = Arc::new(Elevation::load(map)?);
    tracing::info!(
        file = %map.file,
        levels = loaded.levels.len(),
        ms = start.elapsed().as_millis(),
        "elevation map"
    );
    maps.push((map.file.clone(), loaded.clone()));
    Ok(loaded)
}

/// A planet ready to make tiles: its world and its map, loaded.
#[derive(Clone)]
pub struct Planet {
    /// Its world.
    pub world: PlanetWorld,
    /// Its elevation map, when the world names one.
    pub map: Option<Arc<Elevation>>,
}

impl Planet {
    /// `world` with its map loaded (once a process).
    pub fn new(world: PlanetWorld) -> Result<Self> {
        let map = world.map.as_ref().map(elevation).transpose()?;
        Ok(Self { world, map })
    }

    /// The ground's height over the sea's level (the map's reference) in `direction` (from the
    /// planet's centre), metres, holding nothing narrower than `min_wavelength`; negative under
    /// the sea.
    pub fn height(&self, direction: DVec3, min_wavelength: f64) -> f64 {
        let planet = &self.world.planet;
        match (&self.map, &self.world.map) {
            (Some(map), Some(params)) => {
                let base = map.height(direction, planet.radius, min_wavelength);
                let p = direction.normalize() * planet.radius;
                let (width, count, strength, gain) = params.detail;
                let rough = 0.25 + 0.75 * smooth(base / params.rough_above);
                let detail = octaves(
                    Seed::new(planet.seed).derive_str("detail"),
                    p,
                    (width, count, gain),
                    min_wavelength,
                    |n| n,
                );
                base + strength * rough * detail
            }
            _ => planet.height(direction, min_wavelength),
        }
    }

    /// The ground as the tiles draw it: [`Self::height`], the sea's floor flattened at its level
    /// where the world has a sea.
    pub fn ground(&self, direction: DVec3, min_wavelength: f64) -> f64 {
        let height = self.height(direction, min_wavelength);
        let sea = self.world.map.as_ref().is_none_or(|m| m.sea);
        if sea { height.max(0.0) } else { height }
    }

    /// The point the descent ends over: the world's target, or the first coast north from the
    /// equator at longitude 0 at least `height` metres over the sea.
    pub fn target(&self, height: f64) -> Option<DVec3> {
        match self.world.view.target {
            Some((lat, lon)) => Some(direction(lat, lon)),
            None => self.coast(direction(0.0, 0.0), direction(90.0, 0.0), 500.0, height),
        }
    }

    /// The first point at least `height` metres over the sea from `from` walking towards `toward`
    /// along the great circle through both, in steps of `step` metres, after at least one point
    /// of sea: a coast to look at from its hills. `None` within a quarter of the circumference.
    pub fn coast(&self, from: DVec3, toward: DVec3, step: f64, height: f64) -> Option<DVec3> {
        let start = from.normalize();
        let axis = start.cross(toward.normalize()).normalize();
        let angle = step / self.world.planet.radius;
        let min_wavelength = self.world.min_wavelength(self.world.tiles.finest);
        let steps = (std::f64::consts::FRAC_PI_2 / angle) as u32;
        let mut sea = false;
        let mut at = start;
        // Rodrigues' rotation by `angle` about `axis` (perpendicular to `at`), step by step,
        // renormalised: the same arithmetic on every machine.
        let (sin, cos) = dmath::sin_cos(angle);
        for _ in 0..steps {
            let h = self.height(at, min_wavelength);
            if h >= height && sea {
                return Some(at);
            }
            sea |= h <= 0.0;
            at = (at * cos + axis.cross(at) * sin).normalize();
        }
        None
    }

    /// The cache key text of its tiles: every setting that shapes them, the map's digest and the
    /// digest of the code that makes and cooks them (#208), so any of them remakes them.
    pub fn tile_key(&self) -> String {
        // Only what shapes the ground: the colour maps tint it as it is drawn.
        let map = self
            .world
            .map
            .as_ref()
            .map(|m| (&m.file, m.size, m.detail, m.rough_above, m.sea));
        format!(
            "{:?}, map {:?} {:016x}, {} samples, made and cooked by {:016x}",
            self.world.planet,
            map,
            self.map.as_ref().map_or(0, |m| m.digest),
            self.world.tiles.samples,
            code_digests::CODE_PLANET_TILES
        )
    }
}

/// `t` clamped to 0..1 and eased (`3t² − 2t³`).
fn smooth(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Octaves of noise at `p` (metres): the first `wavelength` metres wide, each half as wide and
/// `gain` times as strong, `shape` applied to each, normalised by the strengths of all `count`.
/// Those narrower than `min_wavelength` are left out, fading over the octave above it, so the
/// sum is the same function at every level, smoothed below what a level samples.
fn octaves(
    seed: Seed,
    p: DVec3,
    (wavelength, count, gain): (f64, u32, f64),
    min_wavelength: f64,
    shape: impl Fn(f64) -> f64,
) -> f64 {
    let (mut sum, mut norm, mut strength, mut width) = (0.0, 0.0, 1.0, wavelength);
    for octave in 0..count {
        norm += strength;
        let fade = smooth(width / min_wavelength - 1.0);
        if fade > 0.0 {
            // Each octave's lattice is offset, so no point is a lattice point of them all.
            let q = p / width + DVec3::new(0.371, 0.613, 0.229) * f64::from(octave);
            let n = gradient3(seed.derive(u64::from(octave)).value(), q.x, q.y, q.z);
            sum += strength * fade * shape(n);
        }
        strength *= gain;
        width *= 0.5;
    }
    sum / norm
}

impl PlanetParams {
    /// The ground's height from noise alone over the sea's level in `direction` (from the
    /// planet's centre), metres, without the octaves narrower than `min_wavelength`; negative
    /// under the sea.
    pub fn height(&self, direction: DVec3, min_wavelength: f64) -> f64 {
        let seed = Seed::new(self.seed);
        let p = direction.normalize() * self.radius;
        let plain = |n: f64| n;
        // The continents: their sign is land or sea, their size the relief's.
        let (width, count) = self.continents;
        let c = octaves(
            seed.derive_str("continents"),
            p,
            (width, count, 0.5),
            min_wavelength,
            plain,
        ) + self.land_bias;
        let (rise, depth) = self.relief;
        let base = if c >= 0.0 {
            rise * smooth(c / 0.5)
        } else {
            -depth * smooth(-c / 0.3)
        };
        let inland = smooth(c / 0.35);
        // The ranges: ridges, in belts over the land.
        let (width, count, height) = self.ranges;
        let ridge = octaves(
            seed.derive_str("ranges"),
            p,
            (width, count, 0.5),
            min_wavelength,
            |n| {
                let r = 1.0 - n.abs().min(1.0);
                r * r
            },
        );
        let belts = octaves(
            seed.derive_str("belts"),
            p,
            (2.0 * width, 3, 0.5),
            min_wavelength,
            plain,
        );
        let ranges =
            height * ridge * ridge * smooth((c - 0.05) / 0.3) * smooth((belts + 0.1) / 0.4);
        // The hills, rougher inland than on the sea's floor.
        let (width, count, height, gain) = self.hills;
        let hills = height
            * octaves(
                seed.derive_str("hills"),
                p,
                (width, count, gain),
                min_wavelength,
                plain,
            )
            * (0.25 + 0.75 * inland);
        base + ranges + hills
    }
}

/// A tile's place: its cell's centre on the sphere at the sea's level, in the planet's axes,
/// metres from its centre. The tile's vertices are relative to it.
pub fn tile_origin(world: &PlanetWorld, cell: CellId) -> DVec3 {
    world.sphere().cell_center(cell)
}

/// The depth of the skirt hanging from a tile of `level`'s edge, metres: twice the strength
/// of the noise's octaves it leaves out and the relief of the map's levels it skips, where a
/// finer neighbour may stand higher or lower, plus a quarter of its spacing for the straight
/// edges between its samples.
pub fn skirt_depth(world: &PlanetWorld, level: u8) -> f64 {
    let min_wavelength = world.min_wavelength(level);
    let p = &world.planet;
    // The strength of the octaves of `(wavelength, count, gain)` below `min_wavelength` (with
    // the one fading out), over all their strengths.
    let dropped = |(wavelength, count, gain): (f64, u32, f64)| {
        let (mut left, mut norm, mut strength, mut width) = (0.0, 0.0, 1.0, wavelength);
        for _ in 0..count {
            norm += strength;
            if width < 2.0 * min_wavelength {
                left += strength;
            }
            strength *= gain;
            width *= 0.5;
        }
        left / norm
    };
    let noise = match &world.map {
        Some(map) => {
            let (width, count, strength, gain) = map.detail;
            // The map's relief within a few of its samples: a slope of one in three over the
            // spacing it smooths away.
            strength * dropped((width, count, gain)) + 0.15 * min_wavelength.min(30_000.0)
        }
        None => {
            p.hills.2 * dropped((p.hills.0, p.hills.1, p.hills.3))
                + p.ranges.2 * dropped((p.ranges.0, p.ranges.1, 0.5))
                + (p.relief.0 + p.relief.1) * dropped((p.continents.0, p.continents.1, 0.5))
        }
    };
    2.0 * noise + 0.25 * world.spacing(level)
}

/// The ground's mesh over `cell`: its samples on the sphere at their heights
/// ([`Planet::ground`]), relative to [`tile_origin`], normals from the samples around each (one
/// beyond the edge, so neighbours of a level share their edge's normals), two triangles per cell
/// facing out, and the skirt ([`skirt_depth`]) under the edge.
pub fn tile_mesh(planet: &Planet, cell: CellId) -> TriMesh {
    let world = &planet.world;
    let n = world.tiles.samples as i64;
    let level = cell.level();
    let face = Face::from_index(cell.face());
    let cells = f64::from(1_u32 << level);
    let (x, y) = cell.xy();
    let start = DVec2::new(x as f64, y as f64) / cells * 2.0 - 1.0;
    let step = 2.0 / cells / (n - 1) as f64;
    let min_wavelength = world.min_wavelength(level);
    let radius = world.planet.radius;
    let origin = tile_origin(world, cell);
    // The samples and a ring beyond, row-major from (−1, −1).
    let side = (n + 2) as usize;
    let mut points = vec![DVec3::ZERO; side * side];
    for j in -1..=n {
        for i in -1..=n {
            let st = start + DVec2::new(i as f64, j as f64) * step;
            let direction = CubeSphere::direction(face, st);
            let height = planet.ground(direction, min_wavelength);
            points[(j + 1) as usize * side + (i + 1) as usize] = direction * (radius + height);
        }
    }
    let at = |i: i64, j: i64| points[(j + 1) as usize * side + (i + 1) as usize];
    let mut mesh = TriMesh::default();
    let count = (n * n) as usize;
    mesh.positions.reserve(count + 4 * n as usize);
    mesh.normals.reserve(count + 4 * n as usize);
    mesh.uvs.reserve(count + 4 * n as usize);
    // Over the tile 0 to 1, for its normal map ([`tile_normal_map`]).
    let uv = |i: i64, j: i64| [i as f32 / (n - 1) as f32, j as f32 / (n - 1) as f32];
    for j in 0..n {
        for i in 0..n {
            let p = at(i, j);
            // Along u, then along v: the face's axes are right-handed, so the cross product
            // points out of the planet.
            let normal = (at(i + 1, j) - at(i - 1, j))
                .cross(at(i, j + 1) - at(i, j - 1))
                .normalize_or(p.normalize());
            mesh.positions.push((p - origin).as_vec3().to_array());
            mesh.normals.push(normal.as_vec3().to_array());
            mesh.uvs.push(uv(i, j));
        }
    }
    let index = |i: i64, j: i64| (j * n + i) as u32;
    mesh.indices
        .reserve(((n - 1) * (n - 1) * 6 + 4 * (n - 1) * 6) as usize);
    for j in 0..n - 1 {
        for i in 0..n - 1 {
            let (a, b, c, d) = (
                index(i, j),
                index(i + 1, j),
                index(i, j + 1),
                index(i + 1, j + 1),
            );
            mesh.indices.extend_from_slice(&[a, b, c, b, d, c]);
        }
    }
    // The skirt: the edge's samples copied (their own vertices, so the surface's edge stays
    // an open border the simplifier locks), and again lowered towards the centre, joined by
    // quads facing out of the tile. The edge walked counter-clockwise seen from outside.
    let depth = skirt_depth(world, level);
    let mut edge: Vec<(i64, i64)> = Vec::with_capacity(4 * (n - 1) as usize + 1);
    edge.extend((0..n - 1).map(|i| (i, 0)));
    edge.extend((0..n - 1).map(|j| (n - 1, j)));
    edge.extend((0..n - 1).map(|i| (n - 1 - i, n - 1)));
    edge.extend((0..n - 1).map(|j| (0, n - 1 - j)));
    let first = mesh.positions.len() as u32;
    for &(i, j) in &edge {
        let p = at(i, j);
        let normal = mesh.normals[index(i, j) as usize];
        let down = p - p.normalize() * depth;
        mesh.positions.push((p - origin).as_vec3().to_array());
        mesh.positions.push((down - origin).as_vec3().to_array());
        mesh.normals.push(normal);
        mesh.normals.push(normal);
        mesh.uvs.push(uv(i, j));
        mesh.uvs.push(uv(i, j));
    }
    let ring = edge.len() as u32;
    for k in 0..ring {
        let (top, bottom) = (first + 2 * k, first + 2 * k + 1);
        let next = (k + 1) % ring;
        let (next_top, next_bottom) = (first + 2 * next, first + 2 * next + 1);
        // Walking the edge counter-clockwise seen from outside, the tile lies to the left:
        // the quad (top, bottom, next bottom, next top) faces away from it.
        mesh.indices
            .extend_from_slice(&[top, bottom, next_top, next_top, bottom, next_bottom]);
    }
    mesh
}

/// The cache name of `cell`'s tile on the planet called `body`: `body@face-level-x-y`.
pub fn tile_name(body: &str, cell: CellId) -> String {
    let (x, y) = cell.xy();
    format!("{body}@{}-{}-{x}-{y}", cell.face(), cell.level())
}

/// Texels a side of a tile's normal map ([`tile_normal_map`]).
pub const NORMAL_MAP_SIZE: u32 = 256;

/// The normal map of `cell`'s tile (#220): [`NORMAL_MAP_SIZE`]² texels over the tile (its
/// UVs), each the ground's normal at its centre in the planet's frame, from the height of the
/// next level down.
///
/// It holds the slopes the tile's children add: their finest wavelength, two of the tile's
/// samples, is the map's Nyquist. A tile swapped for its children then changes no slope that
/// shows, and the map's mips filter the slopes a coarse cluster's vertices only sample (the
/// cluster DAG keeps each vertex's own normal). RGBA8 with `(n + 1) / 2` in RGB, rows of
/// increasing `v` first, level 0 only ([`normal_map_levels`] makes the mips).
pub fn tile_normal_map(planet: &Planet, cell: CellId) -> Vec<u8> {
    let world = &planet.world;
    let size = NORMAL_MAP_SIZE as usize;
    let level = cell.level();
    let face = Face::from_index(cell.face());
    let cells = f64::from(1_u32 << level);
    let (x, y) = cell.xy();
    let start = DVec2::new(x as f64, y as f64) / cells * 2.0 - 1.0;
    // Samples half a texel apart: the texels' centres are the odd ones, their neighbours the
    // even ones either side.
    let half = 2.0 / cells / (2 * size) as f64;
    let min_wavelength = world.min_wavelength(level + 1);
    let radius = world.planet.radius;
    let side = 2 * size + 1;
    let mut points = vec![DVec3::ZERO; side * side];
    for j in 0..side {
        for i in 0..side {
            let st = start + DVec2::new(i as f64, j as f64) * half;
            let direction = CubeSphere::direction(face, st);
            points[j * side + i] = direction * (radius + planet.ground(direction, min_wavelength));
        }
    }
    let at = |i: usize, j: usize| points[j * side + i];
    let mut texels = Vec::with_capacity(size * size * 4);
    for j in 0..size {
        for i in 0..size {
            let (ci, cj) = (2 * i + 1, 2 * j + 1);
            // Along u, then along v, as the tile's own normals.
            let normal = (at(ci + 1, cj) - at(ci - 1, cj))
                .cross(at(ci, cj + 1) - at(ci, cj - 1))
                .normalize_or(at(ci, cj).normalize());
            texels.extend_from_slice(&encode_normal(normal));
        }
    }
    texels
}

/// A unit normal as RGBA8: `(n + 1) / 2` in RGB, alpha 255.
fn encode_normal(n: DVec3) -> [u8; 4] {
    let byte = |v: f64| ((v * 0.5 + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(n.x), byte(n.y), byte(n.z), 255]
}

/// A normal map's every level from its first ([`tile_normal_map`], `size`² texels, a power of
/// two): each texel of a level the mean of its four below, normalised.
pub fn normal_map_levels(first: Vec<u8>, size: u32) -> Vec<Vec<u8>> {
    let decode = |t: &[u8]| {
        DVec3::new(f64::from(t[0]), f64::from(t[1]), f64::from(t[2])) / 127.5 - DVec3::ONE
    };
    let mut levels = vec![first];
    let mut side = size as usize;
    while side > 1 {
        let below = levels.last().expect("a level");
        let half = side / 2;
        let mut level = Vec::with_capacity(half * half * 4);
        for j in 0..half {
            for i in 0..half {
                let texel = |di: usize, dj: usize| {
                    let at = ((2 * j + dj) * side + 2 * i + di) * 4;
                    decode(&below[at..at + 4])
                };
                let sum = texel(0, 0) + texel(1, 0) + texel(0, 1) + texel(1, 1);
                level.extend_from_slice(&encode_normal(sum.normalize_or(DVec3::Z)));
            }
        }
        levels.push(level);
        side = half;
    }
    levels
}

/// [`tile_normal_map`] of `cell` on the planet called `body`, kept in the world cache by the
/// tiles' key: made once, then loaded.
pub fn tile_normal_map_cached(planet: &Planet, body: &str, cell: CellId) -> Vec<u8> {
    let key = forge_core::derived::KeyHasher::new()
        .debug(&planet.tile_key())
        .number(cell.0)
        .key(code_digests::CODE_PLANET_TILES);
    crate::keys::derived_cache()
        .get_or_make(&format!("{}-normals", tile_name(body, cell)), key, || {
            tile_normal_map(planet, cell)
        })
        .value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small() -> Planet {
        let mut world = PlanetWorld::default();
        world.tiles.samples = 9;
        world.tiles.finest = 6;
        Planet::new(world).unwrap()
    }

    #[test]
    fn the_world_file_reads_over_the_code_s_values() {
        let world = PlanetWorld::parse("[planet]\nseed = 5\n").unwrap();
        assert_eq!(world.planet.seed, 5);
        assert_eq!(world.planet.radius, 6_371_000.0);
        assert!(world.map.is_none());
        assert!(PlanetWorld::parse("[planet]\nsize = 5\n").is_err());
        for file in ["earth.toml", "moon.toml"] {
            let path = crate::workspace_root().join("assets/worlds").join(file);
            let world = PlanetWorld::load(&path).unwrap();
            assert_eq!(
                PlanetWorld::parse(&world.to_toml().unwrap()).unwrap(),
                world
            );
        }
    }

    #[test]
    fn latitude_and_longitude_round_trip() {
        for (lat, lon) in [(0.0, 0.0), (43.7, 7.3), (-60.0, -120.0), (10.0, 179.0)] {
            let (la, lo) = lat_lon(direction(lat, lon));
            assert!((la.to_degrees() - lat).abs() < 1e-9 && (lo.to_degrees() - lon).abs() < 1e-9);
        }
        // Longitude 0 towards +Z, east towards +X, north +Y.
        assert!((direction(0.0, 0.0) - DVec3::Z).length() < 1e-12);
        assert!((direction(0.0, 90.0) - DVec3::X).length() < 1e-12);
        assert!((direction(90.0, 0.0) - DVec3::Y).length() < 1e-12);
    }

    #[test]
    fn a_map_is_read_through_its_samples_and_smoothed_by_its_pyramid() {
        // A map of 8 × 4: a ramp in longitude.
        let levels = vec![(
            8,
            4,
            (0..32).map(|i| (i % 8) as i16 * 100).collect::<Vec<_>>(),
        )];
        let mut map = Elevation { levels, digest: 0 };
        let mut pyramid = vec![];
        let (w, h, grid) = &map.levels[0];
        let at = |x: u32, y: u32| i32::from(grid[(y * w + x) as usize]);
        pyramid.push((
            w / 2,
            h / 2,
            (0..h / 2)
                .flat_map(|y| (0..w / 2).map(move |x| (x, y)))
                .map(|(x, y)| {
                    ((at(2 * x, 2 * y)
                        + at(2 * x + 1, 2 * y)
                        + at(2 * x, 2 * y + 1)
                        + at(2 * x + 1, 2 * y + 1)
                        + 2)
                        / 4) as i16
                })
                .collect(),
        ));
        map.levels.extend(pyramid);
        // At a sample's centre: its value; halfway between two: their mean.
        let lon = |x: f64| ((x + 0.5) / 8.0 - 0.5) * std::f64::consts::TAU;
        assert!((map.sample(0, 0.3, lon(2.0)) - 200.0).abs() < 1e-9);
        assert!((map.sample(0, 0.3, lon(2.5)) - 250.0).abs() < 1e-9);
        // A wavelength under two of the grid's samples reads the full grid; a wider one blends
        // towards the next level.
        let radius = 1000.0;
        let spacing = std::f64::consts::TAU * radius / 8.0;
        let d = direction(10.0, lon(2.0).to_degrees());
        assert_eq!(
            map.height(d, radius, spacing),
            map.sample(0, lat_lon(d).0, lat_lon(d).1)
        );
        let blended = map.height(d, radius, 3.0 * spacing);
        let (a, b) = (
            map.sample(0, lat_lon(d).0, lat_lon(d).1),
            map.sample(1, lat_lon(d).0, lat_lon(d).1),
        );
        assert!((blended - (a + b) / 2.0).abs() < 1e-6, "{blended} {a} {b}");
    }

    #[test]
    fn the_cut_covers_the_sphere_once_and_is_finest_at_its_target() {
        let planet = small();
        let world = &planet.world;
        let target = DVec3::new(0.3, 1.0, 0.2).normalize();
        let cut = world.tile_cut(target);
        // Once: the cells' areas at their levels add up to six faces.
        let area: f64 = cut
            .iter()
            .map(|c| 1.0 / f64::from(1_u32 << (2 * c.level())))
            .sum();
        assert!((area - 6.0).abs() < 1e-9, "{area}");
        // No cell holds another.
        for (i, a) in cut.iter().enumerate() {
            for b in &cut[i + 1..] {
                assert!(!a.contains(*b) && !b.contains(*a), "{a:?} {b:?}");
            }
        }
        let sphere = world.sphere();
        let under = sphere.cell_of(sphere.surface_point(target, 0.0), world.tiles.finest);
        assert!(cut.contains(&under));
    }

    #[test]
    fn a_tiles_normal_map_points_out_of_the_planet_and_its_mips_end_at_a_texel() {
        let planet = small();
        let world = &planet.world;
        let sphere = world.sphere();
        let cell = sphere.cell_of(
            sphere.surface_point(DVec3::new(0.3, 1.0, 0.2).normalize(), 0.0),
            6,
        );
        let map = tile_normal_map(&planet, cell);
        let size = NORMAL_MAP_SIZE as usize;
        assert_eq!(map.len(), size * size * 4);
        // Every texel within a right angle of the way up at the tile's centre.
        let up = tile_origin(world, cell).normalize();
        for t in map.as_chunks::<4>().0 {
            let n =
                DVec3::new(f64::from(t[0]), f64::from(t[1]), f64::from(t[2])) / 127.5 - DVec3::ONE;
            assert!(n.dot(up) > 0.0, "{n} against {up}");
            assert_eq!(t[3], 255);
        }
        let levels = normal_map_levels(map, NORMAL_MAP_SIZE);
        assert_eq!(levels.len(), 9);
        assert_eq!(levels.last().unwrap().len(), 4);
    }

    #[test]
    fn a_cut_around_points_holds_each_ones_and_coarsens_with_height() {
        let planet = small();
        let world = &planet.world;
        let sphere = world.sphere();
        let radius = world.planet.radius;
        let (a, b) = (
            DVec3::new(0.3, 1.0, 0.2).normalize(),
            DVec3::new(-0.6, 0.2, 0.7).normalize(),
        );
        let cut = world.tile_cut_around(&[a * radius, b * radius]);
        let area: f64 = cut
            .iter()
            .map(|c| 1.0 / f64::from(1_u32 << (2 * c.level())))
            .sum();
        assert!((area - 6.0).abs() < 1e-9, "{area}");
        for p in [a, b] {
            let under = sphere.cell_of(sphere.surface_point(p, 0.0), world.tiles.finest);
            assert!(cut.contains(&under));
        }
        // From high over `a`, no cell is much smaller than the height: its parent didn't reach.
        let height = 0.05 * radius;
        let high = world.tile_cut_around(&[a * (radius + height)]);
        let reach = world.tiles.rings + std::f64::consts::FRAC_1_SQRT_2;
        for c in &high {
            assert!(
                c.level() == 0 || 2.0 * reach * sphere.cell_size(c.level()) >= height,
                "{c:?}"
            );
        }
        assert!(high.len() < cut.len());
    }

    #[test]
    fn heights_are_deterministic_and_band_limited() {
        let planet = Planet::new(PlanetWorld::default()).unwrap();
        let world = &planet.world;
        let d = DVec3::new(0.2, 0.9, -0.4);
        let fine = planet.height(d, 4.0);
        assert_eq!(fine, planet.height(d, 4.0));
        // A coarse level leaves out the fine octaves, by less than its skirt.
        for level in [3, 8, 12] {
            let coarse = planet.height(d, world.min_wavelength(level));
            assert!(
                (coarse - fine).abs() < skirt_depth(world, level),
                "level {level}: {coarse} vs {fine}"
            );
        }
        // Land and sea both, kilometres of relief.
        let (mut lo, mut hi) = (f64::MAX, f64::MIN);
        for i in 0..400 {
            let t = f64::from(i) * 0.031;
            let d = DVec3::new(
                t.sin() * (3.0 * t).cos(),
                t.cos(),
                t.sin() * (3.0 * t).sin(),
            );
            let h = planet.height(d, 1000.0);
            lo = lo.min(h);
            hi = hi.max(h);
        }
        assert!(lo < -1000.0 && hi > 500.0, "{lo} {hi}");
    }

    #[test]
    fn neighbouring_tiles_share_their_edge_and_its_normals() {
        let planet = small();
        let world = &planet.world;
        let a = CellId::cube(2, 4, 5, 7);
        let b = CellId::cube(2, 4, 6, 7);
        let (ma, mb) = (tile_mesh(&planet, a), tile_mesh(&planet, b));
        let (oa, ob) = (tile_origin(world, a), tile_origin(world, b));
        let n = world.tiles.samples as usize;
        for j in 0..n {
            // a's right edge is b's left edge.
            let (ia, ib) = (j * n + n - 1, j * n);
            let pa = DVec3::from(ma.positions[ia].map(f64::from)) + oa;
            let pb = DVec3::from(mb.positions[ib].map(f64::from)) + ob;
            assert!((pa - pb).length() < 0.05, "row {j}: {pa} {pb}");
            let (na, nb) = (ma.normals[ia], mb.normals[ib]);
            assert!(
                (0..3).all(|k| (na[k] - nb[k]).abs() < 1e-5),
                "{na:?} {nb:?}"
            );
        }
        // Every triangle of the surface faces out of the planet.
        for t in ma
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .take((n - 1) * (n - 1) * 2)
        {
            let p = |i: u32| DVec3::from(ma.positions[i as usize].map(f64::from)) + oa;
            let normal = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
            assert!(normal.dot(p(t[0])) > 0.0);
        }
    }

    #[test]
    fn a_coast_lies_between_sea_and_land() {
        let planet = Planet::new(PlanetWorld::default()).unwrap();
        let coast = planet.target(50.0).expect("a coast");
        let fine = planet.world.min_wavelength(planet.world.tiles.finest);
        assert!(planet.height(coast, fine) >= 50.0);
    }
}
