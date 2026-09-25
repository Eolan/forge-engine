# Forge — Decisions

Numbered, dated, never deleted. A decision cites the research that backs it and the demo that
proved it. Status: ✅ **accepted** (in code, measured), 🟡 **proposed** (needs the owner's
yes), ⏳ **pending** (research not finished), 🅿️ **parked**.

Read `ARCHITECTURE.md` for the principles these decisions implement and `ROADMAP.md` for
when each one gets built.

---

## D-001 — Rust for the whole engine ✅ (2026-09-24)

Client, server, tools and pipelines are Rust. The argument, honestly:

- **For.** Memory safety without a garbage collector in a multithreaded engine (the job
  system's scoped borrows are checked by the compiler, see `forge-task`); one toolchain for
  Windows and Linux servers; first-class Vulkan (`ash`), meshoptimizer, QUIC (`quinn`) and
  ECS (`bevy_ecs`) crates; `cargo` for builds, tests and dependency auditing; deterministic
  builds; the previous three projects were Rust, so the bricks port directly.
- **Against, and what we do about it.** The best physics engines (Jolt, PhysX, box3d) and
  audio middleware are C/C++: bind them behind a Forge trait (D-009, D-011) rather than
  rewrite them. Compile times: dependencies are built optimised once (`profile.dev.package`),
  engine crates stay small. `unsafe` is confined to two crates with documented invariants.
- **Not chosen.** C++ (no safety net for the concurrency we need), C# (GC pauses against a
  real-time audio thread), Zig/Jai (ecosystem too small for Vulkan + QUIC + physics bindings).

## D-002 — Vulkan 1.3+ through `ash`, shaders in Slang ✅ (2026-09-24)

Baseline: dynamic rendering, synchronization2, timeline semaphores, buffer device address,
descriptor indexing, scalar block layout, host query reset, draw indirect count. Optional,
detected at start: `VK_EXT_mesh_shader`, ray query + acceleration structures,
`VK_EXT_sampler_filter_minmax`. Shaders are Slang compiled by `slangc` to SPIR-V 1.6 into a
content-hashed cache; one file per pipeline family; `-matrix-layout-column-major` so glam
matrices are used as-is.
*Why not wgpu:* trails raw Vulkan on mesh shaders, ray tracing and descriptor features and
adds a layer; the previous project reached the same conclusion. *Why not DX12:* Windows only;
the server and a Linux build matter. *Why Slang over HLSL/GLSL/rust-gpu:* one language for
task, mesh, compute and ray-tracing stages, modules, generics, pointers to device-address
buffers, Khronos-hosted; rust-gpu still lacks buffer device address.
*(research: gpu-geometry.md §8–9; demo: meshlets)*

## D-003 — GPU-driven geometry: clusters, compute culling, mesh shaders, indirect-count fallback ✅ (2026-09-24)

Meshes are cooked into meshlets (≤ 128 triangles; 64 v / 124 t today via meshoptimizer);
culling (frustum, normal cone, two-pass hierarchical-Z occlusion) and emission run on the GPU
from device-address buffers; the CPU issues one draw per pass. Geometry shaders are never used. Every
mesh-shader path gets a compute + `vkCmdDrawIndexedIndirectCount` fallback producing the same
pixels (checked with `tools/imgdiff`). Since issue #5 the culling itself is compute (an
instance cull, then a cluster cull per mesh pass) and appends to a compacted list of visible
clusters **in a fixed order** (a single-pass prefix sum over workgroups): depth ties resolve
by draw order, so the order must not depend on timing. Mesh shaders (no task stage) or, on
GPUs without them, one indexed draw per listed cluster through
`vkCmdDrawIndexedIndirectCount` (the cooked one-byte triangle lists as the index buffer) draw
the same list. Since issue #3 the first pass's dense clusters (under 2 pixels of bounding
rectangle per triangle) can go to a software rasteriser in compute instead, on both paths,
when a frame holds enough of them to repay its fixed cost (D-021). Since issue #33 the
occlusion keeps no state per cluster. Pass 1 draws what the previous frame's pyramid
shows, from the previous culling camera, and pass 2 re-derives that to test only the rest
against this frame's pyramid. The work lists are sized by demand, so the scene can hold a
million instances (197 MiB for 980 k rocks, against 2.9 GiB before). Since issue #37 an
instance whose roots alone are the cut lists them in a root list instead of taking work
items, 32 roots of any instances to a cluster-cull item. Next: cluster acceleration
structures for ray tracing on RTX.
*Measured:* 127 M-triangle scene, 1152 instances: 6.1 ms brute force → 1.1 ms with
occlusion, 0 pixels different. Compute culling (#5): the two paths 0 pixels apart; the mesh
path costs what the task path did (0.180 ms bench, 0.326 against 0.322 ms for the ballad),
the fallback's draw about twice the mesh draw. Software rasteriser (#3): full detail
2.27 → 1.15 ms on the bench (6.41 → 2.48 without occlusion, 3.92 → 1.20 through the
fallback), the ballad at full detail 4.37 → 2.02; the LOD views unchanged (it stays off).
Root lists (#37): 980 k rocks 5.95 → 1.36 ms, the million-instance city 4.90 → 1.06 ms,
the same pixels.
*(research: gpu-geometry.md; demo: meshlets)*

## D-004 — Coordinates: f64 nested frames, camera-relative f32, Y-up metres ✅ (2026-09-24)

`(frame_id, f64 position)` inside a hierarchy integer sector → star system → body →
construct; the GPU receives `f32` relative to the camera or an anchor; reversed-Z with an
infinite far plane in `D32_SFLOAT`. **No origin rebasing** (unsupported in multiplayer by
Epic's own account; each client renders relative to its own camera instead). Right-handed,
+Y up, −Z forward, 1 unit = 1 metre; Vulkan's Y-down framebuffer handled once by a negative
viewport height; Z-up sources (Blender, GIS) swapped at import.
*(research: large-worlds.md §1–2, §9)*

## D-005 — Our own job system, with the "leave cores free" rule ✅ (2026-09-24)

`forge-task`: work-stealing deques per worker, three priorities, dependency counters that
fire continuations (no waiting inside jobs), helping waits, borrowed fork-join scopes, task
graphs, a separate blocking pool for I/O. Client pools use `physical cores − 2` workers;
servers use every hardware thread. Background jobs stay ≤ 200 µs.
*Measured:* ×5.98 on 6 workers for a 16 M-element map, 5 µs job round trip, 5.2 M jobs/s;
with a worker on every hardware thread the audio stand-in misses 103/471 deadlines and the
frame tail hits 34 ms, versus 0 misses and 2.2 ms p99 with the rule.
*Not chosen:* rayon (16 spinning workers by default, no priorities, no continuations),
`bevy_tasks` (three more pools), fibers (unsafe in Rust). *(research: task-system.md; demo:
task-bench)*

## D-006 — ECS: `bevy_ecs` storage and queries, Forge executor ✅ (2026-09-24)

Use `bevy_ecs` 0.19 standalone for archetypal storage, queries, change detection,
relationships, hooks and observers, and replace its multithreaded executor with a ~500-line
Forge one built on `forge-task` (its `System::run_unsafe`, access sets and `UnsafeWorldCell`
are public). Determinism: commands applied in (system, entity) order, per-item seeds.
*Not chosen:* full Bevy (f32 transforms and its renderer fight planet scale), `hecs` (no
change detection), `flecs_ecs` (alpha bindings), custom SoA tables (only if profiles indict
bevy_ecs). *(research: task-system.md §E)* Accepted by the owner 2026-09-24.

## D-007 — One material record for every system ✅ (2026-09-24)

A `Material` is one row: render layers; physics (static/dynamic friction, restitution,
combine rules, density, solidity/penetrability); audio (footstep and impact sound sets,
absorption); gameplay tags (slippery, sinkable, deformable, flammable, climbable, buoyant);
weather state overrides (wet, frozen, snow depth) mutated at run time; a `deform` block for
soft materials. Terrain layers, mesh sections, decals and the visibility buffer all reference
materials by ID; a hit anywhere resolves to one (per-triangle material ids are native in
Jolt). Deformable materials own a clip-mapped displacement layer that physics contacts
(feet, paws, wheels, impacts) write into, that rendering and audio read, and that weather
refills (Batman: Arkham Origins 2014, Rise of the Tomb Raider snow).
*(research: vegetation-materials.md §8, physics-fluids.md, audio.md §4)* Accepted by the owner 2026-09-24.

## D-008 — Lighting tiers and the ray-tracing policy ✅ (2026-09-24)

Four tiers on one renderer: **T0 raster** (SDF-updated probe clipmaps, screen-space
indirect, SSR, cascaded shadow maps), **T1 hybrid** (ray-query DDGI probes, ReSTIR direct
lighting, hybrid reflections, NRD — the RTX 3080 target), **T2 RT GI** (ReSTIR GI + radiance
cache + Ray Reconstruction), **T3 path traced** (ReSTIR PT + neural radiance cache, opacity
micromaps, cluster acceleration structures) used as the golden-image reference. Sky:
Hillaire 2020 (lifted from the previous project). Upscaling: DLSS 4.5 via Streamline on
NVIDIA, FSR 3.1 / XeSS on Vulkan elsewhere.
**Decision needed:** ship with hardware ray tracing *required* (Doom: The Dark Ages and
Indiana Jones do; it removes T0 from the product) or keep T0 for integrated GPUs and tools?
Recommendation: RT required for players, T0 kept only for tools and servers.
*(research: lighting-gi.md)*

## D-009 — Physics: Jolt first, box3d tracked ✅ (2026-09-24)

Bind **Jolt Physics** through a Forge-owned fork of JoltC (`joltc-sys` on crates.io is stuck
at 5.0; Jolt is at 5.6): `CROSS_PLATFORM_DETERMINISTIC` on (≈ 8 % slower, needs precise FP
and no FMA contraction), `JPH_DOUBLE_PRECISION` on (5–10 %), one physics system per
construct/space, `CharacterVirtual` with arbitrary up for characters, its vehicle constraint,
motorised ragdolls (D-012), soft bodies for ropes and cloth. First three tests: Jolt's
documented non-deterministic corners (broad-phase query order, narrow-phase result order,
listener callback order). **box3d** (Erin Catto, alpha since May 2026, worker-count-
independent determinism by default, trivial to bind) is tracked behind the same trait and
re-evaluated at its 1.0 or in twelve months. PhysX 5 rejected for authority (same-platform
determinism only, bindings archived), Avian rejected (Bevy-coupled), Rapier kept as the
pure-Rust reserve. Water: far = analytic spectrum evaluated identically on CPU and GPU;
mid = server-authoritative column model shadowed by a GPU shallow-water heightfield; near =
GPU particles, visual only; boats by submerged-triangle hydrostatics.
*(research: physics-fluids.md)* Accepted by the owner 2026-09-24.

## D-010 — Netcode: QUIC transport, our own replication ✅ (2026-09-24)

Transport: QUIC via `quinn`/`rustls` behind a `Transport` trait — unreliable datagrams for
inputs and snapshots, reliable streams for events, TOFU-pinned certificates now, signed login
tokens later; WebTransport for browser clients when needed. Replication is ours: 60 Hz
simulation, 30 Hz snapshots near the player, ~800-byte packets under the 1200-byte datagram
floor, per-client byte budget with a priority accumulator, 32-baseline ack ring per entity,
cell-relative positions and smallest-three quaternions in a hand-written bit writer, cell
interest management, authority handoff between workers. Inputs: 60 Hz, four redundant per
datagram, a server-reported jitter buffer the client paces itself to (a starved buffer waits,
never repeats); reconcile above a threshold and fade corrections over ~100 ms; predict
physics contact groups; 1 s hit-volume history for lag compensation. Clients talk only to the
replication layer; workers are stateless `bevy_ecs` processes; MongoDB write-behind;
RabbitMQ for events only. Proof stages: 2-player handoff → 100 bots at ≤ 24 KB/s over
100 ms / 2 % loss for 10 min → 8-player hit registration → 200 bots crossing a worker
boundary with replay diff → browser client.
*(research: netcode.md)* Accepted by the owner 2026-09-24.

## D-011 — Audio: own data layer and mixer, Steam Audio spatialiser ✅ (2026-09-24)

`cpal` device I/O → our own lock-free mixer graph (bus tree, sends, RTPCs, states and
switches, HDR loudness culling, virtual voices, banks) → Steam Audio through `audionimbus`
for HRTF, occlusion and reflections → a third-order ambisonic bus rendered binaurally or to
speakers → HDR master. Events authored as data in Wwise vocabulary. SADIE II HRTFs
(Apache-2.0) by default. Acoustic LOD in three tiers: near ray-traced, mid ISO 9613 + SDF,
far virtual. Impacts and footsteps from modal banks fitted per material (D-007); rain, wind
and water textures driven by the weather fields; Opus for voice. Wwise/FMOD are the yardstick
(both free under indie thresholds) but not dependencies.
*(research: audio.md)* Accepted by the owner 2026-09-24.

## D-012 — Animation: layered, kinematic first, powered ragdolls ✅ (2026-09-24)

Server simulation object → **clip layer** (blend spaces of a few clips with inertialization;
`ozz-animation-rs` as the deterministic clip runtime; motion matching only once capture data
exists) → **procedural layer** (two-bone and FABRIK IK, foot locking through
inertialization, look-at, motion warping for ledges and vaults; emits foot-down events with
position, normal, pressure, foot shape, material and tick for the deformation system, D-007)
→ **physics layer** (Jolt powered ragdolls driving motors to the kinematic pose with a
strength schedule, so hits, shoves and falls are simulated rather than played; later a
learned tracker in the DReCon → SuperTrack line). Generated creatures are authored against
chain roles with gaits derived from the body plan (Spore's architecture) and IK onto the
ground. Tiers: full stack near, clips only mid, texture-animated instances far. Networking
replicates parameters and hit events, never poses.
*(research: animation.md)* Accepted by the owner 2026-09-24.

## D-013 — Vegetation, impostor ladder, trim sheets ✅ (2026-09-24)

Trees are grown, not modelled: Weber–Penn parameter files per species, space colonisation
with competition for the skeleton, scanned bark and leaf atlases (CC0 Poly Haven / ambientCG,
the free Megascans slice). Rendering ladder: 0–40 m alpha-tested geometry with wind and leaf
translucency; 40–150 m billboard-cloud leaf cards; 150–600 m hemi-octahedral impostors with a
light-facing shadow pass and an ellipsoid ray-tracing proxy; beyond, a lit canopy volume —
behind an interface a voxel aggregate (Nanite Foliage's direction) can replace. Grass as in
Ghost of Tsushima: GPU-generated blades from a hash, shared wind field, per-tile interaction.
Buildings use **trim sheets** (still standard: Sunset Overdrive, Uncharted 4, Helldivers 2):
two to four regional trim sheets, six to ten tileables, a decal atlas and vertex paint, the
shape grammar snapping tagged faces onto trim rows; trims can be generated procedurally from
SDF profiles. One normal-compositing rule (Mikkelsen's surface-gradient framework); KTX2 +
Zstd with BC7/BC5/BC4.
*(research: vegetation-materials.md, large-worlds.md §7)* Accepted by the owner 2026-09-24.

## D-014 — Terrain representation ✅ (2026-09-24)

Equi-angular cube-sphere quadtree for planets and a flat grid for islands, CDLOD heightfield
far field, dual contouring / Transvoxel volumetric near field for overhangs and caves, SDF
bricks for edits, genesis (uplift, stream-power erosion, hydrology) baked per region and a
deterministic runtime detail layer. Lifted from the previous project's design and code.
*(research: large-worlds.md §8, RESEARCH.md §1)* Accepted by the owner 2026-09-24.

## D-015 — Services: MongoDB persistence, RabbitMQ events only ✅ (carried over)

Unchanged from the previous project: persistence is write-behind to MongoDB (transactions
need a replica set), RabbitMQ carries asynchronous events (persistence jobs, chat, economy,
telemetry) and never real-time replication. Both stay on the Pi 5 until load requires more.

## D-016 — Determinism rules ✅ (2026-09-24)

No platform float functions in generation and simulation (`forge_core::dmath`, lint coming
with `forge-sim`); seeds derived per item (`Seed::derive`), never shared generators; parallel
results merged by index; CI digests at 1 and 6 workers, debug and release. Lifted from the
previous project's harness.

## D-017 — Demos are milestones; golden images ✅ (2026-09-24)

A system is done when its demo runs on both machines with numbers in `docs/demos/`. Every
demo supports `--frames` and `--capture`; `tools/imgdiff` compares captures with a tolerance
and an exit code. p50/p99/max, never means.

## D-018 — Memory and streaming ✅ (2026-09-24)

**CPU:** a tagged heap over one reserved virtual range (2 MiB blocks, per-worker blocks,
bulk free by frame/stage or by cell id), `slotmap` pools for tables, `bumpalo` frame arenas
until Rust's `Allocator` trait is stable, mimalloc as the global allocator for the long tail;
every allocator reports to Tracy. **GPU:** keep `gpu-allocator` now; replace with an in-house
TLSF layer (256 MiB blocks) or `vk-mem` when budgets, aliasing and defragmentation are
needed — budget from `VK_EXT_memory_budget` minus 10 %, render-graph aliasing of transients,
incremental defragmentation on the transfer queue (~16 MiB per frame). **Resizable BAR** is
used only for sequential, 64-byte-aligned, never-read per-frame data (what `CpuToGpu`
already does); bulk uploads go through staging on the transfer queue. No sparse residency:
paging is software page tables over ordinary pools. **Streaming:** dedicated I/O threads
(IOCP now, io_uring on Linux), a request scheduler that fetches by need (screen error,
distance, camera prefetch) and evicts by lowest need with recency only as a tie-breaker (the
cure for the previous engine's LOD popping), per-frame upload budgets, never a wait in the
frame; CPU zstd/LZ4 decompression by default (they out-decode the NVMe on this CPU), GPU
GDeflate through `VK_EXT_memory_decompression` when probed and when CPU headroom is short.
Cluster pages after Nanite: fixed 128 KB pages, root and hierarchy always resident,
GPU-emitted page requests read back asynchronously. **Container:** a table of BLAKE3
content-addressed, 256 KiB block-compressed, dependency-ordered, 4 KiB-aligned chunks; KTX2
per-level supercompression for textures. Proof: a 64 km² world flown at 300 m/s with
residency and bandwidth graphs and no frame over 20 ms, degrading to blur when the drive is
throttled. *(research: memory-streaming.md)* Accepted by the owner 2026-09-24.

## D-019 — Weather as one shared state ✅ (2026-09-24)

Cloud coverage, precipitation, temperature, wind vector, humidity in one struct written by
the simulation and read by rendering (precipitation, wetness, snow), materials (D-007),
audio and physics (wind forces). *(research: lighting-gi.md — weather rendering not yet
researched, vegetation-materials.md §8 — pending)*

## D-020 — Render graph: declared accesses, derived barriers, aliased transients ✅ (2026-09-24)

Every pass declares the images (per mip level where it matters) and buffers it reads and
writes, with an access kind (`ColorAttachment`, `DepthAttachment`, `Sampled(stages)`,
`StorageWrite(stages)`, `IndirectArgs`, …); the graph derives every barrier and layout
transition from the tracked state of each subresource and records the passes in
declaration order, one profiler zone per pass label. Per-frame images are transients of the
graph, laid out in one heap from their lifetimes (largest first, first fit among the images
whose lifetimes intersect), so images that never coexist share memory; their first use in
a frame waits for whatever last touched that memory, in this frame or the previous one.
Persistent images and buffers (`GraphImage`, `GraphBuffer`) carry their state across
frames; anything a frame in flight may still use is destroyed through the frame slots
(`Frames::destroy_later`). Nothing is culled or reordered, there is one queue, and nobody
outside `forge-gpu` records a barrier. Async compute and transfer queues are the next
extension (a queue per pass, timeline waits and ownership transfers on crossing edges),
as are transient buffers. The graph lives in `forge-gpu` (not `forge-render` as first
planned) because the app shell and every renderer draw through it and the barrier
vocabulary is Vulkan's; the module is written without `unsafe`.
*Measured:* the ballad and the bench through the graph are pixel-identical to the
hand-written barriers (0 pixels over seven captures, and 0 between the aliased heap and
`FORGE_GRAPH_NO_ALIAS`), validation and synchronization validation clean; a ballad frame
is 19 passes, 37 image barriers and 3 memory barriers, its three transients 25.6 MB. Nothing
aliases in that frame yet (colour, depth and motion vectors are all alive at the resolve);
the heap pays once post-processing chains arrive. *(research: task-system.md §F,
memory-streaming.md §2; issue #1)*

## D-021 — Visibility buffer: a 32-bit id next to the hardware depth, shading in compute ✅ (2026-09-24)

The rasterisers write no attributes. The hardware path writes a 32-bit id
(`visible slot << 7 | triangle`, the slot indexing a per-frame visible-cluster list of
`(instance, cluster)` filled by the cluster cull) into an `R32_UINT` target next to the
hardware depth, and a compute pass shades once per pixel from the id, reconstructing the
perspective-correct barycentrics and their derivatives analytically (no `ddx`, no helper
lanes; `docs/ARCHITECTURE.md` §4). The plan's 64-bit `depth | id` atomic target for every
rasteriser was built and measured with the software rasteriser (#3) and not kept: the
fragment atomic, the export of its depth and the clear cost 0.05 ms at the LOD views (bench
0.19 → 0.24 ms, ballad 0.34 → 0.40) where the software rasteriser brings nothing. The
hardware keeps this 32-bit id and its depth test; the software rasteriser keeps 64-bit
samples only where they beat the hardware's pixel, with the same rule (nearer, or at equal
depth the larger id: the depth test keeps the last drawn and the hardware draws in id
order), and a merge pass writes them into the id and the depth (amended 2026-09-24).
Material classification and per-material
dispatches sit on top of this resolve and read the D-007 table (D-026, issue #20). Chosen over shading
in the mesh passes because it makes shading cost independent of overdraw and triangle
size, gives the mesh-shader, software and fallback rasterisers one shading path and is the
input the material table needs; chosen over `VK_KHR_fragment_shader_barycentric` in a
fullscreen fragment pass because the compute form has no 2×2 quads to waste on small
triangles and is the form the material passes will take.
*Measured:* ballad frame 0.27 → 0.30 ms (mesh pass 1 0.07 → 0.06, resolve 0.03), bench
0.15 → 0.18: a small loss with one light and one normal, expected until materials get
expensive. Culling A/B at 0 pixels; against the forward-shaded captures 39 of 1.44 M pixels
differ by more than two levels (slivers at silhouettes). The visibility transient is the
graph's first aliasing customer (it shares memory with the motion vectors).
*(research: gpu-geometry.md, Burns & Hunt 2013, Schied & Dachsbacher 2015, Hable 2021;
issue #6)*

## D-022 — Physical light units, pre-exposure, histogram exposure, tone curves as data ✅ (2026-09-24)

Lights are given in photometric units and every pass writes **pre-exposed luminance**
(Lagarde & de Rousiers 2014): the sun is an illuminance in lux (128 000 for the Sun at
1 AU), its disc a luminance equal to that illuminance over its solid angle (1.9 · 10⁹
cd/m² at 0.267°), surfaces return albedo × E / π in cd/m², and each shader multiplies by
the frame's exposure so fp16 targets hold values near 1 from starlight to noon (sky values
are clamped at 16 384 before the fp16 limit). Exposure is a camera value, EV100, with
exposure = 1 / (1.2 · 2^EV100). **Automatic exposure** meters a 256-bin log2 luminance
histogram of the finished HDR image (a compute pass counting in shared memory, then
device-local memory, copied to a cached readback buffer per frame slot and read two
frames later, no stall): the black bin is ignored, the key is the log-average of the
samples between the 50th and 98th percentiles of the rest, EV100 follows it at 1.5 per
second towards darker and 0.8 per second towards brighter (Narkowicz 2016), with
compensation and a clamp, and snaps on the first metered frame. The adaptation runs on
the CPU (unit-tested, deterministic under `--fixed-step`); a GPU-resident loop is the
option if the two-frame latency ever matters. Temporal filters rescale their history by
the ratio of exposures. **The display transform is data** chosen at run time: AgX (engine
default, hue-safe), ACES as Hill's fit of the 1.x RRT + sRGB ODT (ACES 2.0's output
transform is not implemented), Khronos PBR Neutral; one Slang module serves the TAA resolve
(history and display image in one pass) and a stand-alone display pass, with CPU mirrors
under test. Emissives that cannot be physical at the same exposure (the ballad's stars and
nebula, eight orders of magnitude below a sunlit rock in reality) are authored in units of
a sunlit white Lambertian surface and documented as art-directed. Readbacks declare a
`HostRead` access in the render graph so device writes are visible to the host.
*Measured:* along the ballad's 90-second path the exposure stays within EV100 14.4–15.0,
changes at most 0.55 EV per second and reverses by more than 0.1 EV ten times (one swing
every nine seconds, the largest 0.52 EV): adaptation without pumping. The histogram costs
0.02 ms of GPU, the frame 0.30 → 0.30–0.31 ms. Culling A/B at 0 pixels, two runs
bit-identical. The ballad defaults to ACES (its toe keeps space black; AgX's 16.5-stop log
encoding lifts the nebula to a flat grey), the bench to AgX. *(research: lighting-gi.md §5
and §10; issue #7)*

**Bloom** (issue #44, 2026-09-25) follows lighting-gi.md §10 (Jimenez 2014). It is computed
from the pre-exposed HDR frame, before the tone curve:
- a chain of six half-size levels, down with the 13-tap filter, the first step weighting each
  box by 1 / (1 + luma) against fireflies;
- back up with a 3×3 tent, each level adding the one below;
- the TAA resolve blends the top level, averaged over the levels, into the image it shows
  (4 % by default, `--bloom`, **B**).

The history stays unbloomed, so bloom never feeds back. It costs 0.04 ms at 1600×900 and
0.08 ms at 1440p.

## D-023 — Atmospheres: Hillaire 2020 tables in the graph, a per-pixel march from space ✅ (2026-09-24)

An atmosphere belongs to a planet (or any body massive enough to hold one): a Rayleigh
layer, an aerosol (Mie, Cornette–Shanks phase) layer and an ozone tent over a spherical
ground, in kilometres and per-kilometre coefficients (`AtmosphereParams`, Earth from
Hillaire 2020 / Bruneton 2017 as the preset). Empty space has none. Two tables are built by
compute graph passes (`sky/atmosphere tables`) only when the atmosphere changes:
**transmittance** to the top of the atmosphere (256 × 64, Bruneton's mapping) and the
**multiple-scattering** transfer (32 × 32: second order from 64 directions, then the
geometric series). Luminance is per unit of sun illuminance, so the result is multiplied by
the same pre-exposed illuminance as everything else (D-022). **Seen from outside the
atmosphere** (the ballad, orbit), the sky pass marches each pixel's ray through the shell
with both tables: 16 segments packed towards the ray's lowest point and the ground, exact
per-segment integration, the planet's own shadow; the ground under it is lit by the sun
through the air plus π × the multiple-scattering term as skylight, and whatever lies
behind the air (stars, the sun's disc) is multiplied by the ray's RGB transmittance.
Ray–sphere spans are computed from the point of closest approach, `(r − h)(r + h)`, which
keeps metre precision 20 000 km out. **Seen from inside** (ground, flight), Hillaire's
sky-view and aerial-perspective tables come with the first demo that stands on a planet
(Phase 2). *Measured:* the CPU mirror of the transmittance integral (tests) gives Earth's
noon sun 0.87 in green and a horizon sun red over blue by more than 20×; 16 segments are
within 4/255 of a 128-segment reference; the sky pass costs 0.11 ms with the planet out of
view, 0.14–0.16 ms with the ballad's planet (18° radius) in view and 0.31 ms for a planet
filling the screen, which a planet-view table would make a lookup (#26). *(research:
lighting-gi.md §6; issue #8)*

**The ground view** (issue #43, 2026-09-25) is `forge_render::sky`, three passes a frame over
the same tables:
- **A sky-view table** (192 × 108): the in-scattered light around the camera, the elevation
  squashed towards the horizon, the azimuth taken from the sun's. Rays that meet the planet
  add its ground, lit by the sun and the sky and seen through the air.
- **An aerial-perspective volume** (32 × 32 froxels × 32 slices to 8 km, quadratic in
  depth, a 1024 × 32 atlas): the light gathered and the mean transmittance from the camera
  to each slice.
- **A compose pass:** the sky and the sun's disc where the depth is empty; elsewhere
  `colour × T + L`, from the pixel's distance.

The scene's sunlight takes the sun's colour through the air (`MeshletRenderer::sun_color`;
the ballad keeps its space white). City-blocks stands on the Earth's surface: at 1600×900
the three passes cost 0.013 + 0.011 + 0.025 ms.

**Sky light** (issue #47, 2026-09-25). A fourth pass, `sky/irradiance`, projects the
sky-view table onto nine spherical-harmonic coefficients convolved with the clamped cosine
(Ramamoorthi and Hanrahan 2001; `shaders/sh.slang`, mirrored in `forge_render::sky` with
tests):
- one workgroup sums 4096 directions of a Fibonacci sphere and reduces them in a fixed
  order, so the coefficients are the same on every run;
- the table holds the planet's sunlit ground below the horizon, so the coefficients carry
  its bounce as well as the sky's light.

The tables' passes now run before the resolve (`GroundSky::tables`) and the compose after
it (`GroundSky::compose`). The resolve lights every material class with the sun plus that
irradiance for the pixel's normal, in the same units (a white Lambertian surface facing
the sun). Scenes without a sky keep the constant fill.

**A moving sun** (issue #57): with `city-blocks --day`, the sun crosses the sky and every pass
above follows it per frame; the sunlight's colour is recomputed through the air, and the
exposure is metered (D-022).

This is the sky term of research step (3), without the probes: it is the same everywhere in
the scene, so nothing occludes it yet. Screen-space ambient occlusion is the next step, and
probe GI the one after. At the default sun, a roof receives 0.075 of the sun and a wall about
0.20, most of it from the ground; the pass costs 0.016 ms.

## D-024 — DLSS through Streamline's interposer, optional, TAA as the default ✅ (2026-09-24)

DLSS Super Resolution runs through NVIDIA Streamline 2.14 (the SDK in `streamline-sdk/`,
git-ignored), with the binding lifted from the `world` project into `forge-gpu`
(`streamline.rs`, every FFI structure's size and padding checked at compile time). It
is **optional**: the `dlss` feature (Windows) loads `sl.interposer.dll` in place of the
Vulkan loader (`Instance::with_streamline`, `AppConfig::streamline`). The device offers
`Device::dlss()` when its GPU runs it. Without the feature `Dlss` is an uninhabited type,
so renderers and demos compile without feature gates. **TAA stays the default and the
fallback**; U switches TAA → DLAA → Quality → Balanced → Performance → Ultra Performance
at run time (`--upscaler` at start), and the profiler zones are `temporal/TAA resolve` or
`temporal/DLSS` + `post/display transform`.

DLSS reads the same inputs as the TAA resolve: the jittered pre-exposed HDR colour, the
reversed-Z depth, the motion vectors (UV offsets, unjittered) and a 1 × 1 exposure of 1.
Pre-exposure is passed relative to EV100 15; DLSS's output kept the input's level. The
scene is drawn at DLSS's input size, with a jitter sequence of 8 × (output / render)² phases
and **the LOD error measured in output pixels**, so the geometry stays as detailed as the
screen (in render pixels the Quality and Performance modes drew visibly faceted rocks).

The render graph was extended rather than bypassed: resolved images carry their usage and
sampled layout for Streamline's tags, and a `Custom` access (explicit layout, stages and
access) declares the output, which NGX clears at the transfer stage before writing it
from compute (synchronization validation found the missing transfer stage). Tags are
`eValidUntilPresent`: `eOnlyValidNow` made Streamline copy every input first. The device
enables Vulkan 1.3's `privateData`, which Streamline uses. Through the interposer, MAILBOX
presents were held to the display's refresh in most runs, so the swapchain prefers
IMMEDIATE when Streamline is loaded.

*Measured* (1600×900 output, RTX 5070 Ti, ballad path average): TAA 0.333 ms GPU; DLAA
0.768, Quality (1067×600) 0.690, Balanced 0.662, Performance (800×450) 0.646, Ultra
Performance (533×300) 0.600. The DLSS pass itself costs 0.43–0.46 ms in every mode (it
scales with the output), the scene saves 0.1–0.2 ms at the smaller sizes, and Streamline
adds about 0.07 ms of CPU to recording. In a 0.33 ms scene DLSS costs more than it saves.
It pays off once the scene costs more than about half a millisecond more at native size
than at the input size (the city-blocks target at 1440p). Validation and synchronization
validation are silent in every mode and across the switch. *(research: lighting-gi.md §9;
issue #8)*

## D-025 — Cluster pages and their residency ✅ (2026-09-25)

D-018 fixed the frame: 128 KB pages, the roots always resident, GPU requests read back,
eviction by need. This is what the pages hold and how residency follows the cut
(`forge_geom::page`, `forge_render::streaming`, issue #36).

**The page format.** A page is 128 KiB of cluster payloads. A cluster's payload is its own
vertices, then its triangles. Each vertex is 16 bytes: the exact f32 position and a 16-bit
octahedral normal (at most 0.0035° off). Each triangle is three one-byte local indices.
A page needs nothing outside itself.
- **The roots** fill the first pages of each mesh, loaded at start and never evicted.
- **A DAG group never spans two pages.** Every cluster records the page of its children
  (the members of the group that produced it) as `child_page`.
- **Groups go level by level,** finest first, in Morton order of their spheres.
- **The hierarchy** (the 112-byte cluster records, the mesh records) stays resident.
- **On disk,** the pages follow the hierarchy in the mesh-cache file, 4 KiB-aligned, so
  one page is one positioned read.
- **The GPU** holds a pool of page slots and a page table (page → slot). The fallback
  path binds the pool as its index buffer.

**The cut through what is resident.** A cluster draws when its page is resident, its
parent is too coarse, and it is fine enough or its children's page is absent. One page
holds all of a group's children, so this is one watertight cut through any resident set
that contains the roots. A missing page costs detail, never a piece of surface. The cull
records, per page, the largest projected error the page takes away: a drawn cluster keeps
its own page, a too-coarse one asks for its children's. A second cull pipeline, compiled
with streaming, carries that code, so resident scenes do not pay for its registers
(+10 % when they shared one).

**Residency stays closed upwards,** after Nanite's dependencies:
- A page loads only when every page holding a parent of its clusters is resident.
- Only pages with no resident children are evicted.
- A page holds about a dozen groups whose parents lie in several pages, some of which the
  cut may not want for themselves. A wanted page therefore lends its need to all its
  ancestors, which load first and stay while it does. Without that loan the static view
  kept 21 pages waiting forever.

**Priorities.** Reads go to an I/O thread (blocking positioned reads for now; IOCP with
D-018's container), the neediest first, a bounded number in flight. Uploads happen at
most `upload_pages` per frame, the neediest first, into free slots, or into the slot of
the evictable page with the lowest need (the least recently wanted among equals). A page
is uploaded only when it is needed more than the page it replaces. The upload is one
`streaming/upload` graph pass before the culls.

*Measured* (city-blocks, 1 M instances, 7 868 pages = 983 MiB; RTX 5070 Ti, 1600×900):
- **The static view** settles in 39 frames (44 ms) from the roots alone, with 395 pages
  (49 MiB) resident, at 1.12 ms against 1.05 with every page resident.
- **The flight at 300 m/s** keeps 540–640 pages resident. With a 48 MiB pool (384 slots)
  it uploads 0–1.6 pages a frame in real time at 60 Hz, draws the same frames as with
  every page resident (0 pixels apart at the captured frames), and holds geometry to
  225 MiB against 1 160.
- **Pools too small for the cut** (16 and 24 MiB) draw coarser surfaces, never holes: no
  pixel inside a surface of the resident image shows the sky.

Left for later: compressed vertices (quantised to a per-mesh grid) and D-018's container
(BLAKE3 chunks, zstd), IOCP reads, and a transfer-queue upload. *(research:
memory-streaming.md §4, gpu-geometry.md; demo: city-blocks)*

## D-026 — Materials on the GPU: a row per instance, shading by class ✅ (2026-09-25)

D-007's record is `forge_core::material::Material`:
- a render layer: the shading class, two base colours, a cavity term, roughness, the
  highlight's weight, emission, the ice's scattering, and two textures with their scale;
- a physics layer: density, static and dynamic friction, restitution;
- gameplay tags.

Physics and audio will read the same rows. The renderer uploads the render layers as a table
of 80-byte rows (`forge_render::material`), and every instance names its row: its mesh's by
default (`set_mesh_material`), or its own (`add_instance_with_material`). GPU placement copies
the mesh's (issue #20).

**Shading by class.** The resolve runs one pass per shading class:
- **The standard class** covers the target. It shades its own pixels, writes the background
  into empty ones, and ORs, per 8×8 tile, the other classes the tile shows (a wave OR, then a
  group OR).
- **Every other class** (the ice today) keeps a list of those tiles and shades its pixels there
  in an indirect dispatch, 64 groups wide and as many rows as the list needs. A class can
  therefore hold more tiles than one dimension of a grid allows.

Each class compiles only its own code.

**Textures without coordinates.** Cluster pages hold no texture coordinates (D-025), so
textures are projected along the object's three axes:
- the weights are the normal's components to the fourth power;
- normal maps use the whiteout blend (Golus 2017);
- `SampleGrad` takes the derivatives of the object-space position, which the analytic
  barycentric derivatives give (D-021);
- a value noise over a few repeats varies the brightness, so the repeat does not show.

The demos' textures are procedural: rock, concrete, brick and grass, 512 × 512, tileable,
their mips averaged in linear light. The city generates them in 140 ms at start-up.

**Why the standard class does the classifying.** A separate classify pass was built first
and measured. It cost what the old resolve cost (0.026 ms on the bench at 1600×900, 0.077 at
1440p), because it reads the visibility buffer just as the shading does. Carrying the class
in the visible list, so the classify skipped two dependent loads, saved nothing and cost the
cull 0.008 ms. Folding the classification into the class that covers most pixels leaves
one pass over the buffer for a frame of standard materials.

**Why not an übershader.** The ice pays for its code only in its own tiles, and a new class
(translucent ice #12, foliage, terrain layers #42) adds a pass rather than registers to
every pixel.

**Why a row per instance, not per triangle.** The visibility id already leads to the
instance. Per-triangle materials, such as a building's glass, need material sections through
the cluster DAG (#41).

*Measured* (RTX 5070 Ti, 1600×900 unless noted):

| View | Old single resolve | Shading passes | Whole frame |
|---|---|---|---|
| bench | 0.029 | standard 0.040 + ice 0.009 | 0.177 → 0.197 |
| ballad | 0.024 | standard 0.035 + ice 0.011 | 0.318 → 0.340 |
| city (now textured) | 0.052 | standard 0.099 | 1.257 → 1.249 |
| city flight at 1440p (textured) | 0.090 | standard 0.215 | 1.652 → 1.788 |

- **The ballad's rock and ice** are two rows that reproduce the Phase 0 shading. Its
  captures without TAA are identical to the single resolve's (0 pixels at frames 100, 240,
  300, 500 and 600, both paths). With TAA, sub-8-bit float differences accumulate in the
  history: 0.08 % of pixels differ by more than two levels at frame 600. Each build is
  identical to itself from run to run.
- **The mip check** (`meshlets --mip-check`) compares the level `SampleGrad` picks from the
  reconstructed derivatives with the level a fragment shader picks from its 2×2 quads, over
  a ground quad at a grazing angle: at most 0.062 of a level apart (mean 0.0085) over
  781 624 pixels.

*(research: gpu-geometry.md, vegetation-materials.md §8; D-007, D-021; demos: meshlets,
asteroids, city-blocks)*

## D-027 — Material sections through the cluster DAG ✅ (2026-09-25)

A mesh may carry material sections, a number per triangle (`TriMesh::sections`). An
instance draws section `s` with the material row `s` after its own, so the instance still
names one row, and an override still picks all of them (issue #41). The city's buildings
have two sections: the facade, and the window panes, the flat backs of the recesses. Their
rows follow each other in the table.

**Through the cook:**
- **Vertices on section borders are split,** one copy per section, so no triangle joins two
  sections.
- **Group borders are found by position** (meshoptimizer's position remap), so both copies of
  a border vertex lock together and neighbouring groups stay watertight.
- **The section is also a vertex attribute of the simplification,** weighted at 0.5 m of
  error per unit. Meshes with sections simplify in meshoptimizer's permissive mode: a
  collapse may cross a section border, and pays that weight for it. With the borders kept as
  seams instead, the window outlines outlived every level. The south view then drew 71 k
  clusters instead of 50 k, and its culls took 0.37 ms instead of 0.23.
- **A cluster holds at most two sections.** Its triangles are sorted by section, and its
  record packs `a | b << 8 | split << 16`. Splitting every cluster at section borders had
  lowered the fill from 0.69 to 0.65 and drawn 79 k clusters. The rare cluster that meets
  three sections is clustered again, section by section.

**On the GPU** the resolve takes the triangle's section from its index (`triangle_section`)
and reads row `instance.material + section`. The builder refuses an instance whose rows
would run past the table.

*Measured* (city-blocks, RTX 5070 Ti; the other demos have no sections and draw the same
pixels as before):
- **The buildings** have 3–4 % more clusters (fill 0.69 → 0.67).
- **The south view** draws 58 k clusters and 3.72 M triangles instead of 50 k and 3.43 M
  (windows keep their glass until they are a few pixels wide). GPU per frame: 1.10 → 1.35 ms
  with every page resident, 1.25 → 1.47 ms streamed (the culls 0.27 → 0.37 each); the flight
  at 1440p 1.79 → 1.83 ms.

**Note (issue #51, 2026-09-25):** at coarse levels, permissive simplification can move a
pane's corner onto the facade's copy of a border vertex. The triangle's section was its first
vertex's, so some panes showed half their area in the facade's colour; full detail had none.
A triangle's section is now the one most of its vertices carry, and the cook version went to 4.
The large half-panes are gone. A sliver remains where two of the three corners are facade
copies; fixing those needs the section carried from the finer level through simplification.

*(research: gpu-geometry.md (Nanite's materials per triangle); D-007, D-026; demo:
city-blocks)*

## D-028 — Terrain in layers: a layer map and the rows after it ✅ (2026-09-25)

Ground is one mesh, but it is made of many materials: asphalt, sidewalk and paving in the
city, grass, soil and rock in the hills. A terrain row of class `layered` (issue #42) names
a **layer map**:
- one byte per texel (`R8_UINT`) over a square of the object's x and z;
- layer `k` is shaded as the standard row `k + 1` after the layered row, textures, highlight
  and all.

The layered pass reads the four texels around the pixel and weighs each layer by the
bilinear weights of its texels. It shades the two heaviest layers and blends them, so a
street's edge is a metre-wide transition rather than a step between texels.

**Why a map, not rules in the shader.** Rules would have to know the city's grid, or the
island's rivers. A map is data that any generator writes: the city's layout today, the
island's genesis in Phase 2 (altitude, slope, moisture, D-014). It costs a byte a square
metre: 16 MB for the city's 4 km. The rules that fill it live on the CPU:
- `placement::ground_layer` gives asphalt down the streets, sidewalks 3.5 m wide along
  them, paving on the plazas and grass on the lots;
- beyond the city, rock where the ground rises more than 0.45, grass elsewhere.

*Measured:* the city's layer map (4000²) is generated in 45 ms on the CPU. `shading/layered`
costs 0.031 ms on the south view at 1600×900 (GPU 1.465 → 1.491 ms) and 0.05 ms in the flight
at 1440p (1.830 → 1.868 ms). The other demos have no layered rows, and their captures are
unchanged.

Left for later: layer maps streamed in tiles with the terrain's cells (Phase 2), a height per
layer for sharper transitions (height blending), and decals for road markings.
*(research: vegetation-materials.md §8, large-worlds.md; D-007, D-014, D-026; demo:
city-blocks)*

## D-029 — Sun shadows by ray query: a BLAS per mesh from its DAG, a TLAS over the instances ✅ (2026-09-25)

D-008 makes hardware ray tracing required for players, and its first tier casts the sun's
shadows with rays. This is that tier's first piece (issue #45). It is built on ray queries
from the compute resolve, with no ray-tracing pipeline and no shader binding table.

**`forge-gpu`** enables `VK_KHR_acceleration_structure` and `VK_KHR_ray_query` on devices
that have them (`FORGE_NO_RAY_QUERY=1` turns them off for testing). It also enables
`VK_KHR_ray_tracing_pipeline`, though no pipeline uses it: Slang's address-to-structure
conversion declares `SPV_KHR_ray_tracing`, which the validation layers accept only with that
extension on. Every RTX card has all three.
- `Device::build_blases` and `Device::build_tlas` build static structures in one-shot
  submissions: prefer fast trace, scratch aligned to 256 bytes.
- Shaders reach a structure by its device address (`RaytracingAccelerationStructure(address)`,
  SPIR-V's `OpConvertUToAccelerationStructureKHR`), so the bindless set needs no new binding.

**The geometry.** One bottom-level structure per mesh (`forge_render::raytrace`):
- **The cut:** the finest cut of its cluster DAG that fits 40 000 triangles (600 000 for the
  terrain), at a single object-space error. That is one watertight surface, read from the
  cluster pages, streamed ones included.
- **Why not the full detail:** it would cost 26 M triangles for the city's twenty props and
  its terrain. The cut costs 1.39 M. A shadow needs the silhouette rather than the bricks.
- **The price:** the traced surface may stand up to the cut's error off the drawn one. That is
  0.01–0.35 m for most props, about 1 m for the two towers (3 M triangles down to 40 000)
  and 0.02 m for the terrain. Shadow rays start 0.15 m off the surface, along the normal and
  towards the sun. A recessed window pane may be shadowed by the coarse facade in front of
  it; the glass is dark anyway.

**The instances.** One top-level structure over every instance. A compute pass
(`tlas_instances_main`) writes the 64-byte records from the scene's instance table, because
the city places its million instances on the GPU:
- the model's top three rows;
- the instance index;
- back faces traced too;
- the mesh's structure.

**The rays.** The resolve's `_rt` entry points, compiled only on devices with ray queries,
trace one ray per sun-facing pixel. Its flags are accept-first-hit, skip-closest-hit and
force-opaque. The standard, ice and layered classes scale the sun's diffuse and specular
light by the result (`CullFlags::SHADOWS`).

**Left for later:**
- soft shadows: done in #54 (below), with one ray and TAA instead of several and a denoiser;
- structures that follow streamed and moving geometry (refits, rebuilds, cluster
  structures; the ballad's rocks tumble in Phase 3).

*Measured* (city-blocks, RTX 5070 Ti):
- **The build, once:** 1.39 M BLAS triangles in 58 ms (the cuts read from the pages included);
  the TLAS over 1 000 001 instances in 12 ms. 278 MiB in all.
- **The shadow rays:** `shading/standard` 0.094 → 0.136 ms and `shading/layered` 0.030 → 0.040
  at 1600×900 (the south view 1.591 → 1.660 ms); 0.203 → 0.326 and 0.041 → 0.057 in the 1440p
  flight (2.023 → 2.192 ms).
- **Checks:** with `--no-shadows`, or without ray queries (`FORGE_NO_RAY_QUERY=1`), the captures
  are those of the previous build. The other demos have no top-level structure and are
  unchanged, though they run the `_rt` pipelines. Synchronization validation is silent.
**The ballad** (issue #46, 2026-09-25) builds its structures once at start: 267 k BLAS
triangles for its seven meshes in 8 ms, and the TLAS over 3000 asteroids in 1 ms, 16 MiB in
all. Its rays take 0.027 ms at 1600×900. `--no-shadows` or **J** turns them off, in both
demos.


**Soft shadows** (issue #54, 2026-09-25). The shadow ray aims at a point of the sun's disc:
over TAA's 8-frame jitter cycle, a pixel takes the eight points of a Vogel disc, turned by an
angle of its own from D-030's noise (`noise.slang`, since #55; independent points per frame
streaked in wide penumbrae). TAA averages them into the penumbra: one ray per pixel, no
denoiser. In motion TAA still smears wide penumbrae, so the ballad keeps hard shadows by
default. `Frame::sun_angular_radius` is 0 in the ballad (hard) and the Sun's at 1 AU in
the city. On a static view the slow change stays at hard shadows' 0.035 %, and the cost is
0.02 ms at 1440p. The penumbra of a thin occluder (a lamp post) fades with distance, as it
should. Wider penumbrae (a larger sun, an area light) would need more samples than TAA's
eight.
*(research: lighting-gi.md; D-008; issues #45, #46, #54; demos: city-blocks, asteroids)*

## D-030 — Ambient occlusion: GTAO from the depth, after XeGTAO, occluding the sky's light ✅ (2026-09-25)

Research step (3) feeds the sky's light to every surface (D-023's note, issue #47). Nothing
occluded that light, so contacts and recesses were lit like open ground. lighting-gi.md §8
recommends GTAO (Jimenez, Wu, Pesce, Jarabo 2016) for every tier, porting Intel's XeGTAO
(MIT). This is that port (issue #48, `shaders/gtao.slang`, `forge_render::gtao`). XeGTAO's
notice is in `shaders/third-party/XeGTAO-LICENSE.txt`.

**The passes** (all compute, on transients of the frame's size):
- **A distance chain:** the view-axis distance from the reversed-Z depth, and four levels,
  each a 2×2 average weighted towards the near samples (XeGTAO's filter, so a thin
  occluder survives).
- **GTAO:** 3 slices and 3 steps per side, which is XeGTAO's "high" preset. The samples are
  placed by a Hilbert-curve index into the R2 sequence, and their distance picks the level
  they read. The normal is rebuilt from the depth with XeGTAO's edge-aware cross products.
  The effect radius is 1.5 m (×1.457); the constants are XeGTAO's.
- **One 3×3 denoise** that does not cross depth edges.

**In the resolve.** The occlusion scales only the sky's irradiance: the sun has its own
shadow ray (D-029). It is applied through the paper's multi-bounce fit on the surface's
albedo, so bright materials lose less. Scenes without a sky do not compute it.

**One departure from XeGTAO:** the noise repeats with TAA's jitter (8 frames) instead of
every 64. With 64, TAA's history drifted between patterns: on a static view, 0.24 % of the
pixels changed by more than two levels over 32 frames, against 0.09 % without AO. With 8 it
is 0.10 %. A second denoise pass did not move either figure.

**Left for later:**
- specular occlusion and bent normals from the same horizons;
- a normal from the visibility buffer instead of the depth (exact on thin geometry);
- occlusion at the scale of a street, beyond a few metres of screen-space radius. That is
  the probes' job, or rays against the TLAS the shadows already use.

*Measured* (city-blocks, RTX 5070 Ti): the passes cost 0.13 ms at 1600×900 (the south view
1.683 → 1.852 ms) and 0.25 ms at 1440p (the flight 2.204 → 2.456 ms). With `--no-ao` the
captures are those of the previous build; mesh and fallback stay at 0 pixels apart.

**The ballad** (issue #55) applies it to its space fill, the wrap and the nebula's fill, with a
2 m radius: 0.085 ms at 1600×900, 0.463 → 0.554 ms.
*(research: lighting-gi.md §8; issue #48; demo: city-blocks)*

## D-031 — Reflections start with the sky: Fresnel-weighted, from the sky-view table ✅ (2026-09-25)

Research step (4) is hybrid reflections: rays on the RTX tiers (SSR first, then rays on a
miss), and the sky where a ray finds nothing. This is that last term, the sky, and it is the
whole reflection until the rays come (issue #49).

**The model** (`sky_reflection` in `shaders/meshlet.slang`, every material class, under a
sky):
- Schlick's Fresnel with F0 = 0.04 (a dielectric), its rise at grazing angles bounded by
  `1 − roughness`. The roughness is recovered from the Blinn-Phong exponent the rows carry.
- What is reflected: the sky-view table in the mirror direction, blended towards the sky's
  irradiance (D-023's note) by 2 × roughness². The table has no blurred levels, and its
  low frequency suits the smooth rows (glass) best.
- A specular occlusion from GTAO's visibility (D-030), after Lagarde and de Rousiers 2014.
- The diffuse part is scaled by 1 − F.

The table's frame (up, the sun's azimuth) and sampled index follow the nine coefficients in
the sky-light buffer. The index is stored as a float value: its bits as a float would be a
denormal, which a GPU may flush.

**Next:** mirror rays against the shadows' TLAS for the smooth rows. Glass is flat, so one
ray per pixel is exact and needs no denoiser. The hit is shaded from the cut's triangle and
the instance's row, and the sky is kept for misses. Metals (F0 from albedo) come with the
material work.

*Measured* (city-blocks, RTX 5070 Ti): about 0.01 ms of shading at 1600×900 (the frame
1.820 → 1.824 ms), 0.03 ms at 1440p (the flight 2.468 → 2.497 ms). With `--no-reflections`
the captures are those of the previous build.

**Mirror rays** (issue #50, 2026-09-25):
- **Which rows:** a Blinn-Phong exponent of 60 and above, the glass. They trace their mirror
  ray in the `_rt` resolve; a miss keeps the sky.
- **The hit data:** the cuts stay on the GPU after the BLAS builds, 34 MiB with each
  triangle's section. A hit reads its instance from the record's custom index and its
  triangle from the cut.
- **The hit's light:** the row's colour times its texture's average, the sun through a
  shadow ray, the sky's SH.
- **The start:** hits count from 1.5 m, past the cut's error.

The rays cost 0.09 ms, and their code 0.03–0.07 ms of registers across the resolve: 0.17 ms
at 1440p in all. A pass of their own over the smooth rows' tiles would recover the
registers. Done in #52: `shading/reflections` traces over the tiles the standard pass lists,
from the direction and weight the resolve stores; the resolve is back to its cost without
rays, and the city's frame drops 0.06 ms at 1440p. Glass reflects 4 % head-on, so the change is modest; coated curtain walls need a
reflectance per row.
**A reflectance per row** (issue #56, 2026-09-25). D-007's render layer gains `reflectance`, F0 at
normal incidence, 0.04 by default. The city's dark glass is a coated curtain wall (0.3,
smooth, no normal map) and its windows are mildly coated (0.08), so the mirror rays now show:
the towers mirror the sky and their neighbours. Stability and cost are unchanged. Metals (F0
from the albedo) come with the material work.
*(research: lighting-gi.md §7; issues #49, #50, #56; demo: city-blocks)*

## D-032 — Participating media start with the belt's dust: a froxel volume, shadowed by rays ✅ (2026-09-25)

lighting-gi.md §6 names the froxel volume as the near-field fog to build (Wronski 2014,
generalised by Hillaire 2015), with the aerial-perspective table as the far field. The
ballad's roadmap and the owner's reference look ask for "volumetric light between the rocks".
This is the first froxel volume (issue #58, `shaders/dust.slang`, `forge_render::dust`):
- **The volume:** 160 × 90 froxels × 64 slices, quadratic in depth to the volume's far end.
  It is laid out as 2-D atlases, as the aerial perspective is, since the bindless set holds
  2-D images.
- **The medium:** extinction from value noise in world space, all of it scattering, with a
  Henyey–Greenstein phase (g = 0.7).
- **The light:** the sun through one shadow ray per froxel against the scene's TLAS (the
  shafts), plus a small fill.
- **Temporal:** the sample jitters within the froxel over TAA's 8-frame cycle, and TAA averages
  it. The noise is D-030's.
- **The integration:** front to back per column, exact for constant light over a slice; each
  pixel reads it at its depth, and the sky reads the whole volume.

It needs ray queries for its shadows, as the ballad's other rays do, and without them the
ballad has none.

**Left for later:** local lights injected into the same grid, temporal reprojection of the volume
instead of TAA alone, the city's haze in the same volume (it has the aerial perspective's
far field), and volumetric GI from the probes.

*Measured* (asteroids, RTX 5070 Ti, 1600×900): 0.105 ms in all; `dust/light` is 0.073 of
it. In motion, consecutive frames change no more than without dust (7.7 % against 8.8 % of the
pixels by more than four levels). With `--no-dust` the captures are those of the previous
build.
*(research: lighting-gi.md §6; issue #58; demo: asteroids)*
