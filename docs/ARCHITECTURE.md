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
all run in task/mesh/compute shaders from device-address buffers. Geometry shaders are never
used: they serialise output and are the stage mesh shaders replaced. Every GPU-driven path
has a compute + indirect-count fallback that produces the same pixels.
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
| `forge-core` | built | `Seed`/`SplitMix64`, `dmath` (libm-backed), `hash` (pcg3d/pcg4d/mix64), generational `Handle` |
| `forge-task` | built, measured | work-stealing pool with 3 priorities, `Counter` continuations, `scope`/`join`/`par_*`, `TaskGraph`, `BlockingPool`, `Task<T>` |
| `forge-gpu` | built | `ash` Vulkan 1.3+ device (mesh shaders, ray query, min-reduction samplers detected), `gpu-allocator`, RAII `Buffer`/`Image`(with mip views)/`Pipeline`/`Surface`, swapchain, Slang compiler with cache, the global bindless set (sampled/storage images, samplers), mesh and compute pipelines, `Frames` (timeline semaphore, 2 in flight, GPU timestamps), safe `Commands` |
| `forge-geom` | built | meshlet building and the cluster LOD DAG (`meshopt`), procedural cube-sphere asteroid, shared GPU layouts |
| `forge-render` | phase 0 built | `MeshletSceneBuilder`/`MeshletScene` (many meshes, instances, visibility bits), `MeshletRenderer` (two-pass HZB occlusion, statistics), `Taa` (jittered HDR target, motion vectors, clipped history), `Starfield` (stars, nebula, sun, planet). Next: render graph (declared barriers, deferred deletion), cluster LOD DAG, visibility buffer, material resolve, lighting tiers, atmosphere, post, upscalers |
| `forge-world` | planned | reference frames, cube-sphere/grid partition, cell streaming, HLOD, material table, weather state |
| `forge-physics` | planned | binding of the chosen engine behind Forge types, per-construct spaces, material lookup, deformation writes |
| `forge-anim` | planned | clips, blend graph, motion matching, IK, powered ragdoll tracking, contact events |
| `forge-audio` | planned | real-time thread, mixer graph, spatialiser, material-driven sounds, weather ambience |
| `forge-net` | planned | transport, replication, prediction, interest management, replay |
| `forge-sim` | planned | `bevy_ecs` storage with the Forge executor, gameplay systems, simulation LOD |
| `forge-procgen` | planned | terrain genesis (uplift, erosion, hydrology), ecosystems, settlements, grammars, noise/SDF library |
| `forge-app` | built | window, input, frame loop on `Frames`, swapchain transitions, PNG capture, fly camera, Tracy frame marks/zones (`profiling`). Next: debug UI overlay |
| `tools/imgdiff` | built | pixel comparison of captures (the golden-image check; exit code for CI) |

## 3. Frame model

- **Main thread**: window events, input, orchestration; helps the pool while waiting.
- **Workers** (`PoolConfig::client()`: physical cores − 2): simulation, culling preparation,
  streaming decode, procedural generation at `Low` priority in ≤ 200 µs jobs.
- **Render thread**: one per frame slot at first (the main thread), later a dedicated
  submitter; record from workers in parallel once the render graph exists.
- **Audio thread**: real-time priority, never touches the pool, communicates by lock-free
  queues.
- **Network thread**: `tokio` runtime for I/O only; packets are handed to the simulation.
- **GPU**: one graphics queue now; async compute and a transfer queue when the render graph
  lands. Two frames in flight on a timeline semaphore; the CPU never waits on the whole queue.
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

## 5. Conventions

- Units: SI. Axes: right-handed, +Y up, −Z forward. Vulkan's Y-down framebuffer is handled
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
