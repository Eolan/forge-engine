# Demo: `meshlets` — GPU-driven geometry with task and mesh shaders

Run: `cargo run --release -p meshlets` (options `--side N`, `--detail N`, `--roughness R`,
`--vsync`, `--validate`, `--no-occlusion`, `--lod-error PX` (1.0), `--no-lod`, `--orbit`,
`--frames N`, `--capture file.png --capture-frame N`, `--overlay`).
Controls: WASD/QE move, Shift fast, right mouse drag to look, **F1** profiler, **F** freeze
culling (move the camera to see what was culled), **C** cone culling, **V** frustum culling,
**O** occlusion culling, **L** cluster LOD, **K** LOD colours, **[** / **]** LOD threshold,
**M** meshlet colours (on by default here), **Tab** wireframe. The bench draws straight to
the swapchain without anti-aliasing on purpose: it measures culling, not looks; the
`asteroids` demo is where TAA and the sky live.

With the cluster LOD DAG and the instance cull pass (2026-09-24, see
[asteroids.md](asteroids.md)) the static view draws 15 k meshlets and 1.09 M triangles in
**0.15 ms** at a 1 px threshold; the numbers below are the full-detail (`--no-lod`) figures
that measure culling alone.
Machine: RTX 5070 Ti, driver 617.14, Vulkan 1.4, Slang 2026.13, 1600×900, 2026-09-24.
Research behind it: [research/gpu-geometry.md](../research/gpu-geometry.md).

![1152 procedural asteroids, one colour per meshlet](images/meshlets-asteroids.png)

## What it proves

- The whole Forge GPU stack works on this machine: `ash` device with the Vulkan 1.3 baseline
  plus `VK_EXT_mesh_shader`, Slang → SPIR-V through `slangc` with a content-hashed cache,
  timeline-semaphore frames in flight, GPU timestamps, validation clean.
- **No descriptor sets.** Every buffer is reached through its device address from one
  per-frame `Frame` block; the CPU pushes a single 8-byte pointer. This is the bindless model
  the rest of the renderer will use.
- **The CPU issues one draw.** `vkCmdDrawMeshTasksEXT(instances × ⌈meshlets/32⌉)`; the task
  shader culls instances and meshlets (frustum spheres, normal cones), compacts survivors with
  wave intrinsics and launches mesh workgroups that emit the clusters.
- Meshlets come from meshoptimizer (`meshopt` 0.6): ≤ 64 vertices, ≤ 124 triangles, cone
  weight 0.5, after vertex-cache optimisation. Bounds and cones are meshoptimizer's.
- **Two-pass occlusion culling** (Haar & Aaltonen 2015, Nanite 2021). Pass 1 draws the
  meshlets whose visibility bit was set last frame. A compute pass builds a hierarchical-Z
  pyramid (1024×512, 11 levels, linear min-reduction sampler, conservative by construction)
  from that depth. Pass 2 projects every remaining meshlet's sphere to a screen rectangle
  (Mara & McGuire 2013), samples the pyramid at the rectangle's four corners at the level
  where a texel is at most the rectangle's size (provably conservative, see the shader),
  draws the survivors and rewrites the bits; previously visible meshlets are re-tested
  without drawing so the set never creeps. Normal-cone culling skips clusters whose cone is
  wider than a hemisphere (meshoptimizer marks them with a unit cutoff and a zero axis). Everything runs through the same global bindless set (images) and device-address
  buffers; the visibility bits are one bit per (instance, meshlet) on the GPU.

## Numbers — occlusion culling

Same scene, default roughness, 1600×900, validation clean:

| Camera | pass 1 + pass 2 meshlets drawn | triangles | occlusion-culled | GPU |
|---|---|---|---|---|
| static, occlusion off | 1 140 k | 106 M | 0 | 6.30 ms |
| static, occlusion on (steady state) | 325 k + 0 k | **30 M** | 816 k | **2.16 ms** |
| turning and drifting (`--orbit`), occlusion on | 399 k + 16 k | 38.5 M | 376 k | **2.66 ms** |

**Correctness proof:** `--orbit --capture` at frames 120 and 900 with and without occlusion,
compared with `tools/imgdiff`: **0 of 1 440 000 pixels differ** (tolerance 2, max channel
error 0). The pyramid is conservative, so culling never removes a visible triangle.

The 13.2 M triangles at 1.14 ms this table showed before 2026-09-24 were measured with two
culling bugs (a point-sampled depth pyramid and clusters culled by a NaN normal cone) that
silently removed about a quarter of the visible geometry — from both sides of the
comparison, which is why the pixel proof still passed. The bugs, the A/B harness that found
them and the fixes are described in [asteroids.md](asteroids.md).

The whole frame — two mesh-shader passes, an 11-level pyramid, one 8-byte push constant per
pass — costs 0.13 ms of CPU.

## Numbers — culling without occlusion

Scene: 1152 instances (24 × 24 × 2 layers) of a 110 592-triangle asteroid (cube-sphere,
detail 96, welded seams), 1194 meshlets per instance → **1.4 M meshlets, 127 M triangles**.

| Roughness | visible instances | meshlets drawn (frustum + cone) | triangles drawn | GPU | CPU (record) |
|---|---|---|---|---|---|
| 0.35 (default) | 1020 / 1152 | 1 140 k | 106 M | **6.3 ms** | 0.01 ms |
| 0.05 (smooth)  | 1015 / 1152 | 774 k (**32 % fewer**: cone-culled) | 74 M | 5.5 ms | 0.02 ms |

- 106 M triangles in 6.3 ms is ~17 G triangles/s; most of them are sub-pixel at distance,
  which is exactly the case that the cluster LOD DAG (next step) removes.
- Cone culling depends on the content: on the rough asteroid the mean cone cutoff is 0.913
  (a 124-triangle cluster spans ~66° of normals) and many clusters are wider than a
  hemisphere, so few are ever fully back-facing. Smooth surfaces cull a third. This matches
  meshoptimizer's guidance; it is not a bug.
- CPU time per frame is the cost of writing one 424-byte block and recording ~10 commands.

## Design notes confirmed

- Slang handles task/mesh/fragment stages and raw `T*` buffer pointers from one file; the
  wave-op capability upgrade warning is harmless and filtered.
- `-matrix-layout-column-major` makes glam matrices usable as-is with `mul(M, v)`.
- Negative viewport height gives a Y-up NDC with counter-clockwise front faces; reversed-Z
  with `D32_SFLOAT` and an infinite far plane (`GREATER_OR_EQUAL`).
- Frame statistics are written by the shaders into a per-slot host-visible buffer and read
  back two frames later, so the window title shows real visible counts without a stall.
- Resources are RAII (`Buffer`, `Image`, `Pipeline`, `Surface` hold their device). Until the
  render graph brings deferred deletion, anything destroyed mid-run waits for idle first.

## Next steps (from the research recommendation)

1. Move culling into compute (so the fallback shares it) and add the compute + indirect-count
   path for GPUs without mesh shaders, pixel-diffed against this path with `imgdiff`.
2. Cluster LOD DAG with meshoptimizer's `clusterlod` (QEM with locked group borders) and
   per-cluster screen-space error selection, plus a software rasteriser for sub-pixel clusters.
3. Visibility buffer (64-bit depth | cluster | triangle) and material resolve in compute.
4. Streaming of cluster pages and, on RTX hardware, cluster acceleration structures
   (`VK_NV_cluster_acceleration_structure`) so the same clusters feed ray tracing.
