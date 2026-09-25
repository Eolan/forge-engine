# Forge — Architecture

Forge is a professional-grade engine for very large procedural worlds (islands, continents,
planets, universes) shared by several players, built one system at a time. Each system gets a
research file (`docs/research/`), a numbered set of decisions (`DECISIONS.md`), a demo with
measured numbers (`docs/demos/`) and tests before the next system starts.

This document is the map. It is updated whenever a system lands; `ROADMAP.md` says what comes
next and `RESEARCH.md` indexes the evidence.

## 1. Principles

**P1 — The GPU owns the geometry.** The CPU decides *what exists* and hands the GPU pointers;
the GPU decides what is drawn. Culling (frustum, normal cone, occlusion), level of detail,
instancing, amplification (grass, clutter, tessellation) and eventually visibility resolution
all run in compute and mesh shaders from device-address buffers. Geometry shaders are never
used: they serialise output and are the stage mesh shaders replaced. Every GPU-driven path
has a compute + indirect-count fallback that produces the same pixels (for the geometry
since issue #5: culling runs in compute for both, and the compacted cluster list is drawn by
mesh shaders or by one `vkCmdDrawIndexedIndirectCount`, 0 pixels apart).
*(research: gpu-geometry.md; demo: meshlets)*

**P2 — One material, every consumer.** A material is one record that the renderer, the physics
engine, the audio engine, gameplay and the weather system all read: look, friction and
restitution, density and penetrability, footstep/impact sound set, gameplay tags (slippery,
sinkable, deformable, flammable, climbable) and a runtime weather state (wet, frozen, snow
depth). Ice slips, snow takes footprints, brick sounds like brick because the asset says so,
never because a designer wired it per object.
*(research: vegetation-materials.md §8, physics-fluids.md, audio.md)*

**P3 — Deterministic by construction.** Anything the client and the server both compute (world
generation, simulation) is a pure function of seeds and inputs: no platform math
(`forge_core::dmath`), no shared random generators (`forge_core::Seed`), no dependence on
thread timing (results merged by index). Determinism is tested in CI at one worker and at
six.
*(research: task-system.md, RESEARCH.md §3)*

**P4 — Two precisions, nested frames.** Positions are `f64` inside a hierarchy of reference
frames (integer sector grid → star system → body → construct); the GPU only ever sees `f32`
relative to the camera or a nearby anchor; depth is reversed-Z with an infinite far plane.
Origin rebasing is not used (it breaks in multiplayer). 1 unit = 1 metre, right-handed,
+Y up, −Z forward.
*(research: large-worlds.md)*

**P5 — Nothing waits inside a job.** The job system runs continuations on dependency
counters; a thread that must wait helps instead of sleeping. The pool leaves cores free for
the main, render and audio threads: a worker on every hardware thread is a worker too many
(measured: 103 missed audio deadlines in 471 versus 0).
*(research: task-system.md; demo: task-bench)*

**P6 — Rules are data.** World generation, placement, materials and behaviours are authored as
data (graphs, tables, grammars) evaluated by fixed engine mechanisms, hot-reloadable, cached
by content hash. Shaders are compiled from source at run time in development for the same
reason.
*(research: RESEARCH.md §2, §6)*

**P7 — Measure the distribution.** Every demo reports p50/p99/max and never a mean. GPU time
comes from timestamp queries, CPU time from Tracy zones, and both are read back without
stalling the queue.

**P8 — Client and server share the simulation crates.** The server is authoritative; the
client predicts. Both link the same `forge-sim`, `forge-physics` and `forge-procgen` and
must produce identical results (P3).
*(research: netcode.md)*

**P9 — Bricks from the previous projects are lifted, not linked.** Proven pieces of
`world`, `tropical-island` and `shooter` (seeds, deterministic math, hydrology, cube-sphere,
DLSS hook, netcode lessons) are ported into Forge crates with their tests, never depended on
in place.

**P10 — Demos are the milestones.** A system exists when its demo runs on both machines with
the numbers in its doc. Docs live in `docs/`; each demo doc records the machine, the driver
and the date next to every number.

## 2. System map

```
                 ┌──────────────────────── tools / editor (later) ────────────────────────┐
                 │                                                                        │
  procgen ──► world (frames, partition, streaming, LOD, materials) ──► sim (ECS, gameplay) │
     │              │                    │                      │           │             │
     │              ▼                    ▼                      ▼           ▼             │
     │           render ◄──── geom     physics ◄──── animation      audio       net       │
     │              │                                                                     │
     └──────────►  gpu  (instance, device, memory, swapchain, shaders, frames, commands)  │
                    │                                                                     │
                   task  (workers, counters, scopes, graphs, blocking pool)               │
                    │                                                                     │
                   core  (seeds, deterministic math, hashes, handles, time)               │
```

| Crate | Status | Contents |
|---|---|---|
| `forge-core` | built | `Seed`/`SplitMix64`, `dmath` (libm-backed), `hash` (pcg3d/pcg4d/mix64), generational `Handle`, the material record (`material`: `Material` with render, physics and tag layers, `MaterialTable`, D-007) |
| `forge-task` | built, measured | work-stealing pool with 3 priorities, `Counter` continuations, `scope`/`join`/`par_*`, `TaskGraph`, `BlockingPool`, `Task<T>` |
| `forge-gpu` | built | `ash` Vulkan 1.3+ device (mesh shaders, ray query, min-reduction samplers, memory budget detected), acceleration structures and ray queries (`accel`: BLAS and TLAS builds, reached by device address; D-029), `gpu-allocator` with every allocation counted by category and every host write counted as upload (`memory_report`: per-heap usage and budget from `VK_EXT_memory_budget`, issue #9), RAII `Buffer`/`Image`(with mip views)/`Pipeline`/`Surface`, swapchain, Slang compiler with cache, the global bindless set (sampled/storage images, samplers), mesh, vertex and compute pipelines, indirect dispatches and indexed indirect-count draws, `DeviceOptions` (mesh shaders left off for `--force-fallback`), `Frames` (timeline semaphore, 2 in flight, GPU timestamps, deferred deletion), safe `Commands`, and the **render graph** (`graph`: declared accesses → derived barriers, transient images aliased in one heap, per-pass profiler zones, host reads declared for readbacks, `Custom` accesses for third-party work; D-020), and `dlss` (DLSS through NVIDIA Streamline's interposer behind the `dlss` feature: modes, render sizes, tagging graph images, evaluation inside a graph pass; D-024) |
| `forge-geom` | built | meshlet building and the cluster LOD DAG (`meshopt`; material sections through it, #41, D-027), 128 KiB cluster pages (`page`, D-025), the mesh cache (`cache`), procedural meshes (asteroid, the city's props and terrain), shared GPU layouts |
| `forge-render` | phase 1 in progress | `MeshletSceneBuilder`/`MeshletScene` (many meshes, instances), `MeshletRenderer` (cluster LOD DAG, instance and cluster culls in compute appending in a fixed order, drawn by mesh shaders or the indirect-count fallback (`GeometryPath`, issue #5), a software rasteriser for dense clusters (#3), two-pass HZB occlusion from the previous frame's pyramid with no per-cluster state (#33), cluster pages resident or streamed through a pool (`streaming`, #36, D-025), the visibility buffer and its resolve by material class (#20, D-026), statistics), `material` (the GPU material table, `TextureSet`, stock rock and ice rows), `textures` (procedural tileable textures with mips), `mipcheck` (the resolve's texture LOD against a fragment shader's), `sky` (sky-view table, the sky's irradiance in nine SH coefficients (#47), aerial perspective, compose; D-023), `bloom` (the downsample/upsample chain; D-022), `gtao` (ambient occlusion from the depth, after XeGTAO; D-030), `raytrace` (BLAS per mesh from its DAG, TLAS over the instances; D-029), `Taa` (jittered HDR target, motion vectors, clipped history rescaled by exposure, display output), `DlssUpscaler` (DLSS in place of the TAA resolve, the scene drawn at DLSS's input size), `Starfield` (stars, nebula, a physical sun disc, a planet under its atmosphere), `Atmosphere` (Hillaire 2020 transmittance and multiple-scattering tables as graph passes, the per-pixel march for views from space), `LuminanceMeter` + `AutoExposure` (histogram metering, EV100), `Display` + `Tonemap` (AgX, ACES fit, PBR Neutral as run-time data); every renderer declares graph passes, none writes a barrier. Next: lighting tiers (soft shadows, GI) |
| `forge-world` | planned | reference frames, cube-sphere/grid partition, cell streaming, HLOD, material table, weather state |
| `forge-physics` | planned | binding of the chosen engine behind Forge types, per-construct spaces, material lookup, deformation writes |
| `forge-anim` | planned | clips, blend graph, motion matching, IK, powered ragdoll tracking, contact events |
| `forge-audio` | planned | real-time thread, mixer graph, spatialiser, material-driven sounds, weather ambience |
| `forge-net` | planned | transport, replication, prediction, interest management, replay |
| `forge-sim` | planned | `bevy_ecs` storage with the Forge executor, gameplay systems, simulation LOD |
| `forge-procgen` | planned | terrain genesis (uplift, erosion, hydrology), ecosystems, settlements, grammars, noise/SDF library |
| `forge-app` | built | window, input, frame loop that owns each frame's `FrameGraph` (swapchain import, demo passes, overlay, capture, present), PNG capture, fly camera, the profiler overlay (F1: GPU and CPU zones, the memory group) and Tracy frame marks, zones and memory plots (`profiling`), the Vulkan API through Streamline when asked (`AppConfig::streamline`, feature `dlss`) |
| `tools/imgdiff` | built | pixel comparison of captures (the golden-image check; exit code for CI) |
| `tools/credits` | built | the Rust crates in the build with version, licence, authors and repository (`docs/credits-crates.md`), from `cargo metadata`; `--check` in CI. The rest of the credits are in `CREDITS.md` |

## 3. Frame model

- **Main thread**: window events, input, orchestration; helps the pool while waiting.
- **Workers** (`PoolConfig::client()`: physical cores − 2): simulation, culling preparation,
  streaming decode, procedural generation at `Low` priority in ≤ 200 µs jobs.
- **Render thread**: one per frame slot at first (the main thread), later a dedicated
  submitter; pass bodies are the unit that workers will record in parallel.
- **The frame on the GPU is a render graph** (D-020): the shell imports the swapchain image
  into a `FrameGraph`, every renderer declares passes with the images and buffers it reads
  and writes (per mip level where it matters), and `RenderGraph::execute` derives every
  barrier and layout transition from the tracked state of each resource, lays out the
  frame's transient images (depth, HDR colour, motion vectors) in one heap where lifetimes
  allow aliasing, records the passes in declaration order with a profiler zone each, and
  carries the final states into the next frame. Persistent images (`GraphImage`: depth
  pyramids, TAA histories) and buffers (`GraphBuffer`: work lists, visible-cluster lists, indirect
  arguments) keep their state between frames; resources a frame in flight may still use go
  through `Frames::destroy_later`. Nobody outside `forge-gpu` records a barrier.
- **Audio thread**: real-time priority, never touches the pool, communicates by lock-free
  queues.
- **Network thread**: `tokio` runtime for I/O only; packets are handed to the simulation.
- **GPU**: one graphics queue now; async compute and a transfer queue are the graph's next
  extension (a queue per pass, timeline waits and ownership transfers on crossing edges).
  Two frames in flight on a timeline semaphore; the CPU never waits on the whole queue.
- **Pipelining**: simulation of frame N+1 overlaps rendering of frame N through an immutable
  frame packet (positions, transforms, visibility inputs) written by the simulation and read
  by the renderer.

## 4. Data model on the GPU

Every buffer carries a device address. A frame writes one small block (`Frame`) that holds
pointers to the scene tables (instances, clusters, vertices, materials, lights) and the
camera; shaders walk the scene from there. Textures and storage images sit in one global
update-after-bind set indexed by integer handles, so a later switch to descriptor heaps or
descriptor buffers is a back-end change. Layouts are `std430`, mirrored by `#[repr(C)]`
structs in Rust and checked by tests.

**Geometry reaches the screen through a visibility buffer** (issue #6, 2026-09-24). The
mesh passes rasterise only positions: the cluster cull (compute) appends every drawn cluster
to the frame's visible-cluster list (`(instance, cluster)`, in a fixed order, issue #5; sized to
the demand, issue #27), the mesh shader emits the cluster's triangles with `visible_slot << 7 | triangle` as the
per-primitive id (the fallback's vertex shader carries the slot, the primitive id gives the
triangle), and the fragment shader writes that id into an `R32_UINT` target next to the hardware
depth (`u32::MAX` = nothing drawn). A compute pass then shades once per pixel: it reads the
id, fetches the triangle's three vertices through the visible list, projects them with the
frame's jittered camera and reconstructs the attributes at the pixel centre from
**analytic perspective-correct barycentrics**: `b_i / w_i` is affine in screen space, so it
is rebuilt from its value at vertex 0 and its gradient, the sum gives `1 / w` at the pixel,
and `lambda_i = w · b_i / w_i`; the screen-space derivatives of `lambda`, needed for texture
LOD later, follow from the same gradients (`∂lambda_i/∂x = w · (∂(b_i/w_i)/∂x − lambda_i ·
∂(1/w)/∂x)`) with no `ddx`/`ddy`, no 2×2 quads and no helper lanes (Schied & Dachsbacher
2015; Hable 2021). The formula lives in `shaders/meshlet.slang` and, mirrored with a unit
test, in `forge_render::visibility`. Empty pixels are left to the sky pass (or filled with a
background colour). What this buys: shading cost independent of overdraw and triangle size,
one shading code path for the mesh-shader, software and fallback rasterisers, and the entry
point for material classification (D-007).

**Shading goes by material class** (issue #20, D-026). The D-007 record
(`forge_core::material`) has a render layer. Its GPU form is an 80-byte row of the material
table, and every instance names its row: its mesh's by default, or its own.

The resolve is three kinds of pass:
- **`shading/classify`**: a thread per pixel over 8×8 tiles. It writes the background into
  empty pixels and appends the tile to the list of every shading class the tile shows.
- **One indirect dispatch per class** (`shading/standard`, `shading/ice`): it shades only its
  own pixels in its own tiles, so each class compiles only its own code.
- **Textures without coordinates**: triplanar projection of the object-space position,
  weighted by the normal. `SampleGrad` takes the derivatives of that position, which follow
  from the analytic barycentric derivatives above. The mip check (`meshlets --mip-check`)
  compares its level choice with a fragment shader's.
- **Sections within a mesh** (issue #41, D-027): a triangle's section offsets its instance's
  row, so a building's windows are glass and its facade brick. Clusters hold up to two
  sections, sorted, the split in the cluster record.
- **Shadows by ray query** (issue #45, D-029): the resolve's `_rt` variants trace one ray per
  sun-facing pixel against a TLAS over the scene's instances, whose BLASes are cuts of the
  meshes' DAGs. The city and the ballad (#46) build theirs once at start.
- **Sky light and its occlusion** (issues #47, #48; D-023's note, D-030): under a sky, a
  surface also takes the sky's irradiance for its normal (nine SH coefficients a frame,
  from the sky-view table), scaled by GTAO computed from the depth before the resolve.
- **Ground in layers** (issue #42, D-028): a `layered` row names a layer map, a byte a texel,
  and each layer is the standard row after it; the layered pass blends the two heaviest
  layers around each pixel.

**Dense clusters go to a software rasteriser** (issue #3). The cluster cull marks a cluster
dense when it is in front of the near plane, under 64 pixels across and has fewer than two
pixels of its bounding sphere's screen rectangle per triangle: there the hardware's fixed
cost per primitive dominates. In the first pass, when the renderer runs it, those clusters
go to a second raster list instead of the hardware's: a compute workgroup per cluster
transforms and snaps its vertices exactly as the fixed-function stages do (perspective
division, the viewport, round-to-nearest-even to 1/256 pixel), then a thread per triangle
culls back faces, walks the pixel centres of its bounding box with 32-bit edge functions and
the top-left rule, interpolates the depth linearly in screen space and keeps
`depth << 32 | id` with a 64-bit atomic maximum, only where it beats the hardware's pixel of
that pass (nearer, or at equal depth the larger id: the hardware's depth test keeps the last
drawn and it draws in id order, so every path resolves ties alike). A full-screen merge (an
indirect draw, empty when no cluster went to software) writes those samples into the
visibility buffer and the depth and clears them; everything after the draw, from the depth
pyramid to TAA, sees one image. The renderer runs it (`SwRaster::Auto`) when the recent
frames held 1.5 M dense triangles or more, until they fall below 0.75 M: the raster pass and
the merge cost about 0.02 ms, which a million dense triangles repay. The pass-2 clusters
(the few newly visible) stay in hardware. It needs 64-bit buffer atomics; without them
every cluster is drawn in hardware.

**Colour is physical and pre-exposed** (issue #7, D-022). Lights carry photometric units
(the sun in lux, its disc in cd/m² from its solid angle) and every pass writes luminance
multiplied by the frame's exposure, so the HDR targets stay near 1 in fp16 whatever the
scene. The exposure is an EV100: fixed, or automatic from a 256-bin log-luminance histogram
of the finished HDR image (`exposure/luminance histogram`: clear, count in shared then
device-local memory, copy 1 KB to a cached per-slot readback, `HostRead`), read two frames
later and followed on the CPU with separate speeds up and down. Temporal passes rescale
their history by the exposure ratio. The display transform is chosen at run time from
`tonemap.slang` (AgX, ACES fit, Khronos PBR Neutral) and applied where the last HDR pass
writes the display image (the TAA resolve in the ballad, the stand-alone display pass
elsewhere); nothing upstream knows which curve is on screen. Bloom (issue #44) is a half-size
downsample/upsample chain of the same pre-exposed image, blended into the displayed image
before the curve; the TAA history never sees it.

**Atmospheres belong to planets** (issue #8, D-023). A planet's air is two tables built by
compute passes when it changes (transmittance, multiple scattering; Hillaire 2020) and a
per-pixel march through the shell for views from space, in the same pre-exposed units:
the ground is lit through the air, and the stars and the sun seen through it are dimmed
and reddened by its transmittance. Empty space has no medium. Cameras inside an atmosphere
(issue #43) add a sky-view table around the camera, an aerial-perspective volume to 8 km, and
a compose pass that draws the sky and the sun behind the scene and hazes the scene by distance;
the sunlight on the scene takes the sun's colour through the air.

**The resolve is TAA or DLSS** (issue #8, D-024). The scene is drawn jittered into a
pre-exposed HDR target either way, and the motion vectors (UV offsets from depth and the two
cameras) are their own pass. TAA resolves at the output size and writes the display image
through the tone curve in the same pass. DLSS (builds with `--features dlss` on an RTX GPU,
the Vulkan API through Streamline's interposer) draws the scene at its input size, upscales
into an HDR image at the output size, and the display pass applies the curve. The DLSS pass
is a graph pass like any other: it declares every image it hands to Streamline, and its
output with a `Custom` access, because NGX clears it at the transfer stage before writing it.

## 5. Conventions

- Units: SI; light in photometric units (lux, cd/m²), colour targets pre-exposed (D-022).
  Axes: right-handed, +Y up, −Z forward. Vulkan's Y-down framebuffer is handled
  by a negative viewport height; clip depth is reversed.
- `unsafe` is forbidden at the crate root except in `forge-task` (scoped lifetimes) and
  `forge-gpu` (Vulkan); every block carries a `SAFETY:` comment, enforced by lints.
- Deterministic crates deny platform float functions (clippy `disallowed-methods`, coming
  with `forge-sim`).
- Shaders: Slang only, one file per pipeline family, `-matrix-layout-column-major`, cached
  by content hash under `shader-cache/`.
- Every demo takes `--frames N` and `--capture file.png` so it can run headless in CI and
  produce golden images.
- Secrets never enter the repository (`.env`, `server-auth.md` are ignored).

## 6. Machines and services

- Dev PC: Ryzen 7 9800X3D, 61 GB, RTX 5070 Ti (Blackwell, 16 GB), Windows 11, Vulkan 1.4
  driver 617.14, Vulkan SDK 1.4.357 (Slang 2026.13).
- Server PC: RTX 3080, 8 threads, `192.168.1.14` (Windows now, Linux later).
- Raspberry Pi 5 at `192.168.1.80`: MongoDB (32017) and RabbitMQ (32672) for persistence and
  asynchronous events only, never on the tick path. Credentials stay in the ignored
  `server-auth.md` of the previous projects.
