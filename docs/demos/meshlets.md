# Demo: `meshlets` — GPU-driven geometry: compute culling, mesh shaders, indirect-count fallback

Run: `cargo run --release -p meshlets` (options `--side N`, `--detail N`, `--roughness R`,
`--vsync`, `--validate`, `--no-occlusion`, `--lod-error PX` (1.0), `--no-lod`, `--orbit`,
`--frames N`, `--capture file.png --capture-frame N`, `--overlay`, `--ev100 EV` (15),
`--tonemap agx|aces|neutral`, `--force-fallback`).
Controls: WASD/QE move, Shift fast, right mouse drag to look, **F1** profiler, **F** freeze
culling (move the camera to see what was culled), **C** cone culling, **V** frustum culling,
**O** occlusion culling, **L** cluster LOD, **K** LOD colours, **[** / **]** LOD threshold,
**M** meshlet colours (on by default here), **Tab** wireframe, **G** tone curve. The bench
draws without anti-aliasing or a sky on purpose: it measures culling, not looks; since
2026-09-24 it shades through the visibility buffer like the ballad (a compute resolve into
a pre-exposed HDR image at a fixed EV100 of 15, then the display transform into the
swapchain); the `asteroids` demo is where TAA, the sky and automatic exposure live.

With the cluster LOD DAG and the instance cull pass (2026-09-24, see
[asteroids.md](asteroids.md)) the static view draws 15 k meshlets and 1.09 M triangles in
**0.18 ms** at a 1 px threshold (0.15 before the visibility buffer's resolve pass); the
numbers below are the full-detail (`--no-lod`) figures that measure culling alone, taken
before the visibility buffer.
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
- **The CPU issues one draw per pass.** Compute passes cull instances and clusters (frustum
  spheres, normal cones, LOD, the depth pyramid) into a compacted list of visible clusters,
  and one indirect draw rasterises it: `vkCmdDrawMeshTasksIndirectEXT` with a mesh workgroup
  per listed cluster, or `vkCmdDrawIndexedIndirectCount` on GPUs without mesh shaders (see
  "Two paths, one culling" below; until issue #5 the culling ran in a task shader).
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

## Two paths, one culling (issue #5, 2026-09-24)

Culling moved out of the task shader into compute, and both ways of rasterising draw what it
produces:

- **Instance cull** (one thread per instance): the frustum test and the LOD levels that can
  hold selected clusters at the instance's distance; appends the instance's work items
  (groups of 32 clusters) to the work list and writes the cluster cull's indirect grid.
- **Cluster cull** (one per mesh pass; 8 work items per 256-thread workgroup): LOD
  selection, frustum, normal cone and, in the second pass, the depth pyramid and the
  "visible last frame" bits; appends the survivors to the frame's visible-cluster list (pass
  1 from the front, pass 2 from the back, so neither draw needs the other's count). The last
  workgroup writes the pass's draw arguments.
- **Mesh-shader path:** `vkCmdDrawMeshTasksIndirectEXT`, one mesh workgroup per listed
  cluster, no task stage.
- **Fallback** (GPUs without `VK_EXT_mesh_shader`): the cluster cull also writes one
  `VkDrawIndexedIndirectCommand` per listed cluster and each pass issues one
  `vkCmdDrawIndexedIndirectCount`. The index buffer is the cooked triangle lists themselves
  (one byte per index, `VK_KHR_index_type_uint8`); a draw's vertex offset is the cluster's
  window of `meshlet_vertices` and its first instance the cluster's slot in the visible list.
  The vertex shader reads the slot back through `SV_VulkanInstanceID` (Slang's
  `SV_InstanceID` subtracts the base instance, the D3D meaning), the fragment shader adds the
  primitive id, which restarts at 0 in every draw: the same `slot << 7 | triangle` id the mesh
  shader writes, so the resolve cannot tell the paths apart. The only extra memory is the
  draw commands (20 B per listed cluster, 20 MiB per frame slot).
- `--force-fallback` (both demos) creates the device **without** `VK_EXT_mesh_shader`: the
  fallback runs here exactly as on a GPU that lacks it, and a stray mesh-shader call would
  fail validation.

**Pixel proof** (`tools/imgdiff`, tolerance 0, of 1 440 000 pixels): the fallback against the
mesh path, and the new mesh path against the task-shader path of the previous commit.

| Capture | fallback vs mesh | new mesh vs old task path |
|---|---|---|
| `meshlets` static, frame 60 | 0 | 0 |
| `meshlets --orbit`, frames 120 and 900 | 0, 0 | 0, 0 |
| `meshlets --orbit --no-lod` (full detail), frame 120 | 0 | 0 |
| `meshlets --orbit --no-occlusion`, frame 120 | 0 | 0 |
| `asteroids --fixed-step --no-taa`, frame 600 | 0 | 0 |
| the A/B harness on each path (`--no-occlusion`, `--no-cone`, `--show-culled`, `--no-group-window`) | 0 each | — |
| `asteroids --fixed-step` with TAA and automatic exposure (the golden), frame 600 | 0 | 14 946 at ±1 (see the draw order below) |

Validation and synchronization validation are silent on both paths in both demos, and with
the DLSS build.

**Numbers** (RTX 5070 Ti, 1600×900, GPU time per frame averaged over whole runs; the old task
path measured on commit 231d674 the same day):

| View | old task path | mesh path | fallback | drawn |
|---|---|---|---|---|
| `meshlets` static, LOD at 1 px | 0.180 ms | **0.180 ms** | 0.270 ms | 15 k clusters, 1.09 M triangles |
| `meshlets --no-lod` (occlusion on) | 2.09 ms | **2.15 ms** | 3.73 ms | 325 k clusters, 30.0 M triangles |
| `meshlets --no-lod --no-occlusion` | 5.70 ms | **5.77 ms** | 11.07 ms | 1 049 k clusters (list full, 92 k dropped), 97 M triangles |
| `meshlets --orbit`, 1200 frames | — | **0.114 ms** | 0.140 ms | |
| `asteroids` path (20 000 frames, TAA) | 0.322 ms | **0.326 ms** | 0.360 ms | 8 k clusters, 0.63 M triangles |

- **The mesh path costs what the task path cost** (within 3 %). Split: the two cluster culls
  take 0.014 + 0.013 ms of the ballad's frame and the draw of pass 1 0.038 ms (it was 0.06
  with the culling inside it); at full detail the culls are 0.11 + 0.12 ms and the draw 1.8.
- **The fallback's draw costs about twice the mesh draw** (0.159 against 0.071 ms at 1 px,
  3.40 against 1.82 at full detail, 10.9 against 5.6 without occlusion): one indexed draw per
  cluster through the vertex pipeline, against a mesh workgroup per cluster. The culling is
  the same work on both. On the GPUs the fallback exists for (integrated, older) the absolute
  numbers will differ; what this measures is that nothing but the draw differs.
- **The list caps at 1 048 576 clusters.** The full-detail view without occlusion needs
  1.14 M and drops 92 k per frame on both paths (and did in the old one): the counters and the
  title now say so. Sizing and growing the list is issue #27.

**The draw order is part of the output.** The first version appended with atomics: frame by
frame pixel-identical, yet two runs of the ballad's golden capture (TAA on) came out 3 140
pixels apart about half the time (2 864 with a fixed exposure), while the old task path
gave the same image 17 times in 17. Everything the culling decides was identical run to
run (the per-frame statistics of both outcomes matched frame for frame), and without TAA or
without the planet's bright sky the difference vanished. The mechanism: two triangles can
meet a sample at exactly the same depth (sub-pixel rocks, crossing faces), and the depth test
(`GREATER_OR_EQUAL`) keeps the one drawn last. Building the old path with a strict `GREATER`
(first wins) changed its TAA capture by 13 933 pixels and its TAA-off capture by none: the
ties are real, rare, and only TAA's history accumulates them into something a diff can see.
A list filled by racing atomics draws in a different order every run; the task path drew in
work-list order, which happened to be stable.

So both culls now append **in a fixed order**, instance order then cluster order, with a
single-pass prefix sum (decoupled look-back, Merrill & Garland 2016): each workgroup takes a
ticket in the order workgroups start, publishes how many entries it appends, adds up its
predecessors' published counts (a window of 32 per round) and writes at that offset. Every
rerun since has been identical (twelve with a fixed exposure, five of the golden itself on the
final code). It cost 0.036 ms at first (0.206 ms at 1 px against 0.170 for the atomic
version): one ticket, one status word and at least one look-back read per work item, and at
1 px most work items draw nothing. Eight work items per workgroup brought it back to
0.181 (four: 0.185; sixteen: no better at 1 px, worse at full detail). The look-back relies
on a waiting workgroup never starving the one it waits for, which Vulkan does not promise:
it holds on this GPU; GPUs without that guarantee (Apple's M-series is the documented case)
need the bounded-spin variant, "Decoupled Fallback" (research/gpu-geometry.md; issue #28).

The golden image of the ballad moved once because of it: against the previous commit, 14 946
pixels differ by ±1 (711 by more than 2, at most 16) with TAA and automatic exposure; with a
fixed exposure, or without TAA, frame 600 is identical. The ties resolve differently in a few
earlier frames, the histogram of those frames moves by a hair, and the exposure carries it.
The new capture is the reference; it is identical from run to run.

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
Measured before the visibility buffer's cluster list existed: today the list caps the
default row at 1 049 k clusters and drops the rest (see "Two paths, one culling"; #27).

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
- Resources are RAII (`Buffer`, `Image`, `Pipeline`, `Surface` hold their device). Anything a
  frame in flight may still use is retired through `Frames::destroy_later` (dropped when
  that frame has completed); resizes still wait for idle.
- Since 2026-09-24 the bench draws through the render graph (D-020): the cull pass, the two
  mesh passes and the eleven pyramid levels declare their buffers and images (per mip for
  the pyramid) and the graph derives the 28 image barriers and 2 memory barriers of a frame;
  the depth buffer is a transient. `FORGE_GRAPH_LOG=1` prints the plan. Captures are
  identical to the hand-written barriers (0 pixels, static and orbiting, occlusion on and off).
- Since 2026-09-24 the bench shades through the visibility buffer (issue #6): the mesh
  passes write the 32-bit id, the resolve shades into a bench colour transient (a background
  colour where nothing was drawn) and a blit pass copies it to the swapchain: 17 passes, 32
  image + 4 memory barriers, three transients in a 19.2 MB heap (the colour image aliases
  the depth buffer). Occlusion on vs off while orbiting: still 0 pixels.
- Since issue #7 (same day) the rocks are lit in physical units (the sun at 128 klux) and
  drawn at a fixed EV100 of 15 (`--ev100`), and the display pass (AgX by default, **G**
  cycles the curves) replaces the blit: 17 passes, 32 image + 3 memory barriers, GPU
  0.18 ms unchanged. Occlusion on vs off while orbiting: 0 pixels.

## Next steps (from the research recommendation)

1. ✅ (issue #5, 2026-09-24) Culling in compute, shared by the mesh-shader path and the
   indirect-count fallback, 0 pixels apart ("Two paths, one culling" above).
2. Cluster LOD DAG with meshoptimizer's `clusterlod` (QEM with locked group borders) and
   per-cluster screen-space error selection, plus a software rasteriser for sub-pixel clusters.
3. Visibility buffer: done for the hardware path (a 32-bit id next to the hardware depth,
   analytic barycentrics in compute, issue #6); next the material classification and the
   material table (#20), and the 64-bit depth | id target with the software rasteriser.
4. Streaming of cluster pages and, on RTX hardware, cluster acceleration structures
   (`VK_NV_cluster_acceleration_structure`) so the same clusters feed ray tracing.
