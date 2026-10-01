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
| Hydrology: rivers as polylines with Strahler orders and widths, lakes with levels and outlets, the depressions under 5 ha filled (stage 4, the lake rule of #97) | ✅ `forge_procgen::hydrology` |
| The water's fields: the signed coast distance; the sea's directional spectrum (JONSWAP/TMA, Horvath's spreading) synthesised by an inverse FFT on the CPU into a tiling patch of heights, displacements, slopes and the Jacobian | ✅ `forge_procgen::coast`, `forge_procgen::ocean`; the GPU's three cascades (#105, `forge_render::water`) agree with it within 3 × 10⁻⁶ ("The sea on the GPU" below); the surface drawn from them with `--water`, reflecting the island through traced mirror rays |
| Amplification to 2 m per tile with halos (stage 5) | started on the CPU: ×2 with a detail erosion, tiles with halos equal to the untiled field (`forge_procgen::amplify`, `genesis --amplify`; "Amplification" below); drawing at 2 m in tiles planned |
| Materials from the fields, the layer map (stage 6) | started: sea floor, sand, grass and rock from the height and the slope, dry and lush grass by the wetness index, the rivers and lakes painted in (`forge_procgen::slope_layers`, `paint_rivers`, `paint_lakes`; "In the engine" below); moisture, soil and the rivers' banks planned |
| The hand-off to the cluster-DAG cook: the island drawn by today's renderer (stage 7) | ✅ drawn on the 5070 Ti (2026-09-26): `city-blocks --island SEED`, with its own ground, a sea floor, rocks and a stand-in sea ("In the engine" below, #96) |
| The planet: the same stages on the cube sphere's coarse graph, tiles amplified at streaming time | planned |

```
cargo run --release -p genesis -- --spacing 16 --steps 150 --out captures/island
cargo run --release -p city-blocks -- --island 7
```

`genesis`: `--seed N`, `--spacing M` (16: 1025² samples; 4: the 4097² target), `--steps N`,
`--k` (erodibility), `--diffusion`, `--channel-ha H` (the catchment from which a channel
carries the hillslopes' material away; 0, the default, lets the diffusion raise every cell,
#109), `--uplift` (metres per step at the heart), `--every N` (a
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
   diffusion sweep smooths the hillslopes. The channels can carry away what the hillslopes
   shed into them (`ErosionParams::channel_area`, `genesis --channel-ha`, `city-blocks
   --island-channel-ha`; 0 and off for now, see "The rivers' grading" below): the sweep then
   never raises a cell draining that much, and raises a smaller channel's by the share of its
   catchment short of it. The height keeps its depressions: a cell below its
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
sediment rule fills more slowly. The lake rule (below, 2026-09-30) settles it with an area
limit: 2.7 % at 4 m after it.

**The network** (seed 7, 150 steps): at 16 m, 43 rivers, 27 of them to the sea, 68 km in all,
the longest 6.3 km, orders up to 3; at 4 m, 42 rivers, 24 to the sea, 82 km, the longest
7.5 km, orders up to 2 (the same square kilometres of catchment are more cells, and the finer
network branches differently), the widest 14 m at both; the tracing takes 0.7 s at 4 m. These
polylines are what the water research (`docs/research/water.md`, item 3 of its
recommendation) turns into river ribbons with flow maps (`network.png` below draws them by
order, first-order streams pale, the trunks deep). The lakes: 11 at 16 m, the largest
41.5 ha, the deepest 19.7 m; 2 614 at 4 m, the largest 52.2 ha, the deepest 30.9 m (the finer
grid's many small depressions). The lake rule below keeps 7 lakes at 16 m, 14 at 8 m and 15 at
4 m.

**The lake rule** (#97, 2026-09-30; `hydrology::fill_small_depressions`,
`IslandParams::lake_min_area_m2`, `genesis --lake-min-ha`). After the erosion, every
depression of the flood under 5 ha fills with sediment to its spill level, and the larger ones
stay as the lakes. A depression is a 4-connected patch where the priority flood stands over the
field. Its cells take the flood's height, which keeps the flood's ε rise, so water still
crosses the filled floor.
- **Why an area:** small closed hollows silt up, and an area is the same limit at every
  spacing. The finer grid's erosion leaves many more small hollows, which the sediment rule
  fills slowly: 2 614 lakes at 4 m against 11 at 16 m.
- **Why 5 ha:** at 1 ha the depressions kept were 11 at 16 m, 32 at 8 m and 42 at 4 m. At 5 ha
  they are 7, 14 and 15. The two spacings the engine draws keep the same lakes, and their
  water covers 2.11 and 2.22 km² (2.90 at 4 m before the rule). The 16 m field is a different
  erosion and keeps fewer.
- **The fill:** 5 413 hollows and 56 485 samples at 4 m in 1.3 s, 828 at 8 m, 20 at 16 m.
- **The counts:** `genesis`'s lakes line still counts the patches deeper than half a metre
  (496 at 4 m, 75 at 8 m): one lake with shallows splits into several there.
- **In the engine:** the 8 m island paints 15 lakes of a hectare or more over 128 459 texels,
  against 31 and 153 040 before.

![The 4 m island without the lake rule, and the 8 m and 4 m islands with it: the same lakes at both spacings](images/island-lake-rule.png)

**The rivers' grading** (#109, 2026-10-01; `ErosionParams::channel_area`, `genesis
--channel-ha`, `city-blocks --island-channel-ha`). Every river of the 8 m island reaches the
sea steeply: of its 25 mouths, all fall over 5 % across their last 160 m and 21 over 10 %, a
14 m river at 15 % and the 4 m ones at 15–24 % (the demo's line `the rivers' last 160 m to
the sea`). The fall hardly depends on the river's size, so it is not the stream power's
profile, whose slope falls with the catchment. It is the hillslope diffusion: on 8 m cells a
valley floor is one cell wide between walls at 40° or more, and the sweep pours both walls
into that cell each step, about a metre, sixteen times what it poured at the 32 m spacing the
parameters were tuned at; the river re-cuts it, and the balance is a slope of about the fill
per cell whatever the river. Real channels carry that material away, and `channel_area` lets
them: the sweep never raises a cell draining that much, and raises a smaller channel's by the
share of its catchment short of it (a 3 × 3 unit test in `erosion`). At 25 ha (half the
drawn rivers' catchment) every mouth falls 2–5 %, one over 5 %, none over 10 %. But two things
go with the fill:
- **The lakes.** The island's 14 lakes were dams of the same fill at the valleys' narrows:
  at 25 ha there are none, at 100 ha 3 (and the 4 m rivers are back at 7–12 %), at 400 ha 5
  (14 mouths over 10 %).
- **The valley floors.** The fill is, in effect, the island's alluvium: 150 steps of it lift
  the valley floors by up to 150 m over the stream power's profile (the same camera stands 3 m
  over a river on a plain before, and 150 m over a canyon after). Without it a floor is a slot
  a cell wide. The rivers' smoothed courses leave it at the D8 corners and land on the walls:
  the water stands up to 12.4 m under its banks (3.9 before) at a third of the points (6 314
  over 2 m, 621 before), which the carve cuts as gorges.
So the rule stays off (`channel_area` 0). D-040 proposed transporting the sediment down the
channels instead; `city-blocks --island 7 --island-channel-ha 25` shows the island with the
rule, and `reports/2026-10-01-109/grading.md` the sheets. The coastal plain below grades the
mouths without either: the fill is as large as the valley walls are steep, and the hills rose
straight out of the sea.

**The coastal plain** (D-041, 2026-10-01; `IslandParams::plain`, `plain_uplift`,
`plain_wander`; `genesis --plain`, `city-blocks --island-plain`, 0 for the island before).
The uplift's square root lifted the coast's foothills from the shoreline, so the steepest
valley walls, and the most fill, were at the mouths: the largest river's floor stood at
24.7 m only 160 m inland. Now the uplift stays at 3 % of the hills' starting rate over the
first quarter of the radius inland, that width wandering along the coast over its 5 km
scale (`plain_wander` 3: from none, where the hills still meet the sea as cliffs, to about
twice as wide), and the hills rise from the plain's inner edge on the square root eased in
over its first eighth, so they leave the plain on a slope rather than a wall.
- **At 8 m** (the engine's island): 19 mouths, every one falling 1–5 % over its last 160 m
  (25 mouths, all over 5 %, before); 50 rivers up to 17 m wide (43 up to 14); 11 lakes (14);
  the water at most 3.5 m under its banks, 125 points over 2 m (3.9 m and 621); the heights
  −60 to 525 m. The first view now finds its beach at z = 5 072 m.
- **At 16 m** (`genesis`): 46 rivers, 18 to the sea (27 before), up to 17 m wide, 3 lakes
  (7). `--plain 0` gives the island before, the same digest (2d17199dba8598bf).
- The rivers cross the plain winding to the sea, more of them join before the coast, and the
  mouths are calm; the massif keeps its torrents in V valleys. The sand rule (gentle ground
  under 2.5 m) now reaches further inland around the mouths.

![Seed 7 at 16 m without the plain (left) and with it (right): the plain wanders from cliffs to a wide lowland, and the rivers join across it](images/island-coastal-plain.png)

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
D-038 ✅, accepted 2026-09-30).

![The network at 16 m: rivers by Strahler order over the hillshade, the lakes flat](images/island-network-16m.png)

**Digests** (D-016). `genesis` ends with a 64-bit FNV-1a of the field's bits
(`Field2::digest`), the same on every machine and with any thread count; seed 7 after 150
steps: `0189d031eff0fb84` at 16 m (the same with `--threads 0`), `9eacfe0f827fa7dd` at 4 m,
both from the cloud container. A different value on the owner's machine is a D-016 bug to
find before the planet's tiles depend on it. *On the owner's machine (2026-09-26, Ryzen 7
9800X3D, 16 workers): the same two digests.* Those are the eroded field's, which
`--lake-min-ha 0` still gives. With the lake rule (2026-09-30, below) the island is
`2d17199dba8598bf` at 16 m, `9b712f95a51a2ea1` at 8 m and `9e1b2858f066b672` at 4 m. There the 4 m erosion takes 19.2 s (0.128 s a
step: drain 0.106, incise 0.015, diffuse 0.005, uplift 0.003) and the whole 4 m run 23 s; the
16 m run takes 1.4 s.

**Amplification** (stage 5, 2026-09-30; `forge_procgen::amplify`, `genesis --amplify`). The
eroded field goes to half its spacing, after Schott et al. 2024 (each finer level erodes under
the drainage the coarser one fixed):
1. The field is filtered by [1, 2, 1] / 4, then upsampled ×2 by Catmull-Rom. The eroded field
   carries a faint checkerboard at its sample scale, which Horn's gradient ignores but the cubic
   turned into a hatching over the whole hillshade; the filter removes it.
2. Half a metre of fractal detail goes on the land, its largest features 60 m across.
3. Twenty explicit iterations of three operators, each reading the previous iteration's field:
   - incision along the fine grid's own steepest descent, by the stream-power law with the
     coarse drainage's catchment, filtered twice (0.0006 · A^0.5 · S; stronger, it carved the
     coarse D8's straight runs into canals);
   - talus towards 0.9 (42°), symmetric between neighbours;
   - a little linear diffusion.

Every operator reads only a sample's eight neighbours, so the field is worked in tiles of 512
samples with a halo of 22. The tiles give the untiled field to the bit, with any number of
workers (a test): the planet's tiles can be amplified alone at streaming time and still agree
along their borders.

From 4 m to 2 m (8 193² samples) takes 1.5 s on the 9800X3D; from 8 m to 4 m, 0.4 s. Seed 7's
digests: `05a922f2380e03c5` at 2 m, `ce7d9331b56187e3` at 4 m from 8 m (with the lake rule;
`ede478ecd0115cff` and `96fc0548933e001f` before it). The island in the
engine still draws the 8 m or 4 m field: drawing 2 m needs the cook in tiles with locked
borders (134 M triangles, the next step).
The cook already locks them. Its simplifier runs with `SimplifyOptions::LockBorder`
(`forge-geom`, `lod.rs`), which keeps every vertex on a mesh's open edge in place at every
level. Tiles cooked alone therefore keep the same vertices along a shared edge and meet without
cracks at any pair of levels. The cost is that the edges never coarsen, which the planet
research's skirts avoid (`docs/research/planet-terrain.md`).

![A 2 km window of the island at 2 m, upsampled alone: smooth, the 4 m field's valleys](images/island-upsampled-2m.png)

![The same window amplified: the valleys deepened by the detail erosion, the slopes rougher](images/island-amplified-2m.png)

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
- Before those, the grass takes its moisture. The measure is the topographic wetness index,
  ln(a / tan β) (Beven & Kirkby 1979; `forge_procgen::wetness`): the catchment per metre of
  contour over the slope, box-blurred over 32 m so that D8's one-sample channels read as
  valleys. The driest quarter of the grass texels becomes dry grass on the ridges, the wettest
  quarter lush grass in the valley bottoms (`paint_moisture`), which brings out the relief from
  afar. That's 1.2 M texels each way; the whole step now takes 0.6 s at the start.
- Every textured row of the island takes hex tiling (#66). Near the coast the rock's 6 m repeat
  showed as a grid on the steep slopes. The layered shading grows from 0.118 to 0.154 ms, and
  the first view from 1.34 to 1.39 ms.

To place a view on the island, the log's line `island first view` gives the first view's
position and the beach it found (for seed 7 with the coastal plain: the beach at z = 5 072 m,
the camera at 0,25,5 222; 5 080 and 5 230 before). `--view` takes x, y and z in metres, then
the yaw and the pitch in degrees; the height is absolute, so a view inland must clear the
ground (up to 525 m).

![From 1 500 m: the rivers wind down the valleys to the coast, past the highland lakes](images/island-engine-rivers.png)

The sea is a **stand-in** until the water pass (D-038 ✅): one opaque plane at 0 m, 262 km across
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

## The sea on the GPU (issue #105, 2026-09-30)

D-038 was accepted on 2026-09-30. Its first step puts the sea's waves on the GPU:
`forge_render::water` and `shaders/water.slang`. The island draws its sea from them ("The
surface" below), its rivers and its lakes: by default since 2026-10-01, the owner's call after
the night's review (`--water` was the opt-in before; it is still accepted). `--no-water` draws
the stand-in sea, and the rivers and lakes painted into the ground's layers.

- **Three cascades** of 256² samples (`OceanParams::cascades`): patches of 1 024, 128 and 16 m.
  - Each holds the waves the one before is too coarse for. The bands meet at 32 m and 4 m
    (eight samples a wave at the coarser cascade), and the shortest wave is 0.25 m.
  - The breeze of `genesis`'s stage 5 (12 m/s, 200 km of fetch, a 50 m shelf) gives a
    significant height of 3.4 m over the three.
- **Four passes each frame on the async compute queue:**
  - `water/evolve`: the spectrum at the frame's time, and the spectra of eight fields packed
    two to a complex value. Their spectra are Hermitian, so the transform of A + iB is a + ib.
  - `water/fft-rows` and `water/fft-cols`: the inverse transform, radix-2 Stockham in 16 KB of
    groupshared memory, a workgroup per line.
  - `water/derive`: two half-float images per cascade. One holds the displacement (x, height,
    z); the other the slopes and the Jacobian, which marks the whitecaps.
- **The spectrum is the CPU's:** `Ocean::gpu_samples`, uploaded once. The GPU transforms the
  CPU's amplitudes, and its surface is `Ocean::surface` in single precision.
  - At the four wave vectors that are their own opposite (0 and N/2 on each axis), the evolve
    pass keeps the real part of each field's coefficient, as the CPU's transform does.
- **The start-up check:** at frame 120 the island reads every cascade back and compares its
  six fields with `Ocean::surface` at the same time. It passes when each field's largest
  difference is under 10⁻³ of its largest value, the half floats' precision. On the 5070 Ti the
  largest is 2.9 × 10⁻⁶ (the 1 024 m cascade's height: 1.8 µm of 2.94 m).
- **The cost:** 0.07 ms of the frame at 1600 × 900 and 0.15 ms at 1440p (`docs/PROFILE.md`),
  and 13 MiB with the mips. They run with the water only (not with `--no-water`).
- **Checks:** the batch is unchanged, and `tools/validate.sh` is clean, synchronization
  included.

**The surface** (step 2, `--water`). The sea is drawn by two passes after the sky's compose:
`water/scene-copy` (the HDR image and its depth, for what the water lets through), then
`water/surface`.
- **The mesh:** a clipmap of 13 levels of 128 × 128 quads, the finest 0.5 m apart and 64 m
  across, the coarsest 262 km across (Losasso & Hoppe 2004).
  - Each level is centred on the camera snapped to twice its spacing, and leaves out what the
    finer level covers.
  - Each level's lattice points are its vertices, indexed by its quads (since 2026-10-01): five
    sets of indices, the whole level for the finest, and a level less its finer level's hole
    in each of the two places it can sit along each axis. The quads go in blocks of 32 × 32,
    and a frame draws only the blocks whose box (40 m more every way, for the waves) can
    show.
  - Near its edge a level's odd vertices slide onto the coarser lattice, all the way by the
    edge, so the levels meet without cracks and nothing pops.
  - The cascades displace the vertices, each read at the mip that matches the spacing
    (`water/mips` builds the chains).
- **The shading:**
  - The normal and the roughness come from the slopes' mips: the mean slope, and the variance
    of those the mips average away, as GGX's α² (Bruneton, Neyret & Holzschuch 2010). Far away
    the sea turns rough instead of shimmering.
  - Schlick's Fresnel (F0 0.02) with the sky in the mirror direction, and the sun's GGX
    highlight.
  - Under the water, the copied scene is dimmed along the light's path to it (absorption
    0.35, 0.07, 0.05 m⁻¹), with the light the water scatters back. The shallows show the sand,
    and the deep sea turns blue. The path is the depth under the surface over the refracted
    ray's fall (Snell's law, n = 1.333; since 2026-10-01, `reports/2026-10-01-105/refraction.md`):
    at a grazing angle about 1.5 times the depth, where the straight view ray's was ten times,
    and every wave changed it by metres and speckled the shallows.
  - Foam where the Jacobian drops under 0.45, then the aerial perspective.
- **The graph:** the surface writes depth, so TAA's motion vectors and the culling harness see
  it as a surface. `--sea-time T` holds the waves still.

![The coast view and the view from the sea: the stand-in on the left, the water on the right](../../reports/2026-09-30-105/sheet.png)

- **What it lacked:** the island's reflection (step 3, below), the sun's shadow on the water
  (step 4, below), and the shore's waves and foam line. That is why `--water` stayed opt-in
  until the owner had judged it (`reports/2026-09-30-105/`; the default since 2026-10-01).
- **Stability** (still camera, waves still, TAA on): pixels changing from one frame to the
  next, 0.43 % at the coast (the stand-in 0.37 %) and 0.57 % from the sea (0.34 %). Over 32
  frames at the same jitter phase, 0.0016 % and 0.0003 %: nothing crawls.
- **Cost:** 1.26 → 1.43 ms at the coast, 1.11 → 1.32 ms from the sea, 2.38 → 2.66 ms at 1440p
  (`docs/PROFILE.md`).
- **Checks:**
  - The batch gains the water's four captures: the occlusion A/B and mesh against fallback
    are at 0 px.
  - `tools/validate.sh` gains the water runs and is clean. It first showed the render graph
    naming graphics stages in a compute-queue barrier after a vertex shader read the mips;
    `forge-gpu` fixes that, with a test.

**The island in the water** (step 3). The surface asks for a mirror ray per pixel, and the
glass's ray tracing answers (#50, #52).
- **The request:** `water/surface` writes a second target beside its colour. It holds the
  mirror direction it read the sky in, and the weight of what a ray meets there instead: the
  Fresnel term, less the blur towards the sky's irradiance on a rough surface, the foam and the
  air in front. The target starts at zero, so a pixel without water asks nothing.
- **The rays:** `water/reflections` (`MeshletRenderer::trace_requested`), one thread a pixel.
  It rebuilds the water's point from the depth the surface wrote, traces the ray against the
  TLAS, and on a hit adds weight × (hit − sky). The hit is lit as the glass's are: the sun
  through a shadow ray, and the probes' light.
- **The keys:** **Y** (`--no-ray-reflections`) and **F** turn the rays off, as for the glass. A
  GPU without ray queries keeps the sky.

![The coast view and the view from the sea: the stand-in, the water with the sky alone, the water with the island traced](../../reports/2026-09-30-105/reflection-sheet.png)

- **From the sea:** a darker, greener band under the island, broken by the waves.
- **At the coast:** the wave faces turned towards the camera reflected the bright sky low over
  the horizon. They now show the grass slopes and the beach behind the shore, in olive and sand
  patches; the faces turned away still show the sky.
- **The sea floor stays out:** a false colour of each ray's hit distance shows the rays from the
  water near the camera reaching the slopes more than 100 m away. Next to none hit anything
  under 0.5 m above the sea, so the floor's traced surface, which may stand up to a metre above
  the drawn one, doesn't show through.
- **Stability** (still camera, waves still, TAA on): 1.62 % of the pixels change from one frame
  to the next at the coast (0.43 % with the sky alone) and 0.98 % from the sea (0.57 %). They
  are isolated pixels on the edges of the reflected slopes, where a wave face's ray flips
  between the island and the sky as the jitter moves; 0.21 % change by more than 8 levels at
  the coast. Over 32 frames at the same jitter phase, 0.0021 % and 0.0003 %: nothing crawls.
- **Cost:** `water/reflections` takes 0.17 ms at the coast, 0.09 ms from the sea and 0.43 ms at
  1440p (`docs/PROFILE.md`).

**The island's shadow on the water** (step 4). The surface asks for a shadow ray per pixel too,
and the same pass traces it.
- **The request:** a third target holds the share of the pixel's colour the sun lights, through
  the same foam and air. It counts the highlight and the sunlight the water scatters back:
  under a shaded surface the water loses the sun too. The sea floor seen through the water was
  shaded by its own rays in the resolve. A share under 1 % of the pixel asks no ray.
- **The ray:** towards a point of the sun's disc, as the ground's shadow rays, so TAA averages
  the eight points into a penumbra. A blocked ray takes the share away. It starts 2 m off the
  water, the terrain's own start (`TERRAIN_SHADOW_START`): the sea floor's traced surface may
  stand a metre above the drawn one, above the water in the shallows.
- **The key:** **J** (the shadows), as for everything else.

![The island from the north-west against a 10° sun: without the water's shadow and with it](../../reports/2026-09-30-105/shadow-sheet.png)

- **Where it shows:** under a low sun, on the side of the island away from it. At the default
  30° the island's slopes facing the sea are gentler than the sun, so they shade almost none
  of it: 54 pixels change in the view from the north-west. Under a 10° sun (`--sun-elevation
  10 --view=-5600,300,-4200,-126.9,-4.6`), the shadow cuts the sun's glitter short of the shore,
  and the shaded water keeps only the sky's blue.
- **Stability:** under the 10° sun, 1.42 % of the pixels change from one frame to the next
  (1.37 % without the shadow), and 0.017 % over 32 frames (0.021 %). The coast and the sea
  views don't change.
- **Cost:** the shadow rays add 0.03 ms to `water/reflections` at the coast and from the sea,
  0.07 ms at 1440p and 0.07 ms looking into the 10° sun, where they cross the island
  (`docs/PROFILE.md`).

**The shore** (step 5, D-038's shore). Its first part: the waves feel the floor.
- **The fields:** `water/surface` reads the island's floor height and signed coast distance
  (`WaterShore`: one `RG16F` image of the 8 m field, 16 MiB at 2 049²).
- **The damping:** each cascade is damped by the root of Kitaigorodskii's TMA factor (Bouws
  et al. 1985) at the frequency its energy centres on (`Ocean::mean_frequency`: periods of
  7.3, 3.5 and 1.2 s), in the local depth, relative to the 50 m its spectrum was made for. The
  displacement, the slopes, their variance and the Jacobian all follow. Long waves die first
  in the shallows, the open sea keeps its waves, and the factor falls to zero at the waterline
  (as the square root of the depth), which stands for D-038's shore fade.
- **What changes:** 150 m off the beach (6 m deep) the swell's cascade keeps 46 % of its
  height, the others 89 % and 100 %. The patches of sand that showed through near the shore,
  where the swell's troughs dipped under the floor, are gone.

![The coast view and a low view 80 m off the beach, before the damping (left) and with it (right)](../../reports/2026-09-30-105/shore-damping.png)

- **Cost:** `water/surface` 0.074 → 0.082 ms at the coast, 0.090 → 0.101 ms from the sea (a
  texture read per vertex and fragment).
- **Checks:** the batch changes only the four water images (ꟻLIP mean 0.036); the A/B harness
  and mesh against fallback at 0 px; validation clean.

Its second part: the shore's own waves (Uncharted 3's recipe, Gonzalez-Ochoa & Holder 2012).
- **The trains:** three Gerstner trains of 9, 7 and 12 s (0.9, 0.6 and 0.5 m high in deep
  water, around the swell's 7.3 s) come in along the coast distance's gradient.
  - Their phase is ω (t + τ), τ the time a crest takes from the point to the shore.
    `forge_procgen::shore` tabulates it against the coast distance, from the floor's mean depth
    at each distance and the dispersion relation (a crest takes 47 s from 200 m out). So the
    crests slow and bunch up over the shallows.
  - Their height grows by shoaling (the energy flux kept: `√(cg₀ / cg)`, Green's law in
    shallow water) until together they stand over 0.78 of the depth (McCowan's breaker
    index). Past that they follow the depth down: the surf zone, about 40 m wide on the
    south beach.
  - Along the coast the sets rise and fall and the crests drift a little out of line (two
    slow value noises), so the lines of surf are not ruled.
  - They fade in where the floor is under a fifth of their deep wavelength (the cascades fade
    out there), and out where they are shorter than four of the mesh's spacings or the
    pixel's footprints.
- **The foam:** the broken water on the front of each crest in the surf zone, left behind as
  the crest moves on. Also the swash's leading edge, where its sheet is under 3 cm thick (from
  the view ray's path to the floor). Two octaves of value noise that drift shorewards and fade
  to their mean below two pixels break up the broken water. They only fray the edge's line
  (40 % of its cover at most): cut into the noise's blobs, a band that narrow read as a dashed
  line.
- **The swash:** at the shoreline each wave runs up the beach as a thin sheet: a quick
  uprush, then a slow backwash, 0.3 of the trains' deep-water height at most.

![The south beach from 60 m up at 10, 12 and 14 s: the crests come in, break and leave their foam](../../reports/2026-09-30-105/shore-waves.png)

- **Stability** (still camera, waves still, TAA on), frame to frame and over 32 frames: the
  beach from 60 m up 0.019 % and 0.0002 %, the coast 1.46 % and 0.0019 %, from the sea 0.83 %
  and 0.0001 %. The damped shallows move less than before, so the coast and the sea are
  steadier than at step 4 (1.62 % and 0.98 %).
- **Cost:** `water/surface` 0.082 → 0.136 ms at the coast, 0.101 → 0.164 ms from the sea (the
  whole coast's shallows in view), 0.095 (before the damping) → 0.204 ms above the beach. The
  frame: 1.65 → 1.72, 1.45 → 1.52 and 1.64 → 1.76 ms against step 4. The surface was drawn
  without indices, each vertex six times: indexed and culled by blocks since (`docs/PROFILE.md`).
- **From low down** the surf is seen edge on and reads as a light band along the beach. The
  crest that curls over is D-038's later step for the golden shots (a baked mesh along the
  wavefront, as Horizon's).

Its third part: the wet sand.
- **Where it is wet:** the swash is a function of position and time, shared by the water and
  the ground (`shaders/shore.slang`), so the layered ground computes where the sheet ran up
  without a texture. It takes the highest reach over the last 19.5 s (14 instants 1.5 s
  apart), each dimmed by the drying since (25 s to lose 1/e), and stays damp for 0.25 m of
  height above it. The floor under the sea is left to the water, which shades what it covers.
- **What wet does** (Lagarde 2013): the albedo halves, and the surface turns smooth, with
  water's F0 (0.02) and a sharp sky reflection and sun highlight.
- **Against D-038:** the decision named a clip texture the water writes and the terrain
  reads. The same maximum run-up, decaying, comes from the functions themselves, with no pass,
  no clip edge to fade and no resolution limit, and it stays deterministic (D-016).
- **The resolve** reads the shore's block (`WaterSurface::wet_ground`, `AmbientLight::wet_ground`):
  the fields' frame, the trains and the sea's time.
- **What shows the island's grid:** the swash's edge and the wet band follow the beach's
  height contours, which step along the 8 m field's cells near sea level (the sand and grass
  boundary above them steps the same way). The 2 m amplification is what smooths them (#106).
  (Smoothed since, below: "The coast's definition".)

![The beach from above before the wet sand (left) and with it (right), at 14 s](../../reports/2026-09-30-105/shore-wet.png)

- **Cost:** `shading/layered` +0.012 ms at the coast, nothing measurable from the sea, +0.045 ms
  above the beach, where most pixels are within the swash's reach (`docs/PROFILE.md`).
- **Stability** (still camera, waves still): the beach from above 0.020 % frame to frame
  (0.016 % dry: the wet sand's sharper highlight), 0.0001 % over 32 frames either way; the
  coast unchanged.
- **Checks:** the batch changes the water images by the wet band (9 046 px, ꟻLIP mean 0.0031);
  the A/B harness and mesh against fallback at 0 px; validation clean.

**The rivers** (step 6, D-038's rivers). The rivers of stage 4 become water of their own:
`forge_procgen::river` makes each a ribbon, and `water/surface` draws the ribbons after the sea.
- **The courses:** the drawn field's rivers above 0.5 km² of catchment, as the layer map traced
  them (43 at 8 m), each from its head to the sea or to the larger river it joins.
  - Four passes of a binomial filter and three of Chaikin's corner cutting smooth the D8
    course, its ends kept, and it is resampled every 4 m: 16 397 points. The grid's stair steps
    straighten, and its right-angled turns open into bends.
  - Per point: the width from the catchment as before (`w = 0.005 √A`: 3.5 m at the
    threshold, 14 m at the largest mouth); the depth `0.3 (A / km²)^⅜` m (Leopold & Maddock's
    exponent, 0.23 to 0.65 m here); the speed by Chézy's formula, `15 √(d S)` over the smoothed
    bed's slope, from 0.3 to 3 m/s; the direction downstream.
  - In a bend the half width stays under 0.8 of the bend's radius, so the inner bank never
    folds over itself. A river fades in over its first 40 m (since 2026-10-01 it grows from its
    spring instead: from a sixth of its width and a third of its depth over those 40 m, its water
    in from the first 8 m).
- **On the ground:** the 8 m field cannot hold a bed a few metres wide. A channel carved into it
  would be a trench of 8 m triangles, and the water's outline would follow them. So the ribbon
  lies on the ground as the island's mesh draws it, and its outline is its own. Each vertex
  (four quads across) stands at the highest the ground reaches under the quads around it,
  sampled every half metre with the cells split as the mesh splits them, plus 0.1 m and a
  quarter of the depth. The depth the water shows is the ribbon's profile across, deepest in the
  middle. Once the 2 m field is drawn (#106), the beds can be carved and the water set at a level
  in them. (Superseded the next day: the channels are carved on cells of a metre along the
  rivers, below.)
- **In the pass:** after the sea, blended over what is under by how much of the pixel the
  river covers:
  - the banks, soft over half a metre or two pixels;
  - the head's fade, and the last 30 m to the coast, where the sea's swash takes over;
  - far away, a pixel either side at least, with the coverage scaled by the share the river
    fills (after Persson's phone-wire anti-aliasing), lifted by one and a half pixels' footprint
    over the ground's coarser levels of detail.
  The tributaries are drawn first, and the larger river, lifted more, covers their ends.
- **The water:**
  - the finest cascade's slopes on an 8 m tile, carried down the river by a flow map (Vlachos
    2010): two phases 2 s apart, cross-faded, each restarting where the other is strongest,
    their timing offset by a noise so the fade does not pulse; faster in the middle than by the
    banks;
  - the sky and the sun as the sea has them (the shading is shared), and the mirror and shadow
    rays asked for as the sea's, so the banks and the rocks show in it;
  - under it, the ground's light on a grey-brown sediment towards the middle, through the
    ribbon's depth (absorption 1.4, 0.4, 0.5 m⁻¹: fresh water with a little tannin and silt);
  - white water in riffles about 30 m apart, where the bed falls more than 6 % and the stream
    runs fast.
- **The ground:** with the water the rivers are no longer painted into the layer map (they are
  with `--no-water`). The lakes still were, until the lakes' step.

![A stream on the eastern plain from 16 m up: painted into the ground's layers before (left), a ribbon of water now (right)](../../reports/2026-09-30-105/rivers-stream.png)

![The largest river down its valley to the west coast, from 200 m up: before and now](../../reports/2026-09-30-105/rivers-valley.png)

- **What to look at:**
  - From above, a river over a plain shows its bed and a faint sky, greyer than the painted
    stand-in, which mirrored the sky more than water does.
  - In a narrow 8 m valley the ribbon rests on the highest ground under it, so its water can
    stand up to about a metre over the valley's bottom. From low down across the valley, it
    shows as a sheet a little above the ground.
  - The valleys' sides step every 8 m (#106), and their shadows cross the rivers in stripes.
  - No waterfalls: the eroded field has no cliffs (the steepest reach falls 33 % over 24 m).
    The steep reaches have white water instead.
- **Stability** (still camera, waves held at 12 s, TAA on; pixels changing by more than two
  levels, frame to frame and over 32 frames): the stream from 16 m 0.18 → 0.26 % and 0.0017 →
  0.0020 %; the plain 0.31 → 0.33 % and 0.0019 → 0.0021 %; the valley 0.59 % and 0.015 % either
  way; the island from 2.5 km 0.87 → 0.82 % and 0.0003 → 0.0006 %. The ripples' reflections move
  with the jitter from one frame to the next, and nothing crawls.
- **Cost:** `water/surface` +0.015 to 0.026 ms (the ribbons' vertices, drawn whether in view or
  not, and the river's pixels), and `water/reflections` +0.02 to 0.03 ms where a river fills
  the view: the frame 1.46 → 1.54 ms for the stream from 16 m (`docs/PROFILE.md`). At start,
  77 ms for the drainage, the courses and the ground under the ribbons; 17 MiB on the GPU, the
  ground's heights in full precision (16 MiB at 8 m) and the points.
- **Checks:** the batch changes the water images by a river mouth and a gully in the first
  view (506 px, ꟻLIP mean 0.0003), #71's flake aside; the A/B harness and mesh against
  fallback at 0 px; validation clean; tests (two new in `forge_procgen::river`, among them
  that the ribbon never dips under the drawn ground), clippy, fmt.

**The rivers' beds, level water, mouths and stones** (2026-10-01, after the owner's look at step
6). The owner saw water that was not level across (it followed the ground's V), ribbons standing
over the ground or leaving gaps at the banks, no bed to see, and rivers stopping short of the sea;
and asked for water that flows around what stands in it. The ribbon on the ground gave way to
water in a carved channel:
- **Cells of a metre along the rivers.** `forge_procgen::Channels` lists the 8 m cells within
  8 m of a river's water plus half a cell's diagonal (36 267, 2.9 M fine vertices) and gives the
  ground there at any point. `forge_geom::city::refined_heightfield_mesh` draws those cells in
  8 × 8 quads and every coarse cell beside them as a fan from its centre over its edges' fine
  vertices, the same two planes as before: the vertices on an edge are shared, so there is no
  T-junction and no crack (a test checks the twins of every edge). The cook takes the channels
  from the samples (1.5 s) and the island's DAG simplifies them like the rest.
- **The ground around a river** is the field's samples through a Catmull–Rom cubic within a
  metre of the water, blended back to the 8 m planes by 8 m out, so the valleys the rivers run
  in are smooth where the water meets them, and the cells beyond are untouched.
- **The course settles on the valley's floor.** The corner cutting leaves a course on the
  valley's side in a narrow bend; four passes move each point half way to the lowest ground
  within 8 m across (a move costing 2 cm per square metre of it) and smooth it again.
- **The water is level across** and only falls downstream: at each point the lowest ground in the
  middle and on both banks (a little up and down the course), less a freeboard of `0.05 m + 4 %`
  of the width; then the running minimum from the head, a fall steeper than 60 % spread upstream,
  and two smoothing passes that keep both. Through a lake of a hectare the level is the lake's;
  it never goes under the sea's; a tributary ends on its river's course at that river's level or
  over it, and gives way to it in its channel. The depth is now `0.4 (A / km²)^⅜` m (0.31 to
  0.87 m), a little deeper, which the future underwater view will want.
- **The channel** is a parabola across from the level at the water's edge to the depth in the
  middle, and past the edge a bank rising `0.5 x + 0.1 x²` until it meets the ground, which it
  only ever lowers; where two channels meet the lower wins. The ribbon reaches `0.5 m + 10 %` of
  the half width past the edge, under the banks, so the ground draws the water's outline.
- **The bed** is a layer of its own, grey-brown gravel at the pebbles' scale (`island: river bed`,
  painted at the water's width), and the water shows it: the surface pass sees the scene under it
  through the true depth along the view ray, as the sea sees its floor (absorption 0.9, 0.35,
  0.45 m⁻¹, a little tannin and silt). The faked sediment tint is gone. The edge fades over the
  last 2 cm of depth, or a quarter of a pixel's footprint, wherever the water thins against
  something.
- **Not over nothing:** a river or a lake fades where the view ray finds nothing within the deepest
  it can be under its surface (its own depth and 5 m more): the ground not drawn yet, its pages
  still streaming in, where the owner saw ribbons floating.
- **Far away** the channel can be under a pixel and the DAG fills it: past a footprint of a fifth
  of the channel's depth under its banks the ribbon rises onto the ground as before (resting on
  the carved ground now), and past four fifths it lies there in full, a pixel wide either side.
- **The mouths.** A river's channel runs through the beach to the sea, and where its level comes
  down to the sea's the sea fills it. The ribbon fades out over the last 0.3 m of level, and from
  that point (25 mouths) the sea's own shading takes the river on: up the channel the water is
  the river's, with its flow-mapped ripples and its absorption and scattering; out to sea it is
  a jet that widens by 0.2 m a metre and slows as the root of the distance, its water mixing into
  the sea's over 60 half widths, the plume's edge broken up by noise. The same ripple function
  draws the river, the sea at a mouth and later the lakes (still water is the same ripples
  unmoved), so no two animated layers ever fade over each other. A grid of 128 m cells lists
  the mouths whose plume reaches each cell. A river that reaches the sea in white water brings
  it along (since 2026-10-01, `reports/2026-10-01-105/spill.md`): its share at the mouth (the
  rapids' rule over its last 16 m) whitens the sea up the channel and out along the plume,
  fading over six half widths, in the sea's foam pattern drifting out with the flow. Before, the
  white water stopped where the ribbon faded. The log names two mouths: the largest river's
  (`into_sea`, a cascade) and the gentlest of a river 6 m wide or more (`gentle_sea`), where no
  white water hides the handover and the plume.
- **The stones.** Past each point of a river drawn in full, a boulder with a chance of 3 %,
  rising to a third where the water falls 15 %: across the middle 70 % of the water, 0.25 to
  0.85 m and at least 0.8 of the depth there, so most break the surface (2 525 stones, 2 051
  of them through the water). They are instances of the island's boulders standing on the
  carved bed, and the GPU's rocks keep off the refined cells (their 8 m samples no longer follow
  the ground there). In the surface pass the water parts around a stone's outline at the level
  as a potential flow parts around a cylinder, piles up white in front of it and trails a wake
  behind, widening and fading over 8 radii; a stone under the water roughens the surface over
  it. Anything else standing in the water shows by the contact, the water's edge fading where it
  thins against it; the stones' list is the place for a game's objects to join the flow. (A
  line of foam wherever fast water thinned against anything drew the banks' metre triangles and
  was left out.)
- **White water** (the riffles', the stones') is broken by two octaves of noise in streaks down
  the river (since 2026-10-01, `reports/2026-10-01-105/streaks.md`):
  - The cells are 0.35 m across and four times that long, laid out in the ribbon's own
    coordinates: metres along the river from its head, metres across it.
  - The pattern is carried down at the river's speed in the flow map's two phases.
  - Before, the pattern rode the ripples' flow, whose shear around a stone drew it into smooth
    bands metres long and a swirl. Where the flow ran straight, it stayed round blotches.
  - Streaking along the local flow in world space showed as harsh lines across it from low down
    (rotating coordinates thousands of metres from the origin).
  - `stone_view` in the log is a view of the stone in the fastest water.

![Down a river from 3 m over its water: a sheet of water over the valley's floor before (left), the river in its channel now (right)](../../reports/2026-10-01-105/beds-down.png)

![A steep stream to the sea: stones breaking the water, white in front and in their wakes (right; before, this point was inside the uncarved ground)](../../reports/2026-10-01-105/beds-stones.png)

![The largest river's mouth from 110 m: before, the ribbon stopped short of the sea; now the channel runs through the beach and the river's water goes out in a plume](../../reports/2026-10-01-105/beds-mouth.png)

- **What to look at:**
  - The plume reads as a brownish band from above; it may want to be fainter.
  - From far away a river sits in its channel and shows less than the old ribbon did, which lay
    on the ground and mirrored the sky at grazing angles.
  - Where the 8 m field jumps 5 to 15 m between samples (#106's roughness), the level water cuts a
    gorge through the spurs: a fifth of the points stand more than 2 m under their lowest bank.
  - The 8 m cells beside the channels keep their facets and their shadows (#106).
- **Cost** (`docs/PROFILE.md`): the frame +0.03 to 0.09 ms over seven views, most where the bed
  fills the view (its layer blends with the grass's, `shading/layered` +0.09 ms down a river);
  `water/surface` +0.003 to 0.011 ms, the plume's grid keeping the sea's share to 0.006 ms. At
  start 0.75 s for the rivers' water and channels (twice: the ground's layers and the water), and
  1.5 s more in the cook.
- **Stability:** frame to frame the same or better in every view (down a river 1.00 → 0.42 %,
  the old sheet of water shimmered over the stepped floor), over 32 frames within 0.004 %.
- **Checks:** the batch changes only the island's images (52 582 px, ꟻLIP mean 0.0099; with the
  water 56 139 px, 0.0106); the A/B harness and mesh against
  fallback at 0 px; validation clean; tests (the refined mesh without a crack, the level under
  the banks, the tributary at its river's level, the channel's profile and seams, the stones on
  the bed), clippy, fmt.

**The lakes** (step 7, D-038's lakes, 2026-10-01). The 15 lakes of a hectare or more were painted
into the layer map; with the water each is now water of its own, a level plane, and the rivers
through it take its level:
- **The plane and its mask** (`forge_procgen::lake_waters`): the priority flood's depression at
  the lake's level (from the lake's samples, every neighbour where the flood stands at that level:
  its shallow margins too, which the lakes' 0.5 m threshold leaves out) and a sample more all
  round. The plane covers the mask's box at the level; the mask, a bit a sample and softened over
  half of one, clips it to the lake, and within it the ground rising through the plane draws the
  shore.
- **The shore on cells of a metre.** The 8 m cells of a mask whose ground spans the level within
  a metre, and a cell more all round, are drawn finer like the rivers' channels (51 958 cells
  refined in all, 4.2 M fine vertices), on the smoothed ground, not carved: the cubic's weight is
  1 wherever every cell around a sample is refined and 0 on the refined region's outline, so the
  shore is a smooth curve where the 8 m triangles drew a polygon.
- **Its water is the rivers'**: the same shading (`fresh_water` in `water.slang`: the ripples on
  the flow, the sky, the sun, the rays, the bed through the true depth), still but where a river
  runs in: the 20 points where a river enters a lake join the mouths' list, and the river's jet
  carries its ripples out into the lake as at the sea. The lakes are drawn before the rivers, and
  a river's ribbon fades out over its last three points into the lake at the lake's own level, so
  the handover is between two surfaces with the same ripples, level and water. Patches of calm
  and of wind ripples drift over a lake (the ripples' slopes from 0.15 to 1 over a noise 60 m
  across, at 1.3 m/s), the river's own ripples where it runs in.
- **No channel through a lake:** a river's channel stops where its course is under the lake's
  water at both ends of a segment (`ChannelParams::carve_lakes`): carved on, it showed as a dark
  trench under the water (`reports/2026-10-01-105/lake-outlets.png`).
- **The bed** under a lake is dark mud (`island: lake bed`) wherever the plane stands over the
  ground (the ground as drawn, refined cells included), and the lake's water its own, darker
  than the rivers' (absorption 1.0, 0.6, 1.2 m⁻¹: the dissolved matter taking the blue), mixing
  to the rivers' along an inflow's jet; the GPU's rocks keep off the lakes. (At first the bed was
  the sea floor's silt and the water the rivers': from the air the lakes read as pale patches,
  `reports/2026-10-01-105/lakes-dark.png`.)

![The largest lake from 30 m over its south shore: painted before (left), water now (right)](../../reports/2026-10-01-105/lakes-west.png)

![A long lake on the eastern plain: the sky in it, its shore drawn by the ground](../../reports/2026-10-01-105/lakes-east.png)

![A round lake: its shore a smooth curve on cells of a metre where the 8 m triangles drew a polygon](../../reports/2026-10-01-105/lakes-round.png)

- **What to look at:** the depression at a lake's level reaches up the inflowing valleys, where
  the water is centimetres deep over silt; the far shores still show the stepped 8 m slopes
  beyond the refined band (#106); where the ground stands under the level at the edge of a mask
  (an outlet's channel), the plane ends there, softened over half a sample.
- **Cost** (`docs/PROFILE.md`): +0.04 to 0.09 ms in the lake views, most of it their mirror and
  shadow rays (`water/reflections` +0.02 to 0.05 ms) and the silt's layer; the other views
  unchanged within 0.01 ms.
- **Stability:** frame to frame the west lake 0.36 → 0.50 % (its ripples' reflections move with
  the jitter), the round lake 0.33 → 0.24 %; over 32 frames at most 0.0035 %.
- **Checks:** the batch changes only the island's images (26 633 px, ꟻLIP mean 0.0061; with the
  water 31 296 px, 0.0070); the A/B harness and mesh against fallback at 0 px; validation clean;
  tests (a basin's water covers its depression and a sample more), clippy, fmt.

**The coast's definition** (#106, 2026-10-01). The coast's contours ran in straight segments with
corners every 8 m: the water's edge, the wet sand's, the sand's top. The field itself terraced
there, not only its triangles: `sea_floor` sets the samples at sea from the coast distance and
leaves the eroded land at the sea's level, so along a coast that runs across the grid the samples
alternate between land at 0 m and floor at −0.3 m, and the sea's level traced through them
steps with them (a cubic through them scallops instead).
- **The shore smoothed** (`forge_procgen::smooth_shore`): four passes of a 3 × 3 binomial filter
  over the samples within 3.5 m of the sea's level (the sand's top at 2.5 m among them), the rest
  kept. The coast's contours then run smooth through the samples.
- **Drawn finer:** the cells the sea's level or the sand's top cross, and a cell more, join the
  refined cells (76 856 in all with the rivers' and the lakes', 6.2 M fine vertices), on the
  smoothed ground, so the water's edge and the wet sand's follow a curve and not the 8 m
  triangles.
- **The layers' edges wander:** the layer map's lookup is offset by a texel (4 m) over a noise
  three texels wide (`RenderLayer::cavity` for the layered class; 0 for the city's streets), so
  the sand's top and every other layer's edge stop stepping along the map's 4 m grid.
- **What remains:** the sand's top still shows the 4 m map's texels softly (drawn by the ground's
  height since, below: "The sand's top"); the slopes inland keep
  their 8 m facets and their shadows' steps, and the ground there is as rough as the 8 m erosion
  made it, which the 2 m amplification is for.

![#106's view over the south beach: the coast's contours in 8 m segments before (left), smooth now (right)](../../reports/2026-10-01-106/coast-above.png)

- **Cost:** up to +0.03 ms a frame (the wandering lookup up to 0.02 ms of `shading/layered`, the
  rest the refined cells). **Stability:** along the beach 0.44 → 0.26 % frame to frame, the same
  or better everywhere. **Checks:** only the island's images change; the A/B harness and mesh
  against fallback at 0 px; validation clean (`reports/2026-10-01-106/coast.md`).

**The ground's steps** (#106, 2026-10-01). The owner found the ground to the right of the first
view very uneven, and the valleys' sides step every 8 m; with the sun's shadows off the steps
still show, so they are the field's, not the shadows': the 8 m erosion leaves steps two samples
apart on the slopes (one sample a ridge, the next a gully). One pass of the 3 × 3 binomial filter
over the whole field (`GROUND_SMOOTHING` in the demo, before the sea floor) takes out what
alternates every sample and halves what repeats every four, and leaves the valleys, hundreds of
metres across, as they were.
- The valleys' walls lose their stripes, and the sand's top its teeth (they were the same
  gullies crossing 2.5 m).
- The rivers stand under their banks by 3.9 m at most where it was 14.5 m: the gorges the level
  water cut were mostly through those steps (621 points more than 2 m under their banks, from
  about 4 000). 14 lakes of a hectare or more where there were 15: the smoothing changes the
  depressions too.
- The rocks follow the new slopes.

![The largest river's valley from 200 m, the lakes' commit (left) and with the coast and the ground smoothed (right)](../../reports/2026-10-01-106/ground-valley.png)

![The ground east of the first view: before (left), smoothed (right)](../../reports/2026-10-01-106/ground-right.png)
- **Cost:** none to draw; the frame within 0.04 ms of the lakes' commit over six views. Over 32
  frames as stable or more. **Checks:** only the island's images change; the A/B harness and mesh
  against fallback at 0 px; validation clean (`reports/2026-10-01-106/ground.md`).

**The swash's line** (#106, 2026-10-01). From above, a grey band with light dashes ran along the
beach where the waves run up, like a road's centre line. It was the foam at the swash's edge: the
sheet was whitened where under 8 cm (2 m wide on this beach), then cut up by the foam's 2.5 m
pattern.
The edge is now a line where the sheet is under 3 cm, which the pattern only frays (40 % of its
cover at most); the surf's broken water keeps its pattern.

![18 m over the south beach at 12 s (top) and 14 s: the dashes before (left), the line now (right)](../../reports/2026-10-01-106/swash-close.png)

- **Cost** and **stability:** `reports/2026-10-01-106/swash.md`. **Checks:** only the island's
  water images change (ꟻLIP mean 0.0008); the A/B harness and mesh against fallback at 0 px;
  validation clean.

**The sand's top** (#106, 2026-10-01). The sand and the grass met in teeth along the coast, a few
metres deep and about ten apart: the layer map's 4 m texels, through the lookup's wander. The
map's rule for the sand is a height (gentle ground under 2.5 m), and the drawn ground there is on
cells of a metre, so the layered pass now applies that rule under each pixel
(`RenderLayer::contour`):
- Where the map shows the sand or one of the grasses (grass, dry, lush), the pixel takes the sand
  under 2.5 m and a grass over it: the map's own grass, or where the map shows sand, the heaviest
  grass among the four texels around the pixel (none: the sand stays).
- The height wanders by 0.3 m over two octaves of noise 7 m and 2.3 m wide, so the edge bends
  along the beach instead of running as a contour line. The layers blend over 8 cm of height, or
  the height a pixel spans far away.
- The other layers keep the map's edges: rock, the sea floor, the rivers' and lakes' beds. Where
  one of them meets the sand and the grass, three layers are shaded.
- The traced rays' hits take the sand under the height too.

![The coast east of the first view from 70 m: the sand's top in teeth before (left), along the ground now (right)](../../reports/2026-10-01-106/sand-side.png)

- **Cost:** 0.010 to 0.023 ms of `shading/layered` over six views. **Stability** as before. **Checks:** only the island's images change (ꟻLIP mean
  0.0045, 0.0053 with the water); the city's layered ground, the A/B harness and mesh against
  fallback at 0 px; validation clean (`reports/2026-10-01-106/sand.md`).

**The water up a valley from low** (#113, 2026-10-01). The owner looked up a steep valley from
just over its stream: the water showed only near the camera, and appeared further up as the
camera rose. Not the streaming nor the levels of detail (the same with every page resident):
the fresh water's contact fade measured its depth along the view ray under a *level* surface,
`path × v.y`. Looking up a river that falls 10–20 %, the water 50 m ahead stands metres over the
camera, so `v.y` was negative there, the depth zero and the water discarded, though its surface
tilts down towards the camera and is plainly in view. The depth is now taken under the surface
as it lies, tilted by the river's fall as drawn (`FreshWater::tilt`, uncapped where the shading's
`fall` stops at 30 %). From the logged view `up a steep river from low` the water runs up the
valley (ꟻLIP mean 0.015 on that view); elsewhere only the water's edge moves, by a thin line
(0.0004 down a lowland river, 0.008 at the largest mouth, under 0.00002 on the lake and the
first view). Sheet: `reports/2026-10-01-113/up-valley.png`.

**The rivers' banks, beds and lake entries** (#114, 2026-10-01). From the owner's view of a river
entering a lake: the channel read as a trench, the bed's colour ran wider than the channel and
fanned into the lake, and the channel stopped in a step at the lake's edge.
- **The bed, per pixel.** A pixel of the layered ground blends the four texels around it and the
  lookup wanders by one, so a bed painted on the 4 m map showed up to 8 m onto the banks, wider
  than most of the rivers. With the water the map now paints no river bed, and the lakes' mud
  only under a metre of water or more; the fresh water turns the ground it covers into its bed
  instead, at the ground's own brightness (grey-brown gravel for a river at 85 %, brown silt for
  a lake at 90 %, blended through the river's plume), exactly within its own coverage.
- **The banks by the bend.** In a straight reach the bank rises `0.3x + 0.06x²` from the water's
  edge (was `0.5x + 0.1x²`); in a bend the inner bank's slope falls to a fifth, a point bar, and
  the outer grows by three fifths, a cut bank (`ChannelParams::bend`: the curvature times the
  half width plus 4 m, times 2, is the bend's share).
- **Into and out of a lake** the channel shoals over 8 m plus three of its widths, to a fifth of
  its depth at the lake's edge (`ChannelParams::shoal`), so its bed meets the lake's shallows
  without a step.
- From 40 m (`reports/2026-10-01-114/`): a river across the plain is a clean band where it was a
  blurred, jagged strip of bed texels (`beds-plain.png`, ꟻLIP mean 0.020); the lake entry has a
  crisp shore and silty water where it had a brown smear and a jagged grass edge
  (`beds-lake.png`, 0.14); the largest mouth's channel reads as shallow water over sand
  (`beds-mouth.png`, 0.054).

**The riparian strip** (D-041, 2026-10-01; `forge_procgen::paint_banks`, `RIPARIAN_STRIP`). A
river across the plain was a clean band in a uniform field of grass. The grasses within 6 m plus
two of a river's widths of its water now become the banks' growth, reeds, sedges and shrubs: a
deeper, bluer green than the lush grass, the grass's texture at a coarser scale
(`island_layer::RIVERBANK`, the row and id the river bed's layer had until the water drew its bed
per pixel). Near the coast the sand's contour still takes it under 2.5 m, per pixel. 88 202
texels at 4 m. From 300 m a darker corridor follows each river; from 3 m the banks are green to
the water (`reports/2026-10-01-112/riparian.png`, against da572f7, which also had the bed's
smear: ꟻLIP mean 0.014 from 300 m, 0.14 from 3 m).

**The estuaries** (D-041, 2026-10-01; `RibbonParams::estuary`, `ChannelParams::beach`). With the
coastal plain the mouths were calm, but each crossed the beach as a narrow channel with steep
walls, a canal. Under 1.5 m over the sea a river now widens towards its mouth, to twice its width
at the sea's level, and shallows by two fifths; its banks flatten there to three tenths of
their rise. The widest water at a mouth is 34 m (17 m before). From 40 m the mouths flare into
the surf (`reports/2026-10-01-110/estuary.png`: the gentlest and the largest mouth, from 4 m and
from 40 m; ꟻLIP means 0.11, 0.055 and 0.066).
