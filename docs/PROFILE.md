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

## `asteroids` — the ballad, after the cluster LOD DAG (frame 600, LOD 1 px, TAA on)

Frame **1.12 ms** (893 fps), p50 1.09, p99 1.40. GPU zones sum to 1.10 ms; CPU main thread
0.18 ms of work, the rest blocked on the GPU.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| geometry | meshlet pass 1 (visible last frame) | 0.46 | 41 % | 8 k clusters, 0.62 M triangles drawn; the time is now the task-shader walk over 145 k groups of 32 cluster slots (4.6 M slots over 2 458 visible instances), most exiting on the per-level window. Next: a cluster hierarchy or fatter task groups so far instances cost a handful of groups, not hundreds (issue #4). |
| geometry | meshlet pass 2 (newly visible) | 0.42 | 38 % | The same walk again to find what became visible; with the DAG it draws almost nothing (0 k) and is pure traversal. Same fix as pass 1; or test only clusters that pass 1 skipped. |
| geometry | depth pyramid | 0.02 | 2 % | Negligible; stays. |
| sky | starfield + planet | 0.14 | 12 % | Unchanged in absolute terms, now the second item. Full-screen procedural noise; when the atmosphere arrives, render the far sky at lower resolution or into a cached cube. |
| temporal | TAA (motion + resolve + blit) | 0.06 | 6 % | Cheap. DLSS replaces it on NVIDIA. |
| app | overlay | 0.01 | 1 % | The profiler itself. |
| cpu | wait for GPU (frame slot) | 0.91 | — | Still GPU-bound, at 900 fps. The main thread does 0.18 ms of work per frame. |
| cpu | record / submit + present | 0.07 / 0.11 | — | Driver cost; a render thread hides it later. |

Counters: 3000 asteroids, 195 M leaf triangles, 28 k clusters in the DAG tables, 4.6 M
cluster slots; drawn 2 458 instances, 8 k meshlets, 0.62 M triangles, mean LOD level 6.3.

**Before the DAG (same frame, full detail):** GPU 5.5 ms, of which pass 1 4.6 ms and pass 2
1.2 ms for 78 M sub-pixel triangles. The LOD removed 99.2 % of the triangles and 80 % of the
frame; the sub-pixel problem (54 triangles per pixel) is gone.

**Priority list from these numbers:** (1) the task-shader walk (issue #4: cluster hierarchy /
fatter groups; both passes are launch-bound at ~145 k groups), (2) the sky at 12 %, (3) the
software rasteriser for the smallest clusters (issue #3) matters again only once the walk is
cheap, (4) then lighting and the CPU simulation become the numbers to watch.

## `meshlets` — the culling bench (static view, occlusion on, LOD 1 px)

Frame **0.6 ms**, GPU 0.58 ms: 15 k meshlets, 1.09 M triangles (full detail: 325 k
meshlets, 30 M triangles, 2.18 ms; without occlusion at full detail: 106 M at 6.30 ms).

## Not measured yet

- **Memory**: VRAM residency and per-heap budgets (`VK_EXT_memory_budget`), upload bandwidth,
  streaming queue depth — arrive with the memory/streaming work (D-018) as an overlay group
  (issue #9).
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
