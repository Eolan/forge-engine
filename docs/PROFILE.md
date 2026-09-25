# Forge — Where the time goes

The on-screen profiler (F1 in every demo) mirrored here at each checkpoint, with a verdict
per item: what is expensive, why, and how it gets attacked. The overlay is the live truth;
this page is the record and the discussion.

How it measures: GPU zones are timestamp queries written at pass boundaries
(`Commands::mark("group/name")`), read back two frames later, smoothed (8 % per frame); the
first timestamp waits for all earlier commands so the first zone does not absorb the previous
frame's tail. CPU zones are wall-clock spans of the main-thread frame loop. Frames overlap on
the GPU (two in flight), so the per-pass sum can exceed the wall-clock frame: the frame time
is the truth, the zones are the split. With `--features profiling` the same GPU zones appear
in Tracy's GPU timeline next to the CPU zones. At exit every demo logs the GPU zones (`gpu:`)
and the CPU zones (`cpu:`) averaged over the whole run, unsmoothed, and the memory counters.

Machine: RTX 5070 Ti, driver 617.14, 1600×900, 2026-09-24 (shading rows updated 2026-09-25, #20).

## `asteroids` — the ballad with the planet's atmosphere (frame 400, LOD 1 px, TAA on, ACES, overlay in full mode)

Frame **0.36 ms** (2 746 fps), p50 0.35, p99 0.64; over a 20 000-frame scripted run without
the overlay the GPU averages **0.326 ms** along the path (0.322 through the task shader the
same day, 0.360 through the indirect-count fallback; 0.325 before issue #5, 0.315 before
the atmosphere, 0.30 before the exposure histogram, 0.27 with the rocks shaded in the mesh
passes, 0.33 with the sky drawn first, 0.34 before the graph) and 0.34 ms facing the planet
(`--look`). GPU zones sum to 0.36 ms with the overlay and the capture copy in the frame;
CPU main thread 0.18 ms of work (record 0.08, submit + present 0.10; the exit log's `cpu:`
line over 20 000 frames). Issue #21 moved the meshlet statistics into device memory, copied
into a host-cached readback, and declared that read and the capture's as host reads in the
graph: record 0.076–0.079 ms before, 0.078–0.079 after (meshlets 0.047–0.049 both). Every zone below is a
graph pass; the graph's own counters are the first line of the COUNTERS block, the
exposure (EV100, target, compensation, curve) the last.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| geometry | meshlet pass 1 (visible last frame) | 0.04 | 12 % | 7–8 k clusters, 0.5 M triangles, positions and a 32-bit id only (0.06 while the task shader culled inside it). Through the indirect-count fallback: 0.07. |
| geometry | cluster cull 1 / cluster cull 2 (occlusion) | 0.02 / 0.02 | 5 % each | Compute since issue #5: LOD selection, frustum, normal cone and the depth pyramids (the previous frame's in both, this frame's in the second, since issue #33), appending the survivors in a fixed order (a prefix sum over workgroups, so the image does not depend on timing). The same list feeds both paths. |
| geometry | instance cull | 0.02 | 5 % | One thread per instance: frustum, per-level LOD window, work-list append in instance order. At this size the timestamps' own granularity shows. |
| geometry | depth pyramid | 0.02 | 7 % | Eleven graph passes, one zone. Negligible; stays. |
| geometry | meshlet pass 2 (newly visible) | 0.01 | 2 % | Almost nothing becomes newly visible per frame at 1 px. |
| shading | standard / ice | 0.035 / 0.011 | 13 % | By material class since #20 (D-026): the standard pass covers the target, shades the rock, writes nothing where the sky goes and lists the 8×8 tiles holding ice; the ice pass shades those. 0.024 as one resolve; the split costs 0.02 ms here and keeps each class's code (and registers) to its own pixels as shading grows. |
| sky | starfield + planet | 0.11 (0.14–0.16 with the planet in view) | 30 % | **Still the largest single item.** Drawn after the rocks with a depth test, so its noise runs only on the uncovered pixels. Since #8 the planet is a ground under Earth's air, marched per pixel in 16 segments through Hillaire's tables (D-023); pixels outside the atmosphere's cone skip it, so the planet costs only where it is: +0.01 ms out of view, +0.04–0.06 in view, 0.31 ms for a planet filling the screen. The planet-view table (#26) turns that into a lookup. The tables themselves are built once (`sky/atmosphere tables`, first frame only). |
| exposure | luminance histogram | 0.02 | 7 % | Four passes in one zone: clear 1 KB, count every pixel into 256 log2 bins (shared-memory atomics, one global atomic per non-empty bin and group), copy to the slot's cached readback, host read. Could meter a quarter-resolution image if it ever matters; it does not now. |
| temporal | motion / TAA resolve | 0.01 / 0.05 | 16 % | The resolve rescales the history by the exposure ratio and writes the display image through the tone curve in the same pass: the curve is free. DLSS replaces the resolve on NVIDIA. |
| app | overlay / present | 0.01 / 0.00 | 3 % | The profiler itself; the present transition is free. |
| cpu | record / submit + present | 0.10 / 0.10 | — | Record includes compiling the graph (26 passes, 49 barriers) and the exposure update (a 256-bin walk). At 3 000 fps the driver's submit and present are a third of the frame; a render thread and fewer, larger submissions fix that when it matters. |
| cpu | wait for GPU (frame slot) | 0.15 | — | The frame is GPU-bound: the main thread's 0.20 ms of work finishes first and waits for the slot. |

**Since this table (2026-09-25):**
- Bloom (#44, `post/bloom` 0.04 ms) brought the 1500-frame average from 0.337 to 0.387 ms.
- The textured rock and the sun's ray-traced shadows (#46, D-029) bring it to 0.444 ms
  (0.393 with both off, the same build). `shading/standard` goes 0.041 → 0.078 ms, half for
  the textures and half for the rays, and `shading/ice` 0.010 → 0.016 ms.
- The structures are built once at start: 267 k BLAS triangles in 8 ms, the TLAS in 1 ms,
  16 MiB in all.
- GTAO on the fill (#55, D-030) brings it to 0.554 ms: 0.085 ms of passes. The mirror-ray
  code of the city (#50) had raised `shading/standard` to 0.095 ms without AO; since #52 moved
  it to a pass of its own, 0.082 ms, and the ballad takes 0.540 ms (0.448 without AO).
- The belt's dust (#58, D-032) adds 0.105 ms: 0.538 → 0.644 ms. `dust/light` takes 0.073 of it,
  one shadow ray per froxel.
- Translucent ice (#59, D-033): `shading/ice` 0.018 → 0.065 ms, 0.694 ms in all.
- The rock chunks (#60) simplify better than the round rocks: 0.709 → 0.665 ms over 600 frames.
- Ice of three densities (#61): no measurable cost, `shading/ice` 0.056–0.058 ms either way.
- The denser belt (#23): 10 000 asteroids in 28 shapes, 0.665 → 0.808 ms. The shapes cost nothing measurable;
  the extra asteroids add shading, dust shadow rays and geometry.
- The weathered crust (#62): its sections add seams, 12.6 → 13.2 M cluster slots, and no
  measurable GPU time (0.831–0.837 ms with and without, alternating runs).
- The ice blocks (#63): 42 meshes, the build 1.8 → 2.4 s; the GPU 0.815 → 0.80 ms, as the smoother blocks
  simplify better.

The software rasteriser (issue #3) does not run in this frame. The ballad holds 0.08 M
triangles in dense clusters, and auto mode starts at 1.5 M. Forced on, the frame costs
0.374 ms against 0.364 forced off. At full detail (`--no-lod`) it halves the geometry:
4.37 → 2.02 ms.

Counters: graph 27 passes (with the overlay), 40 image + 10 memory barriers, transients 4
images, 32.8 MB requested in a 26.2 MB heap (the visibility buffer and the motion vectors
share 6.4 MB), 1 heap build, 0 retired; 3000 asteroids, 195 M leaf triangles, 28 k clusters
in the DAG tables, 4.6 M cluster slots; drawn 2 531 instances, 8 k meshlets, 0.63 M
triangles, mean LOD level 6.25; EV100 14.40 automatic, ACES, sun 128 klux.

**The road here (same frame):** full detail 5.5 ms → DAG with the old dispatch 4.1 ms →
exact task tables 1.09 ms → instance cull pass 0.34 ms → render graph 0.33 ms (the same
work; the graph is about correctness and structure, not speed) → sky drawn last 0.28 ms →
resolve straight into the swapchain 0.27 ms → visibility buffer 0.30 ms → physical light,
histogram exposure and tone curves 0.30–0.31 ms (structure again: the image is now a
function of light in lux, a camera value and a curve) → the planet under a physical
atmosphere 0.325 ms (0.34 facing it) → culling in compute, appending in a fixed order, drawn by
mesh shaders 0.326 ms (issue #5; the task shader 0.322 the same day, the indirect-count
fallback 0.360) → the software rasteriser for dense clusters, off here by its own measure
(0.346 → 0.350 ms, noise; issue #3). The rendering stays pixel-identical
to brute force at every step of the A/B harness, and the golden captures of the three
curves are bit-identical from run to run.

**Priority list from these numbers:** (1) nothing in this frame is worth another pass on
its own: the sky's 0.11 ms is the starfield's per-pixel price, the planet adds 0.04–0.06
when in view and #26 would take most of that back; (2) the resolve is where shading cost will grow, and material classification (#20)
keeps that growth per material; (3) the CPU submit/present path only when a real scene
makes it visible; (4) geometry is done until triangle counts rise again, and when they do the
software rasteriser takes the dense clusters by itself (full detail 2× faster); (5) DLSS stays
optional (below): its 0.45 ms only pays in a heavier scene.

### The same frame with DLSS (`--features dlss`, U; issue #8)

| anti-aliasing | drawn at | GPU per frame | geometry | sky | temporal | post | Verdict |
|---|---|---|---|---|---|---|---|
| TAA (Streamline build) | 1600×900 | 0.333 ms | 0.14 | 0.10 | 0.06 (motion 0.01, resolve 0.05) | — | The default. |
| DLAA | 1600×900 | 0.768 ms | 0.14 | 0.10 | 0.47 (DLSS 0.46) | 0.01 | DLSS at native size: the best image here, 0.44 ms more. |
| DLSS Quality | 1067×600 | 0.690 ms | 0.11 | 0.06 | 0.46 (DLSS 0.45) | 0.01 | The scene saves about 0.1 ms; DLSS costs 0.40 ms more than the TAA resolve. |
| DLSS Performance | 800×450 | 0.646 ms | 0.10 | 0.05 | 0.46 (DLSS 0.45) | 0.01 | |
| DLSS Ultra Performance | 533×300 | 0.600 ms | 0.09 | 0.03 | 0.44 (DLSS 0.43) | 0.01 | |

GPU per frame: path averages of 6000-frame runs; zones: F1 at frame 1200. **Verdict:**
the DLSS pass costs what the output costs (0.43–0.46 ms at 1600×900 whatever the input
size), so it cannot pay in a 0.33 ms scene. It pays once the native frame costs about
0.5 ms more than the frame at the input size: the million-instance city at 1440p (#13) is
where to measure that. The LOD error is scaled to output pixels, so every mode draws the
same 0.56 M triangles and the geometry passes shrink only with the pixel count. Streamline
adds about 0.07 ms of CPU to recording (0.17 against 0.10).

## `city-blocks` — a million instances (issues #33–#37 and #36, the city from its south edge)

GPU **1.12 ms** for 1 000 001 instances (a 4 km terrain; 2 304 buildings, 9 600 lamp
posts, 180 plaza props and 988 k rocks placed by a compute pass), streamed through a
512 MiB pool of which the view keeps 49 MiB: 48 k clusters and 3.32 M triangles drawn.
- **Before:** 4.96 ms before #37 packed far instances' roots 32 to a cluster-cull item.
- **Every page resident** (`--stream-pool 0`): 1.05 ms, with 1 160 MiB of geometry
  against 689.
- **CPU:** 0.25 ms of work, the rest waiting for the GPU.

| Subject | Zone | ms | share | Verdict |
|---|---|---|---|---|
| geometry | instance cull | 0.31 | 27 % | A thread per instance, a million of them, no hierarchy: cells would skip whole hills (#38). It was 0.54 before it stopped writing 526 k work items. |
| geometry | cluster cull 1 / 2 | 0.26 / 0.27 | 47 % | 30 k work items and 791 k roots (25 k items) for 48 k drawn clusters: most rocks are hidden behind the hills or the buildings, which instance occlusion would drop before any work (#38). They were 2.0 and 2.1 ms; the streamed cut adds 0.03 each (0.23 with every page resident). |
| geometry | meshlet pass 1 | 0.20 | 18 % | 3.3 M triangles in hardware: a streamed start is coarse and leaves the auto software raster off (0.16 + 0.03 with it). |
| shading | standard | 0.10 | 8 % | Textured since #20 (D-026): brick, concrete, plaster, glass, grass and rock rows, two triplanar textures and an anti-tiling noise per pixel (0.05 untextured). No ice in the city, so the ice pass dispatches nothing. |
| streaming | upload | 0.00 | 0 % | Nothing to upload once the view has settled (39 frames). The flight at 300 m/s uploads 0–1.6 pages a frame. |

**At 1440p with TAA** (#13) the flight at 300 m/s takes 1.64 ms of GPU, its worst frame
2.58 ms against the 8.33 of the 120 fps target; the south edge 1.58 ms, the orbit 2.19.
With the textured materials of #20 the flight takes 1.79 ms (shading 0.09 → 0.22), and
1.83 ms with the glass windows of #41 (D-027); the south edge 1.47 ms, its culls 0.36 and 0.38
(the windows keep the far buildings' cut a little finer: 58 k clusters instead of 50 k).
With the streets of #42 (D-028: `shading/layered` 0.03–0.05 ms) the flight takes 1.87 ms.
Under the sky of #43 (`sky/*` 0.05 ms at 900p, the compose 0.055 at 1440p) it takes 1.94 ms.
With bloom (#44, `post/bloom` 0.08 ms at 1440p) it takes 2.02 ms.
With the sun's ray-traced shadows (#45, D-029: about 0.14 ms of rays at 1440p) it takes 2.19 ms.
With the sky's light (#47: `sky/irradiance` 0.016 ms, shading unchanged) it takes 2.20 ms.
With its ambient occlusion (#48, D-030: `ao/*` 0.25 ms at 1440p) it takes 2.46 ms.
With the sky's reflection (#49, D-031: 0.03 ms of shading) it takes 2.50 ms.
With the mirror rays in the glass (#50: 0.09 ms of rays, 0.03–0.07 ms of registers across the
resolve) it takes 2.66 ms.
With soft shadows (#54: 0.02 ms) it takes 2.67 ms.
With the mirror rays moved to a pass of their own (#52: the resolve 0.523 → 0.376 ms, the
rays 0.098 ms) it takes 2.61 ms.
**Priority:** the RTX 3080 run (#39). Nothing here needs work for the target; the culls'
next step (#38) waits for a scene that does. Details in [city-blocks.md](demos/city-blocks.md).

## `meshlets` — the culling bench (static view, occlusion on, LOD 1 px)

GPU **0.20 ms** (0.197 since the material classes of #20, 0.177 with one resolve pass; 0.15 with the rocks shaded in the
mesh passes): the bench resolves the
visibility buffer into a pre-exposed HDR image at a fixed EV100 of 15 and the display pass
(AgX) writes the swapchain: 20 passes, 32 image + 6 memory barriers (issue #5 added the
clear of the instance cull's look-back words and a cluster cull before each mesh pass), three transients,
25.6 MB requested in a 19.2 MB heap since the colour image reuses the depth buffer's
memory; 15 k meshlets, 1.09 M triangles (full detail, the list reserved for it since #27:
325 k meshlets, 30 M triangles, 2.27 ms in hardware and 1.15 with the software rasteriser,
which auto mode turns on there (issue #3); without occlusion 1 140 k meshlets, 106 M
triangles, 6.41 → 2.48 ms). Through the indirect-count fallback (`--force-fallback`): 0.270 ms, the draw of pass 1
taking 0.159 ms instead of 0.071; both paths and the old task path compared in
[meshlets.md](demos/meshlets.md).

## Memory — both demos (the overlay's memory group, issue #9)

The group reads `VK_EXT_memory_budget` four times per second and on every captured frame
(the query costs 8–10 µs, 3 % of this CPU frame if it ran every frame): each heap's usage
for the whole process against the budget the OS gives it. It also shows the engine's own
allocations by category, what the process holds outside the allocator, the allocator's
blocks, and the host's writes to and reads from GPU-visible memory per frame. The exit log
prints the same numbers, with the traffic averaged over the run (`memory: …`). MiB
throughout; the overlay's graph counter line stays in MB.

| | asteroids | meshlets | asteroids, Streamline build: TAA | DLAA | DLSS Quality | DLSS Performance |
|---|---|---|---|---|---|---|
| VRAM used by the process (budget 14.87 GiB) | **357 MiB** (2.3 %) | **357 MiB** | 419 | 616 | 616 | 572 |
| system RAM used by the process | 77 MiB | 13 | | | | |
| allocated by the engine | 112.9 MiB | 44.2 | 120.2 | 120.2 | 91.1 | 80.7 |
| — geometry | 39.3 | 3.8 | 35.5 | 35.5 | 35.5 | 35.5 |
| — render targets | 41.5 | 16.3 | 40.3 | 40.3 | 25.8 | 19.6 |
| — transient heap | 25.0 | 18.8 | 25.0 | 25.0 | 10.5 | 6.3 |
| — GPU work buffers | 6.6 | 4.8 | 18.8 | 18.8 | 18.8 | 18.8 |
| — per-frame data, textures, staging + readback | 0.5, 0.03, 0.00 | 0.5, 0.03, 0.00 | | | | |
| allocator blocks | 384 MiB (28 % used) | 384 (11 %) | 384 | 384 | 384 | 384 |
| outside the allocator | 50 MiB | 50 | 161 | 364 | 364 | 316 |
| uploads per frame, overlay off (full overlay) | 1.20 KiB (32.3) | 1.09 KiB (32.2) | 1.11 | 1.11 | 1.11 | 1.11 |
| read back per frame | 1.03 KiB | 0.03 KiB | 1.03 | 1.03 | 1.03 | 1.03 |

1200-frame scripted runs, exit log; the meshlets bench and the TAA build with the overlay
off. The first two columns are re-measured after issues #5, #29 and #27. #5: the culls' look-back status
words add 2.2 and 1.5 MiB of work buffers and their argument resets 0.1 KiB of uploads per
frame. Issue #29 then dropped the task-group table no shader read any more (4 B per work
item: geometry 0.56 and 0.35 MiB less, 8 B less per frame block). #27 sized the visible-cluster
list to the demand: 65 536 slots per frame slot instead of 1 M, 15 MiB less in both (it
grows when a frame drops clusters; `--no-lod` reserves the scene's finest clusters, 2.10 M
and 1.38 M: work buffers 37.1 and 24.3 MiB). The Streamline columns are from issue #9 (with
the 1 M list). The indirect-count fallback adds 2.5 MiB of draw commands at that size
(work buffers 8.5 and 6.8 MiB; 117.2 and 76.7 under `--no-lod`).

Issue #3 added the software rasteriser's samples: 11.0 MiB of render targets at
1600×900, allocated when the GPU has 64-bit atomics even while auto mode keeps the
rasteriser off. It also made the look-back words 64-bit (work buffers 8.8 and 6.2 MiB).
Issue #33 then dropped the per-cluster visibility bits and sized the work list by demand:
work buffers 4.3 and 3.3 MiB, plus a second depth pyramid (2.7 MiB of render targets). At
a million instances that is what keeps the bench at 197 MiB instead of 2 938
(`docs/demos/meshlets.md`). Issue #37 added the root list, as long as the work list: work
buffers 6.6 and 4.8 MiB (84 MiB at `--side 700` and in the city). Issue #36 stores each
cluster's own 16-byte vertices in 128 KiB pages instead of a shared 32-byte vertex buffer
and its index lists: geometry 39.3 and 3.8 MiB (13 % more, the vertices on cluster borders
stored once per cluster). **Verdicts:**

1. **The allocator's block size, not the data, sets the VRAM figure.** `gpu-allocator`
   reserves 256 MiB device blocks and 64 MiB host-visible ones. Each demo holds one of each
   in VRAM (per-frame data sits in Resizable BAR memory, so its block is device-local too),
   plus a 64 MiB system-memory block when something reads back. The meshlets bench's 30 MiB
   and the ballad's 94 MiB both come to 357 MiB. That is harmless on a 16 GB card. The
   in-house TLSF layer of D-018 sizes its pools to what is resident when streaming arrives
   (Phase 9).
2. **Outside the allocator: 50 MiB.** The three 1600×900 swapchain images are 16.5 MiB, the
   rest is the driver. Loading Streamline adds 111 MiB even while the TAA runs. The DLSS
   feature adds 203 MiB at a 1600×900 output (DLAA and Quality alike) and 155 MiB in
   Performance mode. The upscaler's output image (12.5 MiB of render targets) is allocated
   at start-up, even when the TAA is selected; creating it on the first switch would save
   that in the default mode.
3. **Traffic is negligible.** The renderer writes 1.2 KiB per frame: camera blocks, indirect
   arguments and statistics reset, the planet. The full overlay adds 31 KiB, because it
   rewrites its whole cell grid every frame (90 MiB/s at 2 900 fps). The ballad reads back
   1 KiB per frame (the exposure histogram and the meshlet statistics). The counters are
   there for streaming, whose budget is 64 MB per frame (D-018).
4. **The warning.** With `FORGE_VRAM_BUDGET_MB=390`, the ballad's 359 MiB is 92 % of the
   budget: the VRAM heap line and its bar turn red (capture in
   [asteroids.md](demos/asteroids.md)).

## Not measured yet

- **Streaming**: residency pools, request queue depth, drive and decompression throughput.
  These arrive with the streaming work (D-018, Phase 9) as lines of the memory group.
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
