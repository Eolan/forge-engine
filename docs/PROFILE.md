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
in Tracy's GPU timeline next to the CPU zones.

Machine: RTX 5070 Ti, driver 617.14, 1600×900, 2026-09-24.

## `asteroids` — the ballad with the planet's atmosphere (frame 400, LOD 1 px, TAA on, ACES, overlay in full mode)

Frame **0.35 ms** (2 836 fps), p50 0.34, p99 0.59; over two 6000-frame scripted runs without
the overlay the GPU averages **0.325 ms** along the path and 0.34 ms facing the planet
(`--look`) (0.315 measured the same day before the atmosphere, 0.30 before the exposure
histogram, 0.27 with the rocks shaded in the mesh passes, 0.33 with the sky drawn first,
0.34 before the graph). GPU zones sum to 0.34 ms with the overlay and the capture copy in
the frame; CPU main thread 0.18 ms of work (record 0.09, submit + present 0.09). Every zone below is a
graph pass; the graph's own counters are the first line of the COUNTERS block, the
exposure (EV100, target, compensation, curve) the last.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| geometry | meshlet pass 1 (visible last frame) | 0.06 | 16 % | 7–8 k clusters, 0.5 M triangles, positions and a 32-bit id only. |
| geometry | instance cull | 0.02 | 7 % | One thread per instance: frustum, per-level LOD window, work-list append. At this size the timestamps' own granularity shows. |
| geometry | depth pyramid | 0.02 | 6 % | Eleven graph passes, one zone. Negligible; stays. |
| geometry | meshlet pass 2 (newly visible) | 0.02 | 6 % | Almost nothing becomes newly visible per frame at 1 px. |
| shading | visibility resolve | 0.03 | 8 % | One 8×8 compute group per tile, shading once per covered pixel, now in cd/m² times the exposure. Where shading cost will grow; material classification (#20) keeps it per material. |
| sky | starfield + planet | 0.11 (0.14–0.16 with the planet in view) | 32 % | **Still the largest single item.** Drawn after the rocks with a depth test, so its noise runs only on the uncovered pixels. Since #8 the planet is a ground under Earth's air, marched per pixel in 16 segments through Hillaire's tables (D-023); pixels outside the atmosphere's cone skip it, so the planet costs only where it is: +0.01 ms out of view, +0.04–0.06 in view, 0.31 ms for a planet filling the screen. The planet-view table (#26) turns that into a lookup. The tables themselves are built once (`sky/atmosphere tables`, first frame only). |
| exposure | luminance histogram | 0.02 | 5 % | Four passes in one zone: clear 1 KB, count every pixel into 256 log2 bins (shared-memory atomics, one global atomic per non-empty bin and group), copy to the slot's cached readback, host read. Could meter a quarter-resolution image if it ever matters; it does not now. |
| temporal | motion / TAA resolve | 0.01 / 0.05 | 17 % | The resolve rescales the history by the exposure ratio and writes the display image through the tone curve in the same pass: the curve is free. DLSS replaces the resolve on NVIDIA. |
| app | overlay / present | 0.01 / 0.00 | 3 % | The profiler itself; the present transition is free. |
| cpu | record / submit + present | 0.09 / 0.09 | — | Record includes compiling the graph (23 passes, 47 barriers) and the exposure update (a 256-bin walk). At 3 000 fps the driver's submit and present are a third of the frame; a render thread and fewer, larger submissions fix that when it matters. |
| cpu | wait for GPU (frame slot) | 0.14 | — | The frame is GPU-bound: the main thread's 0.18 ms of work finishes first and waits for the slot. |

Counters: graph 24 passes (with the overlay), 40 image + 8 memory barriers, transients 4
images, 32.8 MB requested in a 26.2 MB heap (the visibility buffer and the motion vectors
share 6.4 MB), 1 heap build, 0 retired; 3000 asteroids, 195 M leaf triangles, 28 k clusters
in the DAG tables, 4.6 M cluster slots; drawn 2 531 instances, 8 k meshlets, 0.63 M
triangles, mean LOD level 6.3; EV100 14.40 automatic, ACES, sun 128 klux.

**The road here (same frame):** full detail 5.5 ms → DAG with the old dispatch 4.1 ms →
exact task tables 1.09 ms → instance cull pass 0.34 ms → render graph 0.33 ms (the same
work; the graph is about correctness and structure, not speed) → sky drawn last 0.28 ms →
resolve straight into the swapchain 0.27 ms → visibility buffer 0.30 ms → physical light,
histogram exposure and tone curves 0.30–0.31 ms (structure again: the image is now a
function of light in lux, a camera value and a curve) → the planet under a physical
atmosphere 0.325 ms (0.34 facing it). The rendering stays pixel-identical
to brute force at every step of the A/B harness, and the golden captures of the three
curves are bit-identical from run to run.

**Priority list from these numbers:** (1) nothing in this frame is worth another pass on
its own: the sky's 0.11 ms is the starfield's per-pixel price, the planet adds 0.04–0.06
when in view and #26 would take most of that back; (2) the resolve is where shading cost will grow, and material classification (#20)
keeps that growth per material; (3) the CPU submit/present path only when a real scene
makes it visible; (4) geometry is done until triangle counts rise again. Next is DLSS
(#8), optional and off by default: the previous project measured it at 0.9–1.1 ms at 1440p
against 0.12 ms for its own resolve.

## `meshlets` — the culling bench (static view, occlusion on, LOD 1 px)

GPU **0.18 ms** (0.15 with the rocks shaded in the mesh passes): the bench resolves the
visibility buffer into a pre-exposed HDR image at a fixed EV100 of 15 and the display pass
(AgX) writes the swapchain: 17 passes, 32 image + 3 memory barriers, three transients,
25.6 MB requested in a 19.2 MB heap since the colour image reuses the depth buffer's
memory; 15 k meshlets, 1.09 M triangles (full detail, measured before the visibility
buffer: 325 k meshlets, 30 M triangles, 2.18 ms; without occlusion at full detail: 106 M at
6.30 ms).

## Not measured yet

- **Memory**: VRAM residency and per-heap budgets (`VK_EXT_memory_budget`), upload bandwidth,
  streaming queue depth — arrive with the memory/streaming work (D-018) as an overlay group
  (issue #9). The render graph's transient heap (images, requested vs allocated bytes,
  rebuilds, pending destructions) is already a counter line.
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
