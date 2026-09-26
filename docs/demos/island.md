# Demo: island

Phase 2's demo (`docs/ROADMAP.md`): a 16 km island generated from a seed, later
orbit-to-ground on the planet variant, and the first step of rebuilding tropical-island (#81).
It is built in steps; the first, the terrain's genesis on the CPU, started on 2026-09-26 in a
cloud session, following `docs/research/terrain-genesis.md` ("Recommendation for Forge").

| Step | State |
|---|---|
| The island's mask, uplift, hardness and rain fields (stages 1–2) | ✅ `forge_procgen::island` |
| D8 drainage, the downstream-first stack, integer areas; depressions by the basin graph each step, by priority flood for the reference and the lakes | ✅ `forge_procgen::flow` |
| The implicit stream-power erosion with diffusion, rows and drainage trees in parallel on the job system; lakes as filling depressions (stage 3) | ✅ `forge_procgen::erosion` |
| PNG previews: height, hillshade, flow, the overview with sea, rivers and lakes, the network by Strahler order | ✅ `forge_procgen::preview`, `tools/genesis` |
| Hydrology: rivers as polylines with Strahler orders and widths, lakes with levels and outlets (stage 4) | ✅ `forge_procgen::hydrology` |
| The water's fields: the signed coast distance; the sea's directional spectrum (JONSWAP/TMA, Horvath's spreading) synthesised by an inverse FFT on the CPU into a tiling patch of heights, displacements, slopes and the Jacobian | ✅ `forge_procgen::coast`, `forge_procgen::ocean`; the first step of `docs/research/water.md`'s plan, the GPU's cascades to be diffed against it |
| Amplification to 2 m per tile with halos (stage 5) | planned |
| Materials from the fields, the layer map (stage 6) | started: sea floor, sand, grass and rock from the height and the slope, the rivers and lakes painted in (`forge_procgen::slope_layers`, `paint_rivers`, `paint_lakes`; "In the engine" below); moisture, soil and the rivers' banks planned |
| The hand-off to the cluster-DAG cook: the island drawn by today's renderer (stage 7) | ✅ drawn on the 5070 Ti (2026-09-26): `city-blocks --island SEED`, with its own ground, a sea floor, rocks and a stand-in sea ("In the engine" below, #96) |
| The planet: the same stages on the cube sphere's coarse graph, tiles amplified at streaming time | planned |

```
cargo run --release -p genesis -- --spacing 16 --steps 150 --out captures/island
cargo run --release -p city-blocks -- --island 7
```

`genesis`: `--seed N`, `--spacing M` (16: 1025² samples; 4: the 4097² target), `--steps N`,
`--k` (erodibility), `--diffusion`, `--uplift` (metres per step at the heart), `--every N` (a
hillshade every N steps), `--threads N` (workers besides the main thread; the default is one
per hardware thread, `0` is serial, the result is the same), `--wind-from W` (a compass point,
north up: orographic rain, see below) with `--rain-contrast C` (1). It prints each stage's
time, the erosion step's breakdown, and writes `uplift.png`, `height.png` (16-bit),
`hillshade.png`, `flow.png` (log drainage), `overview.png`, `network.png` (the rivers by
Strahler order and the lakes over the hillshade), `coast.png`, `sea-height.png`,
`sea-hillshade.png` and, with a wind, `rain.png`.

`city-blocks --island SEED` draws the island in the engine (stage 7, written in the cloud,
first seen on the 5070 Ti on 2026-09-26: "In the engine" below): the heightfield (`--island-spacing`, 8 m by default: 2049²,
8.4 M triangles like the city's ground; 4 m for the 4097² target, 33.5 M) is generated once
into `mesh-cache/island-<key>.f32`, cooked into a cluster DAG through the same path as the
city's terrain (`PropKind::Heightfield`, `forge_geom::city::heightfield_mesh`) and cached.
It is drawn on its own layered ground: sand on the shore, rock where the ground is steeper than
0.45, grass elsewhere, and a sea floor falling away from the coast (`forge_procgen::slope_layers`
with a texel every 4 m, and `forge_procgen::sea_floor`). Around it:
- 300 000 rocks, placed by the GPU on its land;
- a flat plane at 0 m that stands in for the sea out to the horizon;
- the city's sky at a 30° sun, with the sun's shadows, the probes, the mirror rays and TAA.

The camera starts on the south coast looking inland; `--view` and the usual keys apply, and so
do `--origin`, `--sun-elevation` and `--instances`. The first start costs the erosion (5 s at
8 m on the 9800X3D, 13 s on the cloud's four cores) and the cook (11–16 s); the next ones load
both.

## The genesis, on the CPU (2026-09-26)

Every stage is a pure function of the seed and a parameter record, deterministic on every
machine (D-016): the noise is gradient noise on an integer lattice hashed with `pcg3d`, with
eight fixed gradient directions so a sample needs no transcendental; the drainage area is an
integer count; the D8 tie-break and the flood's queue order are pinned; the heavy arithmetic
is `f32` and `f64` with `sqrt` only.

1. **Mask** (`island`): a distance-to-centre shape warped by four octaves of noise, land where
   it is positive; the domain's edge stays sea, since it is the outlet.
2. **Uplift**: zero at the coast, rising inland as the square root of the shape, times ridged
   noise (a 3 km period, four octaves) so the mountains have crests; 4 m per step at the heart.
   Hardness, a factor on the erodibility, in 1.8 km patches. Rain flat by default; with a
   wind (`IslandParams::wind`, `island::orographic_rain`), moisture rides the wind's lines from
   the upwind edge, fills up over the sea (5 km to saturate), rains out as the ground rises
   under it (400 m of climb empties it) and a little on every flat kilometre, so the windward
   slopes are wet and the lee dry; the raw rain is scaled to a mean of 1 over the land and
   pulled towards flat by the wind's contrast; it is refreshed from the relief every ten
   steps. The rain enters the erosion summed over the catchment (the discharge), which is the
   area when it is flat.
3. **Erosion** (`erosion::step`, 150 times): the uplift is added; the water is routed
   (`flow::drain`: D8 receivers on the raw field, Braun & Willett 2013; every pit's basin
   labelled, the lowest pass between adjacent basins, the spanning tree of the passes from the
   sea, and each pit's path to its pass reversed so its water leaves through it, the basin
   graph of Cordonnier, Bovy & Braun 2019 in carve mode; then the downstream-first stack and
   integer areas); each land cell then lowers towards its receiver by `f / (1 + f)` of the
   difference with `f = K · √(A · rain) / Δx` (the implicit stream-power update with `n = 1`,
   `m = 0.5`, unconditionally stable), the receiver already at its new height; an explicit
   diffusion sweep smooths the hillslopes. The height keeps its depressions: a cell below its
   receiver rises towards it by the same rule, which is sediment settling in a lake, so lakes
   appear in the uplifted basins and slowly fill, from the carved outlet path outwards.
4. **Hydrology**: rivers where more than 0.5 km² drains through a sample, traced as
   polylines (`hydrology::trace_rivers`): from every mouth the trunk follows the largest
   tributary upstream to its head, every other river donor met on the way starts a tributary,
   so each river runs from a head to the sea or to its junction with a larger one; Strahler
   orders bottom-up, widths `w = 0.005 √A` (Leopold & Maddock's exponent: 5 m at a square
   kilometre of catchment, 14 m at the island's largest). Lakes (`hydrology::trace_lakes`)
   where a final priority flood (Barnes 2014, once) stands more than 0.5 m over the eroded
   field: each 4-connected patch with the flood's level there, its deepest point and its
   outlet (the lake cell draining out lowest), which is what the water plan's lake planes
   need (polygon, level, outlet).

| Run (seed 7, 150 steps) | Samples | Erosion | Per step | Was (flood, one thread) | Peaks | River samples | Lake samples |
|---|---|---|---|---|---|---|---|
| `--spacing 16` | 1025² | 3.1 s | 0.021 s | 0.13 s | 530 m | 3 657 | 4 387 |
| `--spacing 8` | 2049² | 13 s | 0.087 s | 0.66 s | — | 7 336 | 62 819 |
| `--spacing 4` (the target) | 4097² | 50 s | 0.331 s | 3.13 s | 545 m | 14 809 | 181 377 |

In the cloud container, four cores (the owner's 9800X3D has eight, faster); the 4 m run's
stages 1–2 take 5.4 s, the hydrology 3.0 s, the previews 1.1 s, the whole run one minute. The
8 m row is a 20-step timing run (its counts are after 20 steps).

**Where the time goes** (#97). Before, the priority flood was 89 % of a step: a heap over
every cell, `n log n` with a large constant. Three changes, each measured at 4 m:

1. *The basin graph* (`flow::drain`): linear work on the raw D8 receivers, the depressions
   alone cost anything (523 pits at 8 m; the passes are kept per row, the lowest per basin
   pair, then sorted once); the D8 receivers, the passes, the uplift, the diffusion and the
   implicit update (the stack's segments, whole drainage trees each) on `forge-task`: 3.13 s
   to 0.85 s a step.
2. *The stack in parallel* (`Drainage::build`): the donor lists counted and filled by row
   bands (a receiver is a neighbour, so a band writes one row past itself; the even bands run
   together, then the odd ones, and the lists come out in the sequential order), a parallel
   prefix sum, the trees below each band's outlets walked into a part of their own, the parts
   concatenated with each cell's position stored through atomics, the areas per segment:
   0.85 s to 0.73 s.
3. *Buffers kept across steps* (`Drainage`, `Erosion`): a step touches a dozen arrays of
   67 MB at 4097²; paging fresh ones in each step cost more than the work on them (the
   position copy alone was 53 ms, the area gather 50 ms, the fill copy 54 ms): 0.73 s to
   0.33 s.

A step at 4 m is now `uplift 0.004, drain 0.270, incise 0.042, diffuse 0.014` seconds. What
remains sequential in the drain: labelling the basins (a walk to each cell's root, about
0.1 s), the pass sort and Kruskal (about 0.05 s), the prefix over the parts; the rest is
parallel but memory-bound on four cores. `--threads 0` at 16 m gives 0.055 s a step against
0.021 with four workers. The result is the same bytes with any thread count and with kept or
fresh buffers (tests run the drainage and the erosion with none and with three workers, and
reuse a drainage across two fields). The carve changes the field slightly against the flood's
routing (water leaves a lake by one path rather than over the whole flooded flat), so the
counts above differ from the first runs' (3 487 river and 2 774 lake samples at 16 m); the
pictures below are from the new field. At 4 m the lakes cover 3.5 % of the land samples
(181 k of 5.2 M) against 1.4 % at 16 m: the finer grid holds more small depressions, which the
sediment rule fills more slowly; a lake area limit, or the basin graph's fill mode with a
spill rule, is the part of #97 that remains.

**The network** (seed 7, 150 steps): at 16 m, 43 rivers, 27 of them to the sea, 68 km in all,
the longest 6.3 km, orders up to 3; at 4 m, 42 rivers, 24 to the sea, 82 km, the longest
7.5 km, orders up to 2 (the same square kilometres of catchment are more cells, and the finer
network branches differently), the widest 14 m at both; the tracing takes 0.7 s at 4 m. These
polylines are what the water research (`docs/research/water.md`, item 3 of its
recommendation) turns into river ribbons with flow maps (`network.png` below draws them by
order, first-order streams pale, the trunks deep). The lakes: 11 at 16 m, the largest
41.5 ha, the deepest 19.7 m; 2 614 at 4 m, the largest 52.2 ha, the deepest 30.9 m (the finer grid's many small depressions, #97's
open lake rule).

**The wind** (`--wind-from w`, seed 7 at 16 m): at contrast 1 the windward half of the land
gets a rain of 1.70 and the lee 0.21 (cells from 0.05 to the clamp at 10); the west coast is
cut by dense valleys and the east stays smooth (the picture below); the dry lee keeps its
depressions, 39 lakes against the calm island's 11, the largest 32 ha. At contrast 0.5:
1.34 against 0.61, 20 lakes. The refreshes cost 0.2 s over the run. Which contrast looks right
is the owner's call on the GPU (`city-blocks --island 7 --island-wind w`, another cache
key); the calm island is unchanged (its digest is the same).

![The island with a west wind, at 16 m: the windward coast dissected, the lee smooth](images/island-hillshade-16m-west-wind.png)

**The water's fields** (`genesis`, its `stage 5` line; `coast.png`, `sea-height.png`,
`sea-hillshade.png`). The signed coast distance (`coast_distance`: an exact Euclidean distance
transform, rows then columns in parallel; positive inland, negative at sea, zero on the coast
line) is what the shore's waves, foam line and wet band key on in the water research's plan;
seed 7's island reaches 4.6 km inland at most; 0.04 s at 16 m, about a second at 4 m. The sea
(`Ocean::new`, `surface(time)`): a JONSWAP spectrum for a 12 m/s wind over 200 km of fetch,
the TMA factor for a 50 m shelf, Hasselmann's spreading with Horvath's swell term (0.3),
Gaussian amplitudes from the seed on every wave vector of a 256 m patch at 256² (waves shorter
than 2 m left to the next cascade), the inverse FFT with `dmath` twiddles: a significant wave
height of 3.36 m (`Hs = 4 √m0`, and the tile's variance is checked against it), the tile from
−2.72 to 2.62 m, horizontal displacements up to 2.53 m (choppiness 1), no folding; the eight
transforms of a surface take 18 ms on one core, which is
the CPU side D-009 needs (the lowest cascade re-run for the physics) and the reference the GPU
cascades will be diffed against. What the pictures show: a sea of 30–60 m waves running with
the wind, crests broken by the spreading; nothing of it is drawn in the engine yet (the surface
pass is the water plan's first item on a GPU; its place in the frame and the cascades' queue are
proposed as D-038 🟡).

![The network at 16 m: rivers by Strahler order over the hillshade, the lakes flat](images/island-network-16m.png)

**Digests** (D-016). `genesis` ends with a 64-bit FNV-1a of the field's bits
(`Field2::digest`), the same on every machine and with any thread count; seed 7 after 150
steps: `0189d031eff0fb84` at 16 m (the same with `--threads 0`), `9eacfe0f827fa7dd` at 4 m,
both from the cloud container. A different value on the owner's machine is a D-016 bug to
find before the planet's tiles depend on it. *On the owner's machine (2026-09-26, Ryzen 7
9800X3D, 16 workers): the same two digests.* There the 4 m erosion takes 19.2 s (0.128 s a
step: drain 0.106, incise 0.015, diffuse 0.005, uplift 0.003) and the whole 4 m run 23 s; the
16 m run takes 1.4 s.

![The 16 km island at 16 m after 150 steps: the sea, hypsometric tints under a hillshade, rivers above 0.5 km² of catchment, lakes](images/island-overview-16m.png)

![Its hillshade: ridges, valleys, the drainage cut to the coast](images/island-hillshade-16m.png)

**What the pictures say, and what is next.** The drainage is dendritic and reaches the coast
everywhere; the crests follow the ridged uplift; the highland basins hold lakes. Visible now:
the D8 network's straight runs on gentle slopes (D∞ for the moisture, and the amplification,
soften them), a coast without cliffs or beaches (the materials and the near-field detail come
with stages 5–6), and highlands more uniform than a real range (orographic rain and a hardness
with layers are the levers). The immediate next step is stage 7 with what exists: the
heightfield handed to the cluster-DAG cook the city's ground uses, so today's renderer draws
the island with TAA and the F1 overlay, and the look is judged in the engine, not on a map.

## In the engine, first seen on the 5070 Ti (2026-09-26)

`cargo run --release -p city-blocks -- --island 7` works on the owner's machine: the first
start generates the 8 m field in 5.2 s and cooks it in 16.2 s (8.39 M triangles, 195 568
clusters, 1 922 pages of 240 MB, streamed); the next ones load both. The GPU takes 0.72–0.78 ms
a frame at 1600 × 900 (the probes' rays 0.16–0.24 ms, the layered shading 0.12–0.14), under
the city's sky with TAA.

![The island from the default view, over the sea to the south: the relief reads, but the sea is a green plain, the rock white, and the slopes carry dark patches](images/island-engine-first.png)

![From 1 200 m: the dark polygons across the slopes are shadow rays that hit the traced surface; with `--no-shadows` they are gone](images/island-engine-shadows.png)

What the first look shows, for #96 to fix before the props:
- **Shadows where there are none** (fixed the same day). Dark, sharp-edged polygons covered the
  slopes facing the sun; `--no-shadows` removed them all. The shadow structure is a cut of the
  cluster DAG capped at 600 000 triangles for a terrain (D-029), fine for the city's flat
  ground (an error of 0.02 m), but the island's 8.4 M triangles of relief cut to 600 000 have
  an error of 1.03 m, far beyond the rays' 0.15 m start. A terrain's shadow rays now start
  twice its cut's error off (`raytrace::TERRAIN_SHADOW_START`; at once the error, 6 100 false
  pixels were left in the view from 1 200 m): that view is now within 583 px of the one
  without shadows, and the batch is unchanged (0 px; the props and rocks keep 0.15 m, since a
  larger start moved the rocks' own shadows in the ballad).
- **No sea** (a stand-in the same day). The sea floor was the field's 0 m, drawn as grass to the
  domain's edge.
- **Rock that reads as snow** (fixed the same day). Above 380 m and on slopes over 0.45 the
  ground took the city's rock layer, a pale grey that reads white from afar, with
  stair-stepped edges where the layer map's 4 m texels changed.
- **A camera to place with care.** `--view` takes an absolute height, and the land rises past
  500 m: a view at 250 m, 2.5 km in from the south coast, is under the ground.

**The island's own ground (2026-09-26).** The island no longer borrows the city's ground rows:
- `CityMaterials::island_ground` gives it four rows after its layered row: a deeper green, sand,
  the sea and a dark volcanic rock.
- `forge_procgen::slope_layers` takes a `Shore`: the sea at and below 0 m, sand on gentle ground
  up to 2.5 m, rock on slopes over 0.45 (no longer above an altitude: a tropical island is green
  to its peaks), grass elsewhere.
- The slope is Horn's gradient interpolated between the samples, rather than the nearest
  sample's, so a layer's border no longer steps with the 8 m grid. The 4096² map takes 190 ms.
- The rivers of stage 4 are painted over it: the drawn field's drainage (`forge_procgen::drain`),
  the rivers above 0.5 km² of catchment (`trace_rivers`, as `genesis` traces them), and every
  texel within half a river's width of its course (`paint_rivers`). The width comes from the
  catchment, 8 m at least, so a stream narrower than a texel still draws a steady line. They
  are a fifth layer, water over a dark bed, shaded like the sea. At 8 m that makes 43 rivers and
  45 121 texels, in 52 ms.
- So are the lakes of a hectare or more (`trace_lakes` over a priority flood of the drawn field,
  as `genesis` traces them; `paint_lakes`), on the same layer. They cover the depression's floor
  rather than standing flat at their level: a stand-in, like the sea. At 8 m that makes 31
  lakes and 153 040 texels. With the rivers it takes 0.5 s at the start, most of it the flood.

![From 1 500 m: the rivers wind down the valleys to the coast, past the highland lakes](images/island-engine-rivers.png)

The sea is a **stand-in** until the water pass (D-038 🟡): one opaque plane at 0 m, 262 km across
(`sea_prop`), shaded smooth and dark (reflectance 0.02, a Blinn-Phong power of 400). It is smooth
enough for the traced mirror rays (D-031), so it reflects the island and the sky. It reaches the
horizon, where the field alone stopped 8 km out and showed the atmosphere's brown ground.

Under it the field now has a **sea floor** (`forge_procgen::sea_floor`). The erosion leaves the
sea flat at its base level, 0 m. Each sea sample is lowered to 60 m × (1 − e^(d / 1 500 m)), with
`d` its signed distance to the coast: 4 % of slope at the shore, levelling off at 60 m. This is
the floor depth the water research asks the genesis for. It is applied to the drawn field after
the erosion, so the genesis digests do not change. The coast is where the plane meets the ground,
between the samples. With the flat sea it ran along the 8 m grid's edges, a sawtooth at close
range. The layer below 0 m is a wet sand (`island_layer::SEABED`), out of sight under the plane.

**Rocks** (`placement::RockRule::Land`): the GPU placement puts 300 000 of the city's boulders and
rubble (`--instances`) on the island's land above 3 m. Each slot tries up to 32 candidates over
the square, keeping one on land with a chance that rises with the slope: 15 % on the flat, all
of them from a slope of 0.6. They are shaded in the island's dark rock. A slot that finds no land
lies 50 m under the ground. The city's placement is unchanged: its checksum is still
`4e10743a3499dc0e`, and the batch is 0 px. (A first version wrote the city's rocks' height as
`ground − (0.15 r + 0)`, which the compiler no longer fused into one FMA, and 21 pixels of the
city's orbit moved; the expression is the city's again.)

**The first view** is now on the south coast, as #96's plan asks: 25 m over the water, 150 m off
the beach due south of the centre, looking inland. The beach is found in the field (`island_camera`),
so the view holds for any seed. The whole island from the sea is `--view 0,300,6800,0,-0.08`.

Two more changes:
- The island's sun stands at 30° by default (the city keeps 63.4°): from the default view a high
  sun lit the slopes head-on and flattened them.
- The island's prop is named `island`: as `terrain` it shared the city's cache file, and each
  evicted the other (an 11–16 s re-cook at every switch).

GPU, at 1600 × 900:
- **1.36 ms** from the coast, where the probes' rays take 0.43 ms among the rocks, the plane's
  shading 0.19 and its mirror rays 0.13;
- **1.12 ms** for the whole island from the sea, where the distant rocks go to the software
  rasteriser (0.20 ms);
- 0.74 ms before the rocks and the plane.

The batch is unchanged (26 images at 0 px).

**At 4 m** (`--island-spacing 4`, the 4097² target), the first start takes:
- 25 s to generate the field (a 67 MB `.f32`);
- 72 s to cook it into 33.5 M triangles, 784 586 clusters in 16 levels (a 1.10 GB `.fmesh`).

Its 7 721 pages (965 MB) stream through the default 512 MB pool. The shadow structure's cut
stands 2.4 m off (rays start 4.8 m off). The GPU takes 0.84 ms from the default view, against
0.74 at 8 m: cluster cull 1 grows 0.06 → 0.18 ms, and the probes' blend 0.07 → 0.15. The cache
keeps one file per prop, so switching between 8 and 4 m re-cooks.

Much of the 4 m island's lower flanks is rock, where at 8 m it was grass. This is the field,
not the rule: the erosion at 4 m cuts the flanks above 0.45 over 8 m. The rule now measures the
slope over 8 m at any spacing (`LayerRule::slope_over`), which changed 0.3 % of the 4 m frame's
pixels and nothing at 8 m. Whether the 4 m flanks should be that steep is a question for the
erosion's parameters at 4 m (#97), and a look to judge.

![The first view, on the south coast: rocks on the grass and on the steep ground, the beach, the island mirrored in the sea's stand-in](images/island-engine-coast.png)

![The whole island from the sea, 300 m up: its own ground, sand along the shore, rock on the steep slopes, the reflection, a 30° sun](images/island-engine-sea.png)

![From the west, 400 m up: the sea's plane reaches the horizon](images/island-engine-west.png)

Without the traced rays (`--no-reflections`, or a GPU without ray queries), a line runs across
the sea where the probes' last cascade ends, about 900 m out. #68 dims an untraced sky
reflection by what the probes see towards the mirror direction, and their rays, which shade
diffusely, see the sea as nearly black (#100). The traced mirror rays skip that dimming, so the
default frames show no line.
