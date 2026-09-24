# Demo: `asteroids` — the ballad

The engine's living showcase: a scripted flight through an asteroid field, improved with
every system that lands (see the showcase section of [ROADMAP.md](../ROADMAP.md)). This page
records what it shows today and the numbers it produced.

Run: `cargo run --release -p asteroids`.
Options: `--count N` asteroids (3000), `--length M` belt length (1200), `--duration S`
seconds per lap (90), `--sun-dir x,y,z`, `--planet-dir x,y,z`, `--planet-angle DEG` (18; 0
hides it), `--vsync`, `--validate`, `--fixed-step` (path advances per frame, for
deterministic captures), `--frames N`, `--capture file.png --capture-frame N`,
`--capture-every N` (a PNG sequence), `--no-taa`, `--no-occlusion`, `--no-cone`,
`--show-culled`, `--taa-blend F` (1 = jitter without history), `--lod-error PX` (projected
error a drawn cluster may have, 1.0), `--no-lod` (full detail only), `--lod-colors`,
`--no-group-window` (A/B: must not change the image).
With Tracy: `cargo run --release -p asteroids --features profiling` and connect
`tracy/tracy-profiler.exe`.
Controls: **F1** profiling overlay (**1**–**9** fold a group), **P** pause the path and fly
freely (right mouse look, WASD/QE, Shift fast), **T** temporal anti-aliasing, **O** occlusion
culling, **C** cone culling, **L** cluster LOD, **K** LOD colours, **[** / **]** halve /
double the LOD error threshold, **X** culling-error view (what culling rejected is drawn in
red; any red pixel is a bug), **M** meshlet colours, **Tab** wireframe, **Esc** quit.
Machine: RTX 5070 Ti, driver 617.14, Vulkan 1.4, Slang 2026.13, 1600×900, 2026-09-24.

![The ballad, phase 0](images/asteroids-ballad.png)

![Passing the planet](images/asteroids-planet.png)

![The profiling overlay (F1, full view): GPU time per pass, CPU time per zone, counters](images/asteroids-profile.png)

![Clusters coloured by LOD level (K): grey 0, green 1, yellow 2, orange 3, red 4, magenta 5, blue 6, cyan 7+](images/asteroids-lod-levels.png)

The compact view (the default) is the header and one line per subject; a digit opens a
subject, F1 cycles off → compact → full. The verdicts behind the numbers are in
[PROFILE.md](../PROFILE.md).

## What it shows (phase 0)

- Seven procedural asteroid meshes (cube-spheres of 48 to 192 segments per face displaced by
  fractal noise, 28 k to 442 k triangles each), built in parallel on the job system, cooked
  into meshlets by meshoptimizer and concatenated into one set of GPU tables.
- 3000 instances scattered along an S-shaped belt (a flattened disc 140–360 m across with
  density clumps), big rocks rare, no two rocks overlapping (a placement grid rejects
  intersections), a clear corridor kept around the flight path: **195 M source triangles,
  14.2 M meshlets** in the culling universe. A fifth of the rocks are ice: brighter, bluish,
  with a specular highlight.
- The GPU-driven pipeline of the `meshlets` bench: one draw per pass, task-shader culling
  (frustum, normal cone, two-pass hierarchical-Z occlusion), mesh-shader emission, no
  descriptor sets, statistics read back without stalls.
- **A cluster LOD DAG per mesh** (Nanite-style, built with meshoptimizer: groups of eight
  clusters, locked group borders, each group simplified to half and re-clustered, 9 to 13
  levels down to one root cluster, about twice the leaf triangles in the tables). The task
  shader selects, per cluster, the one cut of the DAG whose projected error is under a
  threshold (1 px by default) with monotonic errors and spheres so the cut has no cracks; a
  per-mesh per-level table lets a task group of 32 clusters exit before reading a cluster
  when none of its levels can be selected at the instance's distance.
- **Temporal anti-aliasing**: Halton-jittered projection, motion vectors from depth and the
  two cameras, Catmull-Rom history clipped to the neighbourhood (ported from the previous
  project). Without it, sub-pixel triangles of distant rocks shimmer badly. The scene is
  drawn into an HDR target and resolved to the swapchain. The occlusion test reads the depth
  pyramid where the jittered projection put the sphere, so culling stays exact under TAA.
- A procedural sky as one full-screen triangle: two star lattices whose glow is never
  narrower than a pixel (so stars do not twinkle), a Milky-Way band and dusty nebula, the sun
  with disc, halo and wide glow, and a **planet** (lit sphere with continents, ice caps,
  clouds, night-side city lights, atmosphere rim and a scattered halo beyond the limb).
- A closed Catmull-Rom camera path through the belt, 90 s per lap, looking along the tangent.
- Tracy: frame marks, `update` / `wait for frame slot` / `record` / `submit and present`
  zones and a `gpu ms` plot (`--features profiling`).

## Numbers

| | value |
|---|---|
| scene | 3000 asteroids, 7 meshes, 195 M leaf triangles; DAG tables 28 k clusters, 4.6 M cluster slots over all instances |
| drawn per frame, LOD 1 px (moving, default path) | 8 k meshlets, **0.62 M triangles**, mean LOD level 6.3 |
| GPU per frame, LOD 1 px | **1.09 ms** (sky 0.14 + meshlet pass 1 0.46 + pyramid 0.02 + pass 2 0.42 + TAA 0.06) |
| GPU per frame, LOD 0.5 px / 2 px | 1.13 ms (1.68 M triangles) / 1.11 ms (0.30 M) |
| GPU per frame, full detail (`--no-lod`) | 5.50 ms (862 k + 33 k meshlets, 83 M triangles) |
| CPU per frame (main thread) | 0.18 ms |
| frame time, uncapped, LOD 1 px | p50 1.07 ms, p99 1.36 ms (~930 fps) |
| field build (7 DAGs on 6 workers + scatter + upload) | ~2.5 s |

The `meshlets` bench (1152 rocks, 127 M triangles) goes the same way: 2.18 ms → **0.58 ms**
at 1 px (15 k meshlets, 1.1 M triangles).

Earlier configurations for reference: 6000 rocks in a 60–160 m tube drew 50 M triangles at
7 ms and looked like a cave; the first belt layout without overlap rejection drew 20 M
triangles at 3.4 ms because rocks stacked inside each other. The 58–64 M triangles at
5.6–6.0 ms reported before the culling fixes below were measured with a quarter of the
geometry silently missing; 78 M at 7.0 ms was the honest full-detail number before the DAG.

## The cluster LOD DAG (2026-09-24)

Build (`crates/forge-geom/src/lod.rs`): level 0 is the mesh cut into clusters; each level
partitions the previous level's clusters into spatial groups of about eight
(`meshopt::partition_clusters`), locks every vertex shared between groups, simplifies each
group's triangles to half (`simplify_with_locks`), re-clusters the result and records two
spheres and two errors per cluster: `self` (the group that produced it; error 0 at level 0)
and `parent` (the group that consumed it; infinite for a root). A group's error is at least
its children's and its sphere contains theirs, and every cluster of a group carries the
group's values, so "draw when `project(parent) > t >= project(self)`" selects exactly one cut
with no cracks: siblings decide together, and the children of a group decide with the same
numbers their parents used. Vertices are shared across levels; the tables hold about twice
the leaf triangles (the 442 k-triangle rock: 13 levels, 884 k triangles, 9 434 clusters).

Selection (`shaders/meshlet.slang`): the projected error is the object-space error scaled by
the instance, times the projection scale and half the viewport height, over the distance to
the sphere's nearest point. Every level-0 cluster passes the `self` test; every root passes
the `parent` test. The culling universe is now every instance's real cluster count (a
group-to-instance table, exact task-group counts, per-instance visibility bits) instead of
"largest mesh × instances", and each task group first checks, from a per-mesh per-level table
of error and sphere bounds, whether any of its 32 clusters can be selected at the instance's
distance; most groups of a far instance exit there without reading a cluster. Both are
provably conservative, and `--no-group-window` proves it empirically.

Exactness checks at frame 600 (`imgdiff`): `--no-lod` against the pre-DAG full-detail image,
occlusion against brute force with LOD on, and the group window on against off all differ in
**0 of 1 440 000 pixels**. LOD on against full detail changes 28 % of the pixels by a mean
of 7.7 levels: the far field is simplified, which is the point, and the picture reads the
same (captures above).

Two things cost more than the geometry itself while this was built and are worth
remembering: reading a 368-byte `Mesh` record by value in every task thread, and dispatching
`instances × largest mesh` task groups (885 k per pass, all but 145 k exiting on the first
instruction) because a wrapped line hid the old formula from a replacement. The profiler
showed both passes at exactly 1.94 ms whatever was drawn, which is the signature of launch
overhead, not work.

## The culling A/B check, and the two bugs it found (2026-09-24)

The owner still saw "the surface of the asteroids trembling / glitching" after TAA. Numbers
looked fine and the picture looked plausible, so the demo grew a harness instead of another
guess: with `--fixed-step` every run is deterministic frame by frame, `--no-occlusion`,
`--no-cone` and `--no-taa` switch stages off, and `tools/imgdiff` compares captures. Two
identical runs differ in 0 pixels. Culling on versus off must also differ in 0 pixels, since
culling may only remove what cannot be seen. It differed in 70 000.

1. **The depth pyramid was point-sampled.** The min-reduction sampler used for the
   hierarchical-Z pyramid was created with NEAREST filtering. The reduction mode only applies
   to the texels a filter would read; with NEAREST that is one texel, so every pyramid level
   was an arbitrary subsample of the level below and the occlusion test read one random
   depth instead of the farthest one in the rectangle. Result: holes all over rough rocks,
   flickering as the visibility bits changed. Fix: LINEAR filtering for that sampler (the
   mip level is always chosen explicitly), and a four-tap corner test at the level where a
   texel is at most the rectangle's size, which is provably conservative (see
   `shaders/meshlet.slang`).
2. **Wide normal cones were normalised to NaN.** meshoptimizer marks a cluster whose normal
   cone is wider than a hemisphere with `cone_cutoff = 1` and leaves `cone_axis` at zero.
   `normalize(0)` is NaN on the GPU, every comparison with NaN is false, and the meshlet was
   culled — permanently, whatever the camera did. Twelve percent of the meshlets of the
   roughest rock are wide cones. The CPU tests had hidden it by using `normalize_or_zero`;
   they now evaluate the test exactly as the shader does, and one asserts the convention.
   Fix: skip the cone test when `cone_cutoff >= 1`.

After both fixes: occlusion on versus off, cone on versus off, jittered versus unjittered,
at several frames along the path — **0 of 1 440 000 pixels differ** every time. The `X` view
(`--show-culled`) draws what culling rejected in red; a correctly culled meshlet is
back-facing or hidden and leaves no pixel, so the image must stay free of red, and does.
The `--taa-blend 1` mode (jitter, no history) makes TAA frames comparable one by one.

## Known residual: TAA history is not bit-exact between runs

Two identical runs with TAA on and the camera moving differ at frame 600 in about 700 of
1 440 000 pixels by up to 30 levels (invisible), sometimes in 0. Everything narrower is exact:
TAA off, jitter without history (`--taa-blend 1`), a static camera with history, and the
per-frame CPU inputs (`FORGE_TRACE_FRAMES` traces of two runs are identical). Synchronization
validation and GPU-assisted validation report nothing; a full barrier before every pass
(`FORGE_PARANOID_BARRIERS`), a device wait after every frame (`FORGE_WAIT_IDLE`) and vsync
change nothing; a CPU sleep of 20 ms or more after every frame (`FORGE_STALL_MS=20`) makes
every run bit-identical, and periodic captures do the same. The outcome is bimodal (runs land
on one of two histories), so it is a GPU-side effect that only the temporal feedback loop
amplifies; it is not a culling error. Use `FORGE_STALL_MS=20` for reference captures and treat
it as open (revisit with the render graph's explicit resource states and when TAA is
replaced by DLSS on NVIDIA).

## What the numbers say

- Before the DAG almost every drawn triangle was smaller than a pixel (54 per pixel): a
  442 k-triangle rock 300 m away covers a few hundred pixels. The DAG draws that rock with a
  few clusters of its coarse levels; the geometry passes went from 5.8 ms to 0.9 ms and what
  is left in them is the task-shader walk over the cluster slots, not rasterisation (the
  cluster hierarchy is the next step).
- With a static camera and TAA on, consecutive frames differed in ~1.4 % of the pixels by up
  to 41 levels at full detail: the temporal filter cannot settle on geometry that changes
  every jitter. With the DAG the far field is a few triangles per pixel, which the filter can
  settle; the trembling the owner saw was the culling holes above.
- TAA costs 0.06 ms here (motion + resolve + blit at 1600×900); the sky 0.14 ms.
- The CPU is idle (0.18 ms per frame). Everything below the frame loop is the GPU walking
  pointer tables.

## Reference look (owner's references)

Dense belts against a planet or a sun with a bright halo, volumetric light between the
rocks, ice asteroids in blue, dark rock in the foreground with lit rims; later, bases carved
into the big asteroids, ships in pursuit, lasers, missiles, rocks breaking by mass on impact.

## Next for the ballad

1. Software rasteriser for the sub-pixel clusters and a cluster hierarchy in the task
   shader (fewer, fatter task groups; the two passes are launch-bound at 145 k groups each);
   streaming of cluster pages. Cluster LOD DAG: done (above).
2. HDR exposure and tonemapping, DLSS; a proper sun with ray-traced shadows on the RTX
   tiers; volumetric dust and the nebula lit by the sun.
3. Physics (Phase 3): tumbling, collisions, fracture by mass; then ships, lasers, missiles,
   crashes (Phases 5–7), a second player, spatial audio.
4. Look (owner's request, 2026-09-24, after the systems): rock asteroids as angular
   *chunks* of rock (fractured faces, edges, flat facets — a Voronoi/fracture-based
   generator rather than a displaced sphere), and ice asteroids as ice *blocks/chunks*,
   translucent according to their density and to the thickness of ice between the light and
   the camera (transmission through the body, so the sun glows through thin edges; a
   material-layer job for the lighting phase, with the fracture generator shared with the
   destruction system).
