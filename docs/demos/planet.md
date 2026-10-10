# Demo: planet

Phase 2's planet (D-056, #220): a planet of Earth's size from orbit down to its ground. The
Earth comes from NOAA's real elevation and NASA's Blue Marble, the Moon from NASA's elevation and
colour maps, and noise adds the detail under their resolution. The sky is NASA's map of the real
stars. It is the island demo's second step. The owner wanted it as a demo of its own
(2026-10-09), so that planets, moons and later suns stay apart from the island.

```
tools/fetch-planets.sh                                    # the maps, once: about 750 MB
cargo run --release -p planet                             # the Earth: a 60 s descent to Èze
cargo run --release -p planet -- --shot orbit             # held at a golden shot: orbit, high, ground, top
cargo run --release -p planet -- --target 42.15,9.1 --heading 0 --shot top   # Corsica from the station
cargo run --release -p planet -- --world assets/worlds/moon.toml
```

**P** stops the descent and flies: right mouse to look, WASD, Shift faster. The speed grows with
the height. The other keys are the ballad's: **T** TAA, **O** occlusion, **C** cone culling,
**X** show culled, **K** LOD colours, **J** shadows, **N** ambient occlusion, **G** tone curve,
**-** / **=** exposure, **[** / **]** LOD error.

| Flag | What it does |
|---|---|
| `--world FILE` | the world (`assets/worlds/earth.toml` by default, `moon.toml`); `--print-world` prints it |
| `--shot orbit\|high\|ground\|top` | holds the descent at a golden shot; `top` looks straight down from the orbit's height |
| `--target LAT,LON`, `--heading DEG` | where the descent ends and the way it looks, over the world's |
| `--sun-elevation`, `--sun-azimuth` | the sun over the target, degrees (25° and 240° by default) |
| `--ev100 EV`, `--auto-exposure` | the exposure: sunny 16 (EV 15) by default, as a camera takes anything sunlit; or metered |
| `--stars STOPS` | the stars' brightness: a map value of 1 at 2^STOPS cd/m² (12) |
| `--radius KM`, `--seed N` | over the world's |
| `--tour`, `--tour-stop N` | flies the world's tour, or holds its stop N |
| `--check-swaps DIR` | holds still at each swap of the tiles and saves the frames before and after it, for `tools/swap-check.sh` |
| `--resident` | every page resident instead of the 512 MiB streamed pool (the A/B) |

## How it is made

- **The world file** (`assets/worlds/earth.toml`, `moon.toml`; `forge_terrain::planet`):
  - the radius and the noise;
  - the maps (elevation, colour, sea mask) and the noise under them;
  - the tiles' size and how finely the cut follows the target;
  - the target, the way the descent comes in, and the height it starts from (400 km on the
    Earth; 4 000 km on the Moon, from where it is seen whole);
  - whether there is air, and the sky's map with the body's pole and prime meridian.
- **The maps** (`tools/fetch-planets.sh`, into `assets/planets/`, which git ignores). The
  script pins each download by size and SHA-256, and Blender converts what the engine can't read
  (`assets/blender/planet_*.py`; Forge has no TIFF reader).

  | Map | Source | Converted to |
  |---|---|---|
  | The Earth's elevation | NOAA's ETOPO 2022, 60 arc-seconds (about 1.85 km), public domain | `i16` metres |
  | The Earth's colour | NASA's Blue Marble Next Generation, July 2004, without its relief shaded | 16 384 × 8 192 JPEG (a power of two, AMD's widest image) |
  | The Earth's sea | the elevation under 0 m | 8 192 × 4 096 grey PNG |
  | The Moon's elevation | NASA's CGI Moon Kit, 16 samples a degree (about 1.9 km) | `i16` metres |
  | The Moon's colour | the CGI Moon Kit's 2025 colour map at 4K | PNG |
  | The sky | NASA's Deep Star Maps 2020 at 8K (Gaia, Hipparcos) | sRGB PNG |
- **The height** (`Planet::height`):
  - the map, read through its samples with Catmull-Rom from its mip pyramid, at the level that
    holds nothing narrower than the tile's samples allow (bilinear left flat facets a texel
    wide: Tycho's central peak was a square pyramid);
  - then band-limited 3-D gradient noise (Perlin's improved gradients) for the octaves under the
    map's resolution, a quarter as strong at the sea's level as on the high ground.
  The Earth's ground under 0 m is sea, flattened at its level. The Moon has none.
- **The tiles:** a cell of D-037's equi-angular cube sphere each.
  - 257² samples in the planet's axes, relative to the cell's centre.
  - Normals from a ring of samples beyond the edge, so neighbours of one level share their
    edges' normals.
  - A skirt on vertices of their own hangs from the edge, so the simplifier keeps the edge
    locked.
  - UVs over the tile, for its normal map: 256² texels from the next level's height, mip-mapped,
    which the shading reads instead of the vertices' normals ("The tiles' normal maps" below).
  - Each is cooked into a cluster DAG through the cache (`cache/meshes/earth@face-level-x-y`),
    keyed by the world's shape, the map's digest and the code (not by its colours).
- **The cut** (`PlanetWorld::tile_cut_around`): a quadtree of cells around a few points, the
  camera and where the run heads (the tour's next stop, the descent's target). A cell splits while
  one of them lies within its side plus half its diagonal, down to level 14 on the Earth (tiles of
  611 m, samples 2.4 m apart) and 12 on the Moon. A camera high over the ground splits no cell much
  smaller than its height. Each tile's DAG coarsens with distance, so the 600 m tiles under the
  camera cost almost nothing from orbit.
- **Drawing:** the cluster renderer, streamed through a 512 MiB pool. The start view's pages load
  before the first frame. Each tile is an instance placed at its `f64` position
  (`add_instance_at`), turned so the target stands at the world's origin with +Y up.
- **Shading:** the layered ground with its layers by height, slope and latitude
  (`RenderLayer::planet_layers`, `planet_layers` in `meshlet.slang`).
  - **Near the ground:** on the Earth, sea, sand (where a pixel spans under 40 m), grass, rock on
    slopes over about 0.3, and snow over a line falling from 4 200 m at the equator to 300 m at
    the poles. On the Moon, regolith of a few kinds under its colour map.
  - **From afar** (a pixel spanning 30 m to 400 m and more), the maps stand for the ground: the
    Blue Marble's colour, and the sea from the mask rather than the coarse tiles' triangles, whose
    coasts were kilometre-wide shapes. Water is shaded flat, and on the planet the layers'
    highlights blend in strength and power alike, so no bright line follows the coasts.
  - The textures lie on the scene's frame, one surface over every tile.
  - **Hex tiling** (#66, as on the island) on every textured row: the grass's 12 m repeat showed
    as a grid of stripes over the slopes at Èze (below, before and after). The planet's tiles
    count as one instance for its offset, so it is continuous across them.

    ![the grass at Èze before and after hex tiling](images/planet-hex-tiling.png)
- **The sky:**
  - **The Earth:** Hillaire's atmosphere at the planet's radius.
    - The aerial volume reaches up to 64 km. Each pixel beyond it is marched on its own in 32
      steps closing up towards the ground (`march_beyond`), and the volume's slices longer than
      2 km are marched in steps: from orbit, no rings.
    - When the camera is over 1.5 km up, the ground's sky light and the reflections it takes
      come from a second set of tables at 2 m over the ground under the camera: from orbit, the
      camera's own sky is space's black.
  - **The stars** (`forge_render::SkyBox`): NASA's map, turned by the body's pole and prime
    meridian (the IAU's, the Moon's at J2000) and added over the air's sky, fading through its
    lowest 40 km. On an airless body it draws the sun's disc too. The map is made for display (its
    bright stars clipped), so the stars carry no photometry: at sunny 16 they are faint dots,
    where an eye beside a sunlit body would see none, and `--stars` sets them.
- **Shadows:** the sun's rays against each level's tiles cut as one surface (120 000 triangles a
  level). Every tile is ground to the rays (`set_ray_terrain`), so the finest tiles' shadows
  start clear of the rays' cut, as the island's do.

## The tiles as the camera flies

The cut follows the camera (`demos/planet/src/stream.rs`). Every frame the demo works out the cut
around where the camera will be once a new scene is ready (the last scene's build time ahead, on
the tour and the descent) and where the run heads. When that cut is finer somewhere than the one
drawn, or the one drawn holds half as many tiles again, a worker builds its scene:
- **The tiles:** it keeps those of the scene drawn, loads the ones it lacks from the cache, or
  makes and cooks them on a quarter of the hardware threads (half the cores), so the frames keep
  theirs.
- **The scene:** it builds a whole one, with its tables, its rays' structures and the pages its
  first view wants.
- **The swap:** the demo swaps it in at the start of a frame, and the old one goes to the frames'
  deferred deletion, freed once no frame in flight reads it. The tiles keep their places in the
  world, so TAA keeps its history and the sky stays the same.

A cut that would only coarsen waits for the next one that refines: finer tiles where they are no
longer needed cost little. The tour asks for its next stop's tiles while it holds at the one
before, so they are there when it arrives. They are made once, then loaded from the cache.

| The Earth's tour, uncapped (2026-10-10) | Tiles | A new scene | Swaps |
|---|---|---|---|
| The Alps from 400 km, then on to Corsica | 204 | none: the cut only coarsens | 0 |
| Corsica to Mont Blanc (17 s) | 393–423 | 0.23–0.25 s | 24 |
| Over Mont Blanc, then towards Èze | 525–675 | 0.37–0.39 s | 29 in its first 20 s |

- **A tile not in the cache** adds 0.26 s on a worker thread. The tour's tiles are made once.
- **The first scene,** behind the loading screen: 0.15 s for 204 tiles.
- **A swap's cost to the frames:** the longest frame between two swaps is 7.7 ms on average over
  28 swaps, 14.7 ms at most, against 2 ms uncapped; frames of 6–8 ms come without swaps too.
  A new scene uploads 50–300 MB (its clusters' table, its tiles' root pages, the pages of its
  first view).
  - **Before the fix** these frames reached 23–50 ms, in proportion to the upload. A Tracy
    capture showed the GPU idle after a present while the CPU waited for its frame slot.
  - **The cause:** each upload's staging buffer, larger than the allocator's blocks, was host
    memory of its own, allocated and freed every time.
  - **The fix:** staged uploads (`Device::write_buffer_staged`) now go through one buffer of at
    most 16 MiB, reused chunk by chunk.
  - **No difference:** the pool's size, the rays' structures (`--no-shadows`) and the queue the
    copies took.

**Checking the swaps** (D-056's swap rule): `planet --check-swaps DIR` holds the camera still at
each swap. It saves the frame before it and the frame after, a whole number of TAA's jitter
periods apart, once each has settled. `tools/swap-check.sh DIR` compares the two with ꟻLIP
against D-017's class 2.
- **The tour at a fixed step:** 14 swaps, every mean between 0.0002 and 0.0027, far under 0.02.
  11 swaps peak under 0.15.
- **Three peak at 0.15–0.24** on a patch rather than isolated pixels: the snow's shading on the
  slopes near the camera over Mont Blanc, up to 30 levels, as one cell gives way to its four
  children. Side by side the two frames look alike, but such a patch would show as a faint pop.
  The swap rule, splitting a cell where its error would show rather than at a distance, is the
  remedy to come.
- **Wider rings** (`rings = 2.3`, a sample under a pixel where a cell splits) move the patches
  out to the horizon, fainter, but 2 of 4 swaps still peak at 0.15. The tiles over Mont Blanc
  double (about 1 200) and a new scene takes 0.7 s.
- **Why: the normals, not the heights.**
  - Where a level-13 cell splits, about 2 km from the camera, its children add noise of 10–20 m
    wavelength. Their height differs by about 0.3 m, which is 0.13 px: the research's swap
    criterion, on the height's error, already holds.
  - Their slopes differ by about 8°, which is what the snow's shading and the layers' slope
    rules show.
  - The cluster DAG keeps its vertices' own normals as it simplifies (terrain cooks with
    `normal_weight` 0), so a fine tile's coarse clusters carry samples of its finest slopes,
    where its parent's normals are smooth.
  - Each level adds an octave of slope, and a swap reveals it, until the children's finest
    wavelength falls under a pixel: about 5 rings, three times the tiles.
- **The remedy:** a normal map per tile, mip-mapped, from the next level's height. The parent then
  already shows its children's slopes, and the mips filter the slopes a coarse cluster's vertices
  only sample. See the next section.
- **The horizon test** D-056 planned for the instance cull is left out. The whole instance cull
  takes 0.014 ms a frame for the planet's 400–700 tiles (Tracy, the tour), and the depth pyramid
  already culls the far side's tiles.

What the engine gained for it:
- **`MeshletSceneBuilder::set_material_rows`:** scenes built in turn over one texture set, which
  the demo keeps.
- **`Device::execute_transient_on`:** staged copies run on the transfer queue and structures are
  built on the compute queue, beside the frames rather than between them (the graphics queue
  without them, or with `FORGE_ASYNC=0`).
- **`Device::build_blases`** uploads every mesh's triangles in two copies rather than two a mesh.
  Each copy waited behind the frames in flight: a scene of 200 tiles took 0.65 s to build while
  frames ran, now 0.15 s.
- **The frame's per-instance transients** (the instance culls' status words, the deferred
  instances) are sized for the instances rounded up to 1 024. A scene a few tiles larger or
  smaller keeps the graph's layout, where before the 46 MB heap was laid out again at every swap.

## The tiles' normal maps

Each tile carries a normal map (`forge_terrain::planet::tile_normal_map`).
- **What it holds:** 256² texels over the tile, each the ground's normal at its centre in the
  planet's frame, from the height of the next level down. Its mips are each the mean of the four
  texels below. It is made with the tile and kept in the world cache (`cache/world/`).
- **How it is drawn:** the tile's instance names the map (`set_instance_texture`, in the instance
  record's spare word). The layered ground reads its normal there through the tile's UVs, with
  the pixel's derivatives choosing the mip, and turns it by the instance's rotation.
- **What it changes:**
  - **The swaps:** a tile's map already holds the slopes its children's vertices add, so a
    split changes no slope that shows.
  - **From afar:** the mips filter the slopes a coarse cluster's vertices only sample. The DAG
    keeps each vertex's own normal.
  - **The Alps from orbit**, below, before and after: ridges and valleys where the snow was soft
    blobs that read as cloud, since the layers' slope rule now sees the next level's slopes.
- **Cost:**
  - 341 KB a tile with its mips, 228 MB for the 669 tiles over Mont Blanc.
  - Cooking slows from about 28 tiles a second to 19, for the map's 263 000 heights.
  - A scene with 318 new maps takes 1.0 s to build (their uploads), otherwise 0.4 s.
- **The swaps with the maps** (`--check-swaps`, the tour at a fixed step): 11 swaps, 9 under
  both thresholds, most with under 300 pixels changed. Two still peak:
  - 0.21 on 83 pixels;
  - 0.28 on a patch of snow, where the coarse tile's triangles showed through the ambient
    occlusion (from the depth) and the shadows, which follow the geometry, not its normals.

![the Alps from orbit before and after the tiles' normal maps](images/planet-normal-maps.png)

## The tour and the bodies in the sky

`planet --tour` flies the world's `[[view.tour]]` stops in turn, and `--tour-stop N` holds stop N
for a capture. The camera travels along the great circle between stops, its height eased in its
logarithm and raised over long hops, so it clears the mountains between. Heading, pitch and field
of view ease too. A stop looking up narrows the field of view, because the Moon is half a degree
across.

| The Earth's tour (`earth.toml`) | The Moon's tour (`moon.toml`) |
|---|---|
| the Alps from the station (400 km) | the Moon from 4 000 km |
| Corsica straight down | Tycho from 60 km |
| Mont Blanc from 6 km | on Tycho's floor |
| Èze over the sea | the Earth over Tycho (12° field of view) |
| the Moon over the sea (6° field of view) | |

![the two tours' stops](images/planet-tour.png)

**The bodies** (`[[view.bodies]]`, `forge_render::SkyBody`): the Moon in the Earth's sky, the
Earth in the Moon's. Each is placed by its azimuth and elevation over the target. Its colour map
is the Moon Kit's or the Blue Marble's, turned so a given point faces the camera: the Moon's near
side, or Europe and Africa. It is lit by the same sun as the ground, so its phase follows: the
sun at 240° and the Moon at 100° make it three quarters lit. On the Earth, its light passes through
the air (the CPU's transmittance) and adds to the day sky, which lies in front of it. The Earth's
air is a rim of blue light at its limb.

![the Earth over Tycho, the Moon over the sea](images/planet-bodies.png)

- **Against photographs:** the Moon in the day sky is pale, its maria clear, its dark side lost in
  the blue. The Earth from the Moon has no clouds, since the Blue Marble is cloud-free, where the
  real Earth is about two-thirds covered and white with them.
- **Cost:** `sky/body` 0.01–0.02 ms.

## Numbers (RTX 5070 Ti, 1600 × 900, TAA, 2026-10-10)

| | Earth (Èze) | Moon (Tycho) |
|---|---|---|
| Tiles in the cut (camera and target) | 498 over the ground, 522 from orbit (14–56 a level) | 363–369 |
| Triangles in the tiles' finest levels | 66.3 M | 49.1 M |
| Cluster pages (128 KiB, with the tiles' UVs) | 16 298 | 12 098 |
| The rays' cuts | 1.92 M triangles, 115 MiB (from orbit) | 1.32 M, 79 MiB |
| First start (the maps, then every tile made and cooked with its normal map) | about 35 s for 669 tiles (19 a second) | about 20 s (369 tiles at that rate) |
| Later starts (tiles from the cache) | 8.1 s (the 16K colour map 7 s; the scene 0.6 s with its 522 normal maps) | 7.9 s (the Earth's 16K map, for the Earth in its sky) |
| GPU frame from orbit | 1.20 ms (`sky/compose`'s march 0.30, `shading/layered` 0.28) | 0.74 ms (4 000 km; `sky/box` 0.06) |
| GPU frame straight down over Corsica | 1.46 ms (the march 0.46 over the whole frame) | — |
| GPU frame over the ground | 0.91 ms (`shading/layered` 0.29) | 1.26 ms (`shading/layered` 0.66: hex-tiled regolith over the whole frame) |
| GPU memory | 2.2 GiB allocated: textures 1.25 GiB (the 16K colour map and its mips, 0.18 GiB of normal maps), geometry 0.86 GiB | 1.9 GiB |

- **A tile:** made in about 0.1 s and cooked in 0.16 s on one core (133 000 triangles, about 33
  pages with its UVs), its normal map's 263 000 heights on top: about 19 a second over the
  machine's cores (669 in 35 s; 28 without the maps).

## Left for later (#220 and D-056's steps)

- **A scene that takes and frees tiles in place,** rather than a whole scene built again for each
  new cut, with the stall that brings. Also a tile cooked in milliseconds rather than a quarter of
  a second (a regular grid's DAG built directly).
- **Clouds for the Earth seen from afar,** as the Moon and orbit see it.
- **The Alps look like rolling hills from the Mont Blanc stop,** 6 km up, though their normal
  maps give them ridges from orbit.
  - ETOPO's samples are 1.85 km apart, so peaks and valleys narrower than that are smoothed
    away.
  - The noise under them adds ±90 m at most, and no ridges.
  - They need a finer elevation for the mountains (Copernicus GLO-30, 30 m, free, a download to
    ask for) or ridged detail where the map stands high.
- **Steps where levels meet:** a tile next to a coarser one meets it with a step its skirt fills.
  The research's swap rule (a level only where its parent errs under a pixel) comes with
  streaming.
- **The haze from orbit:** a light veil over the land that the station's processed photographs
  don't show; to compare with raw ones.
- **The Earth's land under the sea's level** (the Netherlands, the Caspian's shores) floods, and
  the Blue Marble is July's: no seasons, no clouds.
- **The 16K colour map's start:** 7 s to decode it and make its mips, every start; to cache.
- **The island on the planet** (D-056's step 2), the sea's waves on the sphere (step 3), the
  genesis across the faces (step 4), the descent's checks (step 5).

## Captures

`tools/captures.sh` takes the planet when its maps are fetched (never in CI):
- the Earth at its three shots (`planet-orbit`, `planet-high`, `planet-ground`);
- the ground's twins with the occlusion off and every page resident;
- Corsica straight down from 400 km at the sun's 55° (`planet-corsica-top`);
- the tour's stop under the Moon over the sea (`planet-tour-moon`);
- the Moon from 4 000 km, on Tycho's floor, and the Earth over Tycho (`planet-moon-earth`).
