# Demo: island

Phase 2's demo (`docs/ROADMAP.md`): a 16 km island generated from a seed, later
orbit-to-ground on the planet variant, and the first step of rebuilding tropical-island (#81).
It is built in steps; the first, the terrain's genesis on the CPU, started on 2026-09-26 in a
cloud session, following `docs/research/terrain-genesis.md` ("Recommendation for Forge").

| Step | State |
|---|---|
| The island's mask, uplift, hardness and rain fields (stages 1–2) | ✅ `forge_procgen::island` |
| Priority flood, D8 drainage, the downstream-first stack, integer areas | ✅ `forge_procgen::flow` |
| The implicit stream-power erosion with diffusion; lakes as filling depressions (stage 3) | ✅ `forge_procgen::erosion` |
| PNG previews: height, hillshade, flow, the overview with sea, rivers and lakes | ✅ `forge_procgen::preview`, `tools/genesis` |
| Hydrology: rivers as polylines with widths, lakes by Fill–Spill–Merge (stage 4) | rivers and lakes are marked, not yet traced |
| Amplification to 2 m per tile with halos (stage 5) | planned |
| Materials from the fields, the layer map (stage 6) | planned |
| The hand-off to the cluster-DAG cook: the island drawn by today's renderer (stage 7) | #96: `PropKind::Terrain` takes a parametric terrain today; a heightfield variant is the hook |
| The planet: the same stages on the cube sphere's coarse graph, tiles amplified at streaming time | planned |

```
cargo run --release -p genesis -- --spacing 16 --steps 150 --out captures/island
```

`--seed N`, `--spacing M` (16: 1025² samples; 4: the 4097² target), `--steps N`, `--k`
(erodibility), `--diffusion`, `--uplift` (metres per step at the heart), `--every N` (a
hillshade every N steps). It prints each stage's time and writes `uplift.png`, `height.png`
(16-bit), `hillshade.png`, `flow.png` (log drainage) and `overview.png`.

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
3. **Erosion** (`erosion::step`, 150 times): the uplift is added; the field is flooded
   (priority flood, Barnes 2014: every depression raised to its spill level with an ε slope)
   and the water routed (D8 receivers, the downstream-first stack, integer areas, Braun &
   Willett 2013); each land cell then lowers towards its receiver by `f / (1 + f)` of the
   difference with `f = K · √(A · rain) / Δx` (the implicit stream-power update with `n = 1`,
   `m = 0.5`, unconditionally stable), the receiver already at its new height; an explicit
   diffusion sweep smooths the hillslopes. The height keeps its depressions: their cells rise
   towards their receivers by the same rule, which is sediment settling in a lake, so lakes
   appear in the uplifted basins and slowly fill.
4. **Hydrology, the first part**: rivers where more than 0.5 km² drains through a sample,
   lakes where the final flood stands more than 0.5 m over the eroded field.

| Run (seed 7, 150 steps) | Samples | Erosion | Per step | Peaks | River samples | Lake samples |
|---|---|---|---|---|---|---|
| `--spacing 32` | 513² | 4.5 s | 0.03 s | 519 m | 1 837 | 63 |
| `--spacing 16` | 1025² | 19.9 s | 0.13 s | 530 m | 3 487 | 2 774 |
| `--spacing 4` (the target) | 4097² | 3.1 s a step (about 8 min for 150; the run's full numbers follow in the handover) | 3.1 s | | | |

Single-threaded, in the cloud container (the owner's 9800X3D will be faster). The per-step
cost is the flood's heap over every cell: the research's basin graph (Cordonnier–Bovy–Braun)
replaces it with work on the depressions alone, and the stack's basins run in parallel on the
job system; both are #97, the speed-up the 4 m run needs.

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
