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
use forge_core::fixed::{
    Cordic, METRE, ONE, Q32, Q61, fade32, fixed, mul_shr, mul32, mul61, rsqrt61, smooth32,
};
use forge_core::pack::par_chunks_mut;
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
    /// Finer elevations over regions of the body, read over this map: the Earth's tour region
    /// from the Copernicus DEM at 90 m (#220).
    #[serde(default)]
    pub regions: Vec<RegionParams>,
}

/// A finer elevation over a region of the body (#220): an equirectangular grid of `i16` metres
/// over the sea's level, the north row first, west to east, [`NO_DATA`] where it has none (open
/// sea), read over the whole map ([`MapParams`]) and blended into it at its edges
/// (`assets/blender/planet_region.py` makes it from the downloaded tiles).
///
/// At or under half a metre its samples are sea: the Copernicus DEM's sea lies at about 0, so
/// there the whole map's depths go on, a metre deeper at least.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionParams {
    /// The grid's file, relative to the workspace.
    pub file: String,
    /// Its samples across (longitude) and down (latitude).
    pub size: (u32, u32),
    /// The bounds of its samples' cells, degrees: west, east, south and north.
    pub bounds: (f64, f64, f64, f64),
    /// Degrees inside its bounds over which it fades into the whole map.
    pub blend: f64,
    /// The noise under its resolution, as [`MapParams::detail`]: narrower than the whole map's.
    pub detail: (f64, u32, f64, f64),
}

/// A region's sample with no height ([`RegionParams`]).
pub const NO_DATA: i16 = i16::MIN;

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
    /// this many of its sides of its centre (plus half its diagonal): the cut by distance
    /// ([`PlanetWorld::tile_cut_around`]).
    pub rings: f64,
    /// The swap rule (#220, [`PlanetWorld::tile_cut_by_error`]): a cell splits where its
    /// samples, seen from the nearest point, would stand more than this many pixels apart, so
    /// that what its children change (their facets, the slopes their normal maps add) is
    /// smaller than this when they come.
    pub spacing_px: f64,
    /// Degrees: the slopes its children add must reach this for the spacing to count. Flatter
    /// ground (the open sea, plains) shows no more for being split.
    pub slope: f64,
    /// A cell also splits where the height its children add would show more than this many
    /// pixels ([`Planet::cell_error`]): the far mountains' outlines.
    pub error_px: f64,
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
    /// How much the air hazes what lies behind it: the air between the camera and the ground is
    /// this many times as dense. 1 is the physical air; below it, the ground stays clearer, and
    /// the sky itself doesn't change.
    pub haze: f64,
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
                spacing_px: 2.5,
                slope: 1.0,
                error_px: 1.0,
            },
            view: ViewParams {
                target: None,
                heading: 0.0,
                orbit: 400_000.0,
                atmosphere: true,
                haze: 1.0,
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

    /// The tiles around `points` by the swap rule (#220, [`TileParams::spacing_px`]): a cell
    /// splits into its four children where, seen from the nearest of `points` at
    /// `pixels_per_radian`, its samples would stand more than `spacing_px` apart on ground whose
    /// children add slopes of `slope` or more, or where the height they add would show more than
    /// `error_px`; down to [`TileParams::finest`]. `errors` gives the errors
    /// ([`Planet::cell_error`]) of a level's cells at a time, which the caller may work out in
    /// parallel and keep. The cells that don't split cover the sphere once, in id order.
    pub fn tile_cut_by_error(
        &self,
        points: &[DVec3],
        pixels_per_radian: f64,
        mut errors: impl FnMut(&[CellId]) -> Vec<f64>,
    ) -> Vec<CellId> {
        let sphere = self.sphere();
        let tiles = &self.tiles;
        let slope = tiles.slope.to_radians();
        let mut level: Vec<CellId> = Face::ALL
            .iter()
            .map(|f| CellId::cube(f.index(), 0, 0, 0))
            .collect();
        let mut cut = Vec::new();
        while !level.is_empty() {
            let errors = errors(&level);
            let mut next = Vec::new();
            for (&cell, &error) in level.iter().zip(&errors) {
                let l = cell.level();
                let spacing = self.spacing(l);
                // Pixels a metre at the cell's nearest point to one of `points` (its bounding
                // sphere, half its diagonal and a little more), a metre away at the nearest.
                let centre = sphere.cell_center(cell);
                let reach = 0.75 * sphere.cell_size(l);
                let distance = points
                    .iter()
                    .map(|&p| ((centre - p).length() - reach).max(1.0))
                    .fold(f64::INFINITY, f64::min);
                let per_metre = pixels_per_radian / distance;
                let splits = (spacing * per_metre > tiles.spacing_px && error >= slope * spacing)
                    || error * per_metre > tiles.error_px;
                match cell.children() {
                    Some(children) if splits && l < tiles.finest => next.extend(children),
                    _ => cut.push(cell),
                }
            }
            level = next;
        }
        cut.sort_unstable();
        cut
    }
}

/// Points a side of the grid [`Planet::cell_error`] samples a cell's error over.
pub const CELL_ERROR_POINTS: u32 = 9;

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

/// Where an elevation grid lies, radians: the bounds of its samples' cells, and whether it is a
/// map of the whole body, wrapping round the longitude, or a region with nothing outside it.
#[derive(Clone, Copy, Debug)]
struct Window {
    west: f64,
    east: f64,
    south: f64,
    north: f64,
    whole: bool,
}

impl Window {
    /// The whole body.
    const WHOLE: Self = Self {
        west: -std::f64::consts::PI,
        east: std::f64::consts::PI,
        south: -std::f64::consts::FRAC_PI_2,
        north: std::f64::consts::FRAC_PI_2,
        whole: true,
    };

    /// A region's, from its bounds in degrees: west, east, south and north.
    fn region((west, east, south, north): (f64, f64, f64, f64)) -> Self {
        Self {
            west: west.to_radians(),
            east: east.to_radians(),
            south: south.to_radians(),
            north: north.to_radians(),
            whole: false,
        }
    }
}

/// An elevation map, loaded: its grid and the levels of its mip pyramid, each half the last's
/// samples a side (a box filter), as `i16` metres.
pub struct Elevation {
    /// The levels, the full grid first: (width, height, samples).
    levels: Vec<(u32, u32, Vec<i16>)>,
    /// A digest of the full grid's bytes, for the tiles' keys.
    pub digest: u64,
    /// Where it lies.
    window: Window,
}

impl Elevation {
    /// Reads the grid `file` of `size` samples lying over `window` and builds its pyramid. A
    /// level's sample is [`NO_DATA`] where any of the four under it is.
    fn load(file: &str, size: (u32, u32), window: Window) -> Result<Self> {
        let path = crate::workspace_root().join(file);
        let bytes = std::fs::read(&path).with_context(|| {
            format!(
                "the elevation map {} (tools/fetch-planets.sh fetches and converts it)",
                path.display()
            )
        })?;
        let (width, height) = size;
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
                    let four = [
                        at(2 * x, 2 * y),
                        at(2 * x + 1, 2 * y),
                        at(2 * x, 2 * y + 1),
                        at(2 * x + 1, 2 * y + 1),
                    ];
                    if four.contains(&i32::from(NO_DATA)) {
                        return NO_DATA;
                    }
                    let sum: i32 = four.iter().sum();
                    // Rounded to the nearest metre, halves away from zero: the same everywhere.
                    (if sum >= 0 { sum + 2 } else { sum - 2 } / 4) as i16
                })
                .collect();
            levels.push((nw, nh, next));
        }
        Ok(Self {
            levels,
            digest,
            window,
        })
    }

    /// The height at latitude `lat` and longitude `lon` (radians) on `level`, Catmull-Rom: a
    /// whole map wraps round the longitude; a region has none outside its window or where any
    /// of the sixteen samples read has no data.
    fn sample(&self, level: usize, lat: f64, lon: f64) -> Option<f64> {
        let (w, h, grid) = &self.levels[level];
        let (w, h) = (*w as i64, *h as i64);
        let window = &self.window;
        let (u, v) = if window.whole {
            (
                (lon / std::f64::consts::TAU + 0.5) * w as f64 - 0.5,
                (0.5 - lat / std::f64::consts::PI) * h as f64 - 0.5,
            )
        } else {
            (
                (lon - window.west) / (window.east - window.west) * w as f64 - 0.5,
                (window.north - lat) / (window.north - window.south) * h as f64 - 0.5,
            )
        };
        let (x0, y0) = (u.floor(), v.floor());
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i64, y0 as i64);
        if !window.whole && (x0 < 1 || y0 < 1 || x0 + 2 >= w || y0 + 2 >= h) {
            return None;
        }
        let mut no_data = false;
        let mut at = |x: i64, y: i64| {
            let x = x.rem_euclid(w);
            let y = y.clamp(0, h - 1);
            let value = grid[(y * w + x) as usize];
            no_data |= !window.whole && value == NO_DATA;
            f64::from(value)
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
        (!no_data).then_some(sum)
    }

    /// The height in `direction` holding no detail narrower than `min_wavelength` metres on a
    /// body of `radius`: the levels whose samples are half that apart, blended. None where a
    /// region has none ([`Self::sample`]).
    pub fn height(&self, direction: DVec3, radius: f64, min_wavelength: f64) -> Option<f64> {
        let (lat, lon) = lat_lon(direction);
        // The full grid's spacing: along the equator for a whole map, from south to north for
        // a region (its longitudes are closer away from the equator); each level doubles it.
        let spacing = if self.window.whole {
            std::f64::consts::TAU * radius / f64::from(self.levels[0].0)
        } else {
            (self.window.north - self.window.south) * radius / f64::from(self.levels[0].1)
        };
        let wanted = 0.5 * min_wavelength;
        let mut level = 0;
        let mut at = spacing;
        while at * 2.0 <= wanted && level + 1 < self.levels.len() {
            at *= 2.0;
            level += 1;
        }
        let here = self.sample(level, lat, lon)?;
        if wanted <= at || level + 1 == self.levels.len() {
            return Some(here);
        }
        // Between this level and the next, by where the wanted spacing falls.
        let t = (wanted - at) / at;
        Some(here + (self.sample(level + 1, lat, lon)? - here) * t)
    }

    /// How far into a region's window `direction` lies, faded over `blend` degrees from its
    /// edges: 0 outside, 1 deep inside. Always 1 for a whole map.
    fn inside(&self, direction: DVec3, blend: f64) -> f64 {
        let window = &self.window;
        if window.whole {
            return 1.0;
        }
        let (lat, lon) = lat_lon(direction);
        let edge = (lon - window.west)
            .min(window.east - lon)
            .min(lat - window.south)
            .min(window.north - lat);
        smooth(edge / blend.to_radians().max(1e-9))
    }
}

/// The loaded maps, one each per file, kept for the process.
/// The maps loaded so far, by file.
type Loaded = Mutex<Vec<(String, Arc<Elevation>)>>;

fn elevation(file: &str, size: (u32, u32), window: Window) -> Result<Arc<Elevation>> {
    static MAPS: OnceLock<Loaded> = OnceLock::new();
    let maps = MAPS.get_or_init(|| Mutex::new(Vec::new()));
    let mut maps = maps.lock().expect("the maps' lock");
    if let Some((_, loaded)) = maps.iter().find(|(loaded, _)| loaded == file) {
        return Ok(loaded.clone());
    }
    let start = std::time::Instant::now();
    let loaded = Arc::new(Elevation::load(file, size, window)?);
    tracing::info!(
        file,
        levels = loaded.levels.len(),
        ms = start.elapsed().as_millis(),
        "elevation map"
    );
    maps.push((file.to_owned(), loaded.clone()));
    Ok(loaded)
}

/// A planet ready to make tiles: its world and its maps, loaded.
#[derive(Clone)]
pub struct Planet {
    /// Its world.
    pub world: PlanetWorld,
    /// Its elevation map, when the world names one.
    pub map: Option<Arc<Elevation>>,
    /// Its regions' finer elevations, in the order of the map's `regions`.
    pub regions: Vec<Arc<Elevation>>,
    /// The seeds of its map detail's octaves, worked out once.
    seeds: Arc<DetailSeeds>,
    /// The CORDIC's constants, for the heights in integers ([`FixedHeights`]).
    cordic: Arc<Cordic>,
}

/// The seeds of a planet's map detail's octaves ([`Planet::height`]): the whole map's and the
/// regions'. Hashed again for every octave of every sample, they took a fifth of a height's time.
struct DetailSeeds {
    detail: Vec<u64>,
    region: Vec<u64>,
}

impl Planet {
    /// `world` with its maps loaded (once a process).
    pub fn new(world: PlanetWorld) -> Result<Self> {
        let map = world
            .map
            .as_ref()
            .map(|m| elevation(&m.file, m.size, Window::WHOLE))
            .transpose()?;
        let regions = world
            .map
            .iter()
            .flat_map(|m| &m.regions)
            .map(|r| elevation(&r.file, r.size, Window::region(r.bounds)))
            .collect::<Result<Vec<_>>>()?;
        let octave_seeds = |name: &str, count: u32| -> Vec<u64> {
            let seed = Seed::new(world.planet.seed).derive_str(name);
            (0..count)
                .map(|k| seed.derive(u64::from(k)).value())
                .collect()
        };
        let detail = world.map.as_ref().map_or(0, |m| m.detail.1);
        let region = world
            .map
            .iter()
            .flat_map(|m| &m.regions)
            .map(|r| r.detail.1)
            .max()
            .unwrap_or(0);
        let seeds = Arc::new(DetailSeeds {
            detail: octave_seeds("detail", detail),
            region: octave_seeds("region detail", region),
        });
        Ok(Self {
            world,
            map,
            regions,
            seeds,
            cordic: Arc::new(Cordic::new()),
        })
    }

    /// The ground's height over the sea's level (the map's reference) in `direction` (from the
    /// planet's centre), metres, holding nothing narrower than `min_wavelength`; negative under
    /// the sea.
    pub fn height(&self, direction: DVec3, min_wavelength: f64) -> f64 {
        let planet = &self.world.planet;
        match (&self.map, &self.world.map) {
            (Some(map), Some(params)) => {
                // A whole map has a height everywhere.
                let whole = map
                    .height(direction, planet.radius, min_wavelength)
                    .unwrap_or(0.0);
                let p = direction.normalize() * planet.radius;
                // The map's height and the noise under its resolution, rougher where it stands
                // high.
                let with_detail = |base: f64, (width, count, strength, gain), seeds: &[u64]| {
                    let rough = 0.25 + 0.75 * smooth(base / params.rough_above);
                    let detail = octaves_by(
                        |octave| seeds[octave as usize],
                        p,
                        (width, count, gain),
                        min_wavelength,
                        |n| n,
                    );
                    base + strength * rough * detail
                };
                // The regions' finer heights over it, faded in from their edges. Their sea (at
                // or under half a metre) takes the whole map's depths, two metres deeper at
                // least, without noise: the sea flattens it anyway, and noise there raised
                // islands of sand along the coasts. Deep inside a region its height alone
                // counts: the whole map's noise, half a height's time, is not made there (#220).
                let mut height = None;
                for (region, grid) in params.regions.iter().zip(&self.regions) {
                    let weight = grid.inside(direction, region.blend);
                    if weight <= 0.0 {
                        continue;
                    }
                    let Some(fine) = grid.height(direction, planet.radius, min_wavelength) else {
                        continue;
                    };
                    let regional = if fine > 0.5 {
                        with_detail(fine, region.detail, &self.seeds.region)
                    } else {
                        whole.min(0.0) - 2.0
                    };
                    height = Some(if weight >= 1.0 {
                        regional
                    } else {
                        let under = height.unwrap_or_else(|| {
                            with_detail(whole, params.detail, &self.seeds.detail)
                        });
                        under + (regional - under) * weight
                    });
                }
                height.unwrap_or_else(|| with_detail(whole, params.detail, &self.seeds.detail))
            }
            _ => planet.height(direction, min_wavelength),
        }
    }

    /// How far `cell`'s tile stands from its children's (#220, the swap rule), metres: the
    /// largest height the next level adds over a grid of points across the cell
    /// ([`CELL_ERROR_POINTS`]² of them), and the sag of its triangles under the sphere that the
    /// children's halved ones take away. Open sea adds no height (its ground is the sea's level),
    /// so only the sag is left there.
    pub fn cell_error(&self, cell: CellId) -> f64 {
        let world = &self.world;
        let level = cell.level();
        let face = Face::from_index(cell.face());
        let cells = f64::from(1_u32 << level);
        let (x, y) = cell.xy();
        let start = DVec2::new(x as f64, y as f64) / cells * 2.0 - 1.0;
        let step = 2.0 / cells / f64::from(CELL_ERROR_POINTS);
        let (coarse, fine) = (world.min_wavelength(level), world.min_wavelength(level + 1));
        let mut error = 0.0_f64;
        for j in 0..CELL_ERROR_POINTS {
            for i in 0..CELL_ERROR_POINTS {
                let st = start + (DVec2::new(f64::from(i), f64::from(j)) + 0.5) * step;
                let direction = CubeSphere::direction(face, st);
                error = error
                    .max((self.ground(direction, fine) - self.ground(direction, coarse)).abs());
            }
        }
        // A chord of `s` sags `s² / 8R` under the sphere; the children's, half as long, a quarter.
        let s = world.spacing(level);
        error + 3.0 * s * s / (32.0 * world.planet.radius)
    }

    /// The ground as the tiles draw it: [`Self::height`], the sea's floor flattened at its level
    /// where the world has a sea.
    pub fn ground(&self, direction: DVec3, min_wavelength: f64) -> f64 {
        self.ground_of(self.height(direction, min_wavelength))
    }

    /// The ground where the height is `height` ([`Planet::height`]): the sea's level over it
    /// where the world has a sea.
    fn ground_of(&self, height: f64) -> f64 {
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
        let mut key = format!(
            "{:?}, map {:?} {:016x}, {} samples, made and cooked by {:016x}",
            self.world.planet,
            map,
            self.map.as_ref().map_or(0, |m| m.digest),
            self.world.tiles.samples,
            code_digests::CODE_PLANET_TILES
        );
        // The regions, when there are any: a world without keeps its tiles' key.
        let regions = self.world.map.iter().flat_map(|m| &m.regions);
        for (region, grid) in regions.zip(&self.regions) {
            key += &format!(", region {region:?} {:016x}", grid.digest);
        }
        key
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
    spec: (f64, u32, f64),
    min_wavelength: f64,
    shape: impl Fn(f64) -> f64,
) -> f64 {
    octaves_by(
        |octave| seed.derive(u64::from(octave)).value(),
        p,
        spec,
        min_wavelength,
        shape,
    )
}

/// [`octaves`] with octave `k`'s seed `seed_of(k)`: a planet's map detail keeps its octaves'
/// seeds ([`DetailSeeds`]) rather than hash them again at every sample.
fn octaves_by(
    seed_of: impl Fn(u32) -> u64,
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
            let n = gradient3(seed_of(octave), q.x, q.y, q.z);
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

/// The fewest rows of samples a thread takes when a tile's heights are spread over the machine's
/// threads (a thread costs tens of microseconds to start).
const ROWS_PER_THREAD: usize = 8;

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

/// The `(cos, sin)` of the equi-angular warp at each column or row of a grid, Q61
/// ([`fixed_warp`]).
type Warp = Vec<(i64, i64)>;

/// The samples of a grid over a tile, on the ground: sample (i, j) at global step
/// `x steps + i`, `y steps + j` of its face cut into `steps << level` (D-058's step 2). In
/// integers where the planet has a map and `steps` is a power of two ([`FixedHeights`]: the
/// same bits on every machine, and on the GPU), in floats otherwise.
struct GridSampler<'a> {
    planet: &'a Planet,
    face: Face,
    min_wavelength: f64,
    /// The tile's constants and its columns' and rows' warp from `first`, in integers.
    fixed: Option<(FixedHeights, Warp, Warp)>,
    first: i64,
    /// The face's coordinates of sample (0, 0) and a step, in floats.
    start: DVec2,
    step: f64,
}

impl<'a> GridSampler<'a> {
    /// For `cell`'s grid of `steps` a tile, its samples from `first` to `first + count − 1`
    /// along each way, its heights holding nothing narrower than `min_wavelength`.
    fn new(
        planet: &'a Planet,
        cell: CellId,
        steps: i64,
        first: i64,
        count: usize,
        min_wavelength: f64,
    ) -> Self {
        let level = cell.level();
        let cells = f64::from(1_u32 << level);
        let (x, y) = cell.xy();
        let fixed = (steps > 0 && (steps as u64).is_power_of_two())
            .then(|| planet.fixed_heights(min_wavelength))
            .flatten()
            .map(|heights| {
                let bits = u32::from(level) + steps.trailing_zeros();
                let warp = |base: i64| -> Vec<(i64, i64)> {
                    (0..count as i64)
                        .map(|k| fixed_warp(&planet.cordic, base * steps + first + k, bits))
                        .collect()
                };
                (heights, warp(x), warp(y))
            });
        Self {
            planet,
            face: Face::from_index(cell.face()),
            min_wavelength,
            fixed,
            first,
            start: DVec2::new(x as f64, y as f64) / cells * 2.0 - 1.0,
            step: 2.0 / cells / steps as f64,
        }
    }

    /// Sample (i, j)'s point on the ground (the planet's frame, metres) at `height` (Q32 metres,
    /// before the sea flattens it), made elsewhere ([`tile_mesh_from`]); in integers only.
    fn point(&self, i: i64, j: i64, height: i64) -> DVec3 {
        let (heights, columns, rows) = self.fixed.as_ref().expect("heights in integers");
        let (ci, rj) = ((i - self.first) as usize, (j - self.first) as usize);
        let d = fixed_direction(self.face, columns[ci], rows[rj]);
        let direction = DVec3::from_array(d.map(|c| c as f64 / ONE as f64));
        let ground = heights.ground(height) as f64 / METRE as f64;
        direction * (self.planet.world.planet.radius + ground)
    }

    /// Sample (i, j): its point on the ground (the planet's frame, metres) and its height before
    /// the sea flattens it.
    fn sample(&self, i: i64, j: i64) -> (DVec3, f64) {
        let radius = self.planet.world.planet.radius;
        match &self.fixed {
            Some((heights, columns, rows)) => {
                let (ci, rj) = ((i - self.first) as usize, (j - self.first) as usize);
                let d = fixed_direction(self.face, columns[ci], rows[rj]);
                let height = self.planet.fixed_height(heights, &self.planet.cordic, d);
                let direction = DVec3::from_array(d.map(|c| c as f64 / ONE as f64));
                let ground = heights.ground(height) as f64 / METRE as f64;
                (direction * (radius + ground), height as f64 / METRE as f64)
            }
            None => {
                let st = self.start + DVec2::new(i as f64, j as f64) * self.step;
                let direction = CubeSphere::direction(self.face, st);
                let height = self.planet.height(direction, self.min_wavelength);
                (direction * (radius + self.planet.ground_of(height)), height)
            }
        }
    }
}

/// The ground's mesh over `cell`: its samples on the sphere at their heights
/// ([`Planet::ground`]), relative to [`tile_origin`], normals from the samples around each (one
/// beyond the edge, so neighbours of a level share their edge's normals), two triangles per cell
/// facing out, and the skirt ([`skirt_depth`]) under the edge.
pub fn tile_mesh(planet: &Planet, cell: CellId) -> TriMesh {
    tile_mesh_with(planet, cell, None)
}

/// [`tile_mesh`] from the tile's heights made elsewhere, the GPU's (D-058's step 2): `heights`
/// in Q32 metres before the sea flattens them, `samples + 2` a side from (−1, −1), rows first,
/// as `shaders/planet_heights.slang` makes them. The same mesh, to the bit, as [`tile_mesh`]'s
/// from the planet's heights in integers.
///
/// # Panics
/// If the planet has no heights in integers ([`Planet::fixed_heights`]) or `heights` is not the
/// grid's size.
pub fn tile_mesh_from(planet: &Planet, cell: CellId, heights: &[i64]) -> TriMesh {
    tile_mesh_with(planet, cell, Some(heights))
}

fn tile_mesh_with(planet: &Planet, cell: CellId, heights: Option<&[i64]>) -> TriMesh {
    let world = &planet.world;
    let n = world.tiles.samples as i64;
    let level = cell.level();
    let origin = tile_origin(world, cell);
    let sampler = GridSampler::new(
        planet,
        cell,
        n - 1,
        -1,
        (n + 2) as usize,
        world.min_wavelength(level),
    );
    // The samples and a ring beyond, row-major from (−1, −1).
    let side = (n + 2) as usize;
    if let Some(heights) = heights {
        assert!(
            sampler.fixed.is_some(),
            "heights in integers for a planet without them"
        );
        assert_eq!(heights.len(), side * side, "a tile's heights");
    }
    let mut points = vec![DVec3::ZERO; side * side];
    // A row a chunk, the rows over the machine's threads: what one new tile waits for.
    par_chunks_mut(&mut points, side, ROWS_PER_THREAD, |row, out| {
        let j = row as i64 - 1;
        for (point, i) in out.iter_mut().zip(-1..=n) {
            *point = match heights {
                Some(h) => sampler.point(i, j, h[row * side + (i + 1) as usize]),
                None => sampler.sample(i, j).0,
            };
        }
    });
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

/// Quads a side of the cells a tile's first level is cut into ([`tile_clusters`]): 8 × 8
/// samples, a cluster's 64 vertices, and 98 triangles.
const TILE_CLUSTER_QUADS: usize = 7;
/// Quads of a tile's skirt in one of its clusters: 32 samples along the edge and 32 under them.
const SKIRT_CLUSTER_QUADS: usize = 31;

/// The clusters of the first level of every tile of `samples`² samples (#220,
/// `MeshletMesh::build_clustered`), each the places of its triangles in [`tile_mesh`]'s: the
/// grid's cells of [`TILE_CLUSTER_QUADS`]² quads, then the skirt's runs of
/// [`SKIRT_CLUSTER_QUADS`] quads. meshoptimizer clustered a tile's 133 000 triangles in 50 ms,
/// a third of its DAG's time, into clusters of about the same size.
pub fn tile_clusters(samples: u32) -> Vec<Vec<u32>> {
    let quads = samples as usize - 1;
    let mut clusters = Vec::new();
    for top in (0..quads).step_by(TILE_CLUSTER_QUADS) {
        for left in (0..quads).step_by(TILE_CLUSTER_QUADS) {
            let mut triangles = Vec::with_capacity(2 * TILE_CLUSTER_QUADS * TILE_CLUSTER_QUADS);
            for j in top..(top + TILE_CLUSTER_QUADS).min(quads) {
                for i in left..(left + TILE_CLUSTER_QUADS).min(quads) {
                    let quad = (j * quads + i) as u32;
                    triangles.extend_from_slice(&[2 * quad, 2 * quad + 1]);
                }
            }
            clusters.push(triangles);
        }
    }
    let first = (2 * quads * quads) as u32;
    let ring = 4 * quads;
    for start in (0..ring).step_by(SKIRT_CLUSTER_QUADS) {
        clusters.push(
            (start..(start + SKIRT_CLUSTER_QUADS).min(ring))
                .flat_map(|k| [first + 2 * k as u32, first + 2 * k as u32 + 1])
                .collect(),
        );
    }
    clusters
}

/// The cache name of `cell`'s tile on the planet called `body`: `body@face-level-x-y`.
pub fn tile_name(body: &str, cell: CellId) -> String {
    let (x, y) = cell.xy();
    format!("{body}@{}-{}-{x}-{y}", cell.face(), cell.level())
}

/// Texels a side of a tile's normal map ([`tile_normal_map`]).
pub const NORMAL_MAP_SIZE: u32 = 256;

/// Samples of a tile over which its normal map's coast (its alpha) runs from the sea's floor to
/// the land ([`tile_normal_map`]): beyond, it is clamped.
pub const COAST_RANGE: f64 = 4.0;

/// The normal map of `cell`'s tile (#220): [`NORMAL_MAP_SIZE`]² texels over the tile (its
/// UVs), each the ground's normal at its centre in the planet's frame, from the height of the
/// next level down.
///
/// It holds the slopes the tile's children add: their finest wavelength, two of the tile's
/// samples, is the map's Nyquist. A tile swapped for its children then changes no slope that
/// shows, and the map's mips filter the slopes a coarse cluster's vertices only sample (the
/// cluster DAG keeps each vertex's own normal). RGBA8 with `(n + 1) / 2` in RGB, rows of
/// increasing `v` first, level 0 only ([`normal_map_levels`] makes the mips).
///
/// Alpha is the coast: the height before the sea flattens it, `0.5 + 0.5 h / r` clamped, with
/// `r` [`COAST_RANGE`] of the tile's samples. Its 0.5 contour, read bilinearly per pixel, is
/// where the sea starts. A coarse tile's triangles drew the coast in straight runs kilometres
/// long; the map draws it at its texels.
pub fn tile_normal_map(planet: &Planet, cell: CellId) -> Vec<u8> {
    let world = &planet.world;
    let size = NORMAL_MAP_SIZE as usize;
    let level = cell.level();
    let min_wavelength = world.min_wavelength(level + 1);
    // Samples half a texel apart: the texels' centres are the odd ones, their neighbours the
    // even ones either side.
    let side = 2 * size + 1;
    let sampler = GridSampler::new(planet, cell, (2 * size) as i64, 0, side, min_wavelength);
    // Only the samples a texel reads: its neighbours (one coordinate odd, the other even) and
    // its centre (both odd), not the corners (both even), each with its height, kept for a
    // centre's coast. A row a chunk, the rows over the machine's threads.
    let mut samples = vec![(DVec3::ZERO, 0.0); side * side];
    par_chunks_mut(&mut samples, side, ROWS_PER_THREAD, |j, row| {
        // An even row holds neighbours at its odd places only; an odd row is all read.
        let (first, step) = if j % 2 == 0 { (1, 2) } else { (0, 1) };
        for i in (first..side).step_by(step) {
            row[i] = sampler.sample(i as i64, j as i64);
        }
    });
    let at = |i: usize, j: usize| samples[j * side + i].0;
    let range = COAST_RANGE * world.spacing(level);
    let mut texels = Vec::with_capacity(size * size * 4);
    for j in 0..size {
        for i in 0..size {
            let (ci, cj) = (2 * i + 1, 2 * j + 1);
            // Along u, then along v, as the tile's own normals.
            let normal = (at(ci + 1, cj) - at(ci - 1, cj))
                .cross(at(ci, cj + 1) - at(ci, cj - 1))
                .normalize_or(at(ci, cj).normalize());
            // The coast: the centre's height, before the sea flattens it.
            let coast = 0.5 + 0.5 * (samples[cj * side + ci].1 / range).clamp(-1.0, 1.0);
            texels.extend_from_slice(&encode_normal(normal, coast));
        }
    }
    texels
}

/// A unit normal as RGBA8: `(n + 1) / 2` in RGB, `alpha` (0 to 1) in alpha.
fn encode_normal(n: DVec3, alpha: f64) -> [u8; 4] {
    let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [
        byte(n.x * 0.5 + 0.5),
        byte(n.y * 0.5 + 0.5),
        byte(n.z * 0.5 + 0.5),
        byte(alpha),
    ]
}

/// A normal map's every level from its first ([`tile_normal_map`], `size`² texels, a power of
/// two): each texel of a level the mean of its four below, normalised, and their coasts' mean.
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
                    (decode(&below[at..at + 4]), f64::from(below[at + 3]) / 255.0)
                };
                let quad = [texel(0, 0), texel(1, 0), texel(0, 1), texel(1, 1)];
                let normal = quad.iter().map(|t| t.0).sum::<DVec3>();
                let coast = quad.iter().map(|t| t.1).sum::<f64>() / 4.0;
                level.extend_from_slice(&encode_normal(normal.normalize_or(DVec3::Z), coast));
            }
        }
        levels.push(level);
        side = half;
    }
    levels
}

// ---------------------------------------------------------------- heights in integers ---

/// A grid's level as a tile reads it in integers ([`FixedHeights`]): where it lies in the
/// samples, and a sample's column and row from its longitude and latitude, `u = lon × u_scale +
/// u_offset` (Q32 texels; Q61 radians times Q32 texels a radian, shifted by 61).
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedLevel {
    /// Its index in the grid's levels.
    pub level: u32,
    /// Columns a radian of longitude, Q32.
    pub u_scale: i64,
    /// The column at longitude 0, Q32.
    pub u_offset: i64,
    /// Rows a radian of latitude, Q32 (negative: the rows run south).
    pub v_scale: i64,
    /// The row at latitude 0, Q32.
    pub v_offset: i64,
}

/// An elevation grid as a tile reads it in integers ([`FixedHeights`]).
#[derive(Clone, Debug)]
pub struct FixedGrid {
    /// A map of the whole body (wrapping round the longitude).
    pub whole: bool,
    /// Its window, Q61 radians: west, east, south, north.
    pub bounds: [i64; 4],
    /// The inverse of its fade from its edges, Q32 per radian (0 for the whole map).
    pub inv_blend: i64,
    /// The level `Elevation::height` picks for the tile's narrowest wavelength, and the next.
    pub levels: [FixedLevel; 2],
    /// Whether the height blends toward the next level.
    pub blended: bool,
    /// How far it blends toward it, Q32.
    pub t: i64,
    /// Its detail's strength, Q32 metres.
    pub strength: i64,
    /// The inverse of its octaves' strengths' sum, Q32.
    pub inv_norm: i64,
    /// Its octaves.
    pub octaves: Vec<FixedOctave>,
}

/// An octave of a grid's detail as a tile reads it ([`FixedGrid`]).
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedOctave {
    /// The inverse of its width, Q40 per metre ([`INV_WIDTH_BITS`]).
    pub inv_width: i64,
    /// Its lattice's offset, Q32 lattice steps.
    pub offset: [i64; 3],
    /// Its fade at the tile's narrowest wavelength, Q32.
    pub fade: i64,
    /// Its strength, Q32.
    pub strength: i64,
    /// Its seed.
    pub seed: u64,
}

/// Fraction bits of an octave's inverse width.
pub const INV_WIDTH_BITS: u32 = 40;

/// Everything a tile's heights need that does not depend on the sample, worked out once a tile
/// from the world's floats (arithmetic only, the same on every machine) and fixed: D-058's step
/// 2, D-016's amendment. [`FixedHeights::height`] is the planet's height in integers; the GPU's
/// pass (`shaders/planet_heights.slang`) reads the same constants and gives the same bits.
#[derive(Clone, Debug)]
pub struct FixedHeights {
    /// The grids: the whole map first, then the regions.
    pub grids: Vec<FixedGrid>,
    /// The planet's radius, Q32 metres.
    pub radius: i64,
    /// The inverse of the height over which the detail is at its roughest, Q32 per metre.
    pub inv_rough: i64,
    /// Whether the sea flattens the ground at its level.
    pub sea: bool,
}

impl Planet {
    /// The CORDIC's constants its heights in integers use ([`FixedHeights`]).
    pub fn cordic(&self) -> &Cordic {
        &self.cordic
    }
}

impl Planet {
    /// The constants of the heights of a tile whose narrowest wavelength is `min_wavelength`
    /// ([`FixedHeights`]), or None for a planet of noise alone.
    pub fn fixed_heights(&self, min_wavelength: f64) -> Option<FixedHeights> {
        let (map, params) = (self.map.as_ref()?, self.world.map.as_ref()?);
        let radius = self.world.planet.radius;
        let grid =
            |elevation: &Elevation, blend: f64, detail: (f64, u32, f64, f64), seeds: &[u64]| {
                let w = &elevation.window;
                // Elevation::height's choice of levels.
                let spacing = if w.whole {
                    std::f64::consts::TAU * radius / f64::from(elevation.levels[0].0)
                } else {
                    (w.north - w.south) * radius / f64::from(elevation.levels[0].1)
                };
                let wanted = 0.5 * min_wavelength;
                let (mut level, mut at) = (0, spacing);
                while at * 2.0 <= wanted && level + 1 < elevation.levels.len() {
                    at *= 2.0;
                    level += 1;
                }
                let blended = !(wanted <= at || level + 1 == elevation.levels.len());
                let mut levels = [FixedLevel::default(); 2];
                for (k, slot) in levels.iter_mut().enumerate().take(1 + usize::from(blended)) {
                    let (gw, gh, _) = &elevation.levels[level + k];
                    let (gw, gh) = (f64::from(*gw), f64::from(*gh));
                    // Elevation::sample's columns and rows from the longitude and latitude.
                    let (u_scale, u_offset, v_scale, v_offset) = if w.whole {
                        (
                            gw / std::f64::consts::TAU,
                            0.5 * gw - 0.5,
                            -gh / std::f64::consts::PI,
                            0.5 * gh - 0.5,
                        )
                    } else {
                        (
                            gw / (w.east - w.west),
                            -w.west * gw / (w.east - w.west) - 0.5,
                            -gh / (w.north - w.south),
                            w.north * gh / (w.north - w.south) - 0.5,
                        )
                    };
                    *slot = FixedLevel {
                        level: (level + k) as u32,
                        u_scale: fixed(u_scale, Q32),
                        u_offset: fixed(u_offset, Q32),
                        v_scale: fixed(v_scale, Q32),
                        v_offset: fixed(v_offset, Q32),
                    };
                }
                // Its octaves (octaves_by): the same widths, offsets and fades.
                let (width, count, strength, gain) = detail;
                let (mut width, mut octave_strength, mut norm) = (width, 1.0, 0.0);
                let mut octaves = Vec::with_capacity(count as usize);
                for octave in 0..count {
                    norm += octave_strength;
                    let offset = [0.371, 0.613, 0.229].map(|o| fixed(o * f64::from(octave), Q32));
                    octaves.push(FixedOctave {
                        inv_width: fixed(1.0 / width, INV_WIDTH_BITS),
                        offset,
                        fade: fixed(smooth(width / min_wavelength - 1.0), Q32),
                        strength: fixed(octave_strength, Q32),
                        seed: seeds[octave as usize],
                    });
                    octave_strength *= gain;
                    width *= 0.5;
                }
                FixedGrid {
                    whole: w.whole,
                    bounds: [w.west, w.east, w.south, w.north].map(|b| fixed(b, Q61)),
                    inv_blend: if blend > 0.0 {
                        fixed(1.0 / blend, Q32)
                    } else {
                        0
                    },
                    levels,
                    blended,
                    t: if blended {
                        fixed((wanted - at) / at, Q32)
                    } else {
                        0
                    },
                    strength: fixed(strength, Q32),
                    inv_norm: fixed(1.0 / norm, Q32),
                    octaves,
                }
            };
        let mut grids = vec![grid(map, 0.0, params.detail, &self.seeds.detail)];
        for (region, elevation) in params.regions.iter().zip(&self.regions) {
            grids.push(grid(
                elevation,
                region.blend.to_radians().max(1e-9),
                region.detail,
                &self.seeds.region,
            ));
        }
        Some(FixedHeights {
            grids,
            radius: fixed(radius, Q32),
            inv_rough: fixed(1.0 / params.rough_above, Q32),
            sea: params.sea,
        })
    }

    /// The ground's height in `direction` (a unit vector, Q61) in integers, Q32 metres, as the
    /// tile of `heights` reads it: [`Self::height`] made again in fixed point (D-058's step 2).
    /// Without the sea flattening it; [`FixedHeights::ground`] does.
    pub fn fixed_height(
        &self,
        heights: &FixedHeights,
        cordic: &Cordic,
        direction: [i64; 3],
    ) -> i64 {
        let [x, y, z] = direction;
        let (lon, horizontal) = cordic.atan2(x, z);
        // The vertical times the CORDIC's gain, as the horizontal came out of it.
        let (lat, _) = cordic.atan2(mul61(y, cordic.gain), horizontal);
        let p = direction.map(|c| mul_shr(c, heights.radius, Q61));
        let whole_grid = &heights.grids[0];
        let map = self
            .map
            .as_deref()
            .expect("a planet with fixed heights has a map");
        let whole = fixed_grid_height(map, whole_grid, lat, lon).unwrap_or(0);
        let detail = |g: &FixedGrid, base: i64| {
            let rough = METRE / 4 + mul32(3 * METRE / 4, smooth32(mul32(base, heights.inv_rough)));
            base + mul32(mul32(g.strength, rough), fixed_octaves(g, p))
        };
        let mut height = None;
        for (g, elevation) in heights.grids.iter().skip(1).zip(&self.regions) {
            let [west, east, south, north] = g.bounds;
            let edge = (lon - west)
                .min(east - lon)
                .min(lat - south)
                .min(north - lat);
            let weight = smooth32(mul_shr(edge, g.inv_blend, Q61));
            if weight <= 0 {
                continue;
            }
            let Some(fine) = fixed_grid_height(elevation, g, lat, lon) else {
                continue;
            };
            let regional = if fine > METRE / 2 {
                detail(g, fine)
            } else {
                whole.min(0) - 2 * METRE
            };
            height = Some(if weight >= METRE {
                regional
            } else {
                let under = height.unwrap_or_else(|| detail(whole_grid, whole));
                under + mul32(regional - under, weight)
            });
        }
        height.unwrap_or_else(|| detail(whole_grid, whole))
    }
}

impl FixedHeights {
    /// The ground at `height` (Q32 metres): the sea's level over it where the planet has a sea.
    pub fn ground(&self, height: i64) -> i64 {
        if self.sea { height.max(0) } else { height }
    }
}

/// A grid's height at `lat`, `lon` (Q61 radians) at the levels `g` picked, blended (Q32
/// metres): Elevation::height and sample in integers; None where a region has no sample.
fn fixed_grid_height(elevation: &Elevation, g: &FixedGrid, lat: i64, lon: i64) -> Option<i64> {
    let mut here = 0;
    for k in 0..1 + usize::from(g.blended) {
        let l = &g.levels[k];
        let u = mul_shr(lon, l.u_scale, Q61) + l.u_offset;
        let v = mul_shr(lat, l.v_scale, Q61) + l.v_offset;
        let h = fixed_taps(elevation, l.level as usize, g.whole, u, v)?;
        here = if k == 0 {
            h
        } else {
            here + mul32(h - here, g.t)
        };
    }
    Some(here)
}

/// Catmull-Rom across the four samples each way around column `u` and row `v` (Q32 texels) of
/// `level`, in integers (Q32 metres).
fn fixed_taps(elevation: &Elevation, level: usize, whole: bool, u: i64, v: i64) -> Option<i64> {
    let (w, h, grid) = &elevation.levels[level];
    let (w, h) = (i64::from(*w), i64::from(*h));
    let (x0, y0) = (u >> Q32, v >> Q32);
    if !whole && (x0 < 1 || y0 < 1 || x0 + 2 >= w || y0 + 2 >= h) {
        return None;
    }
    let weights = |t: i64| {
        let t2 = mul32(t, t);
        let t3 = mul32(t2, t);
        [
            (-t3 + 2 * t2 - t) >> 1,
            (3 * t3 - 5 * t2 + 2 * METRE) >> 1,
            (-3 * t3 + 4 * t2 + t) >> 1,
            (t3 - t2) >> 1,
        ]
    };
    let mask = METRE - 1;
    let (wx, wy) = (weights(u & mask), weights(v & mask));
    let mut sum = 0_i64;
    for (j, wy) in wy.iter().enumerate() {
        let mut row = 0_i64;
        for (i, wx) in wx.iter().enumerate() {
            let x = (x0 + i as i64 - 1).rem_euclid(w);
            let y = (y0 + j as i64 - 1).clamp(0, h - 1);
            let value = grid[(y * w + x) as usize];
            if !whole && value == NO_DATA {
                return None;
            }
            row += wx * i64::from(value);
        }
        sum += mul32(row, *wy);
    }
    Some(sum)
}

/// A grid's octaves of detail at `p` (Q32 metres from the planet's centre), Q32: octaves_by in
/// integers.
fn fixed_octaves(g: &FixedGrid, p: [i64; 3]) -> i64 {
    let mut sum = 0_i64;
    for o in &g.octaves {
        if o.fade <= 0 {
            continue;
        }
        let q = [0, 1, 2].map(|c| mul_shr(p[c], o.inv_width, INV_WIDTH_BITS) + o.offset[c]);
        sum += mul32(mul32(o.strength, o.fade), fixed_gradient3(o.seed, q));
    }
    mul32(sum, g.inv_norm)
}

/// forge_procgen::noise::gradient3 in integers at lattice point `q` (Q32): the same hash and
/// gradients, the fade and the blends in fixed point.
fn fixed_gradient3(seed: u64, q: [i64; 3]) -> i64 {
    let mask = METRE - 1;
    let cell = q.map(|c| (c >> Q32) as i32);
    let f = q.map(|c| c & mask);
    let corner = |k: usize| {
        let d = [k & 1, (k >> 1) & 1, k >> 2].map(|b| b as i64 * METRE);
        let h = forge_core::hash::hash_cell3(
            seed,
            cell[0] + (k & 1) as i32,
            cell[1] + ((k >> 1) & 1) as i32,
            cell[2] + (k >> 2) as i32,
        );
        let (gx, gy, gz) = forge_procgen::noise::GRADIENTS3_INT[(h % 12) as usize];
        i64::from(gx) * (f[0] - d[0])
            + i64::from(gy) * (f[1] - d[1])
            + i64::from(gz) * (f[2] - d[2])
    };
    let t = f.map(fade32);
    let lerp = |a: i64, b: i64, t: i64| a + mul32(b - a, t);
    let near = lerp(
        lerp(corner(0), corner(1), t[0]),
        lerp(corner(2), corner(3), t[0]),
        t[1],
    );
    let far = lerp(
        lerp(corner(4), corner(5), t[0]),
        lerp(corner(6), corner(7), t[0]),
        t[1],
    );
    lerp(near, far, t[2])
}

/// The unit direction of a tile's sample in Q61 from its column's and row's `(cos, sin)` of the
/// equi-angular warp ([`fixed_warp`]): `N cos a cos b + U sin a cos b + V cos a sin b`,
/// normalised; the face's axes are ±1 along one coordinate each.
pub fn fixed_direction(face: Face, column: (i64, i64), row: (i64, i64)) -> [i64; 3] {
    let (normal, u, v) = face.axes();
    let terms = [
        (normal, mul61(column.0, row.0)),
        (u, mul61(column.1, row.0)),
        (v, mul61(column.0, row.1)),
    ];
    let mut d = [0_i64; 3];
    for (axis, value) in terms {
        for (c, a) in d.iter_mut().zip(axis.to_array()) {
            *c += a as i64 * value;
        }
    }
    let r2 = d.iter().map(|&c| mul61(c, c)).sum();
    let y = rsqrt61(r2);
    d.map(|c| mul61(c, y))
}

/// `(cos, sin)` of the equi-angular warp's angle at global sample `index` of a face cut into
/// `1 << bits` steps (Q61): the angle `π/4 (2 index / 2^bits − 1)`, for [`fixed_direction`].
pub fn fixed_warp(cordic: &Cordic, index: i64, bits: u32) -> (i64, i64) {
    let quarter_pi = fixed(std::f64::consts::FRAC_PI_4, Q61);
    let angle = mul_shr(2 * index - (1_i64 << bits), quarter_pi, bits);
    cordic.cos_sin(angle)
}

/// What a GPU pass needs to make [`Planet::height`] (D-058): the elevation grids' levels, where
/// they lie, their detail and its octaves' seeds. Borrowed from the planet: the levels are
/// hundreds of megabytes.
pub struct GpuHeights<'a> {
    /// The grids: the whole map first, then the regions in the world's order.
    pub grids: Vec<GpuGrid<'a>>,
    /// The seeds of the whole map's detail octaves, then of the regions' (every region shares
    /// them).
    pub detail_seeds: &'a [u64],
    /// The seeds of the regions' detail octaves.
    pub region_seeds: &'a [u64],
    /// Heights over which the detail is at its roughest, metres ([`MapParams::rough_above`]).
    pub rough_above: f64,
    /// Whether the sea flattens the ground at its level.
    pub sea: bool,
    /// The planet's radius, metres.
    pub radius: f64,
}

/// An elevation grid for the GPU ([`GpuHeights`]).
pub struct GpuGrid<'a> {
    /// Its levels, the full grid first: width, height, `i16` metres in rows ([`NO_DATA`] where
    /// a region has none).
    pub levels: Vec<(u32, u32, &'a [i16])>,
    /// A map of the whole body, wrapping round the longitude.
    pub whole: bool,
    /// Its bounds, radians: west, east, south, north.
    pub bounds: [f64; 4],
    /// A region's blend from its edges, radians (0 for the whole map).
    pub blend: f64,
    /// Its detail: the widest wavelength, octaves, strength (metres) and gain.
    pub detail: (f64, u32, f64, f64),
}

impl Planet {
    /// Its heights for the GPU ([`GpuHeights`]), or None for a planet of noise alone, which
    /// keeps its normal maps on the CPU.
    pub fn gpu_heights(&self) -> Option<GpuHeights<'_>> {
        let (map, params) = (self.map.as_ref()?, self.world.map.as_ref()?);
        fn grid(elevation: &Elevation, blend: f64, detail: (f64, u32, f64, f64)) -> GpuGrid<'_> {
            let w = &elevation.window;
            GpuGrid {
                levels: elevation
                    .levels
                    .iter()
                    .map(|(w, h, s)| (*w, *h, s.as_slice()))
                    .collect(),
                whole: w.whole,
                bounds: [w.west, w.east, w.south, w.north],
                blend,
                detail,
            }
        }
        let mut grids = vec![grid(map.as_ref(), 0.0, params.detail)];
        for (region, elevation) in params.regions.iter().zip(&self.regions) {
            grids.push(grid(
                elevation.as_ref(),
                region.blend.to_radians(),
                region.detail,
            ));
        }
        Some(GpuHeights {
            grids,
            detail_seeds: &self.seeds.detail,
            region_seeds: &self.seeds.region,
            rough_above: params.rough_above,
            sea: params.sea,
            radius: self.world.planet.radius,
        })
    }
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
        let mut map = Elevation {
            levels,
            digest: 0,
            window: Window::WHOLE,
        };
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
        assert!((map.sample(0, 0.3, lon(2.0)).unwrap() - 200.0).abs() < 1e-9);
        assert!((map.sample(0, 0.3, lon(2.5)).unwrap() - 250.0).abs() < 1e-9);
        // A wavelength under two of the grid's samples reads the full grid; a wider one blends
        // towards the next level.
        let radius = 1000.0;
        let spacing = std::f64::consts::TAU * radius / 8.0;
        let d = direction(10.0, lon(2.0).to_degrees());
        assert_eq!(
            map.height(d, radius, spacing),
            map.sample(0, lat_lon(d).0, lat_lon(d).1)
        );
        let blended = map.height(d, radius, 3.0 * spacing).unwrap();
        let (a, b) = (
            map.sample(0, lat_lon(d).0, lat_lon(d).1).unwrap(),
            map.sample(1, lat_lon(d).0, lat_lon(d).1).unwrap(),
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
    fn a_region_reads_inside_its_window_but_not_by_its_holes_and_fades_in_from_its_edges() {
        // 16 × 8 samples over 10°–18° E and 40°–44° N, 300 m but for a hole of no data at
        // column 12 (16.25° E) and row 2 (42.75° N).
        let (w, h) = (16_u32, 8_u32);
        let mut grid = vec![300_i16; (w * h) as usize];
        grid[(2 * w + 12) as usize] = NO_DATA;
        let region = Elevation {
            levels: vec![(w, h, grid)],
            digest: 0,
            window: Window::region((10.0, 18.0, 40.0, 44.0)),
        };
        let at = |lat: f64, lon: f64| region.sample(0, lat.to_radians(), lon.to_radians());
        assert!((at(42.0, 13.0).unwrap() - 300.0).abs() < 1e-9);
        assert!(at(42.0, 9.0).is_none(), "outside");
        assert!(at(42.5, 16.25).is_none(), "by the hole");
        assert_eq!(region.inside(direction(42.0, 9.0), 0.5), 0.0);
        assert_eq!(region.inside(direction(42.0, 14.0), 0.5), 1.0);
        let edge = region.inside(direction(42.0, 10.25), 0.5);
        assert!(edge > 0.0 && edge < 1.0, "{edge}");
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
        }
        // The coast at the centre texel: the height there before the sea flattens it.
        let half = size / 2;
        let cells = f64::from(1_u32 << cell.level());
        let (x, y) = cell.xy();
        let st = DVec2::new(x as f64, y as f64) / cells * 2.0 - 1.0
            + DVec2::splat((2 * half + 1) as f64) * (2.0 / cells / (2 * size) as f64);
        let direction = CubeSphere::direction(Face::from_index(cell.face()), st);
        let height = planet.height(direction, world.min_wavelength(cell.level() + 1));
        let range = COAST_RANGE * world.spacing(cell.level());
        let expected = 0.5 + 0.5 * (height / range).clamp(-1.0, 1.0);
        let alpha = f64::from(map[(half * size + half) * 4 + 3]) / 255.0;
        assert!(
            (alpha - expected).abs() < 0.003,
            "{alpha} against {expected}"
        );
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
    fn the_swap_rule_splits_rough_ground_where_it_shows_and_leaves_flat_ground_whole() {
        let planet = small();
        let world = &planet.world;
        let sphere = world.sphere();
        let radius = world.planet.radius;
        let a = DVec3::new(0.3, 1.0, 0.2).normalize();
        let k = 780.0;
        let area = |cut: &[CellId]| -> f64 {
            cut.iter()
                .map(|c| 1.0 / f64::from(1_u32 << (2 * c.level())))
                .sum()
        };
        // Rough ground everywhere (slopes of 45°, errors as wide as the samples): the cell
        // under the point down to the finest level, each cell split while its samples would
        // stand over `spacing_px` apart.
        let rough = |cells: &[CellId]| -> Vec<f64> {
            cells.iter().map(|c| world.spacing(c.level())).collect()
        };
        let cut = world.tile_cut_by_error(&[a * radius], k, rough);
        assert!((area(&cut) - 6.0).abs() < 1e-9);
        let under = sphere.cell_of(sphere.surface_point(a, 0.0), world.tiles.finest);
        assert!(cut.contains(&under));
        for c in &cut {
            let l = c.level();
            let d = (sphere.cell_center(*c) - a * radius).length() - 0.75 * sphere.cell_size(l);
            assert!(
                l == world.tiles.finest
                    || world.spacing(l) * k / d.max(1.0) <= world.tiles.spacing_px,
                "{c:?} should have split"
            );
        }
        // Flat ground (the open sea): nothing shows, nothing splits.
        let flat = world.tile_cut_by_error(&[a * radius], k, |cells| vec![0.0; cells.len()]);
        assert_eq!(flat.len(), 6);
        // Gentle slopes split only where their height would show, much nearer.
        let gentle = |cells: &[CellId]| -> Vec<f64> {
            cells
                .iter()
                .map(|c| 0.001 * world.spacing(c.level()))
                .collect()
        };
        let fewer = world.tile_cut_by_error(&[a * radius], k, gentle);
        assert!((area(&fewer) - 6.0).abs() < 1e-9);
        assert!(
            fewer.len() < cut.len(),
            "{} against {}",
            fewer.len(),
            cut.len()
        );
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
    fn a_tile_s_clusters_take_each_triangle_once() {
        for samples in [9_u32, 257] {
            let n = samples as usize;
            let mut seen = vec![0_u8; 2 * (n - 1) * (n - 1) + 8 * (n - 1)];
            for cluster in tile_clusters(samples) {
                assert!(cluster.len() <= forge_geom::MESHLET_MAX_TRIANGLES);
                for t in cluster {
                    seen[t as usize] += 1;
                }
            }
            assert!(seen.iter().all(|&s| s == 1), "{samples} samples");
        }
        let planet = small();
        let mesh = tile_mesh(&planet, CellId::cube(2, 4, 5, 7));
        let built = forge_geom::MeshletMesh::build_clustered(
            &mesh,
            &tile_clusters(planet.world.tiles.samples),
            forge_geom::CookOptions::default(),
        );
        assert_eq!(built.triangle_count, mesh.indices.len() / 3);
        assert!(
            built
                .meshlets
                .iter()
                .all(|m| m.parent_error >= m.self_error)
        );
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

/// The swap rule's calibration (#220), with the Earth's maps (`tools/fetch-planets.sh`): the
/// cells' errors in the cut at three tour stops, and the cut's tiles there under a few settings.
/// `cargo test --release -p forge-terrain calibration -- --ignored --nocapture`.
#[cfg(test)]
mod calibration {
    use super::*;
    use std::collections::HashMap;

    /// D-058's step 2: the heights in integers against the floats they replace, over samples of
    /// every face at four levels (the region over the Alps included), and their time on one core.
    #[test]
    #[ignore = "needs the Earth's maps"]
    fn the_heights_in_integers_stand_by_the_floats() {
        use std::time::Instant;
        let planet = Planet::new(PlanetWorld::load(&planet_file()).unwrap()).unwrap();
        let world = &planet.world;
        let cordic = Cordic::new();
        let (mut worst_direction, mut worst_height, mut count) = (0.0_f64, 0.0_f64, 0);
        let (mut float_s, mut fixed_s) = (0.0, 0.0);
        for level in [4_u8, 8, 12, 14] {
            let heights = planet.fixed_heights(world.min_wavelength(level)).unwrap();
            let bits = u32::from(level) + 8;
            for face in 0..6_u8 {
                let face_enum = Face::from_index(face);
                for k in 0..400_u64 {
                    // Samples spread over the face, and some by Mont Blanc on the face it lies on.
                    let h =
                        forge_core::hash::mix64(k * 6 + u64::from(face) + 1000 * u64::from(level));
                    let (mut i, mut j) = ((h & 0xFFFF_FFFF) as i64, (h >> 32) as i64);
                    let span = 1_i64 << bits;
                    if k % 2 == 0 {
                        let (f, st) = CubeSphere::face_coords(direction(45.83, 6.86));
                        if f != face_enum {
                            continue;
                        }
                        let at = |s: f64| ((s + 1.0) * 0.5 * span as f64) as i64;
                        (i, j) = (at(st.x) + (i % 4096) - 2048, at(st.y) + (j % 4096) - 2048);
                    }
                    let (i, j) = (i.rem_euclid(span + 1), j.rem_euclid(span + 1));
                    let d = fixed_direction(
                        face_enum,
                        fixed_warp(&cordic, i, bits),
                        fixed_warp(&cordic, j, bits),
                    );
                    let st = DVec2::new(i as f64, j as f64) / span as f64 * 2.0 - DVec2::ONE;
                    let float_direction = CubeSphere::direction(face_enum, st);
                    let fixed_d = DVec3::from_array(d.map(to_f64_q61));
                    worst_direction = worst_direction.max((fixed_d - float_direction).length());
                    let t = Instant::now();
                    let float_height = planet.height(float_direction, world.min_wavelength(level));
                    float_s += t.elapsed().as_secs_f64();
                    let t = Instant::now();
                    let fixed_height = planet.fixed_height(&heights, &cordic, d);
                    fixed_s += t.elapsed().as_secs_f64();
                    worst_height =
                        worst_height.max((fixed_height as f64 / METRE as f64 - float_height).abs());
                    count += 1;
                }
            }
        }
        println!(
            "{count} samples: directions within {worst_direction:.2e}, heights within {:.3} mm; a height {:.0} ns in floats, {:.0} ns in integers",
            worst_height * 1e3,
            float_s / f64::from(count) * 1e9,
            fixed_s / f64::from(count) * 1e9,
        );
        assert!(worst_direction < 1e-14, "directions {worst_direction}");
        assert!(worst_height < 1e-3, "heights {worst_height} m");
    }

    fn to_f64_q61(c: i64) -> f64 {
        c as f64 / ONE_Q61 as f64
    }

    const ONE_Q61: i64 = forge_core::fixed::ONE;

    #[test]
    #[ignore = "needs the Earth's maps"]
    fn cell_errors_at_the_stops() {
        let world = PlanetWorld::load(&planet_file()).unwrap();
        let planet = Planet::new(world).unwrap();
        let world = &planet.world;
        let sphere = world.sphere();
        // Pixels a radian at 900 rows and 60° of field of view.
        let k = 450.0 / (30.0_f64).to_radians().tan();
        for (name, lat, lon, over) in [
            ("Mont Blanc", 45.55, 6.55, 6000.0),
            ("Corsica", 42.15, 9.1, 400_000.0),
            ("Eze", 43.73, 7.36, 1500.0),
        ] {
            let d = direction(lat, lon);
            let ground = planet.ground(d, world.min_wavelength(world.tiles.finest));
            let camera = sphere.surface_point(d, ground + over);
            let cut = world.tile_cut_around(&[camera]);
            let started = std::time::Instant::now();
            let mut per_level: Vec<Vec<(f64, f64)>> = vec![Vec::new(); 21];
            for &cell in &cut {
                let e = planet.cell_error(cell);
                let dist = ((sphere.cell_center(cell) - camera).length()
                    - 0.75 * sphere.cell_size(cell.level()))
                .max(over.min(1000.0));
                per_level[usize::from(cell.level())].push((e, e * k / dist));
            }
            let ms = started.elapsed().as_secs_f64() * 1e3;
            println!(
                "{name}: {} tiles, {:.2} ms a cell",
                cut.len(),
                ms / cut.len() as f64
            );
            for (level, list) in per_level.iter_mut().enumerate() {
                if list.is_empty() {
                    continue;
                }
                list.sort_by(|a, b| a.0.total_cmp(&b.0));
                let e: Vec<f64> = list.iter().map(|x| x.0).collect();
                let mut px: Vec<f64> = list.iter().map(|x| x.1).collect();
                px.sort_by(f64::total_cmp);
                println!(
                    "  level {level:2}: {:3} cells, error {:8.3} / {:8.3} / {:8.3} m, px {:7.3} / {:7.3} / {:7.3}",
                    list.len(),
                    e[0],
                    e[e.len() / 2],
                    e[e.len() - 1],
                    px[0],
                    px[px.len() / 2],
                    px[px.len() - 1]
                );
            }
        }
    }

    #[test]
    #[ignore = "needs the Earth's maps"]
    fn swap_rule_cuts_at_the_stops() {
        let world = PlanetWorld::load(&planet_file()).unwrap();
        let mut planet = Planet::new(world).unwrap();
        let sphere = planet.world.sphere();
        let k = 450.0 / (30.0_f64).to_radians().tan();
        let mut memo: HashMap<CellId, f64> = HashMap::new();
        let stops = [
            ("Mont Blanc", 45.55, 6.55, 6000.0),
            ("Corsica", 42.15, 9.1, 400_000.0),
            ("Eze", 43.73, 7.36, 1500.0),
            ("Corsica low", 42.15, 9.1, 3000.0),
        ];
        for (spacing_px, slope, error_px) in [
            (3.2, 1.0, 1.0),
            (2.5, 1.0, 1.0),
            (1.8, 1.0, 1.0),
            (1.0, 1.0, 1.0),
        ] {
            planet.world.tiles.spacing_px = spacing_px;
            planet.world.tiles.slope = slope;
            planet.world.tiles.error_px = error_px;
            println!("spacing {spacing_px} px, slope {slope}°, error {error_px} px:");
            for (name, lat, lon, over) in stops {
                let d = direction(lat, lon);
                let fine = planet.world.min_wavelength(planet.world.tiles.finest);
                let camera = sphere.surface_point(d, planet.ground(d, fine) + over);
                let rings = planet.world.tile_cut_around(&[camera]);
                let cut = planet.world.tile_cut_by_error(&[camera], k, |cells| {
                    cells
                        .iter()
                        .map(|&c| *memo.entry(c).or_insert_with(|| planet.cell_error(c)))
                        .collect()
                });
                let mut levels = [0u32; 15];
                for c in &cut {
                    levels[usize::from(c.level())] += 1;
                }
                println!(
                    "  {name:12}: {:4} tiles (rings {:4}), per level {:?}",
                    cut.len(),
                    rings.len(),
                    levels
                );
            }
        }
    }

    /// Where a new tile's time goes (#220, "tiles cooked in milliseconds"): a tile of each of
    /// the four finest levels of the cut 6 km over Mont Blanc, made, cooked, stored, and its
    /// normal map made, on one core.
    #[test]
    #[ignore = "needs the Earth's maps"]
    fn a_new_tile_s_time() {
        use forge_geom::{CookOptions, GpuVertex, MeshletMesh};
        use std::time::Instant;
        let world = PlanetWorld::load(&planet_file()).unwrap();
        let planet = Planet::new(world).unwrap();
        let world = &planet.world;
        let sphere = world.sphere();
        let d = direction(45.55, 6.55);
        let ground = planet.ground(d, world.min_wavelength(world.tiles.finest));
        let camera = sphere.surface_point(d, ground + 6000.0);
        let cut = world.tile_cut_around(&[camera]);
        let mut cells: Vec<CellId> = Vec::new();
        for &cell in &cut {
            if !cells.iter().any(|c| c.level() == cell.level()) {
                cells.push(cell);
            }
        }
        cells.sort_by_key(|c| std::cmp::Reverse(c.level()));
        let dir = std::env::temp_dir().join("forge-tile-timing");
        std::fs::create_dir_all(&dir).unwrap();
        let ms = |t: Instant| t.elapsed().as_secs_f64() * 1e3;
        for cell in cells.into_iter().take(4) {
            let t = Instant::now();
            let mesh = tile_mesh(&planet, cell);
            let made = ms(t);
            let t = Instant::now();
            let cooked = MeshletMesh::build_with(&mesh, CookOptions { normal_weight: 0.0 });
            let cook = ms(t);
            let t = Instant::now();
            let by_cells = MeshletMesh::build_clustered(
                &mesh,
                &tile_clusters(world.tiles.samples),
                CookOptions { normal_weight: 0.0 },
            );
            let clustered = ms(t);
            println!(
                "  by its cells: {clustered:.1} ms, {} clusters, {} pages, {} levels, {} triangles",
                by_cells.meshlets.len(),
                by_cells.page_count,
                by_cells.levels(),
                by_cells.dag_triangle_count
            );
            // The cook's steps: the DAG alone, then its pages.
            let vertices: Vec<GpuVertex> = (0..mesh.positions.len())
                .map(|i| GpuVertex {
                    position: mesh.positions[i],
                    pad0: 0.0,
                    normal: mesh.normals[i],
                    section: 0.0,
                    uv: mesh.uvs[i],
                })
                .collect();
            let t = Instant::now();
            let mut dag = forge_geom::build_dag(
                &mesh.indices,
                &vertices,
                &vec![0; vertices.len()],
                0.0,
                forge_geom::MAX_LEVELS,
            );
            let dag_ms = ms(t);
            let t = Instant::now();
            let pages = forge_geom::page::pack(&mut dag, &vertices, true, None);
            let pack = ms(t);
            let t = Instant::now();
            forge_geom::cache::save(&dir.join("tile.fmesh"), &cooked, 1).unwrap();
            let save = ms(t);
            let t = Instant::now();
            let normals = tile_normal_map(&planet, cell);
            let map = ms(t);
            let t = Instant::now();
            let levels = normal_map_levels(normals, NORMAL_MAP_SIZE);
            let mips = ms(t);
            println!(
                "level {:2}: made {made:6.1} ms, cooked {cook:6.1} (DAG {dag_ms:6.1}, pages {pack:5.1}), stored {save:5.1}, normal map {map:6.1} + mips {mips:4.1}; {} triangles; its DAG {} clusters, {} pages, {} levels, {} triangles; {} map levels",
                cell.level(),
                mesh.indices.len() / 3,
                cooked.meshlets.len(),
                pages.count(),
                cooked.levels(),
                cooked.dag_triangle_count,
                levels.len(),
            );
        }
    }
}
