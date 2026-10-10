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
        Ok(Self {
            world,
            map,
            regions,
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
                let with_detail = |base: f64, (width, count, strength, gain), seed: &str| {
                    let rough = 0.25 + 0.75 * smooth(base / params.rough_above);
                    let detail = octaves(
                        Seed::new(planet.seed).derive_str(seed),
                        p,
                        (width, count, gain),
                        min_wavelength,
                        |n| n,
                    );
                    base + strength * rough * detail
                };
                let mut height = with_detail(whole, params.detail, "detail");
                // The regions' finer heights over it, faded in from their edges. Their sea (at
                // or under half a metre) takes the whole map's depths, two metres deeper at
                // least, without noise: the sea flattens it anyway, and noise there raised
                // islands of sand along the coasts.
                for (region, grid) in params.regions.iter().zip(&self.regions) {
                    let weight = grid.inside(direction, region.blend);
                    if weight <= 0.0 {
                        continue;
                    }
                    let Some(fine) = grid.height(direction, planet.radius, min_wavelength) else {
                        continue;
                    };
                    let regional = if fine > 0.5 {
                        with_detail(fine, region.detail, "region detail")
                    } else {
                        whole.min(0.0) - 2.0
                    };
                    height += (regional - height) * weight;
                }
                height
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
    let place = |i: usize, j: usize| {
        CubeSphere::direction(face, start + DVec2::new(i as f64, j as f64) * half)
    };
    // Only the samples a texel reads: its neighbours (one coordinate odd, the other even) and
    // its centre (both odd), not the corners (both even). A centre's height is kept for its
    // coast.
    let mut points = vec![DVec3::ZERO; side * side];
    let mut centres = vec![0.0; size * size];
    for j in 0..side {
        // An even row holds neighbours at its odd places only; an odd row is all read.
        let (first, step) = if j % 2 == 0 { (1, 2) } else { (0, 1) };
        for i in (first..side).step_by(step) {
            let direction = place(i, j);
            let height = planet.height(direction, min_wavelength);
            if i % 2 == 1 && j % 2 == 1 {
                centres[j / 2 * size + i / 2] = height;
            }
            points[j * side + i] = direction * (radius + planet.ground_of(height));
        }
    }
    let at = |i: usize, j: usize| points[j * side + i];
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
            let coast = 0.5 + 0.5 * (centres[j * size + i] / range).clamp(-1.0, 1.0);
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
