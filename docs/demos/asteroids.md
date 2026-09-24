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
`--show-culled`, `--taa-blend F` (1 = jitter without history).
With Tracy: `cargo run --release -p asteroids --features profiling` and connect
`tracy/tracy-profiler.exe`.
Controls: **F1** profiling overlay (**1**–**9** fold a group), **P** pause the path and fly
freely (right mouse look, WASD/QE, Shift fast), **T** temporal anti-aliasing, **O** occlusion
culling, **C** cone culling, **X** culling-error view (what culling rejected is drawn in red;
any red pixel is a bug), **M** meshlet colours, **Tab** wireframe, **Esc** quit.
Machine: RTX 5070 Ti, driver 617.14, Vulkan 1.4, Slang 2026.13, 1600×900, 2026-09-24.

![The ballad, phase 0](images/asteroids-ballad.png)

![Passing the planet](images/asteroids-planet.png)

![The profiling overlay (F1, full view): GPU time per pass, CPU time per zone, counters](images/asteroids-profile.png)

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
| scene | 3000 asteroids, 7 meshes, 195 M triangles, 14.2 M meshlets |
| drawn per frame (moving, default path) | 810 k previously visible + 29 k newly visible meshlets, **78 M triangles** |
| GPU per frame | **7.0 ms** (sky + two meshlet passes + 11-level pyramid + motion + resolve + blit) |
| CPU per frame (record) | 0.06 ms |
| frame time, uncapped | p50 7.0 ms, p99 7.4 ms |
| field build (7 meshes on 6 workers + scatter + upload) | ~1.2 s |

Earlier configurations for reference: 6000 rocks in a 60–160 m tube drew 50 M triangles at
7 ms and looked like a cave; the first belt layout without overlap rejection drew 20 M
triangles at 3.4 ms because rocks stacked inside each other. The 58–64 M triangles at
5.6–6.0 ms reported before the culling fixes below were measured with a quarter of the
geometry silently missing.

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

- Almost every drawn triangle is smaller than a pixel: a 442 k-triangle rock 300 m away
  covers a few hundred pixels. The GPU time is spent rasterising and shading geometry that
  cannot be seen — exactly the case the cluster LOD DAG and the software rasteriser (Phase 1)
  remove. This demo is the benchmark for that work: the target is the same image at well
  under 1 ms of geometry.
- With a static camera and TAA on, consecutive frames still differ in ~1.4 % of the pixels
  by up to 41 levels: the temporal filter cannot fully settle on geometry that changes
  every jitter. That residual sizzle on distant rocks is what LOD will remove; the trembling
  the owner saw was the holes above.
- TAA costs about 0.5 ms here (motion + resolve + blit at 1600×900).
- The CPU is idle. Everything below the frame loop is the GPU walking pointer tables.

## Reference look (owner's references)

Dense belts against a planet or a sun with a bright halo, volumetric light between the
rocks, ice asteroids in blue, dark rock in the foreground with lit rims; later, bases carved
into the big asteroids, ships in pursuit, lasers, missiles, rocks breaking by mass on impact.

## Next for the ballad

1. Cluster LOD DAG + software rasteriser (Phase 1): rocks of a million triangles at
   negligible cost, screen-space error selection, streaming of cluster pages.
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
