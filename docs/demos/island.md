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
| PNG previews: height, hillshade, flow, the overview with sea, rivers and lakes | ✅ `forge_procgen::preview`, `tools/genesis` |
| Hydrology: rivers as polylines with widths, lakes by Fill–Spill–Merge (stage 4) | rivers and lakes are marked, not yet traced |
| Amplification to 2 m per tile with halos (stage 5) | planned |
| Materials from the fields, the layer map (stage 6) | planned |
| The hand-off to the cluster-DAG cook: the island drawn by today's renderer (stage 7) | built, to see on a GPU: `city-blocks --island SEED` (#96 for the props and the demo of its own) |
| The planet: the same stages on the cube sphere's coarse graph, tiles amplified at streaming time | planned |

```
cargo run --release -p genesis -- --spacing 16 --steps 150 --out captures/island
cargo run --release -p city-blocks -- --island 7
```

`genesis`: `--seed N`, `--spacing M` (16: 1025² samples; 4: the 4097² target), `--steps N`,
`--k` (erodibility), `--diffusion`, `--uplift` (metres per step at the heart), `--every N` (a
hillshade every N steps), `--threads N` (workers besides the main thread; the default is one
per hardware thread, `0` is serial, the result is the same). It prints each stage's time, the
erosion step's breakdown, and writes `uplift.png`, `height.png` (16-bit), `hillshade.png`,
`flow.png` (log drainage) and `overview.png`.

`city-blocks --island SEED` draws the island in the engine (stage 7, written in the cloud
and not yet seen on a GPU): the heightfield (`--island-spacing`, 8 m by default: 2049²,
8.4 M triangles like the city's ground; 4 m for the 4097² target, 33.5 M) is generated once
into `mesh-cache/island-<key>.f32`, cooked into a cluster DAG through the same path as the
city's terrain (`PropKind::Heightfield`, `forge_geom::city::heightfield_mesh`) and cached, and
drawn as the scene's one instance on the ground's layered material, rock where the ground is
steeper than 0.45 or higher than 380 m and grass elsewhere (`forge_procgen::slope_layers`, a
texel every 4 m), under the city's sky, with the sun's shadows, the probes and TAA. The camera
starts over the sea south of the island, looking north at its coast; `--view` and the usual
keys apply, `--origin` too. The first start costs the erosion (25 s at 8 m on four cores) and
the cook (about the city's 13 s); the next ones load both.

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
   Hardness, a factor on the erodibility, in 1.8 km patches; rain flat for now.
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
4. **Hydrology, the first part**: rivers where more than 0.5 km² drains through a sample,
   lakes where a final priority flood (Barnes 2014, once) stands more than 0.5 m over the
   eroded field.

| Run (seed 7, 150 steps) | Samples | Erosion | Per step | Was (flood, one thread) | Peaks | River samples | Lake samples |
|---|---|---|---|---|---|---|---|
| `--spacing 16` | 1025² | 5.5 s | 0.037 s | 0.13 s | 530 m | 3 657 | 4 387 |
| `--spacing 8` | 2049² | 25 s | 0.167 s | 0.66 s | — | 7 336 | 62 819 |
| `--spacing 4` (the target) | 4097² | 128 s | 0.851 s | 3.13 s | 545 m | 14 809 | 181 377 |

In the cloud container, four cores (the owner's 9800X3D has eight, faster); the 4 m run's
stages 1–2 take 5.6 s, the hydrology 2.3 s, the previews 0.8 s, the whole run 2 minutes 20.
The 8 m row's counts are after 20 steps (a timing run).

**Where the time goes** (#97, the first part). Before, the priority flood was 89 % of a step:
a heap over every cell, `n log n` with a large constant. The basin graph does linear work on
the raw D8 receivers and touches the depressions alone (523 pits at 8 m, a graph of a few
hundred thousand passes sorted once); the D8 receivers, the passes, the uplift, the diffusion
and the implicit update (the stack's segments, whole drainage trees each) run on `forge-task`.
A step at 4 m is now `uplift 0.005, drain 0.727, incise 0.051, diffuse 0.055` seconds. Of the
drain, about a tenth is the parallel D8 and passes; the rest is sequential: labelling the
basins (a walk to each cell's root), and above all the stack (the donor lists, the depth-first
order from the outlets, the areas; 92 ms of the 145 ms at 8 m). `--threads 0` at 16 m gives
0.066 s a step against 0.041 with four workers: about half of the serial step is parallel work.
The next lever is that stack: Cordonnier's parallel construction (the trees below the outlets
on the workers, merged by index) or a fixed-size buffer set kept across steps; either is the
second part of #97. The result is the same bytes with any thread count (a test runs the
erosion with none and with three workers). The carve changes the field slightly against the
flood's routing (water leaves a lake by one path rather than over the whole flooded flat), so
the counts above differ from the first runs' (3 487 river and 2 774 lake samples at 16 m); the
pictures below are from the new field. At 4 m the lakes cover 3.5 % of the land samples
(181 k of 5.2 M) against 1.4 % at 16 m: the finer grid holds more small depressions, which the
sediment rule fills more slowly; a lake area limit, or the basin graph's fill mode with a
spill rule, is still part of #97.

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
