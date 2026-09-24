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

## `asteroids` — the ballad, cluster LOD DAG + instance cull pass (frame 600, LOD 1 px, TAA on)

Frame **0.35 ms** (2 829 fps), p50 0.34, p99 0.60. GPU zones sum to 0.34 ms; CPU main
thread 0.16 ms of work (record 0.07, submit + present 0.10), now comparable to the GPU.

| Subject | Zone | ms | share of GPU | Verdict |
|---|---|---|---|---|
| sky | starfield + planet | 0.13 | 39 % | **Now the largest item.** Full-screen procedural noise (three value-noise octaves, two star lattices of 27 cells, the planet) at every pixel every frame. Render the far sky into a cube map refreshed over several frames, or at half resolution with TAA; the atmosphere (issue #8) will replace this shader anyway. |
| geometry | meshlet pass 1 (visible last frame) | 0.06 | 18 % | 6–8 k clusters, 0.5 M triangles: real work at last, and small. |
| geometry | instance cull | 0.02 | 7 % | One thread per instance: frustum, per-level LOD window, work-list append. Replaces the launch-bound task-shader walk (0.45 ms per pass). |
| geometry | depth pyramid | 0.02 | 6 % | Negligible; stays. |
| geometry | meshlet pass 2 (newly visible) | 0.02 | 6 % | Almost nothing becomes newly visible per frame at 1 px. |
| temporal | TAA (motion + resolve + blit) | 0.07 | 20 % | Second largest now. Resolve straight into the swapchain once the render graph exists (saves the blit); DLSS replaces it on NVIDIA. |
| app | overlay | 0.01 | 3 % | The profiler itself. |
| cpu | record / submit + present | 0.07 / 0.10 | — | At 2 800 fps the driver's submit and present are a third of the frame; a render thread and fewer, larger submissions fix that when it matters. |
| cpu | wait for GPU (frame slot) | 0.16 | — | Still GPU-bound, barely. |

Counters: 3000 asteroids, 195 M leaf triangles, 28 k clusters in the DAG tables, 4.6 M
cluster slots; drawn 2 458 instances, 8 k meshlets, 0.63 M triangles, mean LOD level 6.3.

**The road here (same frame):** full detail 5.5 ms → DAG with the old dispatch 4.1 ms →
exact task tables 1.09 ms → instance cull pass 0.34 ms. The geometry passes went from 5.8 ms
to 0.13 ms, and the rendering is pixel-identical to brute force at every step of the A/B
harness.

**Priority list from these numbers:** (1) the sky at 39 % (cache it, or wait for the
atmosphere pass and design that one cheap from the start), (2) TAA → DLSS and no blit,
(3) the CPU submit/present path only when a real scene makes it visible, (4) geometry is
done until triangle counts rise again (streaming and the software rasteriser then).

## `meshlets` — the culling bench (static view, occlusion on, LOD 1 px)

Frame **0.16 ms**, GPU 0.15 ms: 15 k meshlets, 1.09 M triangles (full detail: 325 k
meshlets, 30 M triangles, 2.18 ms; without occlusion at full detail: 106 M at 6.30 ms).

## Not measured yet

- **Memory**: VRAM residency and per-heap budgets (`VK_EXT_memory_budget`), upload bandwidth,
  streaming queue depth — arrive with the memory/streaming work (D-018) as an overlay group
  (issue #9).
- **Job system**: worker occupancy per frame — arrives with the simulation phase (Tracy shows
  it already under `--features profiling`).
- **Presentation latency**: the time from submit to scan-out — once the render thread exists.
