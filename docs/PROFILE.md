# Forge — Where the time goes

The on-screen profiler (F1 in every demo) mirrored here at each checkpoint, with a verdict
per item: what is expensive, why, and how it gets attacked. The overlay is the live truth;
this page is the record and the discussion.

How it measures: GPU zones are timestamp queries written at pass boundaries
(`Commands::mark("group/name")`), read back two frames later, smoothed (8 % per frame); each
submission's first timestamp waits for the earlier work on its queue, so its first zone does
not absorb the previous frame's tail. Since #77 a frame runs on up to three queues (graphics,
async compute, transfer). A zone off the graphics queue carries its queue's name
(`gi/probe rays [compute]`) and measures time shared with the graphics work beside it, so it
reads longer than the same pass alone, and the zones no longer add up to the frame. The
frame's GPU time is its span, from the first timestamp on any queue to the last, starting at
the previous frame's last timestamp when its async work began earlier (#95: that part
overlaps the previous frame, whose time already counts it). CPU zones are wall-clock spans of the main-thread frame loop. Frames overlap on
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
- At 1440p (`--width 2560 --height 1440`, since tonight's last commit) the whole ballad takes
  1.63 ms over 600 frames, against 0.80 ms at 1600 × 900: every system on, 10 000 asteroids.
- The LOD pops (#65): the chunks weigh their normals when cooked, and keep their shading detail
  until it is under a pixel.
  - Per frame at 1600 × 900: 10 k → 122 k clusters and 0.57 → 8.9 M triangles.
  - The software rasteriser now runs in auto mode: 107 k clusters in 0.20 ms.
  - The cluster culls go 0.02 → 0.14 ms each.
  - In all: 0.795 → 1.30 ms at 1600 × 900, and 1.75 → 2.63 ms at 1440p (alternating runs,
    1500 frames each).
- Hex-tiling the rock's texture (#66): nine samples per texture instead of three.
  `shading/standard` 0.133 → 0.171 ms, and 1.29 → 1.33 ms in all at 1600 × 900.
- Pass 2 rasterises its dense clusters in software too (#30), so occlusion changes no pixel.
  `meshlet pass 2` 0.014 → 0.007 ms, `software raster 2` 0.008 and the second merge 0.009.
  In all: 1.356 ms before and after at 1600 × 900, 2.750 → 2.783 ms at 1440p (the second
  merge), and 5.98 → 5.64 ms at full detail.
- Pass 2 merges by rectangles, one per software cluster (#32): the merge zone 0.058 → 0.039 ms
  at 1440p (2.735 → 2.721 ms in all) and 0.024 → 0.020 ms at 1600 × 900.
- The planet-view table (#26): a planet seen from space is two fetches from a table instead
  of a 16-segment march per pixel, and the stars are no longer worked out behind its ground.
  The sky pass facing a 50° planet goes 0.333 → 0.105 ms, and facing the ballad's 18° planet
  0.159 → 0.132. Along the path it stays 0.117 ms.
- ACES 2.0 (#76), per curve at 1440p (1500 frames, three alternating runs each), in
  `temporal/TAA resolve`:

  | Curve | TAA resolve (ms) |
  |---|---|
  | ACES fit | 0.155–0.158 |
  | AgX | 0.161–0.165 |
  | ACES 2.0, baked table (2 fetches) | 0.163–0.169 |
  | ACES 2.0, per pixel (reference) | 0.253–0.260 |

  The bench's `post/display transform` at 1600 × 900 takes 0.011 ms with the ACES fit,
  0.013 with the table and 0.055 per pixel.
- The HDR output (#94), ACES 2.0's 1000-nit table in HDR10 against SDR, the ballad's flight
  (600 frames, three alternating rounds): `temporal/TAA resolve` 0.173–0.177 → 0.182–0.187 ms
  at 1440p and 0.056–0.057 → 0.060 at 1600 × 900, the PQ encoding and its dither. The
  off-screen mode adds `post/hdr preview`, 0.038–0.042 ms at 1440p and 0.014 at 1600 × 900;
  its frame 2.616–2.648 → 2.659–2.714 ms at 1440p. Each HDR preset's table is baked once per
  process, 10–15 ms.
- MaxCLL and MaxFALL from the frame shown (#125), in every HDR mode: `post/hdr metadata
  histogram` takes 0.033–0.038 ms at 1440p and 0.015–0.016 at 1600 × 900 (the ballad, three
  runs), about what the exposure histogram costs. The first kernel, a thread per pixel with
  the largest signal kept by a shared atomic, took 0.048–0.051 ms. A 2 × 2 quad per thread (a
  quad of one bin counted with one atomic, a wave of them with one) and the largest reduced
  over the wave first took it to 0.033. Sixteen pixels per thread were slower again (0.045). The calibration page
  (`app/hdr calibration`) takes 0.029 ms at 1440p while it is open, and the meter rests then.
- Pass 2's cluster cull over pass 1's rejects only (#92): pass 1 lists the 36 k clusters the
  previous pyramid hid, and pass 2 tests those alone. Cluster cull 2 goes 0.134 → 0.017 ms,
  cluster cull 1 0.139 → 0.167; in all 1.403 → 1.308 ms at 1600 × 900 and 2.82 → 2.67 ms at
  1440p (three alternating runs each).

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

**The start view** (#121, 2026-10-01). The scene loads its first view's pages before the first
frame (473 pages, 59 MiB; 63 ms to work out over the placed instances, 44 ms to read and copy),
so the view is sharp from frame 0 and never uploads. A sharp start switches the auto software
raster on, as with every page resident, where the coarse start had kept it off: the south edge
1.823 → 1.850 ms in today's build (three rounds each against the previous commit; meshlet
pass 1 0.253 → 0.205, the software raster 0.067 and its merge 0.04 more). The orbit, the flight
and the resident city are unchanged. The island's views are unchanged (1.588 ms both ways at
its first view; with water its cluster cull 1 reads 0.008 ms more under the water's async
compute, without it 0.004 less).

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
With the probes (#53, D-036: `gi/probe *` 0.41 ms, their sampling 0.37 ms across the resolve
and the mirror rays) it takes 3.68 ms, against 2.91 ms in the same runs without them (these
runs are 3000 frames of the loop). Its worst frames take 3.94 ms. On the static south view the
probes cost 0.77 ms at 1600 × 900 (1.83 → 2.60) and 1.05 ms at 1440p (3.06 → 4.10), of which
0.46 ms is the sampling. A pass of its own saves only 0.09 ms of it (#70, measured after #68):
the lookup's cost is its own work, not the resolve's registers.
With the sky's reflection dimmed by the probes (#68: a second direction in their lookup,
`shading/standard` 0.67 → 0.80 ms) it takes 3.81 ms, against 3.66 ms in the same runs before;
the south view at 1440p 4.13 → 4.24 ms.
With instance occlusion (#38: the instances the previous pyramid hides skip pass 1, and a
second instance cull tests them against this frame's), the south view at 1600 × 900 goes
2.90 → 2.64 ms. Its culls go 1.13 → 0.87 ms (10 k roots tested instead of 791 k). The orbit
goes 3.13 → 3.03 ms; the flight, where auto leaves it off, is unchanged.
With cells of instances (#38: the table sorted along a Morton curve, a cell cull testing 64
instances at a time, and the terrain's work items written by a whole workgroup) the south
view goes 2.52 → 2.26 ms, the orbit 2.89 → 2.71, the flight 2.36 → 2.13. The instance culls
go 0.40 → 0.14 ms. The instance cull's old 0.31 ms was mostly one thread writing the
terrain's thousands of work items, not the million tests. The clears ahead of the culls now
have their own zone (`geometry/cull clears`, 0.008 ms).
With async compute (#77: the sky's tables and the probes' update on the compute queue, the
streaming copies on the transfer queue), measured against the build before in interleaved
runs (three each):

| View | before | async | `FORGE_ASYNC=0` |
|---|---|---|---|
| south view | 2.36 ms | 2.25 ms | 2.31 → 2.32 ms |
| orbit | 2.86 ms | 2.79 ms | 2.80 → 2.79 ms |
| flight | 2.23 ms | 2.15 ms | 2.21 → 2.21 ms |
| every page resident | 2.24 ms | 2.19 ms | 2.24 → 2.24 ms |

The serial column compares the two builds in a run of its own. The meshlets bench and the
ballad have no async pass and do not move (within 0.01 ms). The probes' zones read 0.60 →
1.35 ms and the geometry passes a third to a half longer, since they now share the GPU: the
frame's span is what shrank. The overlap is limited: the probes still wait for the previous
frame's resolve, which reads their atlases (#95).
Removing that wait (#95, 2026-09-30) gains nothing on the 5070 Ti. With the probe atlases and
state and the sky's tables double-buffered, frame N's probes and sky run during frame N−1's
resolve, TAA and post. The city then takes 2.06 → 2.13 ms south, 2.53 → 2.53 orbiting and
3.57 → 3.66 at 1440p (three runs each, alternating). Serially it takes 2.24 → 2.24,
2.66 → 2.63 and 3.85 → 3.75. The probe blend reads 0.34 → 0.72 ms and the resolve
(`shading/standard`) 0.35 → 0.55 ms: beside the resolve, the probes contend for the units the
geometry passes left idle. The change stays out of the engine, and its patch and numbers are
in `reports/2026-09-30-95/`. What stays is the frame's measure. A frame whose async work starts
during the previous one is timed from that frame's end, so the overlap is not counted twice
(the span read 3.90 ms for a 2.1 ms frame). Without overlap the number is the same.
The frames after a streaming copy no longer wait for it again (#104, 2026-10-02). The graph
leaves out a wait its queue already made, so the flight's frames without a copy run one
compute batch and two graphics batches, where they ran three graphics batches. Over five
alternating runs of 6 000 frames, its CPU submit + present went 0.158–0.162 → 0.149–0.150 ms
and its GPU frame 1.873–1.881 → 1.870–1.873 ms. The first run of the new build was an
outlier at 0.164 and 1.890. The other views of `tools/timings.sh` do not move (within
0.01 ms).
With pass 2's cluster cull over pass 1's rejects only (#92: pass 1 lists the 80–160 k clusters
the previous pyramid hid, pass 2 tests those alone instead of walking pass 1's work again),
against the build before in alternating runs (three each; these runs measure the same build
0.1 ms faster than those of #77):

| View | before | after | cluster cull 2 | cluster cull 1 |
|---|---|---|---|---|
| south view | 2.16 ms | 1.96 ms | 0.278 → 0.033 | 0.308 → 0.351 |
| orbit | 2.71 ms | 2.44 ms | 0.341 → 0.034 | 0.530 → 0.557 |
| flight | 2.17 ms | 2.03 ms | 0.212 → 0.056 | 0.230 → 0.269 |
| every page resident | 2.21 ms | 2.05 ms | 0.272 → 0.032 | 0.258 → 0.302 |

Pass 1 pays 0.03–0.05 ms for its second ordered append.
With #93's instance record (2026-09-26: 80 bytes in cells of 1 km with a quaternion instead of a
96-byte world matrix; the cull builds each instance's pose once per cluster), against the
build before in alternating runs (three each, medians):

| View | before | after | cluster cull 1 |
|---|---|---|---|
| south view | 1.98 ms | 1.99 ms | 0.352 → 0.327 |
| orbit | 2.42 ms | 2.36 ms | 0.535 → 0.477 |
| flight | 1.96 ms | 1.94 ms | 0.254 → 0.255 |
| every page resident | 1.96 ms | 1.94 ms | 0.300 → 0.292 |

The meshlets bench (0.225, 0.143 ms orbiting, 1.33 → 1.32 at side 700) and the ballad (1.26
and 2.67–2.68 ms) do not move. Rebuilding the quaternion's matrix at each of a cluster's five
points instead made the streamed views slower under async compute (south 1.99 → 2.18 ms, cull
1 0.35 → 0.48) while their serial time did not move: the cull overlapped the probe rays worse.
With the probes' cadence (#103, 2026-09-30), a settled probe updates every other frame on its
turn, keeping the hysteresis squared, while new and young probes update every frame. Timed
against cadence 1 and 4 on one build, three rounds alternating (medians):

| View | cadence 1 | 2 (default) | 4 | probe rays, 1 → 2 → 4 |
|---|---|---|---|---|
| south view | 2.03 ms | 1.81 ms | 1.71 ms | 0.81 → 0.62 → 0.48 |
| orbit | 2.50 ms | 2.42 ms | 2.37 ms | 0.66 → 0.56 → 0.47 |
| flight | 2.06 ms | 1.96 ms | 1.91 ms | 0.60 → 0.50 → 0.42 |
| south view, 1440p | 3.46 ms | 3.23 ms | 3.20 ms | 0.97 → 0.71 → 0.55 |

- **Why less than half the rays' time:** the rays pass still runs every probe's young and
  fixed rays each frame. The blend's groups for probes waiting their turn return at once
  (0.32 → 0.29 ms south).
- **Stability** (a static camera, TAA on, pixels changing by more than two levels over 32
  frames): the south view 0.18 → 0.21 → 0.24 %, the street view 0.027 → 0.031 → 0.034 %.
- **The look** against cadence 1 at frame 600: ꟻLIP mean 0.006 at 2 and 0.008 at 4, the
  largest 0.10–0.11. The fixed-step flight at frames 300 and 900 gives the same (0.005–0.006,
  the largest 0.09–0.12), so there is no visible lag at speed.
- **The default is 2:** most of the gain for a small rise in the slow change;
  `--probe-cadence 4` is there for a slower GPU.

**Where it stands (2026-09-25, after #77 and #92):** the flight at 1440p takes 3.38 ms of GPU
(three runs of 3000 frames: 3.37–3.39), its p99 frame 3.8 ms against the 8.33 of the 120 fps
target; 2.6 ms without the probes. It was 3.81 ms after #68.
**Priority:** the RTX 3080 run (#39). Nothing here needs work for the target. Cluster cull 1
(0.35 ms at the south view, 0.56 in the orbit) is now the largest geometry zone. Details in
[city-blocks.md](demos/city-blocks.md) and [meshlets.md](demos/meshlets.md).

## `city-blocks --island 7` — the island (#96, 2026-09-26)

The scene has three parts:
- the island: 8.4 M triangles at 8 m, streamed (drawn at 2 m since #106: 143 M triangles in 64
  tiles, "The ground in tiles" below);
- 300 000 rocks on its land;
- a flat sea plane with traced mirror rays.

No pass of its own; the city's passes at 1600 × 900, 3 000 frames:

| Zone | First view (the coast) | The whole island from the sea |
|---|---|---|
| **GPU in all** | **1.39 ms** (1.34 before hex tiling) | **1.08 ms** |
| gi/probe rays [compute] | 0.42 | 0.25 |
| gi/probe blend [compute] | 0.14 | 0.20 |
| shading/standard (the sea, the rocks) | 0.19 | 0.14 |
| shading/reflections (the sea's mirror rays) | 0.13 | 0.06 |
| shading/layered (the ground, hex-tiled) | 0.15 | 0.04 |
| geometry/software raster 1 (distant rocks) | 0.00 | 0.20 |
| geometry/cluster cull 1 | 0.06 | 0.09 |
| geometry/instance cull | 0.04 | 0.10 |

The probes are the largest part near the rocks. From the sea, the rocks go to the software
rasteriser. Before the rocks and the plane the island took 0.74 ms, and 0.84 ms at 4 m
([island.md](demos/island.md), "In the engine").

**The sea's cascades** (#105 step 1, 2026-09-30). The three FFT cascades, 256² each, are four
passes on the async compute queue. Measured before the surface drew them, without them and
with them, three runs each, from the coast view:

| | 1600 × 900 | 2560 × 1440 |
|---|---|---|
| without the waves | 1.27 ms | 2.38 ms |
| with them | 1.34 ms | 2.53 ms |
| water/fft-cols | 0.10 | 0.10 |
| water/fft-rows | 0.07 | 0.06 |
| water/evolve | 0.03 | 0.03 |
| water/derive | 0.02 | 0.01 |

- **The research's estimate:** 0.1–0.3 ms, hidden behind the geometry. They cost the frame
  0.07 ms at 900p and 0.15 ms at 1440p, where there is more graphics work to share with.
- **The lever:** the column pass reads every line with a stride of a whole row, where the row
  pass reads contiguously. A transpose through groupshared memory would make it coalesced.
- **The cascades' memory:** 12 MiB.

**The sea's surface** (#105 step 2, `--water`, 2026-09-30). The cascades gain mip chains
(`water/mips`), and the sea is drawn by `water/scene-copy` and `water/surface` in place of the
stand-in plane. Three runs each, medians:

| View | stand-in | water | water/surface | the compute chain |
|---|---|---|---|---|
| coast, 1600 × 900 | 1.26 ms | 1.43 ms | 0.07 | 0.22 |
| from the sea, 1600 × 900 | 1.11 ms | 1.32 ms | 0.09 | 0.26 |
| coast, 2560 × 1440 | 2.38 ms | 2.66 ms | 0.12 | 0.25 |

The stand-in's traced reflections (0.13 ms) go with it. Its frame-to-frame stability is in
[island.md](demos/island.md), "The surface".

**The island in the water** (#105 step 3, 2026-09-30). The surface writes a mirror ray request
per pixel, and `water/reflections` traces them against the TLAS (one thread a pixel, the hit
lit through a shadow ray and the probes). Three runs each, alternating: the previous commit, this
one with the rays off (**Y**), and with them.

| View | previous commit | rays off | rays on | water/reflections |
|---|---|---|---|---|
| coast, 1600 × 900 | 1.47 ms | 1.56 ms | 1.64 ms | 0.17 |
| from the sea, 1600 × 900 | 1.44 ms | 1.40 ms | 1.44 ms | 0.09 |
| coast, 2560 × 1440 | | 2.76 ms | 3.19 ms | 0.43 |

- **The request target** costs `water/surface` at most 0.003 ms. The frames with the rays
  off scatter on both sides of the previous commit's: these views vary by about ±0.07 ms from
  run to run (the GPU's clock moves every zone together).
- **The rays** cost what their zone says: they grow with the water's pixels (0.43 ms for about
  1.8 M at 1440p). The stand-in's rays took 0.13 ms at the coast, over a flat mirror whose
  rays stay coherent; the waves scatter them.
- **Step 2's numbers** (1.43 and 1.32 ms) were taken on a quieter GPU: the same build measures
  1.47 and 1.44 ms today, while the stand-in measures as it did (1.25–1.31 and 1.12 ms).
- **The levers,** if the rays' cost matters: trace at half resolution and let TAA fill in, or
  keep the 300 000 rocks out of the water's rays with an instance mask.

**The island's shadow on the water** (#105 step 4, 2026-09-30). The surface also asks for a
shadow ray per pixel, and `water/reflections` traces it beside the mirror ray. Three runs each,
alternating, the previous commit against this one:

| View | previous commit | with the shadow rays | water/reflections |
|---|---|---|---|
| coast, 1600 × 900 | 1.62 ms | 1.65 ms | 0.17 → 0.20 |
| from the sea, 1600 × 900 | 1.42 ms | 1.44 ms | 0.09 → 0.11 |
| north-west into a 10° sun | 1.45 ms | 1.52 ms | 0.07 → 0.14 |
| coast, 2560 × 1440 | 3.10 ms | 3.21 ms | 0.42 → 0.49 |

- **A shadow ray costs less than a mirror ray** (the first hit ends it, and no hit is shaded):
  0.03 ms of the frame at 900p, 0.07 at 1440p.
- **Into a low sun** they cost the most, 0.07 ms: the rays from the water in front of the
  island cross its slopes and rocks on their way to the sun, where elsewhere they leave the
  scene at once.

**The shore's damping** (#105 step 5, 2026-09-30). The surface reads the floor's depth per
vertex and fragment and damps each cascade by it: `water/surface` 0.074 → 0.082 ms at the
coast and 0.090 → 0.101 ms from the sea; the frame 1.64 → 1.66 and 1.45 → 1.46 ms (three runs
each, alternating).

**The shore's waves** (#105 step 5, 2026-09-30). Three trains per vertex and pixel near the
shore (a table read each, the coast distance's gradient, two value noises), the foam's pattern
where there is foam. Against step 4 (before the damping), three runs each, alternating:

| View | water/surface | frame |
|---|---|---|
| coast | 0.076 → 0.136 ms | 1.65 → 1.72 ms |
| from the sea (the whole coast's shallows) | 0.090 → 0.164 ms | 1.45 → 1.52 ms |
| the beach from 60 m up | 0.095 → 0.204 ms | 1.64 → 1.76 ms |

- **Where it goes:** the vertex shader's trains 0.03 ms and the fragment's 0.03 ms from the sea,
  measured by switching each off. The trains skip the water deeper than they reach (45 m for
  the 12 s train), which leaves the shallows all round the island in the sea view.
- **The levers:** an index buffer for the surface (it is drawn without one, so each vertex is
  shaded six times), and the trains starting nearer the shore.

**The wet sand** (#105 step 5, 2026-09-30). The layered ground reads the shore's field once a
pixel and, within the swash's reach above the sea, its run-up at 14 past instants (three trains
each). `shading/layered` +0.012 ms at the coast (0.382 → 0.396), nothing measurable from the sea,
+0.045 ms above the beach (0.465 → 0.510), where most pixels are within reach. Three runs each,
alternating, on a slightly slower GPU than the runs above.

**The rivers** (#105 step 6, 2026-10-01). 43 ribbons of 16 397 points, four quads across,
drawn in `water/surface` after the sea: 390 000 vertices, each reading the ground's heights
twice. Their mirror and shadow rays join the sea's in `water/reflections`. Three runs each,
alternating, against the wet sand's commit (the waves held at 12 s):

| View | frame | water/surface | water/reflections |
|---|---|---|---|
| coast (a river mouth far off) | 1.721 → 1.736 ms | 0.136 → 0.153 | 0.208 → 0.208 |
| the largest river's valley from 200 m | 1.809 → 1.840 ms | 0.142 → 0.162 | 0.120 → 0.121 |
| two streams on the plain from 40 m | 1.536 → 1.577 ms | 0.058 → 0.079 | 0.017 → 0.035 |
| a stream from 16 m | 1.457 → 1.541 ms | 0.056 → 0.082 | 0.017 → 0.050 |
| the island from 2.5 km up | 2.176 → 2.206 ms | 0.179 → 0.195 | 0.080 → 0.081 |

- **The vertices** cost about 0.017 ms wherever the camera is: every ribbon is drawn every
  frame, and the clipper throws away what is off screen. The lever: a draw per river, or per
  stretch of 64 points, culled by its bounds. (Done 2026-10-01: runs of 64 segments culled
  against the frustum on the CPU, neighbours merged into one draw: `water/surface` 0.080 →
  0.070 ms for the stream from 16 m, 0.099 → 0.090 down a river, 0.094 → 0.086 over a lake;
  unchanged where the whole island is in view; the images the same to the pixel.)
- **The rays** grow with the river's pixels, as the sea's do: 0.03 ms for the stream seen from
  16 m.

**The rivers' beds, mouths and stones** (#105, 2026-10-01). The channels carved on cells of a
metre (36 267 cells of 8 m refined, 2.9 M fine vertices in the island's mesh), a gravel layer
under the water, the ribbons level in the channels, the sea's shading mixing the rivers in at 25
mouths (a grid of 128 m cells lists the mouths reaching each), 2 525 stones as boulder instances
and in the ribbons' flow. Three runs each, alternating, against step 6's commit:

| View | frame | water/surface | water/reflections | shading/layered |
|---|---|---|---|---|
| coast | 1.662 → 1.695 ms | 0.148 → 0.154 | 0.196 → 0.200 | 0.342 → 0.343 |
| the valley from 200 m | 1.738 → 1.786 ms | 0.153 → 0.164 | 0.115 → 0.119 | 0.444 → 0.447 |
| the plain from 40 m | 1.495 → 1.530 ms | 0.073 → 0.076 | 0.033 → 0.034 | 0.380 → 0.397 |
| the stream from 16 m | 1.439 → 1.471 ms | 0.078 → 0.082 | 0.044 → 0.043 | 0.349 → 0.369 |
| the island from 2.5 km | 2.082 → 2.122 ms | 0.185 → 0.191 | 0.078 → 0.081 | 0.291 → 0.295 |
| down a river from 3 m | 1.476 → 1.565 ms | 0.089 → 0.092 | 0.137 → 0.120 | 0.288 → 0.377 |
| the mouth from 110 m | 1.720 → 1.777 ms | 0.157 → 0.166 | 0.113 → 0.123 | 0.538 → 0.556 |

- **The bed's layer** blends with the grass's, two textured materials a pixel: +0.09 ms of
  `shading/layered` where it fills the view.
- **The plume** loops over the mouths the pixel's cell lists: 0.006 ms over the coast's sea (a
  loop over all 25 mouths cost 0.04 ms).
- **The rest** is geometry: the refined cells, and the stones drawn and traced.

**The lakes** (#105 step 7, 2026-10-01). 15 planes at their levels, clipped to their masks and
drawn in `water/surface` between the sea and the rivers, shaded as the rivers' water; their
shores refined to a metre (51 958 refined cells in all). Three runs each, alternating, against
the rivers' commit:

| View | frame | water/surface | water/reflections | shading/layered |
|---|---|---|---|---|
| the west lake from 30 m | 1.559 → 1.611 ms | 0.071 → 0.088 | 0.033 → 0.067 | 0.362 → 0.377 |
| the east lake | 1.411 → 1.454 ms | 0.070 → 0.080 | 0.024 → 0.047 | 0.302 → 0.324 |
| the round lake | 1.490 → 1.577 ms | 0.071 → 0.094 | 0.024 → 0.075 | 0.320 → 0.375 |
| coast | 1.684 → 1.676 ms | 0.155 → 0.155 | 0.198 → 0.186 | 0.341 → 0.337 |
| the island from 2.5 km | 2.125 → 2.135 ms | 0.194 → 0.195 | 0.081 → 0.082 | 0.295 → 0.293 |

- **The rays** are most of a lake's cost: a mirror and a shadow ray per pixel of water.
- **The silt** under the shallows blends with the grass's layer: up to +0.055 ms of
  `shading/layered` where a shore fills the view.

**The coast's definition** (#106, 2026-10-01). The shore smoothed in the field, its contours'
cells refined (76 856 refined cells in all), and the island's layer lookup wandering by a texel.
Three runs each, alternating, against the lakes' commit: the first view 1.691 → 1.709 ms, along
the beach from 8 m 1.864 → 1.887 ms, over the south beach from 60 m 1.799 → 1.826 ms, the island
from 2.5 km 2.132 → 2.152 ms. The wandering lookup costs up to 0.02 ms of `shading/layered` (two
noise lookups a pixel of the island's ground); the rest is the refined cells.

**The sand's top by height** (#106, 2026-10-01): the layered pass draws the sand under 2.5 m from
the ground's height under each pixel (`RenderLayer::contour`). Three runs each, alternating,
against 3c651fd: `shading/layered` +0.010 to +0.023 ms over six views (the first view 0.356 →
0.374 ms, the island from 2.5 km 0.309 → 0.332). The noise runs only within the contour's wander
and band (the first version paid for it on every pixel, up to +0.033 ms).

**The sea's surface indexed** (#105, 2026-10-01): each clipmap level's lattice points are its
vertices, indexed by its quads, in blocks of 32 × 32 quads drawn only when their box can show
(the vertex shader ran 1.28 M times a frame, six a quad, the quads under the finer level
included). The same images to the bit. Three runs each, alternating, against 2ccd23c:

| View | frame | water/surface |
|---|---|---|
| coast (the first view) | 1.712 → 1.668 ms | 0.149 → 0.101 |
| the island from 2.5 km | 2.168 → 2.123 ms | 0.190 → 0.144 |
| along the beach from 8 m | 1.849 → 1.804 ms | 0.148 → 0.105 |
| the coast east from 70 m | 1.661 → 1.617 ms | 0.111 → 0.065 |
| over the south beach from 60 m | 1.768 → 1.724 ms | 0.191 → 0.145 |
| the round lake | 1.582 → 1.549 ms | 0.086 → 0.051 |

**The island's water at 2560 × 1440** (2026-10-01, after the rivers' beds, the lakes, the coast,
the ground's smoothing, the rivers' culling; `--water`, three runs each, the waves held at 12 s):

| View | frame | water/surface | water/reflections | shading/layered |
|---|---|---|---|---|
| coast (the first view) | 3.05 ms | 0.242 | 0.411 | 0.890 |
| the stream from 16 m | 2.64 ms | 0.089 | 0.090 | 0.909 |
| down a river from 3 m | 2.97 ms | 0.136 | 0.394 | 0.924 |
| the stone in the fastest water, at the largest mouth | 3.41 ms | 0.295 | 0.514 | 1.153 |
| a lake from 3 m over its water | 2.90 ms | 0.176 | 0.457 | 0.865 |
| the island from 2.5 km | 3.50 ms | 0.350 | 0.195 | 0.723 |

The water's rays are the larger share wherever water fills the view (a mirror and a shadow ray a
pixel); the surface pass stays under 0.35 ms.

The same views after the swash's line, the sand's contour, the indexed surface and the refracted
path (2d3973a; three runs each, the waves held at 12 s):

| View | frame | water/surface | water/reflections | shading/layered |
|---|---|---|---|---|
| coast (the first view) | 3.03 ms | 0.186 | 0.409 | 0.932 |
| the stream from 16 m | 2.61 ms | 0.054 | 0.089 | 0.926 |
| down a river from 3 m | 2.94 ms | 0.100 | 0.394 | 0.930 |
| the stone in the fastest water, at the largest mouth | 3.50 ms | 0.258 | 0.511 | 1.284 |
| a lake from 3 m over its water | 2.88 ms | 0.141 | 0.458 | 0.876 |
| the island from 2.5 km | 3.51 ms | 0.305 | 0.195 | 0.785 |

- The indexed surface takes 0.035 to 0.056 ms off `water/surface`.
- The sand's contour adds 0.011 to 0.131 ms to `shading/layered`: more at 1440p than the 1600 ×
  900 runs showed (0.010 to 0.023), and most where the beach fills the view (the stone's view at
  a mouth). Most of it is the contour's code in the pass, not its work: without the third
  layer's block the compiler drops much of the bookkeeping, and `shading/layered` falls by 0.03
  to 0.065 ms even in the island's view, where three layers meet in few pixels. Shading that
  third layer as a flat colour saved nothing. Restructuring the block (fewer live arrays) is the
  lever. (Not the arrays but the third layer itself: "The contour's third layer" below, #111.)

**The coastal plain's rivers** (#112, #113, #114, 2026-10-01): da572f7 (the plain) against
78f46a0 (the water seen up a steep valley from low, the beds per pixel, the banks by the bend,
the riparian strip); the waves held at 12 s, alternating runs.

| View | 1600 × 900, frame (3 runs) | `shading/layered` | 2560 × 1440, frame (2 runs) | `shading/layered` |
|---|---|---|---|---|
| coast (the first view) | 1.668 → 1.668 ms | 0.325 → 0.326 | 3.118 → 3.119 ms | 0.851 → 0.850 |
| the island from 2.5 km | 2.232 → 2.220 ms | 0.338 → 0.333 | | |
| a river on the plain from 40 m (`-929,45,-4399,83,-40`) | 1.753 → 1.764 ms | 0.620 → 0.625 | 3.557 → 3.580 ms | 1.629 → 1.649 |
| down a lowland river from 3 m (`-929,15.2,-4399,83,-10`) | 1.481 → 1.472 ms | 0.327 → 0.313 | | |
| up a steep river from 2 m (`-1112,80.1,-3083,-97,4`) | 2.007 → 1.963 ms | 0.658 → 0.582 | 4.091 → 3.906 ms | 1.683 → 1.423 |
| a lake entry from 50 m (`-1700,50,-3072,89.9,-62`) | 1.785 → 1.809 ms | 0.445 → 0.475 | 3.725 → 3.790 ms | 1.199 → 1.295 |

- Up the steep river the water now covers the valley's floor: less ground shaded, more water
  (`water/reflections` 0.207 → 0.230 ms, `water/surface` 0.064 → 0.067).
- Over the lake entry the riparian strip's edges add layer pairs: `shading/layered` +0.03 ms,
  +0.1 ms at 1440p (the third layer's cost, #111).
- **Stability** (pixels changing by more than two levels, still camera, TAA on): the same
  within 0.01 % on every view but up the steep river, 0.26 → 0.36 % from one frame to the next
  (the ripples of the water now drawn there); after 32 frames under 0.002 % everywhere.

**The rivers sized by D-041's regional curves, `k` = 3** (#112, 2026-10-01): the same build
with `--river-k 0` (the catchment's square root) against the default; the same views, three
runs each, alternating, 1600 × 900.

| View | frame | `water/reflections` | `water/surface` | `shading/layered` |
|---|---|---|---|---|
| coast (the first view) | 1.653 → 1.668 ms | 0.127 → 0.138 | 0.119 → 0.120 | 0.334 → 0.334 |
| the island from 2.5 km | 2.164 → 2.178 ms | 0.083 → 0.081 | 0.161 → 0.163 | 0.322 → 0.325 |
| a river on the plain from 40 m | 1.688 → 1.690 ms | 0.026 → 0.029 | 0.028 → 0.030 | 0.634 → 0.635 |
| down a lowland river from 3 m | 1.451 → 1.447 ms | 0.059 → 0.075 | 0.043 → 0.049 | 0.319 → 0.320 |
| up a steep river from 2 m | 1.879 → 1.973 ms | 0.216 → 0.284 | 0.061 → 0.072 | 0.550 → 0.540 |
| a lake entry from 50 m | 1.769 → 1.758 ms | 0.212 → 0.211 | 0.108 → 0.109 | 0.456 → 0.448 |

- The cost follows the water on screen: up the steep river the wider water fills more of the
  valley's floor.
- **Stability** (still camera, TAA on): the same on every view but up the steep river, 0.41 →
  0.66 % from one frame to the next (more rippling water); after 32 frames under 0.001 %.

**The rivers' valleys** (#116, 2026-10-01): the same build with `--no-valleys` against the
default; three rounds of every view without then with (a cook per switch), 1600 × 900.

| View | frame | `water/reflections` | `water/surface` | `shading/layered` |
|---|---|---|---|---|
| coast (the first view) | 1.668 → 1.657 ms | 0.138 → 0.131 | 0.118 → 0.120 | 0.334 → 0.333 |
| the island from 2.5 km | 2.176 → 2.174 ms | 0.082 → 0.084 | 0.162 → 0.160 | 0.326 → 0.326 |
| a river on the plain from 40 m | 1.694 → 1.693 ms | 0.029 → 0.030 | 0.030 → 0.031 | 0.635 → 0.633 |
| down a lowland river from 3 m | 1.468 → 1.438 ms | 0.077 → 0.061 | 0.049 → 0.044 | 0.324 → 0.319 |
| up a steep river from 2 m | 2.022 → 1.971 ms | 0.296 → 0.224 | 0.072 → 0.061 | 0.568 → 0.604 |
| a lake entry from 50 m | 1.803 → 1.790 ms | 0.224 → 0.215 | 0.110 → 0.109 | 0.469 → 0.469 |
| the logged slot from 3 m (`-3083,48.3,-376,123.7,-10`) | 1.916 → 1.959 ms | 0.252 → 0.262 | 0.060 → 0.060 | 0.538 → 0.570 |

- The views move with what is on screen: up the steep river, less water and more of its
  floor's ground; in the slot, the walls' foot.
- At start the carve takes 0.57 s, and the island's cook 30.3 → 30.9 s.

**The steep valleys' look** (#118, 2026-10-01): d6699a7 (the baseline worktree) against the
gravel, scree and scrub layers with the bank stones and the rubble; three runs each,
alternating, 1600 × 900.

| View | frame | `shading/layered` | `water/reflections` |
|---|---|---|---|
| coast (the first view) | 1.658 → 1.664 ms | 0.332 → 0.334 | 0.130 → 0.130 |
| the island from 2.5 km | 2.171 → 2.178 ms | 0.324 → 0.325 | 0.084 → 0.084 |
| a river on the plain from 40 m | 1.693 → 1.695 ms | 0.631 → 0.636 | 0.030 → 0.030 |
| up a steep river from 2 m | 1.960 → 2.057 ms | 0.599 → 0.681 | 0.223 → 0.228 |
| the logged slot from 3 m | 1.955 → 2.016 ms | 0.565 → 0.617 | 0.263 → 0.267 |
| the slot from 40 m (`-3083,88.3,-376,123.7,-45`) | 1.881 → 1.936 ms | 0.689 → 0.739 | 0.049 → 0.050 |
| the hills from 250 m (`-3083,250,-376,123.7,-25`) | 1.609 → 1.635 ms | 0.452 → 0.472 | 0.039 → 0.047 |

- The valleys' views pay in `shading/layered`: more of their pixels now shade two layers
  (gravel, scree, scrub and rock). A third layer comes only from the sand's contour (#111).
- At start the three texture sets take 210 ms and the painting 0.14 s.

**The steep rivers' steps and pools** (#122, 2026-10-01): the same build with `--no-steps`
against the default; two rounds of every view without then with (a cook per switch), 1 500
frames each, 2560 × 1440.

| View | frame | `water/surface` | `water/reflections` | `shading/layered` | `geometry/instance cull` |
|---|---|---|---|---|---|
| a stream near the head from low (`-939,325.0,-177,126.5,-6`) | 2.822 → 2.823 ms | 0.049 → 0.069 | 0.109 → 0.069 | 0.952 → 0.935 | 0.060 → 0.061 |
| up a steep river from 2 m | 4.946 → 4.196 ms | 0.240 → 0.185 | 1.278 → 0.954 | 1.809 → 1.174 | 0.044 → 0.045 |
| the island from 2.5 km | 3.497 → 3.588 ms | 0.343 → 0.373 | 0.188 → 0.189 | 0.704 → 0.703 | 0.103 → 0.113 |
| the island from 2.5 km, fewer stones | 3.620 → 3.675 ms | 0.361 → 0.389 | 0.194 → 0.194 | 0.734 → 0.737 | 0.113 → 0.115 |

- Those three rows had a row of boulders on every lip (25 740 stones); the last has one or
  two on three lips in five (5 640 stones, after the owner's look), timed later in the session
  (both columns a little slower than the rows above).
- Up the steep river a lip's boulders now stand in front of the camera and hide much of the
  water and the valley's floor: the view is not the same scene.
- The river points are 38 534 (17 652): from 2.5 km `water/surface` takes 0.03 ms more for the
  steps' segments. The stones' cull took 0.01 ms more with the rows, nothing measurable now.
- At start the steps take no measurable time; the island's tiles cook again when the flag
  changes (their key holds the rivers' parameters).

**Fewer, larger rivers** (#123, 2026-10-01): the same build with `--island-basins 0
--island-grade 0 --no-brooks` (the island before) against the default, a different island;
two rounds of each view without then with, 1 500 frames each, 2560 × 1440.

| View | frame | `water/surface` | `water/reflections` | `shading/layered` | software raster 1 |
|---|---|---|---|---|---|
| the island from 2.5 km (`-6500,2500,-1416,-90,-35`) | 3.539 → 3.555 ms | 0.368 → 0.356 | 0.186 → 0.187 | 0.705 → 0.698 | 0.748 → 0.788 |
| the plain from 200 m (`0,200,5400,0,-8`) | 2.581 → 2.627 ms | 0.153 → 0.155 | 0.124 → 0.120 | 0.699 → 0.693 | 0.129 → 0.214 |

- From 2.5 km the second round; in the first the GPU ran slower both ways (3.620 → 4.083 ms,
  every zone higher).
- Over the plain the software raster draws more of the new ground in view, the island's
  shape, not a cost of the rivers: 32 259 river points (38 534), 4 226 stones (5 640).
- At start the island's heightfield is generated once in 7.1 s, then read from the cache.

**Under the water** (#108, 2026-10-01): the previous commit against this one, two rounds of
each view, 1 500 frames each, 2560 × 1440. Within 20 m of the sea's level the frame finds the
water at the camera (`water/at-camera`), draws the sea with the shader that can see it from
below, and runs `water/under` indirectly: none of its groups while the near plane stands over
the water.

| View | frame | `water/surface` | `water/at-camera` | `water/under` |
|---|---|---|---|---|
| the island from 2.5 km (`-6500,2500,-1416,-90,-35`) | 3.508 → 3.505 ms | 0.355 → 0.353 | — | — |
| the plain from 200 m (`0,200,5400,0,-8`) | 2.629 → 2.608 ms | 0.158 → 0.156 | — | — |
| the largest mouth from 4 m (`4384,4.0,-2840,-88.9,-20`) | 3.239 → 3.263 ms | 0.434 → 0.455 | 0.008 | 0.005 |
| 3 m under the sea, across the floor (`4770,-3.0,-2847,91.1,-10`) | 3.152 ms | 0.442 | 0.018 | 0.095 |

- Over the water within reach, the surface's shader holds the view from below beside the one
  from above: 0.02 ms more at the mouth.
- Under the water, the surface shades from above first in every pixel (a quad's derivatives),
  then from below, and asks for no rays (`water/reflections` 0.04 ms there).
- `water/at-camera` is one group of four threads, the waves stepped back four times to the
  point they carry over the camera, then a corner of the mesh's quad each.

**The caustics** (#108, 2026-10-02): `--no-caustics` against the default, the same build, two
rounds of each view, 1 500 frames each, 2560 × 1440. The floor under the sea samples the slopes
of two cascades four times each, in the layered pass.

| View | `shading/layered` | frame |
|---|---|---|
| coast (the first view, 25 m up) | 1.084 → 1.146 | 3.419 → 3.485 ms |
| the island from 2.5 km | 0.639 → 0.632 | unchanged |
| 3 m under the sea, across the floor | 0.818 → 0.903 | 2.970 → 2.969 ms |
| 6 m under, the floor filling the view | 1.019 → 1.136 | 2.911 → 3.049 ms |

- From 2.5 km the floor's pixels span metres, where the caustics fade out and are skipped.
- `FORGE_SHADER_STATS=resolve_layered`: 96 registers before and after, the binary 2 KB
  larger.
- These rounds ran about 0.25 ms faster in every view than those of the table above (the GPU's
  clocks); compare within a table.

**Under the lakes and the rivers** (#108, 2026-10-02), two rounds, 2560 × 1440:
- 9 m under the largest lake looking up (`2248,20.2,-1184,0,25`): 1.99 ms, `water/under`
  0.080, `water/at-camera` 0.007.
- 1.5 m under it looking across (`2248,27.5,-1184,0,0`): 2.43 ms, `water/under` 0.082.
- In the largest river by its mouth (`4384,-0.2,-2840,-88.9,5`): 2.08 ms, `water/under` 0.084.
- Over the water the rivers and the lakes keep the shader seen from above alone, unless the
  camera stands in or over one of them or within 2 m of the sea: the views of the table above
  are unchanged.

**Moving geometry** (#79, 2026-10-02): `--movers` 0, 1 000 and 10 000 barrels on the rivers, the
same build, two rounds, 1 500 frames each, 2560 × 1440.

| View | frame: 0 → 1 000 → 10 000 | `movers/motion` | `movers/upload` | `geometry/instance cull` |
|---|---|---|---|---|
| the first barrel from 4 m (`-238.2,318.14,-1843.9,135.2,-18.1`) | 3.62 → 3.64 → 3.71 ms | 0.032 | 0.002 → 0.006 | 0.048 → 0.055 → 0.059 |
| the largest mouth from 4 m (`4384,4.0,-2840,-88.9,-20`) | 3.05 → 3.15 → 3.13 ms | 0.032 | 0.002 → 0.006 | 0.031 → 0.036 → 0.039 |

- `movers/cell bounds` 0.003 ms whatever the count; `movers/motion` is a pass over the screen,
  its cost the pixels', not the movers'.
- The rounds' frame times vary by up to 0.09 ms with 10 000 movers (3.759 and 3.666 ms).

With the movers' acceleration structure (the second step), the same views and rounds:

| View | frame: 0 → 1 000 → 10 000 | `movers/tlas` | `water/reflections` | `gi/probe rays` | `shading/layered` |
|---|---|---|---|---|---|
| the first barrel from 4 m | 3.61 → 3.92 → 4.00 ms | 0.139 → 0.178 | 0.581 → 0.683 → 0.682 | 0.532 → 0.596 → 0.556 | 1.096 → 1.168 → 1.151 |
| the largest mouth from 4 m | 3.06 → 3.35 → 3.41 ms | 0.140 → 0.179 | 0.343 → 0.454 → 0.481 | 0.406 → 0.519 → 0.472 | 1.009 → 1.069 → 1.051 |

- About 0.3 ms with 1 000 movers: the build, 0.14 ms, then a second traversal for every ray,
  which the research put at 10–30 % of the ray passes (the reflections +17–32 %, the probes'
  rays +13–28 %, the shading's shadow rays +6 %).
- Going from 1 000 movers to 10 000 adds little: the traversal is the cost, not the movers.
- The build on the async compute queue gained nothing (3.89–3.98 ms against 3.91–3.93), as
  for the probes on this GPU (#95); it stays on the graphics queue.
- The probes' wake (#69, the third step), `gi/probe wake` on the compute queue: 0.010 ms with
  1 000 movers and 0.008 with 10 000 (600 frames from the largest mouth).

**A refit of the movers' structure against its rebuild** (#79's measure, 2026-10-02;
`FORGE_TLAS_REFIT=1`: built once with `ALLOW_UPDATE`, then updated in place). The same views,
two rounds of 1 500 frames, 2560 × 1440:

| View, movers | frame: rebuild → refit | `movers/tlas` | `water/reflections` | `gi/probe rays` |
|---|---|---|---|---|
| barrel, 1 000 | 4.07–4.15 → 4.01–4.07 ms | 0.146 → 0.019 | 0.69 → 0.69 | 0.64–0.67 → 0.50–0.51 |
| barrel, 10 000 | 4.18–4.28 → 4.13–4.21 ms | 0.199 → 0.028–0.030 | 0.71–0.72 → 0.76–0.77 | 0.65–0.67 → 0.54–0.55 |
| mouth, 1 000 | 3.54–3.60 → 3.50 ms | 0.146 → 0.019 | 0.47 → 0.47 | 0.47 → 0.35 |
| mouth, 10 000 | 3.66–3.69 → 3.64–3.73 ms | 0.198 → 0.030–0.033 | 0.49 → 0.52 | 0.50 → 0.39–0.40 |

- The update costs a seventh of the build.
- The tree it keeps updating degrades as the movers travel. With 10 000 the reflections cost
  0.03–0.05 ms more after 25 s, and would cost more the longer the run.
- The probes' rays on the compute queue take 0.1 ms less, likely because they trace the
  movers' structure and wait less for an update than for a build.
- The frame gains 0.03–0.10 ms with 1 000 movers and nothing beyond the rounds' spread with
  10 000. So the rebuild stays, as the research and the vendors advise for a TLAS. A refit with
  a rebuild every few hundred frames is the option if the frame ever needs that 0.1 ms.

**Built to trace fast** (`FORGE_TLAS_FAST_TRACE=1`: `PREFER_FAST_TRACE` instead of
`PREFER_FAST_BUILD`, which NVIDIA's 2022 best-practice post advises for a TLAS rebuilt every
frame, `docs/research/dynamic-scenes.md`). The same views and rounds:
- `movers/tlas` is the same: 0.144–0.146 against 0.147–0.150 ms with 1 000 movers, and
  0.203–0.207 against 0.196–0.200 with 10 000.
- The reflections and the probes' rays stay within the rounds' spread.
- The frame gains nothing: from the barrel with 10 000 it is slower (4.10–4.16 → 4.19–4.23 ms),
  from the mouth the same (3.67–3.73 → 3.66).
- With 10 000 instances or fewer the choice does not show in the traces on this GPU: the fast
  build stays, the flag remains for the A/B.

**Ships in the ballad** (#79's demo, `asteroids --ships N`; `docs/demos/asteroids.md`), the
ballad's flight, two rounds:

| Ships | frame | `movers/tlas` | `movers/motion` |
|---|---|---|---|
| 0 | 2.456–2.463 ms | — | — |
| 24 | 2.564–2.567 ms | 0.053–0.054 | 0.040 |
| 1 000 | 2.318–2.329 ms | 0.118 | 0.087–0.088 |

With 1 000 the ships crowd the corridor and hide rocks, so the frame is cheaper than without.

**Objects in the rivers** (#107, 2026-10-02): the water parting around the floaters, 1 000
barrels with and without `--no-floaters`, two rounds, 1 500 frames each, 2560 × 1440.

| View | `water/surface`: without → with the floaters | frame |
|---|---|---|
| the moored barrel from above (`-1954.0,51.69,249.9,77.1,-49.6`) | 0.069 → 0.081 ms | 3.38 → 3.39 ms |
| the largest mouth from 4 m (`4384,4.0,-2840,-88.9,-20`) | 0.451 → 0.446–0.479 ms | 3.47 → 3.42–3.51 ms |

- A pixel looks at the floaters its cell of a 16 m grid lists. Looping over all 64 in every
  river pixel cost 0.10 ms from the moored barrel and 0.34 ms from the mouth.

**Wakes in still water** (#107's second part, 2026-10-02): wave particles on the compute
queue, 1 000 barrels with and without `--no-wakes`, two rounds, 1 500 frames each, 2560 × 1440.

| View | frame: without → with | `wakes/advance` | `wakes/emit` | `wakes/slopes` | `water/surface` |
|---|---|---|---|---|---|
| the towed barrel (`2168.0,36.90,-1234.0,-102.9,-36.2`) | 3.60–3.62 → 3.68–3.70 ms | 0.101–0.105 | 0.014–0.017 | 0.010–0.011 | 0.267–0.269 → 0.277–0.278 |
| the first barrel (`-237.2,318.39,-1842.9,132.1,-21.2`) | 4.09–4.10 → 4.17–4.18 ms | 0.080–0.083 | 0.018 | 0.010 | 0.361–0.363 → 0.373–0.381 |

- `wakes/clear` 0.004 ms. The compute passes take 0.13 ms and the frame 0.08 ms more: the
  async queue hides some of it.

**Splashes** (#107's third part, 2026-10-02): ballistic spray, emitted and moved on the compute
queue and drawn after the water. 1 000 barrels with and without `--no-splashes`, two rounds,
1 500 frames each, 2560 × 1440.

| View | `splashes/draw` | `splashes/emit` | `splashes/advance` | drops alive at most |
|---|---|---|---|---|
| the dropped barrel, over its 10 s cycle (`2160.0,30.55,-1234.0,0.0,-8.5`) | 0.010 ms | 0.001 | 0.003 | 1 447 |
| the highest step from 8 m (`283.9,31.1,3133.0,-29.3,-8`) | 0.032 ms | 0.004 | 0.003 | 4 830 |

- **The frame's total moves through the async overlap, not the splashes' work.** It reads
  3.47–3.48 → 3.31 ms from the barrel and 3.66 → 3.64 from the step. With the splashes' compute
  passes first on the compute queue, the probes' rays overlap other graphics passes
  (`gi/probe rays` 0.60 → 0.42 ms, `shading/layered` 1.08 → 0.96, `temporal/TAA resolve`
  0.15 → 0.24).
- **Serially** (`FORGE_ASYNC=0`) the frame stays within the runs' spread, and the TAA resolve
  does not change with the reactive mask (0.146–0.171 ms either way).

**The contour's third layer** (#111, 2026-10-01). `FORGE_SHADER_STATS=resolve_layered`
(`docs/PROCESS.md`) gives the layered pass's registers:
- With the sand's contour: 127 registers, no spill.
- Without the contour: 96, with about 19 a thread spilled to shared memory (4 864 bytes a
  group).
- The third layer the contour splits off decides between the two. With it shaded, even as a
  flat colour, the driver takes 127–128 registers and no spill, so an SM holds 16 of the pass's
  warps instead of 21.
- Moving no other part of the code changed that: the noise, the sort, packing the layers' ids,
  the scalars kept after the textures, `[branch]`, or a loop over the layers (128 even without
  the contour).

The pass now shades two layers at most. The lightest of three fades out, its weight taken off
the other two, so the blend stays continuous; and pairs with none of the contour's layers
(the sea floor, the rock) skip its block. Two rounds each (2560 × 1440) and three (1600 ×
900), alternating the committed shader, this one and the one without the contour block:

| View | `shading/layered` 1440p: before → now (none) | 900p: before → now (none) | frame 1440p: before → now |
|---|---|---|---|
| coast (the first view) | 0.884 → 0.845 (0.847) | 0.337 → 0.323 (0.320) | 3.14 → 3.11 ms |
| the stream from 16 m | 0.937 → 0.927 (0.923) | 0.356 → 0.349 (0.346) | 2.93 → 2.90 ms |
| down a river from 3 m | 0.908 → 0.898 (0.899) | 0.348 → 0.341 (0.338) | 2.83 → 2.82 ms |
| the stone at the largest mouth | 1.199 → 1.141 (1.180) | 0.448 → 0.427 (0.445) | 2.97 → 2.92 ms |
| a lake from 3 m over its water | 0.841 → 0.832 (0.823) | 0.322 → 0.320 (0.318) | 2.73 → 2.72 ms |
| the island from 2.5 km | 0.837 → 0.778 (0.766) | 0.330 → 0.309 (0.306) | 3.82 → 3.78 ms |

- Within 0.012 ms of the pass without the contour everywhere, and 0.04 ms under it at the
  stone, where the contour now leaves one layer where the map's pair had two.
- Images: 47 pixels of the capture batch's island view and 26 of its water view change
  (ꟻLIP mean 0.00001), and at most 34 of a contour view's (ꟻLIP max 0.086).
- The valleys of #118 (1600 × 900, three rounds): the slot from 3 m 0.627 → 0.608 ms, up the
  steep river 0.696 → 0.671 ms (frame 2.086 → 2.048), the slot from 40 m unchanged.
- Tried and dropped: the contour's pixels in a pass of their own over the tiles that hold
  them. It kept the layered pass at 96, but the band can fill the view (0.30 ms for the second
  pass from the stone's view), and the pixels it hands over pay twice.

**The ground in tiles** (#106, 2026-10-01). The island's ground cooked as 8 × 8 tiles of 2 km
instead of one mesh of 19.6 M triangles (the same triangles, `docs/demos/island.md`). Two rounds
each at 2560 × 1440, three at 1600 × 900, the previous commit and this one alternating:

| View | frame 1440p: before → now | 900p: before → now | `geometry/cluster cull 1` 1440p |
|---|---|---|---|
| coast (the first view) | 3.11 → 2.95 ms | 1.65 → 1.54 ms | 0.160 → 0.038 |
| the stream from 16 m | 2.90 → 2.78 ms | 1.49 → 1.38 ms | 0.177 → 0.044 |
| down a river from 3 m | 2.82 → 2.68 ms | 1.48 → 1.39 ms | 0.186 → 0.060 |
| the stone at the largest mouth | 2.92 → 2.83 ms | 1.51 → 1.40 ms | 0.168 → 0.038 |
| a lake from 3 m over its water | 2.73 → 2.60 ms | 1.44 → 1.35 ms | 0.173 → 0.070 |
| the island from 2.5 km | 3.79 → 3.61 ms | 2.18 → 2.05 ms | 0.253 → 0.124 |

- The cluster cull walks each instance's DAG from its roots: one mesh of 452 000 clusters was a
  long walk for few threads, 64 tiles spread it.
- The tiles' borders keep their vertices at every level: from 2.5 km the software raster takes
  0.04 ms more (0.726 → 0.762 ms at 1440p).
- The rays' cut: 600 000 triangles over the tiles at one error, 0.349 m where the one mesh's
  was 0.312 m; `gi/probe rays` within ±0.05 ms.
- **Drawn at 2 m** (143 M triangles with the amplification's detail; the default since the
  owner's look on 2026-10-01, `--island-drawn 8` for the 8 m tiles): the frame within 0.05 ms
  of the 8 m tiles' at 1440p (two rounds each, one build after the
  other): coast 2.98 → 3.00, stream 2.79 → 2.79, down a river 2.70 → 2.70, the stone 2.85 →
  2.85, the lake 2.60 → 2.65, the island from 2.5 km 3.62 → 3.67 ms. The cluster cull takes
  0.02–0.04 ms more, the probes' rays up to 0.03 (their cut's error 0.73 m); the rest is the
  same. Pages: 4.2 GB, 0.3 GB more geometry resident.

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
stored once per cluster). Issue #92 adds the list of the clusters pass 1 leaves to pass 2
(512 KiB per frame slot to start) and a third run of the cluster culls' status words: work
buffers 19.96 → 21.76 MiB for the ballad, 5.15 → 6.34 for the bench, 127.8 → 145.9 for the
city. The city's first frames, before instance occlusion's auto mode turns on, leave 820 k
clusters to pass 2 and grow its list to the 8 MiB cap per slot. **Verdicts:**

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

**Transient buffers** (#78, 2026-10-02; `FrameGraph::transient_buffer`,
`docs/research/render-graph-next.md`). Buffers that live within one frame now go in the
graph's transient heap, aliased with the transient images whose lifetimes they miss, instead
of one copy per frame slot. The first ones are the culls' work and root lists and their
status words: 8 192 + 8 192 + 3 168 + 611 KiB per slot in the city. City, frame 60 (the exit
log):
- GPU work buffers: 145.9 → 106.5 MiB.
- The transient heap: 43.75 MiB, unchanged. The lists take the memory the TAA colour and the
  AO images use later in the frame, and the heap equals the frame's peak of live transients
  (`load_bytes`, 44 800 KiB): the placement has no slack.
- `FORGE_GRAPH_POISON=1` fills each transient buffer with `0xDEADBEEF` after its last pass.
  The capture batch is at 0 px in the default, poison and no-alias modes.

The second migration: the visible-cluster list (16 MiB per slot in the city), its raster lists
(8 MiB), the fallback's draw commands and pass 2's rejects (8 MiB). They live until the
resolve, so they share less: the heap grows.

| City, frame 60 | before #78 | the culls' lists | and the visible lists | and the deferred instances |
|---|---|---|---|---|
| GPU work buffers | 145.9 MiB | 106.5 | 42.5 | 34.9 |
| transient heap | 43.75 MiB | 43.75 | 64.19 | 68.01 |
| both | 189.6 MiB | 150.2 | 106.7 | 102.9 |

The last column (#124, 2026-10-02): the list of the instances instance cull 1 defers to
instance cull 2, 4 bytes an instance behind its grid (3.8 MiB per slot in the city). It lives
from the first pass to instance cull 2, beside the culls' lists, so the heap grows by about its
size and the saving is the second slot's copy.

**The CPU's recording** (#78, the issue's "measure first"). `cpu/record commands` is now
three zones: `cpu/declare passes` (the demo's and the overlay's declarations), `cpu/graph
compile` (the order, the transients' layout, the barriers) and `cpu/graph record` (the
barriers and the pass bodies). 1 500 fixed-step frames, two rounds, against the commit before
the transient buffers:

| | before: record commands | after: declare + compile + record |
|---|---|---|
| city | 0.168–0.187 ms | 0.020 + 0.045 + 0.116 = 0.181–0.183 |
| ballad | 0.138–0.146 ms | 0.017 + 0.031 + 0.095 = 0.142–0.144 |

The transient buffers cost no CPU time that shows. Recording is 0.12 ms against the 0.5 ms
of pass bodies `docs/research/render-graph-next.md` sets as the gate for parallel recording,
so that stays unbuilt.

## Not measured yet

- **Streaming**: residency pools, request queue depth, drive and decompression throughput.
  These arrive with the streaming work (D-018, Phase 9) as lines of the memory group.
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
