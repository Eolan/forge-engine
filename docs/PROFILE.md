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

## `asteroids` — the ballad (frame 380 of the default path, TAA on)

Frame **5.06 ms** (198 fps), p50 4.89, p99 6.39. GPU zones sum to 6.01 ms; CPU main thread
0.25 ms of work, the rest of the frame blocked on the GPU.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| geometry | meshlet pass 1 (visible last frame) | 4.59 | 76 % | **The frame.** 783 k meshlets, 77 M triangles for 1.44 M pixels: ~54 triangles per pixel, almost all smaller than a pixel. Rasterisation and shading of invisible detail. Fix: the cluster LOD DAG selects clusters by screen-space error (≈ 1 triangle per pixel → ~3 M triangles), the software rasteriser takes sub-pixel clusters, the visibility buffer shades once per pixel. Target: geometry under 1 ms. |
| geometry | meshlet pass 2 (newly visible) | 1.18 | 20 % | 44 k meshlets that became visible this frame plus the re-test of every remaining slot (14.2 M meshlet slots walked by 447 k task groups per pass). Shrinks with LOD (fewer clusters) and with a cluster hierarchy so the task shader tests groups of clusters instead of every cluster. |
| geometry | depth pyramid | 0.02 | 0 % | 11 levels from 1024×512. Negligible; stays. |
| sky | starfield + planet | 0.14 | 2 % | Full-screen procedural noise (three value-noise octaves, two star lattices of 27 cells each, the planet). Fine now; when the atmosphere and clouds arrive, render the far sky at lower resolution or into a cached cube. |
| temporal | motion vectors | 0.01 | 0 % | Cheap. |
| temporal | TAA resolve | 0.04 | 1 % | Cheap: the 3×3 gather and Catmull-Rom history at 1600×900. The earlier estimate of 0.5 ms for TAA was wrong. DLSS will replace this on NVIDIA; nothing to do. |
| temporal | blit to swapchain | 0.01 | 0 % | Goes away with the render graph (resolve straight into the swapchain). |
| app | overlay | 0.01 | 0 % | The profiler itself. |
| cpu | wait for GPU (frame slot) | 5.73 | — | The main thread is idle 96 % of the frame: the engine is entirely GPU-bound. The job system has the whole frame for simulation, physics and streaming. |
| cpu | record commands | 0.09 | — | One frame block write and ~40 commands. |
| cpu | submit + present | 0.16 | — | Driver cost of submit and present; will hide behind the render thread. |
| cpu | update, acquire | 0.00 | — | The path evaluation and the swapchain acquire. |

Counters at that frame: 3000 asteroids, 7 meshes, 195 M triangles, 14.2 M meshlet slots;
drawn 2435 instances, 783 k + 44 k meshlets, 76.8 M triangles; 779 k meshlets occluded.

**Priority list from these numbers:** (1) cluster LOD DAG, (2) software rasteriser for
sub-pixel clusters, (3) cluster hierarchy in the task shader, (4) visibility buffer. Together
they turn the 5.8 ms of geometry into well under 1 ms, at which point the sky, lighting and
the CPU simulation become the numbers to watch.

## `meshlets` — the culling bench (static view, occlusion on)

Frame **2.07 ms** (483 fps), p50 2.10, p99 2.36. GPU 1.96 ms; CPU 0.17 ms.

| Subject | Zone | ms | share | Verdict |
|---|---|---|---|---|
| geometry | meshlet pass 1 | 1.80 | 92 % | 325 k meshlets, 30 M triangles: the same sub-pixel story with denser, closer rocks. |
| geometry | meshlet pass 2 | 0.13 | 6 % | Steady state: almost nothing becomes newly visible. |
| geometry | depth pyramid | 0.02 | 1 % | Negligible. |
| cpu | record / submit + present | 0.06 / 0.11 | — | Idle main thread. |

Without occlusion culling the same view draws 106 M triangles at 6.30 ms: the two-pass
pyramid pays for itself thirty times over.

## Not measured yet

- **Memory**: VRAM residency and per-heap budgets (`VK_EXT_memory_budget`), upload bandwidth,
  streaming queue depth — arrive with the memory/streaming work (D-018) as an overlay group.
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
