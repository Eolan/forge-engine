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

## `asteroids` — the ballad through the render graph, sky drawn last, no blit (frame 400, LOD 1 px, TAA on, overlay in full mode)

Frame **0.30 ms** (3 341 fps), p50 0.29, p99 0.54; over a 6000-frame scripted run without
the overlay the GPU averages **0.27 ms** (0.28 with the blit, 0.33 with the sky drawn
first, 0.34 before the graph). GPU zones sum to 0.30 ms with the overlay and the capture
copy in the frame; CPU main thread 0.18 ms of work (record 0.08, submit + present 0.10).
Every zone below is a graph pass; the graph's own counters are the first line of the
COUNTERS block.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| geometry | meshlet pass 1 (visible last frame) | 0.07 | 22 % | 7–8 k clusters, 0.5 M triangles: real work at last, and small. |
| geometry | instance cull | 0.02 | 7 % | One thread per instance: frustum, per-level LOD window, work-list append. 0.02–0.05 ms from frame to frame: at this size the timestamps' own granularity shows. |
| geometry | depth pyramid | 0.02 | 7 % | Eleven graph passes, one zone. Negligible; stays. |
| geometry | meshlet pass 2 (newly visible) | 0.02 | 8 % | Almost nothing becomes newly visible per frame at 1 px. |
| sky | starfield + planet | 0.10 | 32 % | **Still the largest single item, down from 0.14.** Drawn after the rocks with a depth test, so its noise (three value-noise octaves, two star lattices of 27 cells, the planet) runs only on the uncovered pixels, about two thirds of the screen here. What remains is the per-pixel cost of the shader itself; the atmosphere (issue #8) replaces it and takes the same depth-tested slot. |
| temporal | motion / TAA resolve | 0.01 / 0.05 | 18 % | Second. The resolve writes the next history and the swapchain in one pass (the blit and its two transitions are gone, issue #18); DLSS replaces the resolve on NVIDIA. |
| app | overlay / present | 0.01 / 0.00 | 4 % | The profiler itself; the present transition is free. |
| cpu | record / submit + present | 0.08 / 0.10 | — | Record includes compiling the graph (18 passes, 39 barriers). At 3 300 fps the driver's submit and present are a third of the frame; a render thread and fewer, larger submissions fix that when it matters. |
| cpu | wait for GPU (frame slot) | 0.11 | — | The CPU and the GPU are now even. |

Counters: graph 19 passes (with the overlay), 38 image + 3 memory barriers, transients 3
images, 26.2 MB in a 26.2 MB heap (nothing can alias yet: colour, depth and motion vectors
are all alive at the resolve), 1 heap build, 0 retired; 3000 asteroids, 195 M leaf
triangles, 28 k clusters in the DAG tables, 4.6 M cluster slots; drawn 2 531 instances,
8 k meshlets, 0.63 M triangles, mean LOD level 6.3.

**The road here (same frame):** full detail 5.5 ms → DAG with the old dispatch 4.1 ms →
exact task tables 1.09 ms → instance cull pass 0.34 ms → render graph 0.33 ms (the same
work; the graph is about correctness and structure, not speed) → sky drawn last 0.28 ms →
resolve straight into the swapchain 0.27 ms. The rendering is pixel-identical to brute force
at every step of the A/B harness and to the pre-graph captures (the last step moves 0.05 %
of the pixels by one level: the sRGB conversion moved from the transfer engine to the
attachment write).

**Priority list from these numbers:** (1) nothing in this frame is worth another pass on
its own: the sky's remaining 0.10 ms is the shader's per-pixel price until the atmosphere
replaces it; (2) the CPU submit/present path only when a real scene makes it visible;
(3) geometry is done until triangle counts rise again (streaming and the software
rasteriser then). The next steps are structural (visibility buffer, HDR), not
profile-driven.

## `meshlets` — the culling bench (static view, occlusion on, LOD 1 px)

Frame **0.16 ms**, GPU 0.15 ms (unchanged through the graph: 15 passes, 28 image + 2
memory barriers, one 6.4 MB transient): 15 k meshlets, 1.09 M triangles (full detail:
325 k meshlets, 30 M triangles, 2.18 ms; without occlusion at full detail: 106 M at
6.30 ms).

## Not measured yet

- **Memory**: VRAM residency and per-heap budgets (`VK_EXT_memory_budget`), upload bandwidth,
  streaming queue depth — arrive with the memory/streaming work (D-018) as an overlay group
  (issue #9). The render graph's transient heap (images, requested vs allocated bytes,
  rebuilds, pending destructions) is already a counter line.
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
