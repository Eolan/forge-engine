# Forge — Research

The evidence behind the engine, one file per system under [`research/`](research/). Every
citation in those files was checked against a reachable page before being written down;
what could not be verified is listed in each file's "Checked and left out" rather than
quietly dropped. Each file ends with an opinionated *Recommendation for Forge*, which is
where `DECISIONS.md` draws from. This page is the index and the one-paragraph verdicts.

Reading order for a newcomer: this page, then `ARCHITECTURE.md`, then the research file of
the system you are about to touch.

## Files

| File | System | Entries | Status |
|---|---|---|---|
| [research/task-system.md](research/task-system.md) | job system, frame pipelining, ECS scheduling | 45 | done, implemented (`forge-task`) |
| [research/gpu-geometry.md](research/gpu-geometry.md) | mesh shaders, culling, virtual geometry, Vulkan features, Slang | 46 | done, phase 0 implemented (`forge-render`) |
| [research/large-worlds.md](research/large-worlds.md) | coordinates, partitioning, streaming, LOD, impostors, terrain | 49 | done |
| [research/lighting-gi.md](research/lighting-gi.md) | GI tiers, path tracing, shadows, sky, upscaling | 45 | done (weather rendering still to add) |
| [research/physics-fluids.md](research/physics-fluids.md) | rigid bodies, engines compared, characters, destruction, water | 45 | done |
| [research/netcode.md](research/netcode.md) | transport, replication, prediction, server topology | 48 | done |
| [research/audio.md](research/audio.md) | mixer, spatialisation, propagation, synthesis, middleware | 46 | done |
| [research/vegetation-materials.md](research/vegetation-materials.md) | trees, impostors, grass, trim sheets, unified materials, deformation | 50 | done |
| [research/animation.md](research/animation.md) | clips, motion matching, IK, physical characters, generated creatures | 42 | done |
| [research/memory-streaming.md](research/memory-streaming.md) | allocators, Resizable BAR, SSD streaming, residency | 40 | done |
| [research/procedural.md](research/procedural.md) | terrain, grammars, noise, ecosystems, settlements, DOD (from the previous projects) | ~90 | done, carried over |

## Verdicts

**Task system.** Nothing on crates.io combines physical-core-aware pools, priorities,
dependency counters with continuations and a main thread that helps rather than sleeps, so
Forge owns a ~2k-line job system on `crossbeam-deque`. Six workers on this CPU, never one
per hardware thread (measured: 103 missed audio deadlines otherwise). Fibers are out in
Rust; continuations and helping waits give the same shape. ECS: `bevy_ecs` 0.19 for storage
and queries, its executor replaced by a Forge one.

**GPU geometry.** Geometry shaders are dead on every vendor; task/mesh shaders replace them
but are "not necessarily a win" by themselves: the win is cluster-level culling. Clusters of
≤ 128 triangles, two-pass occlusion with a hierarchical Z pyramid, a 64-bit visibility
buffer, a cluster LOD DAG (meshoptimizer's `clusterlod`, QEM with locked borders), a software
rasteriser for sub-pixel clusters, cluster acceleration structures for ray tracing on RTX.
Slang covers every stage from one file. Shipped proof: Alan Wake 2, Doom: The Dark Ages,
Assassin's Creed Shadows. Implemented so far: clusters, task-shader culling, two-pass HZB,
bindless by device address — 127 M triangles at 1.1 ms, pixel-identical to brute force.

**Large worlds.** The 2026 consensus equals the previous project's choice: `f64` positions in
nested reference frames, camera-relative `f32` rendering, reversed-Z infinite depth. Origin
rebasing is retired (unsupported in multiplayer by Epic's own account). Right-handed, +Y up,
metres. CPU: SAH BVH per cell for statics, loose octree for movers, spatial hash for space;
GPU: sorted grids and H-PLOC BVHs, the RT acceleration structure as a general query
structure. Streaming: World Partition + HLOD as the template, `u64` cell ids. Octahedral
impostors stay the answer for the 150 m–1 km band with voxel aggregates beyond. Terrain:
equi-angular cube-sphere, CDLOD far, dual contouring near, SDF bricks for edits.

**Lighting.** The top end in 2026 is the ReSTIR stack (DI → GI → PT) behind a denoiser with
a radiance cache; NVIDIA retired probe-based RTXGI, DDGI remains the cheap dynamic GI for
the 3080 tier; radiance cascades are the one noise-free newcomer, its 3D form three months
old. Path tracing has shipped in five titles; two require hardware RT outright. Rockstar's
only primary source is the RDR2 atmosphere talk. Forge's ladder: T0 raster probes → T1
hybrid ray-query DDGI + ReSTIR DI (the 3080 target) → T2 ReSTIR GI + cache → T3 path traced
reference. DLSS 4.5 on NVIDIA; FSR 3.1 / XeSS on Vulkan elsewhere.

**Physics and fluids.** Bind Jolt first: shipped (Horizon Forbidden West, Death Stranding
2), a written determinism contract, double precision, character and vehicle controllers,
motorised ragdolls, soft bodies. box3d is real but a three-month-old alpha: track it as the
second engine behind the physics trait. PhysX rejected for authority, Rapier kept as the
pure-Rust reserve. Solver theory is settled by Catto's Solver2D: sub-stepped soft impulses.
Water tiered far/mid/near: analytic spectrum evaluated identically on CPU and GPU, a
server-side column model shadowed by a GPU shallow-water field, particles near, visual only.

**Netcode.** QUIC via `quinn` behind a transport trait (datagrams for inputs and snapshots,
streams for events), WebTransport later for browsers; own the replication protocol —
acked-baseline deltas, priority accumulator, cell interest, cell-relative quantisation, a
hand-written bit writer. 60 Hz simulation, 30 Hz snapshots near the player, ~800-byte
packets. Inputs redundant four per datagram with a server-reported jitter buffer that waits
rather than repeats. Determinism only same-binary. Star Citizen's replication layer shipped
after five years; SpatialOS is the warning. Clients talk only to the replication layer;
workers are stateless; MongoDB write-behind; RabbitMQ never per tick.

**Audio.** "Pro" is the data layer — events, buses, RTPCs, states, HDR loudness, virtual
voices, banks, live profiler — and that layer does not exist in Rust, so Forge writes it.
Steam Audio (Apache-2.0, via `audionimbus`) is the only open spatialiser at the middleware
bar; wave-based acoustics are all archived. SADIE II HRTFs ship by default. Big games rely
on data-driven ambience rules and a wind field shared with vegetation, not physical
acoustics. Impacts from modal banks fitted per material; rain and wind from the weather
fields. Three-tier acoustic LOD.

**Vegetation and materials.** Grow trees, don't model them: Weber–Penn parameters, space
colonisation plus competition, scanned bark and leaf atlases. Ladder: geometry to 40 m,
leaf cards to 150 m, hemi-octahedral impostors to 600 m, a lit canopy volume beyond; Nanite
Foliage (UE 5.7, experimental) is the voxel challenger to plan for. Overdraw is the cost and
the visibility buffer is the cure. Trim sheets are still standard (Sunset Overdrive,
Uncharted 4, Helldivers 2): a village is two to four regional trim sheets, a few tileables,
a decal atlas and vertex paint, with the shape grammar snapping faces onto trim rows.
Unified materials: one row with render layers, friction/restitution/density/solidity,
footstep and impact sets, gameplay tags, weather overrides and a deform block; per-triangle
material ids are native in Jolt; footprints are written by contacts into a clip-mapped
displacement layer that weather refills.

**Animation.** Production humanoid animation in 2026 is motion matching (For Honor 2016 →
The Last of Us Part II → UE 5.4 on every Fortnite character), but the algorithm is small and
the capture data is the project; without capture, an honest first humanoid is a blend space
of a few clips with inertialization. Contact is solved kinematically first (two-bone and
FABRIK IK, foot locking, motion warping for ledges). "Not just ragdolls" means a powered
ragdoll tracking the kinematic pose, which Jolt exposes directly; the shippable research
line beyond that is DReCon → SuperTrack. Generated creatures follow Spore: author against
chain roles, derive the gait from the body plan, IK onto the ground. Replicate parameters
and events, never poses. The IK layer emits foot-down events (position, normal, pressure,
foot shape, material) for the deformation system.

**Memory and streaming.** CPU: a Naughty Dog-style tagged heap (2 MiB blocks, per-worker
blocks, bulk free by frame/stage or cell tag) over one reserved virtual range, `slotmap`
pools, mimalloc only for the long tail, every allocator under Tracy; Rust's `Allocator`
trait is landing (1.100), `bumpalo` bridges until then. GPU: TLSF sub-allocation from
256 MiB blocks, budgets from `VK_EXT_memory_budget` minus 10 %, render-graph aliasing,
incremental transfer-queue defragmentation; `gpu-allocator` lacks all three and gets
replaced by an in-house TLSF layer or `vk-mem` later. Resizable BAR is already active on
the dev PC but worth single-digit percent: use it only for sequential, never-read per-frame
data. Sparse residency is still not viable (~250 µs per page bind on NVIDIA): paging is
software page tables over ordinary pools. DirectStorage is DX12-only; reuse its formats and
rules; GDeflate on Vulkan exists via `VK_EXT_memory_decompression` and must be probed. On
this CPU zstd and LZ4 out-decode a 7 GB/s NVMe, so GPU decompression is a headroom decision.
The previous engine's LOD popping was an eviction-policy bug: fetch highest need, evict
lowest need, recency only as a tie-breaker. Nanite's fixed 128 KB pages with GPU-emitted
requests are the template for cluster streaming; the asset container is a table of
BLAKE3-addressed, block-compressed, 4 KiB-aligned chunks.

**Procedural generation (carried over).** Terrain from process (uplift, stream-power
erosion, hydrology) not noise; grammars propose and constraints dispose (model synthesis
before WFC); stateless hash-of-position instead of RNG streams; Dendry for locally
evaluable global structure; villages from interest maps; Infinigen as the corpus.

## Still to research

- Planetary environment (`research/planet-environment.md`, queued 2026-09-24 with D-019): climate
  and biome models for Earth-like planets, biome and weather transitions, ecosystems (flora,
  fauna, succession), and weather rendering (rain, snow, wet surfaces, lightning, fog) as one
  file, since the owner wants to simulate whole planets with biomes and their transitions.
- Tools and editor: hot reload, the data model ("The Truth"), creation graphs — the
  previous bibliography's §6 covers the sources; a Forge-specific file comes with the editor.
