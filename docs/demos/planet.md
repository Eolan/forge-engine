# Demo: planet

Phase 2's planet (D-056, #220): a planet of Earth's size from orbit down to its ground. The
Earth comes from NOAA's real elevation, the Moon from NASA's, and noise adds the detail under
their resolution. It is the island demo's second step. The owner wanted it as a demo of its own
(2026-10-09), so that planets, moons and later suns stay apart from the island.

```
tools/fetch-planets.sh                                    # the maps, once: 466 MB + 95 MB
cargo run --release -p planet                             # the Earth: a 60 s descent to Èze
cargo run --release -p planet -- --shot orbit             # held at a golden shot: orbit, high, ground
cargo run --release -p planet -- --world assets/worlds/moon.toml
```

**P** stops the descent and flies: right mouse to look, WASD, Shift faster. The speed grows with
the height. The other keys are the ballad's: **T** TAA, **O** occlusion, **C** cone culling,
**X** show culled, **K** LOD colours, **J** shadows, **N** ambient occlusion, **G** tone curve,
**-** / **=** exposure, **[** / **]** LOD error.

## How it is made

- **The world file** (`assets/worlds/earth.toml`, `moon.toml`; `forge_terrain::planet`):
  - the radius;
  - the elevation map and the noise under it;
  - the tiles' size and how finely the cut follows the target;
  - the target (latitude, longitude) and the way the descent comes in;
  - whether there is air.
  `--print-world` prints it, flags applied.
- **The maps** (`tools/fetch-planets.sh`, into `assets/planets/`, which git ignores):
  - **Earth:** NOAA's ETOPO 2022 at 60 arc-seconds (about 1.85 km), public domain.
  - **Moon:** NASA's CGI Moon Kit, its elevation at 16 samples a degree (about 1.9 km) and its
    2025 colour map at 4K.
  - The script pins each download by size and SHA-256. Blender converts the TIFFs into what the
    engine reads, `i16` metres and a PNG (`assets/blender/planet_elevation.py`,
    `planet_colour.py`), since no TIFF reader is in Forge's dependencies.
- **The height** (`Planet::height`):
  - the map, bilinear from its mip pyramid at the level that holds nothing narrower than the
    tile's samples allow;
  - then band-limited 3-D gradient noise (Perlin's improved gradients) for the octaves under the
    map's resolution, a quarter as strong at the sea's level as on the high ground.
  The Earth's ground under 0 m is sea, flattened at its level. The Moon has none.
- **The tiles:** a cell of D-037's equi-angular cube sphere each.
  - 257² samples in the planet's axes, relative to the cell's centre.
  - Normals from a ring of samples beyond the edge, so neighbours of one level share their
    edges' normals.
  - A skirt on vertices of their own hangs from the edge, so the simplifier keeps the edge
    locked.
  - Each is cooked into a cluster DAG through the cache (`cache/meshes/earth@face-level-x-y`),
    keyed by the world, the map's digest and the code.
- **The cut** (`PlanetWorld::tile_cut`): a quadtree of cells. A cell splits while the target lies
  within its side plus half its diagonal, down to level 14 on the Earth (tiles of 611 m, samples
  2.4 m apart) and 12 on the Moon. Each tile's DAG coarsens with distance, so the 600 m tiles
  under the camera cost almost nothing from orbit.
- **Drawing:** the cluster renderer, streamed through a 512 MiB pool. The start view's pages load
  before the first frame. Each tile is an instance placed at its `f64` position
  (`add_instance_at`), turned so the target stands at the world's origin with +Y up. Packed
  vertices on a coarser grid for the largest tiles (#218: only past 2^20 m of reach, so no other
  mesh changes).
- **Shading:** the layered ground with its layers by height, slope and latitude
  (`RenderLayer::planet_layers`, `planet_layers` in `meshlet.slang`).
  - **The Earth:** sea, sand (where a pixel spans under 40 m), grass, rock on slopes over about
    0.3, snow over a line falling from 4 200 m at the equator to 300 m at the poles.
  - **The Moon:** regolith under its colour map.
  - The textures lie on the scene's frame, one surface over every tile.
- **The sky:**
  - **The Earth:** Hillaire's atmosphere at the planet's radius.
    - The aerial volume reaches up to 64 km.
    - Each pixel beyond it is marched on its own in 32 steps closing up towards the ground
      (`march_beyond`).
    - The volume's slices longer than 2 km are marched in steps.
    - Together these remove the rings a single volume left from orbit.
  - **The Moon:** the ballad's starfield.
- **Shadows:** the sun's rays against each level's tiles cut as one surface (120 000 triangles a
  level). Every tile is ground to the rays (`set_ray_terrain`), so the finest tiles' shadows
  start clear of the rays' cut, as the island's do.

## Numbers (RTX 5070 Ti, 1600 × 900, TAA, 2026-10-10)

| | Earth (Èze) | Moon (Tycho) |
|---|---|---|
| Tiles in the cut | 492 (14–48 a level) | 423 |
| Triangles in the tiles' finest levels | 65.5 M | 56.3 M |
| Cluster pages (128 KiB) | 12 848 | 10 834 |
| The rays' cuts | 1.68 M triangles, 101 MiB | 1.44 M, 86 MiB |
| First start (map, then every tile made and cooked) | 18.1 s | 15.7 s |
| Later starts (from the cache) | 1.3 s (the map 0.5 s) | 1.3 s |
| GPU frame from orbit (400 km) | 0.95 ms (`sky/compose`'s march 0.26) | 0.81 ms |
| GPU frame at 10 km | 0.89 ms | — |
| GPU frame over the ground | 0.86 ms | 0.85 ms |

- **A tile:** made in about 0.1 s and cooked in 0.16 s on one core (133 000 triangles, 23 pages),
  about 28 a second over the machine's cores (492 in 17.6 s).

## Left for later (#220 and D-056's steps)

- **Tiles that come and go as the camera flies:** the cut is made once at start, so the camera
  stays near the target. Streaming tiles needs meshes and instances added and freed while frames
  run (the renderer's scene is fixed today), and a tile cooked in milliseconds rather than a
  quarter of a second (a regular grid's DAG built directly).
- **Steps where levels meet:** a tile next to a coarser one meets it with a step its skirt
  fills, visible from 10 km on the Moon where the finer tiles hold the detail noise the coarser
  ones leave out. The research's swap rule (a level only where its parent errs under a pixel)
  comes with streaming.
- **Coasts at coarse levels:** from orbit the coasts follow the large tiles' triangles. The
  Earth's land under the sea's level (the Netherlands, the Caspian's shores) floods.
- **The Moon's sky:** the ballad's nebula, too bright for a planet. The real sky (`--real-sky`'s
  catalogue) comes later.
- **The island on the planet** (D-056's step 2), the sea's waves on the sphere (step 3), the
  genesis across the faces (step 4), the descent's checks (step 5).

## Captures

`tools/captures.sh` takes the planet when its maps are fetched (never in CI):
- the Earth at its three shots (`planet-orbit`, `planet-high`, `planet-ground`);
- the ground's twins with the occlusion off and every page resident;
- the Moon from orbit and on Tycho's floor.
