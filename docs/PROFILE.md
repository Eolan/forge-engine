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

## `asteroids` — the ballad through the render graph (frame 400, LOD 1 px, TAA on, overlay in full mode)

Frame **0.35 ms** (2 880 fps), p50 0.33, p99 0.59; over a 6000-frame scripted run without
the overlay the GPU averages **0.33 ms** (0.34 before the graph). GPU zones sum to 0.37 ms
with the overlay and the capture copy in the frame; CPU main thread 0.18 ms of work
(record 0.08, submit + present 0.10). Every zone below is now a graph pass; the graph's own
counters are the first line of the COUNTERS block.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| sky | starfield + planet | 0.14 | 37 % | **The largest item.** Full-screen procedural noise (three value-noise octaves, two star lattices of 27 cells, the planet) at every pixel every frame. Render the far sky into a cube map refreshed over several frames, or at half resolution with TAA; the atmosphere (issue #8) will replace this shader anyway. |
| geometry | meshlet pass 1 (visible last frame) | 0.07 | 18 % | 7–8 k clusters, 0.5 M triangles: real work at last, and small. |
| geometry | instance cull | 0.05 | 12 % | One thread per instance: frustum, per-level LOD window, work-list append. 0.02–0.05 ms from frame to frame: at this size the timestamps' own granularity shows. |
| geometry | depth pyramid | 0.02 | 6 % | Eleven graph passes, one zone. Negligible; stays. |
| geometry | meshlet pass 2 (newly visible) | 0.02 | 6 % | Almost nothing becomes newly visible per frame at 1 px. |
| temporal | motion / TAA resolve / blit | 0.01 / 0.05 / 0.01 | 18 % | Second largest. The blit goes once the resolve writes the swapchain as a second target (the graph makes that a two-line change; kept for now so the images stay identical); DLSS replaces the resolve on NVIDIA. |
| app | overlay / present | 0.01 / 0.00 | 3 % | The profiler itself; the present transition is free. |
| cpu | record / submit + present | 0.08 / 0.10 | — | Record now includes compiling the graph (19 passes, 40 barriers): +0.01 ms. At 2 900 fps the driver's submit and present are a third of the frame; a render thread and fewer, larger submissions fix that when it matters. |
| cpu | wait for GPU (frame slot) | 0.17 | — | Still GPU-bound, barely. |

Counters: graph 20 passes (with the overlay), 38 image + 3 memory barriers, transients 3
images, 26.2 MB in a 26.2 MB heap (nothing can alias yet: colour, depth and motion vectors
are all alive at the resolve), 1 heap build, 0 retired; 3000 asteroids, 195 M leaf
triangles, 28 k clusters in the DAG tables, 4.6 M cluster slots; drawn 2 531 instances,
8 k meshlets, 0.63 M triangles, mean LOD level 6.3.

**The road here (same frame):** full detail 5.5 ms → DAG with the old dispatch 4.1 ms →
exact task tables 1.09 ms → instance cull pass 0.34 ms → render graph 0.33 ms (the same
work; the graph is about correctness and structure, not speed). The rendering is
pixel-identical to brute force at every step of the A/B harness and to the pre-graph
captures.

**Priority list from these numbers:** (1) the sky at 37 % (cache it, or wait for the
atmosphere pass and design that one cheap from the start), (2) TAA → DLSS and no blit,
(3) the CPU submit/present path only when a real scene makes it visible, (4) geometry is
done until triangle counts rise again (streaming and the software rasteriser then).

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
