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
the same list. Since issue #3 the dense clusters (under 2 pixels of bounding rectangle per
triangle) can go to a software rasteriser in compute instead, on both paths, when a frame
holds enough of them to repay its fixed cost (D-021); since #30 in both passes, so that a
cluster is drawn the same way whichever pass draws it. Since issue #33 the occlusion keeps
no state per cluster. Pass 1 draws what the previous frame's pyramid shows, from the
previous culling camera, and pass 2 re-derives that to test only the rest against this
frame's pyramid. Since #92 pass 1 lists that rest for pass 2, in its own order, in a list
sized by demand; when it overflows, pass 2 re-derives as before (the city's cluster cull 2
0.28 → 0.03 ms, its frame 2.16 → 1.96 ms). The work lists are sized by demand, so the scene can hold a million
instances (197 MiB for 980 k rocks, against 2.9 GiB before). Since issue #37 an instance
whose roots alone are the cut lists them in a root list instead of taking work items, 32
roots of any instances to a cluster-cull item. Since issue #38 the instance cull can ask
pass 1's question of whole instances: one the previous pyramid hides goes to a second
instance cull against this frame's pyramid, and only pass 2 culls the clusters of those it
lets through. It runs while most of a large crowd is hidden (auto; the city's culls
1.13 → 0.87 ms). Also since #38, scenes of 65 536 instances or more are culled by cells of
64 consecutive instances first. A cell cull tests each cell's bounding sphere against the
frustum and the previous pyramid, and instance cull 1 takes only the cells in view. Cells
hidden whole wait for this frame's pyramid in a second cell cull. The city's table is sorted
along a Morton curve so that its cells are compact. The instance culls go 0.40 → 0.14 ms,
with the same pixels, cells on or off. Next: cluster acceleration structures for ray
tracing on RTX.
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

**Amendment ✅ (proposed 2026-09-25, #93; accepted 2026-09-26): the GPU instance table in
integer cells.** The
renderer does not yet send camera-relative positions: the instance table holds world-space
`f32` (`Instance::model` and `center`), compared with `Frame::camera_pos`. For a million
instances the GPU places itself, camera-relative would mean rewriting the table every frame or
doubles on the GPU. Proposed, after Freese (*Game Programming Gems 4*, 2004): the table stores
`(int3 cell, float3 local)`, with rotation and scale in a `float3x4` so the 96-byte record
holds; the frame block gives the camera the same way. The culls, LOD and draws compute
`float3(cell − camera_cell) · cell_size + (local − camera_local)`, which is exact near the
camera, with a power-of-two cell size in metres. The CPU frames stay `f64`. First, measure: the
city and the belt moved 10⁴ to 10⁷ m from the origin, compared with the origin's image, and GPU
timings before and after. Only that measurement is built until the owner decides.
*The measurement's tooling is built (2026-09-25, a cloud session, not yet run on a GPU):*
`--origin M` in both demos moves the scene and everything anchored to it, `tools/origins.sh`
captures and compares the offsets, and `forge_render::precision` predicts the captures from the
demos' own arithmetic: at 1440p an object 2 m from the camera is drawn 0.5 px off at 10 km, 9 px
at 100 km, 45 px at 1 000 km and 800 px at 10 000 km (its depth 18 % off), the same geometry in
cells of 1 km 0.013 px throughout (`docs/demos/city-blocks.md`, "Far from the origin"). Left:
the captures on the 5070 Ti (step 1), the prototype behind a flag and its timings (steps 2–3),
then the decision. The cell size is part of it: 1 km keeps the local part's spacing at 0.12 mm;
64 km would give 7.8 mm, today's precision at 100 km.
*The record is built (2026-09-26, the same cloud session, not yet run on a GPU;
`docs/HANDOVER.md`):* `Instance` is an `int3 cell`, a `float3 local`, a unit quaternion, a
uniform scale, the centre from the same cell and the radius (80 bytes, from 96;
`forge_render::cells`, `meshlet.slang`); the frame block carries the camera's cell and offset,
and every matrix is camera-relative; the TLAS, the probes and the dust work in a **scene frame**
anchored at the scene's origin (`MeshletScene::origin`), rays starting from
`relative + camera_in_scene`; cells of 1 km. The proposal's `float3x4` gave way to the
quaternion: it is what keeps the record at 80 bytes. `tools/origins.sh` is the acceptance test:
every offset against the origin's image, expected 0 px up to a few pixels from the split's
0.1 mm rounding. The record before stays as the commit before, for `tools/timings.sh`'s A/B.
The decision stays the owner's: keep the commit, change it (the cell size, the frame), or drop
it.
*Accepted (2026-09-26, on the 5070 Ti; the owner: keep it if it improves things with no
issue).* `tools/origins.sh` (`reports/2026-09-27-93/`): offsets of whole cells (1 024 and
10 240 m) give **0 px** in both demos, so nothing depends on the world position any more; at
10⁴–10⁷ m the image differs from the origin's by the same small amount at every offset (the city
2 550–2 750 px, ꟻLIP mean 0.0015–0.0017; the ballad 1 666 px, 0.0005), the rounding of the
offset inside a cell spread by the traced shadows and TAA, where the record before broke down
(1.1–1.3 M px at 10⁶ m, blocks of the frame black). The batch against the record before passes
D-017 as accepted (every mean at most 0.0047, the peaks isolated pixels); the A/B harness, mesh
against fallback and the validation are clean; with the instance's frame built once per cluster
in the cull, every timed view is as fast as before or faster. Two findings on the way: the
highlight took the camera's world position (fixed: the scene frame's), and the cull's
per-point rebuild of the quaternion's matrix (fixed: `InstancePose`). Exact 0 px at every
whole-metre offset would need the offsets on a fixed grid (2⁻¹³ m); not needed now.
*(research: large-worlds.md §1, Freese 2004)*

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

**The deformable layer's first step ✅ (#185, 2026-10-08, `--lab yard`).** A soft material's
layer is `forge_physics::deform::Layer`: its thickness over a hard base on a grid. Its rules
(`Soft`) are a first `deform` block: depth, the least a press leaves, stiffness, the share
packed, and the steepest slope it stands at. Footfalls press their pads into it: as deep as
their pressure sinks them, the rest heaped in a rim, then slumped. It is simulation state:
saved, digested and replayed with the world. It is drawn by displacement through the skin pass:
a ground mesh of one joint that a height field raises, its normals the field's slopes, its
structure refitted for the rays. Mud and wheels ✅ (#186): a rolling wheel presses its patch
swept back along its travel over the step, its rim beside it only, from the vehicle's wheel
contacts and loads. The physics reads it ✅ (#187): a layer a vehicle drives on is a Jolt height
field of every other point, edited in place where it changed, in an object layer only wheels
feel (no other body tests its small triangles); a press sinks to the depth less pressure over
stiffness, so a standing load finds its level; a wheel's press follows its round ahead of its
travel, so it rests in its rut; a rolling resistance from its sinkage and the material's grip
slow it. Detail that is seen but not stood on, a tyre's tread, goes in a relief drawn over the
layer and no part of its thickness (#188), and a vehicle on soft ground finds it with rays down
from its wheels, which its ruts' walls cannot catch. A wheel that slips digs ✅ (#191): by the
distance its tread slid past the ground, along its round (a flat hole's slumped walls held a
wheel on a ray for good), what it tears thrown the way its tread slides. Feet stand on it ✅
(#194) through the creature's IK and its footfalls, not the physics: a creature is held over the
ground under its paws by its balance, and height fields at the point every centimetre a paw
needs cost 0.6 ms a tick in contacts. Round the player ✅ (#197): on the island a window 12 m square
at 2 cm follows the walker in steps of the ground's 2 m cells (`Layer::move_to` keeps its prints
where they lie), drawn as the ground itself: the tiles leave it their fragments there (a ground
window, through the cut-outs' raster), it is shaded by their layered row where it stands, and the
slopes it adds give it their smooth normals untouched; a step with no motion of its own. Not yet:
feet on soft ground in the physics (a ragdoll let go falls through), audio reading it, and
the weather refilling it.

**Physics reads the row ✅ (#203, 2026-10-09, `--lab materials`).** The lab's material table
(`lab/materials.rs`) gives a row's physical layer and tags to the bodies made of it (friction,
the dynamic coefficient, and restitution) and to its drawn row, one record for both. Pairs
combine as Jolt combines them: friction by the geometric mean, restitution by the larger. A
walker's grip on a ground with a row is its sole's pair friction times g, so it slides on ice.
Not yet: a hit's material from Jolt itself (per-triangle materials, a body's user data read
back), the static coefficient, combine rules per row.

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
*Built* (#136, 2026-10-02, the owner's go for the downloads): Jolt 5.6.0's library vendored in
`third_party/jolt`, compiled by `forge-physics`' `build.rs` through `cc` with
`CROSS_PLATFORM_DETERMINISTIC`, double precision, precise floating point and AVX2 without FMA.
The "fork of JoltC" became a C layer of Forge's own written after it: narrow and batched, bound
by hand (JoltC's bindings need bindgen and libclang, and its surface is many times what Forge
calls). Three of §6's tests pass, and the cross-platform one: one hash on Windows (MSVC) and
Linux (gcc), checked by CI on both. Jolt's job system runs its own threads for now (the client's
cores less two); bridging it to `forge-task` waits for `forge-sim`.
*Boats* (#138): "boats by submerged-triangle hydrostatics" built in
`forge_physics::buoyancy` (pressure per submerged piece, pressure and skin drag, radiation
damping near the surface), on the CPU twin of the GPU's cascades: `Ocean::displacement` per tick
for the long ones, sampled as the GPU's filter reads them. No transcendental function, so it
replays to the same digests; `physics-lab --lab sea`.
*Characters* (#139): `CharacterVirtual` with stair stepping and sticking to the floor, saved
and restored after the bodies, its ids numbered per world (Jolt's run across the process, and a
server and a client in one process must agree); `physics-lab --lab walk`.
*Vehicles, joints, ragdolls* (#140–#143): the vehicle constraint (a car), breakable fixed and
distance joints (a brick wall), motorised ragdolls (D-012's physics layer) through the same C
layer; lift and drag in `forge_physics::aero` (an aeroplane).
*The mid water* (#144): "a server-authoritative column model" built in
`forge_physics::shallow`, depths on a grid and velocities on its faces (after Müller-Fischer
2008), deterministic and saved with the world, the buoyancy reading its surface and its flow;
`physics-lab --lab flood`. The water pass draws it as fresh water (a pool) from the columns
uploaded each frame; the GPU's own heightfield shadowing it near the player is still to come.

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
*First stage built* (#137, 2026-10-02, `forge-sim`): commands stamped with their tick and sent
again until acknowledged, a server that takes a late one at its next tick, clients that run
ahead by the delay and two ticks, and reconciliation by digest: a client compares each
snapshot's digest with the one it predicted for that tick and goes back to the server's state
only when they differ, which a deterministic simulation makes the rare case (96 snapshots in
99 predicted to the bit over 100 ms and 2 % loss, the misses the other player's throws). The
link is in-process; the snapshots are whole states (280 KB in the lab), so the baselines,
quantisation and interest management above remain Phase 5's.

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

**Amendment ✅ (proposed and accepted 2026-10-04, option 1): the clip runtime in-house, from
glTF.**
The owner approved downloading `ozz-animation-rs` for step 7's skinned creatures (2026-10-04),
and two things came up when checking it before adding it:
- **It needs nightly Rust.** Its README says so, and its manifest turns on glam's `core-simd`
  feature, which uses `std::simd`. The workspace and CI are on stable (`rust-toolchain.toml`).
- **It reads only `.ozz` archives,** which ozz's C++ tools (`gltf2ozz`) make from glTF. Its
  README says it implements only ozz's runtime and has no plans for the offline part. The C++
  releases (0.15–0.17) ship no prebuilt tools, so they would have to be built from source with
  CMake: a second download and a C++ build in the asset pipeline.

The choices:
1. **Recommended: an in-house clip runtime in ozz's shape** (the research's "or an in-house
   copy", animation.md's recommendation, layer 2). A `forge-anim` crate reads skins and clips
   from glTF with the `gltf` crate already in the workspace. It samples them into SoA local
   transforms with `forge_core::dmath`, so the determinism D-012 chose ozz for holds. Then it
   blends, takes the model-space pass and the skinning matrices, and skins in a compute pass.
   It is a few hundred lines to start, with no C++ step and no new dependency. Compression
   stays deferred until measured, its format ACL-compatible, as the research says.
2. **ozz-animation-rs on nightly:** the whole workspace and CI on a nightly toolchain, plus
   ozz's C++ tools built from source. Not recommended: nightly for one dependency.
3. **A fork of ozz-animation-rs on stable** (glam's SSE2 path instead of `core-simd`), with the
   C++ tools built from source. A fork to maintain, and still the C++ step.

**The owner's answer (2026-10-04):** option 1, after checking that the README (a year old)
is not stale. It is not: on this machine's stable rustc 1.99 (2026-09-28) `use std::simd`
still fails with E0658 (`portable_simd`, rust-lang/rust#86656), and the crate's `src/lib.rs`
at its latest commit (2026-09-05) still starts with `#![feature(portable_simd)]`. Should
`std::simd` become stable, ozz-animation-rs stays a candidate to compare against, but the C++
step for its archives would remain.
*Built* (#165, 2026-10-04):
- **`forge-anim`:** skeletons and clips (step, linear and cubic-spline keys) from glTF's skins
  and animations, poses sampled and blended, the model-space pass and the skinning matrices.
  It uses `+ − × ÷` and `sqrt` only, so the same inputs give the same bits.
- **`forge-geom`:** skinned meshes are cooked as one level of roots, every cluster bounded by
  the sphere of every pose.
- **`forge-render`'s skin pass:** linear blend skinning of four joints a vertex into the pool,
  each mesh's BLAS refitted in place (`forge_gpu::DynamicBlas`), and the previous positions
  for the motion vectors.
- **The lab:** `physics-lab --lab creatures` draws the mannequin and the dog as skinned bodies
  (Blender, bone-heat weights), their ragdolls' bodies moving the bones.

The clips (a walk and an idle each) are read and sampled but not yet played through the
motors: that is the procedural layer's work.
*Built* (#167, 2026-10-08): the clips play through the motors (inertialized switches, a blend
space), two-bone IK and a look-at, packed clips. On uneven ground (`physics-lab --lab course`)
a walking ragdoll is **guided as games carry one by its clip's root motion**: its torso held
upright and at its height over the ground under its paws, over its way's line and at its walk's
pace by damped springs, the IK putting each paw on the ground under it, a swinging one onto the
rise ahead. On its legs alone the dog walked the floor but stalled at a 5 cm step and a 10°
ramp. A ball still shoves it, and the springs let go when it goes limp.
**Foot-down events** (`forge_anim::Footfall`) carry the tick, the position, the ground's normal,
the foot's heading and half sizes, its pressure (its share of the weight over its sole) and the
ground's material, as this entry lists them. The lab finds the ground with a ray among the fixed
bodies, from inside the foot, and draws a print at each. Nothing deforms yet: that is D-007's
deformable layer (Phase 3's materials step), and the footsteps audio's (D-011).
**A flyer** (#184, `--lab flyer`): a gull is one body whose wings are flying surfaces posed by
its clip, the flow over each including its own beat, so the beat gives the thrust. It is guided
the same way: a balance torque holds it facing its flight, its wings at an angle of attack, and
banked to turn onto its circuit.

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

**Amended by D-056 (2026-10-09):** the far field is cluster-DAG tiles on the existing path, one
per cell of D-037's clipmap, not CDLOD.

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

**Amendment ✅ (proposed 2026-09-25, #75; accepted 2026-09-26 with a margin): a perceptual
check beside the pixel count.**
Whenever pixels differ, `imgdiff` also prints LDR-ꟻLIP (Andersson et al. 2020). It is a port
of NVIDIA's reference that matches it to six decimals and to the pixel of its error map.
Which check applies:
1. **The image must not change** (the culling A/B harness, mesh against fallback, a
   refactor, a speed-up): 0 px, as now.
2. **Pixels may move where nobody would see it** (an instance order, TAA history rounding,
   #71's flake, a baked table): proposed pass at the default 67 pixels per degree, every
   pixel below 0.15 and the mean below 0.02 (`imgdiff --max-flip 0.15 --max-flip-mean
   0.02`). Measured: the city's new instance order (#38) peaks at 0.053, #71's flake at
   0.06–0.13 over seventeen flakes (means up to 0.0015; the 0.129 of 2026-09-25 is the
   closest to the threshold yet). ACES 2.0's table against its per-pixel transform (#76)
   is 1–2 levels everywhere: means 0.003–0.012, peaks up to 0.046. A faint one-pixel line
   reaches 0.17 and a 3×3 speck of 20 levels 0.26. (Revised in #76: the first proposal's
   mean of 0.003 failed the ACES 2.0 table, which nobody can tell apart.)
3. **The look is meant to change** (a tone curve, a sampler, AO): no threshold. The report
   gives the mean, p99, largest value and error map, and the owner judges. Measured: GTAO on
   against off, means 0.011–0.026 and peaks 0.50–0.82; AgX against ACES, mean 0.37.
4. **Another GPU** (#39, #67): thresholds after the first captures there.

ꟻLIP's mean is its usual pooled number and is kept for that. It does not separate the
classes: GTAO's means overlap the ACES 2.0 table's. The proposed check leans on the largest
value, with the mean as a guard against a shift over the whole frame. The measurements are
in `docs/PROCESS.md`, "The perceptual check".
*Accepted (2026-09-26), with a margin of error:* the first change of arithmetic to face the
check, #93's instance record in cells, kept every mean far under 0.02 (at most 0.0047) while
22 of the batch's 26 pairs peaked at 0.26–0.43. Each peak was an isolated pixel: a silhouette
pixel or a shadow edge on a rock moved by less than a pixel, and the two batches look the same.
The owner took the thresholds with that margin: the mean below 0.02 is the gate for class 2; a
largest value above 0.15 sends the reviewer to the error map and the crops
(`imgdiff --crop --crops`), and isolated pixels pass, while a cluster of them (a speck, a line,
a patch) fails.

**Second amendment ✅ (proposed 2026-10-02, #126; accepted the same day as proposed):
HDR-ꟻLIP for the HDR captures.** For two
PQ captures (#94), `imgdiff` prints HDR-ꟻLIP (Andersson et al. 2021) instead of LDR-ꟻLIP:
LDR-ꟻLIP of both images tone-mapped at one exposure per stop over the reference's range, the
largest error of each pixel kept. It matches NVIDIA's tool to six decimals. The classes stay
the same; class 2's thresholds change:
- **The largest value stays below 0.15,** isolated pixels passing as above. A speck is about
  as large as in LDR-ꟻLIP: a line of 20 codes reaches 0.24 and a 3 × 3 block 0.31, so both
  fail, while one pixel reaches 0.08. Far from the origin (#93), single pixels in the dark
  space reach 0.52–0.58: isolated, they pass.
- **The mean rises from 0.02 to 0.05.** HDR-ꟻLIP sees the dark space up to 7 stops above the
  display, so a change over the whole frame scores about five times its LDR-ꟻLIP. ACES 2.0's
  HDR table against its per-pixel transform, which nobody can tell apart, scores 0.021–0.025
  (0.0044 in SDR). One code over the whole frame scores 0.037 and passes; two codes, 0.062,
  fail.

Class 3 is unchanged. GTAO off against on peaks at 0.59–0.91 in HDR, against 0.135 for the
SDR frame, whose curve crushes the dark fill GTAO changes. The measurements are in
`docs/PROCESS.md`, "The perceptual check", "HDR captures".

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
declaration order, one profiler zone per pass label. A read after a write waits for the
write; a read in a stage or of an access the earlier readers do not cover waits for them,
and so for the write (since #71; before, only the first reader waited). Per-frame images
are transients of the graph, laid out in one heap from their lifetimes (largest first,
first fit among the images whose lifetimes intersect), so images that never coexist share
memory; their first use in a frame waits for whatever last touched that memory, in this
frame or the previous one.
Persistent images and buffers (`GraphImage`, `GraphBuffer`) carry their state across
frames; anything a frame in flight may still use is destroyed through the frame slots
(`Frames::destroy_later`). Nothing is culled, and nobody outside `forge-gpu` records a
barrier. Transient buffers are the next extension (#78). The graph lives in `forge-gpu`
(not `forge-render` as first planned) because the app shell and every renderer draw through
it and the barrier
vocabulary is Vulkan's; the module is written without `unsafe`.
*Measured:* the ballad and the bench through the graph are pixel-identical to the
hand-written barriers (0 pixels over seven captures, and 0 between the aliased heap and
`FORGE_GRAPH_NO_ALIAS`), validation and synchronization validation clean; a ballad frame
is 19 passes, 37 image barriers and 3 memory barriers, its three transients 25.6 MB. Nothing
aliases in that frame yet (colour, depth and motion vectors are all alive at the resolve);
the heap pays once post-processing chains arrive. *(research: task-system.md §F,
memory-streaming.md §2; issue #1)*

**Queues (issue #77, 2026-09-25).**
- **Picking a queue.** A pass may ask for the async compute queue or the transfer queue
  (`PassBuilder::queue`). The device takes a queue on a compute-only family and one on a
  transfer-only family, never the video or optical-flow engines. `FORGE_ASYNC=0`, or a device
  without them, keeps every pass on graphics.
- **Scheduling.** A pass on another queue moves up to just after the last pass it conflicts
  with (a shared resource that one of them writes). The author picks the queue; the graph
  derives the rest, as in Unreal's RDG, Frostbite and Granite.
- **Batches.** The frame is a list of batches, one submission each: consecutive passes on one
  queue that need the same waits. Each queue signals a timeline semaphore.
- **Waits.** Every resource remembers, from frame to frame, the batch that last wrote it and
  the batches that read it since, per queue. An access from another queue becomes a timeline
  wait at its stages, plus a barrier on its own queue from everything that queue did before.
  The last batch is graphics and waits for every other queue's last one, so a frame slot is
  free when its graphics work is.
- **Waits made once (#104, 2026-10-02).** With `vkQueueSubmit2`, a wait covers every later
  submission of its queue at its stages. So the graph remembers, per queue and per stage, the
  latest value each queue waited for on the others, from frame to frame, and leaves out a
  wait an earlier batch already made. Before, every frame after a streaming copy waited for
  that copy again, and the wait split a graphics batch.
- **Sharing.** Every buffer, and every image except a render target, is `CONCURRENT` over the
  families, with no ownership transfers. NVIDIA ignores the mode; on AMD a `CONCURRENT` image
  loses DCC, so render targets stay `EXCLUSIVE`. The graph refuses them on another queue, and
  transients too, whose memory could be aliased under a pass on another queue.
- **Timing.** Timestamps are per batch. A frame's GPU time is its span, first stamp to last on
  any queue, since zones overlap.

*Measured:* the city's sky tables and probe update run on the compute queue, and its
streaming copies on the transfer queue. Its frame goes 2.36 → 2.25 ms at 1600 × 900, the
orbit 2.86 → 2.79, the flight 2.23 → 2.15. The probes stretch 0.60 → 1.35 ms beside the
geometry passes, which slow by a third to a half, but the span shrinks. The demos without an
async pass do not move, nor does anything with `FORGE_ASYNC=0`. Captures match the serial
frame, and synchronization validation is clean.
*(research: gpu-geometry.md, "Research for issue #77")*

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
compensation and a clamp, and snaps over the first ten metered frames (one until #171, while
the light probes converge). The adaptation runs on
the CPU (unit-tested, deterministic under `--fixed-step`); a GPU-resident loop is the
option if the two-frame latency ever matters. Temporal filters rescale their history by
the ratio of exposures. **The display transform is data** chosen at run time: AgX (engine
default, hue-safe), ACES as Hill's fit of the 1.x RRT + sRGB ODT, Khronos PBR Neutral, and
**ACES 2.0's output transform** (issue #76, 2026-09-25: the SDR 100-nit Rec.709 preset,
ported from OpenColorIO 2.5.2 and matching its test values to 1e-5). ACES 2.0 runs through a
65³ table baked on the CPU at start-up (10 ms), sampled trilinearly on a log2 shaper. On
real frames the table stays within 1–2 levels of the per-pixel transform, ꟻLIP largest
0.046; it adds 0.007 ms to the ballad's resolve at 1440p. The per-pixel transform stays as
the reference (`--tonemap aces2-analytic`, +0.10 ms), and `meshlets --tone-check` compares
both GPU paths with the CPU port. One Slang module serves the TAA resolve
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

**HDR output** (issue #94, 2026-10-02; research: `docs/research/hdr-output.md`). The display
transform's output is encoded for its target, chosen from the target's format:
- **HDR10**: `A2B10G10R10_UNORM_PACK32` in `HDR10_ST2084_EXT`, the shader writing Rec.2100
  PQ over Rec.2020 itself, with half a 10-bit code of spatio-temporal dither against banding.
- **scRGB**: `R16G16B16A16_SFLOAT` in `EXTENDED_SRGB_LINEAR_EXT`, linear Rec.709, 1.0 being
  80 nits.
- **SDR** as before (the capture batch is unchanged to the pixel).

On the display, an HDR mode is entered only when asked (`--hdr`, `FORGE_HDR`, F2) and when the
OS shows the display in HDR (Windows' "Use HDR", read through the display configuration API),
so scripted runs never switch the owner's monitor. `VK_EXT_hdr_metadata` gets Rec.2020
primaries, the preset's peak, the panel's black, and the content's MaxCLL and MaxFALL (#125,
below). **Off-screen** draws the same HDR10 image
into a transient and previews it on the SDR swapchain (`post/hdr preview`: what a display at
paper white shows below it, or false colours): everything but the present runs, and is
captured, on any monitor.

**ACES 2.0's HDR presets.** The Academy's peaks (500, 1000, 2000, 4000 nits, P3-D65 limited;
Rec.2020 limiting available) run the SDR chain with the output clamped at the peak and
converted to Rec.2020. The preset is the largest not above the panel's peak (the calibration's,
else DXGI's), 1000 nits when unknown, and F3 steps through them. Each runs through its own 65³ table of the PQ
signal (`R16G16B16A16_UNORM`, its grid's top at the preset's clamp, 4096 at 1000 nits),
baked once per process (10–15 ms). Against the per-pixel transform it is within 1.3 10-bit
codes at the 99th percentile, where the SDR table is at 1.5 8-bit codes. `meshlets
--tone-check` holds both GPU paths within 0.10 code of the CPU's. At 1000 nits a scene's 0.18
lands at 14.5 nits and its 1.0 at 107 nits.

**Paper white** (the owner's pick, 2026-10-02: the Academy's look): an exposure offset in
stops in front of ACES 2.0, 0 by default, so the transform's own grey and white stand. The
UI, and every other tone curve (their SDR image), are drawn at the UI's white: Windows' SDR
white level for that display, BT.2408's 203 nits when unknown. The overlay is written at
that white and never through a curve.

**Calibration** (issue #125, 2026-10-02). F5 opens three pages in HGiG's and Unity's form,
drawn over the frame in the target's encoding without dither (`app/hdr calibration`):
- **Peak:** a mark inside a square of a tenth of the screen at the signal's top (10 000 nits),
  raised until it disappears: the brightest the display shows (HGiG's MaxTML). Unity fills the
  screen; a tenth keeps a panel's full-screen limit from lowering the answer.
- **Black:** a mark on black, at the darkest value it still shows (MinTML).
- **Paper white:** half the screen at that peak, half black, the mark at the UI's white.

Up and Down move the value by 4 PQ codes (1 with Shift), and it applies at once. The calibrated
values go over the OS's: the peak picks the preset (the largest not above it), the black goes
to the metadata, the UI's white to the overlay and the other curves. ACES 2.0 has no black
parameter, so the black changes nothing on screen. Leaving the last page saves the values per
monitor in `settings/display.txt` (ignored by git). Scripted runs neither read nor write it,
so captures never depend on it. `--hdr-ui-white` sets the UI's white over both, and
`--hdr-stops` the paper-white offset.

**MaxCLL and MaxFALL** come from the frames shown (`post/hdr metadata histogram`): each
pixel's largest channel as a PQ signal over Rec.2020, counted into 256 bins with the largest
signal kept, read back two frames later. The display is told the largest values since the mode
or the preset last changed, and told again when one grows by more than 1 %. MaxCLL is exact to
the code; MaxFALL is within half a bin (2 % of the luminance). On the display the swapchain's
images are sampled for it when the surface allows. When it doesn't, MaxCLL stays the preset's
peak and MaxFALL 0, as before.

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

**The planet-view table** (issue #26, 2026-09-25) makes the view from outside a lookup.
- **What it holds:** from a camera outside the shell, a ray is fixed by its closest approach
  to the planet's centre and its azimuth around the planet's direction, from the sun's side
  (mirror-symmetric). A 512 × 256 table over those two holds the march's luminance. A one-row
  table holds the transmittance, which does not depend on the sun.
- **Its axes:** the closest-approach axis splits at the ground's radius and is never filtered
  across (Bruneton's split at the horizon). Rays to the ground go by 1 − √(1 − b / R): linear
  at the disc's centre, where the ground under the ray moves across the terminator linearly,
  and packed towards its edge. Rays above the ground go by the square root of their lowest
  point's height.
- **Its texels** march 64 segments since #73. Built once, the table affords four times the
  per-pixel march's 16, which are 4/255 off at the limb.
- **When it is built:** when the atmosphere, the camera's position relative to the planet or
  the sun changes (once in the ballad).
- **The reference:** `--planet-march` keeps the per-pixel march.

No published table for views from outside turned up. Hillaire's reference code, like Bevy,
marches every pixel from space; Bruneton's 4-D table moves the camera to the top of the
atmosphere (lighting-gi.md §6).

*Measured:* within 1/255 of the march everywhere at 18° and 50°, lit from the side, from
behind, and with the sun rising over the limb. The sky pass at 50° goes 0.333 → 0.105 ms:
0.215 with the table, and 0.105 once the stars are no longer worked out behind the ground.
That reorder alone takes the march to 0.225. With 64 segments (#73) the table is within
1/255 of a 512-segment march in all those views, where the 16-segment march is up to 4/255
off at the limb (424 pixels beyond 2/255 at 50°). It takes 0.081 ms to build, once.

**The ground view** (issue #43, 2026-09-25) is `forge_render::sky`, three passes a frame over
the same tables:
- **A sky-view table** (192 × 108): the in-scattered light around the camera, the elevation
  squashed towards the horizon, the azimuth taken from the sun's. Rays that meet the planet
  add its ground, lit by the sun and the sky and seen through the air.
- **An aerial-perspective volume** (32 × 32 froxels × 32 slices to 8 km, quadratic in
  depth, a 1024 × 32 atlas): the light gathered and the mean transmittance from the camera
  to each slice.
- **A compose pass:** the sky and the sun's disc where the depth is empty; elsewhere
  `colour × T + L`, from the pixel's distance. Since #171, `L` and `T` count only the air
  beyond 100 m, Unreal's default aerial perspective start depth: the volume's unshadowed air
  hazed Sponza's arcade into a fog at sunrise.

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
0.20, most of it from the ground; the pass costs 0.016 ms. Both followed: GTAO (D-030) and
the probes (D-036), which replace this light where they reach and fall back on it beyond.

## D-024 — DLSS through Streamline's interposer, optional, TAA as the default ✅ (2026-09-24)

DLSS Super Resolution runs through NVIDIA Streamline 2.14 (the SDK in `streamline-sdk/`,
git-ignored), with the binding lifted from the `world` project into `forge-gpu`
(`streamline.rs`, every FFI structure's size and padding checked at compile time). It
is **optional**: the `dlss` feature (Windows) loads `sl.interposer.dll` in place of the
Vulkan loader (`Instance::with_streamline`, `AppConfig::streamline`), and only when the GPU
the device selection prefers is NVIDIA's (issue #67: a plain instance asks first,
`Device::preferred_vendor`); any failure through Streamline falls back to the plain loader.
The device offers `Device::dlss()` when its GPU runs it. Without the feature `Dlss` is an
uninhabited type, so renderers and demos compile without feature gates. **TAA stays the
default and the fallback**; U switches TAA → DLAA → Quality → Balanced → Performance →
Ultra Performance at run time (`--upscaler` at start), and the profiler zones are
`temporal/TAA resolve` or `temporal/DLSS` + `post/display transform`.

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

**The start view** (#121, 2026-10-01). A scene given the camera it starts from
(`MeshletSceneBuilder::set_start_view`) loads that view's pages before its first frame,
after the roots' (`MeshletScene::load_start_view`):
- **Worked out on the CPU** with the culls' metric in `f64`: the pages of every cluster whose
  parent would show more than 0.9 of the threshold (a margin over the culls' `f32`), and the
  pages above them. The instances are read back after the GPU placement. A mesh's 1 024
  nearest instances are worked out cluster by cluster, skipping the LOD levels that cannot
  show at their distance; the rest take the bound of the nearest of them.
- **Loaded unpinned,** neediest first and parents before children, as many as the pool holds.
  They leave like any page once the view moves on.
- **Why:** a fixed view then reads nothing more, so its frames depend on no read's timing.
  The capture batch can draw the island's 2 m ground, whose 4.2 GB of pages exceed a
  resident pool (#106), and a start no longer refines over its first second.
- **Checked:** the island at 8 m streamed this way draws its resident frame to the pixel (a
  pair in `tools/compare.sh`). Neither the island nor the city reads a page past the start
  view in 300 frames.
- **Cost:** the city loads 473 pages (59 MiB; its settled view keeps 395) after 63 ms of work
  over its million instances, then 44 ms of reads and copies. The island at 2 m loads 290
  pages (36 MiB) in 11 + 21 ms. The frames do not change, except that a start that is sharp
  switches the auto software raster on in the city's first view, as with every page resident:
  1.82 → 1.85 ms there (`docs/PROFILE.md`).

Left for later: compressed vertices (quantised to a per-mesh grid; built in #218, D-055) and D-018's container
(BLAKE3 chunks, zstd), IOCP reads, and a transfer-queue upload. *(research:
memory-streaming.md §4, gpu-geometry.md; demo: city-blocks)*

## D-026 — Materials on the GPU: a row per instance, shading by class ✅ (2026-09-25)

D-007's record is `forge_core::material::Material`:
- a render layer: the shading class, two base colours, a cavity term, roughness, the
  highlight's weight, emission, the ice's scattering, and two textures with their scale
  and whether they are hex-tiled;
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
- stochastic textures (rock, sand, concrete) can also be hex-tiled (Mikkelsen 2022, issue
  #66): random hexagonal tiles, each at its own offset and rotation, and an offset per
  instance. This hides the repeat on large faces for 0.04 ms in the ballad. Structured
  textures (brick, tiles) keep plain repeats.

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
  it; the glass is dark anyway. A terrain's rays start twice its cut's error off where that is
  further (2026-09-26, #96: the island's 8.4 M triangles of relief cut to 600 000 have an error
  of 1.03 m, and from 0.15 m its slopes shadowed themselves; at once the error some still did).
  Props keep 0.15 m, since their creases hold contact shadows.

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
  placed by a Hilbert-curve index into the R2 sequence (since 2026-10-03, by interleaved
  gradient noise: see the second departure below), and their distance picks the level
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

**A second departure (2026-10-03, the owner's report from the physics lab's walk):**
- **What was seen:** the samples' noise is Jorge Jimenez's interleaved gradient noise (SIGGRAPH 2014),
  not the Hilbert curve's R2. With the R2, the 3×3 denoise left tiles of horizontal stripes. These
  showed on faces in shadow, which only the sky lights, wherever TAA had no history to average
  them (a moving camera).
- **The cause:** holding the noise constant removed the stripes, so they were the noise's own
  pattern, not the geometry's.
- **The fix:** the gradient noise spreads each 3×3 block of pixels over the whole range, so the
  denoise averages it out. Even without TAA, the occlusion is then a smooth gradient.
- **The cost:** a still view's change over 32 frames is unchanged (the city's start, no clouds:
  0.280 % of the pixels by more than two levels with the R2, 0.278 % with the gradient noise,
  0.277 % without AO).

**Left for later:**
- specular occlusion and bent normals from the same horizons;
- a normal from the visibility buffer instead of the depth (exact on thin geometry);
- occlusion at the scale of a street, beyond a few metres of screen-space radius. That is
  the probes' job, or rays against the TLAS the shadows already use. Done by the probes
  (D-036, issue #53); GTAO stays on top of them for the contacts.

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

## D-033 — Translucent ice by ray-traced thickness ✅ (2026-09-25)

The owner's look for the ballad (ROADMAP, "Next for the ballad", item 4) asks for ice
"translucent according to its density and to the thickness of ice between the light and the
camera". Games usually approximate the thickness with a baked local-thickness map (Barré-Brisebois
and Bouchard, "Approximating Translucency for a Fast, Cheap and Convincing Subsurface
Scattering Look", GDC 2011). With the TLAS and the cuts on the GPU (D-029, D-031), the
thickness can be measured instead (issue #59, `ice_transmission` in `meshlet.slang`):
- **The ray:** from the pixel's point towards the sun, as a non-opaque query that commits
  nothing, so it sees every crossing of the pixel's own instance within its bounding sphere.
  The last crossing is where the sunlight entered. The rule needs no winding and holds on
  either side of the cut's surface.
- **The sun:** a shadow ray from the entry point decides whether the sun reaches it.
- **The light:** transmittance exp(−σ·t) per channel, σ = (0.16, 0.10, 0.065) m⁻¹, since
  ice absorbs red. It is tinted by the ice's albedo and scattered forwards with a
  Henyey–Greenstein phase (g = 0.5) towards the camera, with 30 % of the crossing light
  leaving that way.

It runs in the ice class pass only and is deterministic per pixel, with no noise. **Left for
later:** density as a material parameter (the row's), the fractured chunks (#12), light
scattered inside the body from the sky and the planet, and refraction of what lies behind.

*Measured* (asteroids, RTX 5070 Ti, 1600×900): `shading/ice` 0.018 → 0.065 ms, the ballad
0.646 → 0.694 ms. With `--no-translucency` the captures are those of the previous build.
**Density per row** (issue #61, 2026-09-25). D-007's render layer gains `bubbles`, the share
of the ice's volume in air bubbles of about a millimetre, which scatter 1.5 · bubbles / r per
metre (twice their cross-section, the extinction paradox). The straight beam keeps the
bubbles' forward peak (delta-Eddington, g = 0.8), and the rest diffuses: it leaves the far side
like a Lambertian surface, dimmed by √(3σa(σa + σs')) and by 1 / (1 + ¾σs'L). The albedo
whitens with the bubbles near the surface. Bubbly ice is lighter, 917 (1 − bubbles) kg/m³, so
the physics density follows the same number. The ballad draws clear, bubbly and white blocks at no
measurable cost.
*(research: lighting-gi.md; D-029, D-031; issues #59, #61; demo: asteroids)*

## D-034 — The environment state: a baked climate, weather as a function of seed and time, surface fields near players ✅ (2026-09-25)

Extends D-019 from one weather struct to a planetary climate field. The struct stays the thing
every system reads (rendering, materials (D-007), vegetation, audio, physics and gameplay), but
becomes a *sample*, `weather.sample(position, time) -> WeatherSample`, of three layers.

**Climate atlas** (`forge-procgen::climate`, baked once per planet):
- **Fields:** twelve monthly normals per cell, 16 bytes: mean temperature and diurnal range,
  precipitation and wet-day probability, prevailing wind, humidity, cloud fraction.
- **Grid:** the cube-sphere at level 7 (≈ 78 km on an Earth-sized planet, ≈ 19 MB, resident),
  downscaled per level-6 terrain tile to ≈ 1.2 km with the lapse rate against the real heights
  and Smith–Barstad orographic precipitation (≈ 3 MB per tile, cached).
- **Build:** insolation by obliquity, a diffusive energy balance, circulation-cell winds and a
  moisture sweep along them.
- **Validation:** offline, against ExoPlaSim + `koppenpasta` on the same heightmap. These are GPL
  tools, never linked.

**Biomes:**
- A data table of climate envelopes (BIOME1's indices; Whittaker first).
- A soft nearest-point selection in climate space, blended by a scattered kernel whose radius is
  the pair's transition width, then sharpened by local switches: drainage, slope, soil, fire
  history.
- Written as a biome-weight map (RGBA8 at 4 m) beside D-028's layer map. A species' density is its
  viability × competition, so no vegetation is blended.

**Weather** (`forge-world::weather`), a pure function of the atlas, the planet's seed and world
time:
- Synoptic noise travelling with each latitude band's wind, thresholded to the atlas's wet-day
  probability and totals.
- Convective cells with a diurnal peak; lightning as a seeded Poisson process.
- Accepted only if thirty synthetic years per cell reproduce the atlas's statistics (Richardson
  1981).
- Authored weather is an event (centre, radius, start, duration, profile) blended on top.

**Surface state:** wetness, puddle depth, snow depth and frost in a clipmap around each camera.
- 3 × 512² at 0.5/2/8 m, ≈ 6 MB, updated a few times a second: a soil bucket (Manabe) and
  degree-day snow (Hock), with exposure from one depth-from-above map shared with rain occlusion
  and splashes.
- Texels scrolling in, and the server's gameplay queries, integrate the last 72 game-hours of the
  function at the point.
- D-007's wet/frozen/snow overrides read these values.

**Rendering** reads a camera-centred weather map (256² at 250 m, 64 km): Nubis-style clouds,
precipitation particles shaded from a streak array, rain as extinction in the froxel volume
(D-032), wet and snowy surfaces, lightning, weather fog.

**Replication:** nothing continuous. Only the seed, the clock and weather events (D-010, D-016).

*Not chosen:*
- a stateful, server-authoritative global weather simulation: field replication and authority
  for no visible gain at planet scale;
- a climate model in the engine;
- physically simulated storms: Stormscapes-class models are interactive over tens of kilometres
  only (a regional "hero storm" later);
- neural forecasting;
- per-plant and per-animal simulation beyond the player's region;
- one biome per planet (No Man's Sky).

**The director** (owner, 2026-09-25): the weather can be steered, by a game AI or by the scripts
designers and players write for scenarios, cinematics and ambiance.
- A director's command is an authored event with a mode: *blend* on top of the function (a storm
  front), or *override* it (the weather fixed, over a region or the whole planet, for as long as
  the event lasts).
- Commands chain on game events: rain when the convoy arrives, then fog after the rain.
- They travel in the replicated event stream like any event, so every machine evaluates the same
  weather (D-010, D-016); fixed weather costs nothing to keep.
- The surface state integrates whatever the weather was: rain a director starts wets the ground
  like natural rain.

**Decisions taken** (owner, 2026-09-25), the three recommendations:
1. Weather as a pure function of seed and time, with events on top (and the director's overrides).
2. The global atlas at level 7 (≈ 78 km).
3. ExoPlaSim as an offline validation tool only (GPL, a separate process).

Weather comes after the current rendering work.

**Phases:**
- climate bake and biomes: Phase 2 (`island`);
- weather and surface state: Phase 3 (`materials-yard`);
- weather rendering: Phase 4 (`dusk-town`'s storm front);
- audio: Phase 6;
- ecosystems and seasons: Phase 8 (`four-km-forest`, across an altitude ecotone).

**The clouds by default** (owner, 2026-10-03): the first cloud layer (#145) is on by default at a
coverage of 0.45, fair-weather cumulus, until the weather map from the climate drives it
(`--clouds COVERAGE` sets another, 0 none).

*(research: planet-environment.md; extends D-019; D-007, D-014, D-016, D-028, D-032; issue #11)*

## D-035 — Content as packages: namespaced ids, layered records, a deterministic merge ✅ (2026-09-25)

Extends D-007 from one material table to every data table, and says where game code and mod code
attach. Three layers:
- **engine crates:** mechanisms and the record types they read; never content names except reserved
  defaults (`forge:default`);
- **game crates:** a demo today, the Phase 10 games later; they link the engine statically and
  register systems, record types and extension points through a plugin trait;
- **content packages:** a manifest, RON records and assets. The base game is a package; a mod is a
  package loaded after it.

**Ids:**
- Authoring: `package:path` strings in files, saves, logs and network manifests.
- Runtime: a dense `u32` per table (`MaterialId` today), assigned after the merge (reserved engine
  rows first, then sorted by id), so it depends only on the content set. Never saved; never sent
  without its manifest.
- Cooked: BLAKE3 content hashes (D-018).

**Records** (`forge-data`):
- A Rust struct with `serde` derives, unknown fields rejected, a default for every field added after
  release, doc comments on fields.
- A schema version per table and migrations from older files; renames by alias, removed names never
  reused.
- RON on disk is the only persistent schema; runtime tables may change at every release; GPU rows
  never appear in files.

**Packages and the merge:**
- Manifest: name, semver, the record-schema generation, dependencies with version ranges, optional
  `after` hints.
- Order: dependencies first by depth, then name (Factorio); a cycle is an error.
- Operations per record: `Add`, `Replace` (last wins), `Patch` (named fields; lists by add/remove).
- A conflict report of every field written by more than one package; every reference validated at
  load; registries frozen; indices assigned; tables built.
- In development, a file change re-runs the merge; tables update in place when the id set is
  unchanged.

**Determinism:** the merge is a pure function of the package set. Its manifest hash (BLAKE3 over the
resolved order, names, versions and content hashes) joins D-016's digests and D-010's handshake. A
mismatched client is refused; later it is sent the missing packages by hash. Data evaluated on both
sides goes through engine evaluators on `dmath`.

**Code:**
- Engine and games are linked statically. No binary Rust plugins.
- `subsecond` hot-patching of system bodies is an optional development feature.
- Mod code, if wanted: Wasm components in wasmtime.
  - A WIT API versioned per interface.
  - NaN canonicalisation, deterministic relaxed SIMD, fuel as the per-tick budget.
  - Server-side by default, client-side only for presentation.
- Never native-code mods.

*Not chosen:*
- paths, or UUIDs, as the authoring id (Godot's and Unity's migration and sidecar problems); UUIDs for
  placed instances are left to the editor;
- whole-file or whole-record override only (Bethesda, RimWorld before Alpha 17);
- a code generator, or reflection-driven loading now (`bevy_reflect` or `facet` come with the editor);
- binary plugins (no stable Rust ABI; `abi_stable` cannot unload);
- Lua in the deterministic simulation (Factorio had to replace its maths and iteration order);
- native-code mods (fractureiser, 2023).

**Decisions taken** (owner, 2026-09-25), the four recommendations:
1. Namespaced string ids, with dense per-table indices from the sorted set.
2. RON + serde, with per-table schema versions.
3. Field-level patches, with a conflict report.
4. Mods are a goal for the Phase 10 games: data-only packages first, Wasm code mods later.

**Phases:**
- `forge-data`, and the stock materials moved from demo code into packages: Phase 2 (before
  `island`); D-034's biome envelopes are the second table;
- simulation and physics tables from packages, the manifest hash in the digests: Phase 3
  (`materials-yard`);
- manifests in the handshake, content fetched by hash: Phase 5 (`hundred-bots`);
- audio events and buses as records: Phase 6;
- games as plugin crates, the first data-only mod, the scripting and mod-code decision: Phase 10;
- the editor: its own research file.

*(research: data-driven.md; extends D-007; D-006, D-010, D-016, D-018, D-026, D-034; issue #64)*

## D-036 — Diffuse light from DDGI probes: cascades around the camera, updated by ray queries ✅ (2026-09-25)

Issue #53 set out three ways to occlude the city's sky light at street scale. The owner picked
the first on 2026-09-25: DDGI probes updated by ray queries. It is research step (3) and
D-008's T1 tier, and it brings bounce light with it. The code is written from the two papers
(Majercik et al., JCGT 2019 and 2021; `shaders/probes.slang`, `shaders/probe_update.slang`,
`forge_render::probes`). NVIDIA's RTXGI-DDGI SDK was read for its constants only: its
licence (NVIDIA RTX SDKs License) is free and royalty-free, but ported files would keep
NVIDIA's notice and terms, unlike the workspace's MIT/Apache.

**The layout:**
- **Cascades.** Five cascades of 24 × 12 × 24 probes, 4, 8, 16, 32 and 64 m apart, each
  centred on the camera. The models lab's rooms start at 1 m (#175): in Sponza's narrow
  courtyard, probes 4 m apart stood in the columns and the arcades and missed its bounced light. A probe keeps its slot while the cascade scrolls: slots are world
  cells modulo the counts, so only the planes entering a cascade are new.
- **Maps.** Each probe has an octahedral irradiance map (6 × 6 texels, RGBA16F) and a map of
  the mean and mean square of the distance to what it sees (14 × 14, RG16F), each with a
  one-texel border, in two atlases.
- **Memory:** 85 MiB with the rays' buffer.

**The update:** three compute passes a frame, before the resolve.
- **The rays**, 128 per probe:
  - The first 32 keep fixed directions and reach 3.5 cells; they only tell the probe where
    it stands.
  - The other 96 turn with a random rotation that repeats with TAA's jitter (as D-030's
    noise does), so a still scene's maps settle into one cycle.
  - Hits are lit by the sun through a shadow ray and by the probes of the frame before (one
    more bounce a frame); misses take the sky-view table.
- **The state.** During its first 8 updates a probe moves through back faces out of walls
  and away from surfaces nearer than a quarter of a cell. It turns inactive inside something
  or with nothing within a cell of it. Then it keeps its place and state until it leaves the
  cascade. Left free, probes between two surfaces changed state every frame, and every change
  restarted their maps.
- **The blend.** Each probe's maps keep 97 % of what they held; a new or re-activated probe
  starts from its first rays.

**The lookup** (resolve and mirror-ray hits):
- **The eight probes** around the point, found after the 2021 paper's bias (towards the
  viewer, in proportion to the spacing). Each is weighted by the trilinear weight, a smooth
  back-face term and a Chebyshev visibility test on its distance map, with small weights
  crushed. The square roots of the irradiance are blended, then squared.
- **The cascades,** finest first, each fading out over its last cell measured from the camera
  (not from the volume, so the fade stays still as the volume scrolls). The open sky's
  irradiance (#47) takes over beyond the coarsest.
- **What it replaces:** the open sky's irradiance on the diffuse side, and the rough
  reflections' blur of it. GTAO stays on top for the contacts.
- **What it dims** (#68): the sky in the reflection of the rows without mirror rays, by the
  probes' irradiance towards the mirror direction over the open sky's there (per channel, at
  most 1), from the same probes and weights as the diffuse lookup.

**One driver note:** a signed remainder of a negative number came out as the unsigned one on
the RTX 5070 Ti's driver, misplacing probes by 16 cells. The code takes remainders of
positive numbers only (`wrap_cells`).

*Measured* (city-blocks, RTX 5070 Ti):
- **Cost:** 0.77 ms on the south view at 1600 × 900 and 1.05 ms at 1440p. The passes are
  0.58–0.62 ms of that and the sampling the rest; the 1440p flight costs 0.77 ms.
- **Stability:** the street view's slow change stays at its level without probes (0.026 %
  against 0.029 %); the south view's rises from 0.089 % to 0.18 %.
- **Identity:** `--no-probes` gives the previous build's pixels.

**Left for later** (issues filed):
- the sky's reflection occluded by the probes (#68, done: 0.11 ms at 1440p);
- probes woken again when geometry moves (doors, vehicles, rebuilt blocks; #69);
- a cheaper lookup (#70: a pass of its own saved only 0.09 ms at 1440p, and lost the normal
  map's effect on the light; parked);
- the T0 updater (SDF marches instead of rays) for GPUs without ray queries;
- the froxel fog lit by the probes;
- the update's cadence (#103, done 2026-09-30): a settled probe updates every other frame,
  keeping the hysteresis squared (the south view 2.03 → 1.81 ms, its slow change 0.18 →
  0.21 %).

*(research: lighting-gi.md, Majercik et al. and the implementation notes on the probes; D-008, D-029, D-030; issue #53; demo: city-blocks)*

## D-037 — World partition: sectors of 2⁴⁰ m, cells of 1 km, `u64` cell ids, a clipmap of cells ✅ (2026-09-30)

`forge-world` (the cloud branch, `docs/HANDOVER.md`) fixes four numbers that D-004 left open;
each is a constant or a layout, changed in one place if the owner prefers another.

- **Sectors of 2⁴⁰ m (1.1 × 10¹² m, 7.35 AU), `i64` per axis.** A position inside a sector's
  frame is at most 2⁴⁰ m from its corner, so an `f64` resolves it to 0.24 mm; a star system's
  frame under a sector reaches 10¹³ m (2 mm) as the research asked; the product of a sector
  difference with the size is exact, and `i64` sectors span 2¹⁰³ m. *Not chosen:* a light-year
  (not a power of two: the difference would round) and 2⁵⁰ m (0.125 m at a sector's far corner).
- **Cells of 1 km (2¹⁰ m) for the GPU record** (#93): the offset inside a cell is exact to
  0.12 mm, the record before #93's precision at the origin; the cell difference times the size
  is exact to 2²⁴ cells. *Not chosen:* 64 km (7.8 mm, the old record's precision at 100 km).
- **Cell ids** (`CellId`, 64 bits): kind (2), level (5), face (3), x (27), y (27). On the cube
  sphere 27 levels reach 1.7 cm cells on a 1 500 km planet; on the flat grid ±67 million cells.
  A parent's id and its children's are arithmetic on the bits, as S2's. *Not chosen:* S2's
  Hilbert-curve ids (locality on disk is the page pool's business, not the id's).
- **The streaming plan is a clipmap of cells**, every level from the finest to the coarsest
  loaded within `rings` cells of that level around the viewer (2.5: about twenty cells a level),
  the finest resident cell drawn over a point, so a coarse proxy stands in until the fine cells
  arrive; a loaded cell unloads once its centre lies beyond `(reach + half a diagonal) × (1 +
  hysteresis)` (0.25). The cells within reach are found by sampling the disc at half a cell's
  spacing, which crosses the cube sphere's face borders without a neighbour table. *Not chosen:*
  annuli per level (a hole appears where the fine level's disc and the coarse annulus disagree
  on a border cell).

The frame tree walks a position up to the lowest ancestor two frames share (a ship placed on
its planet never meets its star's 10¹¹ m: Dungeon Siege's space walk) and across sectors by
the integer difference. Everything is deterministic: integers, `f64`, and `forge_core::dmath`
for the cube sphere's `atan` and `tan` (D-016).
*(research: large-worlds.md §1, §5, §8; D-004 and its amendment; issue #93)* Proposed
2026-09-26; accepted by the owner 2026-09-30 as proposed ("go with recommendations").

## D-038 — The water surface: a forward pass after the opaque resolve, FFT cascades on the compute queue ✅ (2026-09-30)

Proposed from `docs/research/water.md` ("Recommendation for Forge") for Phase 2's third item,
the island's sea, shores, rivers and lakes; nothing of it is built on the GPU yet. The CPU side
exists on the cloud branch (`forge_procgen::ocean`, `forge_procgen::coast`, the rivers and lakes
of `forge_procgen::hydrology`; `docs/demos/island.md`, "The water's fields").

- **Where the water sits in the frame.** A forward pass after the opaque resolve and the sky's
  compose, before TAA: `water/scene-copy` (a transient copy of the HDR image for refraction; the
  graph derives the barriers) then `water/surface` (a graphics pass through the mesh path with
  the indirect fallback, depth-tested against the opaque depth and writing depth, colour and
  motion vectors, so TAA and the aerial perspective treat it as a surface). It reads the
  cascades, the shore fields, the sky-view table (D-031), the aerial-perspective volume (D-023)
  and the sun's shadow by ray query (D-029) as the standard class does. *Not chosen:* the water
  as a visibility-buffer material class (it needs the opaque depth for the depth colour and the
  intersection fade, and the shaded HDR image for refraction, both after the resolve); a
  screen-space-only water (no horizon, no far sea).
- **The waves are FFT cascades on the async compute queue** (`.queue(QueueKind::Compute)`,
  #77: they depend on nothing in the frame's geometry): `water/spectrum` once per parameter
  change, then `water/evolve`, `water/fft-rows`, `water/fft-cols`, `water/derive` per cascade
  (displacement, slopes, the Jacobian's mean and variance, foam accumulated and decayed) into
  persistent images with mips. Three cascades of 256² (patches near 1 km, 100 m and 10 m), a
  fourth of 512² at 4 m when the camera is on the deck. A 256-point row is 2 KB of groupshared,
  under the 32 KB cross-vendor limit, with no assumption on the subgroup size. The spectrum is
  the CPU module's (JONSWAP with the TMA depth factor, Horvath's spreading with a swell term,
  Gaussian amplitudes from `pcg3d(kx, ky, seed)`), and the GPU's lowest cascade is diffed
  against `Ocean::surface` sample by sample (the same bytes within fp16) before it ships.
  *Not chosen:* Gerstner sums for the open sea (they repeat and cost per wave); a single large
  FFT (it tiles visibly and wastes texels near the camera).
- **The surface mesh is a viewer-centred clipmap of rings** (Crest's, or Unreal's quadtree
  tile list), built in compute each frame and drawn through the mesh path, displaced by the
  cascades with the high cascades faded by distance. *Not chosen:* a projected grid alone (the
  horizon swims and the tessellation is uneven under motion).
- **Against the shimmer the owner sees first:** the unresolved cascades' slope variance goes
  into the BRDF's roughness (Bruneton, Neyret & Holzschuch 2010 over Ross 2005's Gaussian sea),
  the whitecap coverage is the Jacobian's filtered statistic, and the ꟻLIP between consecutive
  frames of a still camera is the metric (`tools/compare.sh`), not the eye alone.
- **What is deterministic and what is only visual** (D-016): the spectrum's amplitudes and
  phases, the coast distance, the river polylines and the lake levels are seed-derived and the
  same on every machine, so the sea's height at a point and time is a function the server can
  evaluate; the GPU's transform, the foam, the wetness and the reflections are visual and never
  feed gameplay. D-009's "spectrum evaluated identically on CPU and GPU" is not free with an
  FFT: the options are the lowest cascade re-run on the CPU with `dmath` (a 256² transform is
  18 ms on one core today, once per tick), a readback for the client's prediction only, or a
  matched band-limited Gerstner sum for the physics; that choice waits for Phase 3's boats.
- **Order of building, for the look:** the sea (cascades, the ring mesh, depth colour, the
  glitter without shimmer), the shore (TMA damping and a shore fade from the coast distance,
  Gerstner trains along `−∇d`, a foam line, wet sand written to a clip the terrain's material
  reads), the rivers (ribbons from the polylines with flow maps, widths and depths from the
  catchment), the lakes (a plane per lake at its level). Expected at 1440p on the 5070 Ti, to
  be replaced by the F1 overlay's numbers: the FFT chain 0.1–0.3 ms hidden on the compute
  queue, the surface pass 0.3–0.8 ms when the camera is at sea plus 0.1–0.2 ms of raster, the
  shore and river work per water pixel.

*(research: water.md §1–§5 and its recommendation; D-009, D-016, D-020, D-023, D-029, D-031;
issue #96)* Proposed 2026-09-26; accepted by the owner 2026-09-30 as proposed ("go with
recommendations"). The choice for D-009's physics band still waits for Phase 3's boats.

**Splashes (#107, 2026-10-02).** One of the afterwards, picked by the owner the same day: D-009's
near water ("GPU particles, visual only") as the research recommends (`docs/research/water.md`
§7). The particles are ballistic, without a fluid solver: none of the shipped games found uses
one for splashes, and D-009's PBF/FLIP tier stays for hero events.
- **Sources, by rules from the physics.** The caller lists where the water splashes:
  - **Something meeting it:** a crown above 2 m/s, and a jet at 2 √(R / g).
  - **A step's fall:** drops and mist by how far its sheet breaks up, after Horeni.
  - **A bow:** by its Froude number.
  - **Drips.**

  The research's numbers are starting values. The falls' drops are livelier than its (0.2–0.5
  of the impact speed, 200 a metre a second): a 2 m step's barely left the white water.
- **A ring of slots handed out by the CPU, in order.**
  - The draw's order never changes from frame to frame, so overlapping drops don't flicker
    under TAA. This replaces a dead list or a sort.
  - A stream's drops are born at fixed times from its seed, so the same at any frame rate.
  - Emission and motion run on the async compute queue.
- **The draw, after the water, before TAA:**
  - Soft sprites streaked over half a frame, at least a pixel wide with their alpha scaled by
    the area they lack (Persson).
  - Lit through a shadow ray in the vertex shader, against the static TLAS, which is built
    before any frame.
  - A reactive mask, which the TAA resolve takes as the least share of the current frame (FSR
    2's idea, capped at 0.9). Without it the history smears the crown away.
- **Deterministic and visual-only parts.** The sources come from deterministic data (genesis's
  steps, the caller's events) and every drop from its seed. Nothing is read back, and nothing
  feeds gameplay.
- **Measured:** 0.01–0.03 ms of draw and under 0.01 ms of compute at 1440p, with 1 400–4 800
  drops alive (`docs/demos/island.md`, "Objects in the water").
- **Left for later:**
  - the landing coupling: foam and rings where drops fall back;
  - the crown's curtain and the shore's crests;
  - mist in a froxel volume;
  - the mask passed to DLSS.

## D-039 — Buildings are grammar-derived assemblies of kit modules; a style set per district; a proxy per far building ✅ (2026-09-30)

Proposed from `docs/research/city-generation.md` (§3–§6 and its recommendation) for #86, which
asks for this decision early, with #85's layout pipeline around it; nothing of it is built.

A building is a `BuildingPlan`: a mass model fitted to its lot, refined by a CGA-style split
grammar (split, repeat, component split, insert, context queries) whose terminals are modules
of a style set, never raw geometry, except the roof surface from the lot's straight skeleton and
authored landmarks. Modules sit on a grid of 0.5 m steps and per-style floor heights, exterior
walls in the footprint's outer step so interiors share the grid, rotated in quarter turns; each
module is a cooked cluster-DAG prop carrying its material rows, its variants and its tags
(portals, walkable slabs, stair links, structural role, break-up). A style set is a package
record (D-035): module generators and parameters, the control grammar's attributes, a regional
material set and a contrast rule; districts assign style sets. Module instances are ordinary
instances; a building is a cell of the instance hierarchy (#38) with a merged proxy cooked from
its assembly, drawn when the building's projected size falls below a threshold, and blocks get
a proxy beyond. Rooms and portals are records of the plan, instantiated when #88 streams them.
Everything is a pure function of the seed, the terrain fields and the packages (D-016, D-035).

*Not chosen:* unique geometry per building from the grammar (no instancing, no unit for
interiors, destruction or navigation, hours of cooking per city); hand-assembled kits (no art
team, and the assembly is what the grammar automates); Wave Function Collapse as the primary
generator (no hierarchy; kept for interiors and the village's irregular grid); CityEngine or
Houdini in the loop (commercial, editor-time, not a pure function on the client and the server).

The layout around it (#85), from the same research: a district field, roads by a tensor field
under Parish & Müller's constraints (the village by interest maps and cost paths), blocks from
the graph's faces, lots by oriented-box splits and straight-skeleton strips, landmarks as
package overrides merged as layers, a `CityPlan` of typed records with a digest and a map PNG
judged before any building exists. The first style set is a tropical medieval village (#81:
the owner's answer of 2026-09-26, a fantasy world at a medieval level of technology, the
architecture fitting the place, other islands with other climates at the same era next), the
downtown for city-blocks second and the Mediterranean village (#91) after; the climate
response comes from the atlas as `civilisations-styles.md` says. The module grid is the part
that cannot be retrofitted.
*(research: city-generation.md §3–§6; procedural.md §2, §4; vegetation-materials.md §4; issues
#84, #85, #86, #88, #89, #91)* Proposed 2026-09-26; accepted by the owner 2026-09-30 as
proposed ("go with recommendations").

## D-040 — The rivers' grading: the channels carry the hillslopes' material away, and the valley floors widen into floodplains ✅ (2026-10-01)

Proposed from #109 (the island's rivers reach the sea at 9–24 % over their last 160 m, every
one of its 25 mouths over 5 %) and the owner's judgement on #112 (streams rather than rivers,
not integrated into the terrain). Measured in `docs/demos/island.md`, "The rivers' grading".

- **What was found.** The fall is the same for a 14 m river and a 4 m one, so it is not the
  stream power's profile. On 8 m cells the hillslope diffusion pours a valley's two walls into
  its one-cell floor every step (about a metre, sixteen times the fill at the 32 m spacing the
  parameters were tuned at), and the river re-cuts it: the slope settles at about the fill per
  cell, whatever the river's size. The island's 14 lakes are dams of that fill at the valleys'
  narrows.
- **The rule.** `ErosionParams::channel_area`: the sweep never raises a cell draining that much,
  and raises a smaller channel's by the share of its catchment short of it (in the code, off by
  default). At 25 ha every mouth falls 2–5 %. *Not chosen:* grading only the last reach by
  hand after the erosion (the inland rapids stay, and the terrain no longer explains its
  rivers); a larger threshold (100 or 400 ha keep 3 or 5 lakes but the small rivers at
  7–20 %).
- **Why it cannot be the default as it is.** The fill is, in effect, the island's alluvium:
  150 steps of it lift the valley floors by up to 150 m over the stream power's profile
  (`reports/2026-10-01-109/grading-cross.png`: the same camera 3 m over a river on a plain,
  and 150 m over a canyon). Without it a floor is a slot a cell wide, which the rivers'
  smoothed courses leave at the D8 corners (the water up to 12.4 m under its banks at a third
  of the points), and the carve cuts the walls.
- **The proposal: transport the sediment instead of removing it.** What the walls shed into a
  channel travels down the D8 receivers and deposits where the flux exceeds the channel's
  transport capacity `k_t · A · S` (the transport-limited family of landscape evolution models;
  the source to verify first is Davy & Lague 2009, "Fluvial erosion/transport equation of
  landscape evolution models revisited"): at equilibrium the floor's slope is `q_s / (k_t A)`, and since the
  shed material grows slower than the catchment, a large river's floor is gentler than a
  stream's, a graded, concave profile, with the alluvium kept in the valleys, floodplains
  where the slope flattens, and fans where a river enters a lake or the sea. Deterministic
  (sums in the stack's order, D-016), one more pass per step. *Not chosen:* a floodplain
  pass that lowers the ground along the channels after the erosion (it keeps the slot's
  profile and only widens it); grading the last reach by hand (the inland rapids stay).
- **The lakes** are then a design choice, not an artefact: either none on this island, or
  basins placed in the uplift field (bowls the erosion fills slowly), or dams kept on purpose.
  Taken with the recommendations: the island keeps its lakes as a feature, placed on purpose
  where the transport removes the dams.

*(research: terrain-genesis.md §1, water.md §3, rivers.md; D-016, D-038; issues #109, #112,
#114)* Proposed 2026-10-01; accepted by the owner the same day ("Go with recommendations for
D-040 and D-041"), with the goal restated: "beautiful and playable, not overly realistic". The
order of building puts D-041's valley carve first (it grades the rivers by construction and
gives the look control); the transport term follows on #109.

## D-041 — The island's rivers as valleys: reach types, channels from the regional curves, floodplains in the 8 m field, fewer and larger rivers, the far water in the terrain ✅ (2026-10-01)

Proposed from `docs/research/rivers.md` ("Recommendation for Forge") for #112: the owner's
judgement that the island's rivers read as small streams and don't feel natural or integrated.
Its first finding is that nobody who shipped a river made it read as one with the water alone:
the engines and tools (Unreal's Water plugin, World Machine, Houdini, R.A.M) carve a valley into
the heightfield, paint bed and banks from the same hydraulic fields, and only then lay a thin
flow-mapped surface in it. The second is that Forge's channels are already 1.9–2.5× wider and
1.3–1.9× deeper than their catchments warrant by the regional curves: a 16 km island has
brooks (its largest basin is about 8 km²), and what is missing is the valley, the floodplain,
the bars, the sinuosity and the far-field drawing.

- **Reach types** from the slope and the catchment (Rosgen's classes as the research reads
  them): over 4 % steps and pools in a V valley with no floodplain; 2–4 % rapids with a bench
  one to two widths wide; under 2 % riffles and pools with a floodplain; under 0.5 % near the
  coast or on a plain, braids and bars; a lake or sea entry with enough catchment, a delta.
- **The channel from the regional curve, with an explicit exaggeration:** bankfull width
  `W = k · 2.7 A^0.37` m and depth `D = k_d · 0.3 A^0.21` m (`A` in km²), `k = 2`, `k_d = 1.5`,
  so the width grows downstream at nature's rate and `W / D` stays over 12. Today's
  `w = 0.005 √A` is an unintended `k ≈ 2` with the discharge exponent. The depth coefficient
  does not grow further: a river's apparent size is its width, its turbidity and its valley.
- **A cross-section per type:** a parabola of depth `D` whose deepest line moves to the outer
  bank in bends, a point bar on the inner bend, a cut bank one bankfull depth high on the
  outer, the low-flow water 0.3–0.5 `D` below the bank top so the bank shows. *Not kept:* the
  present banks `0.5x + 0.1x²`, which rise indefinitely into a trench (#114).
- **Floodplain and valley, carved into the 8 m field:** on the gentle reaches a flat
  floodplain `max(6 W, three samples)` wide at the bank top plus 0.3–0.5 m, the channel
  wandering inside it; a bench of one to two widths on the steeper ones; none in the
  mountains; beyond, the valley wall with a shape per setting, blended into the ground with
  low-amplitude noise. In the base field so that every LOD carries it and no coarser mesh
  rises over the water (#113); the metre refinement adds the channel's detail only.
- **The last reach graded** (`S ≤ S_max(A)` downstream, the trunk's last kilometre under 0.5 %
  and its last 200 m under 0.2 %, the bed and the floodplain lowered to meet it), or by
  D-040's transport term in the erosion itself; a small river meeting a cliff may keep a
  waterfall on purpose, never the trunk (#109).
- **Mouths and lake entries:** over the last ten widths the channel widens 1.5–2×, the bed
  drops below the sea's level, the water is the sea's, distributaries split around bars where
  the catchment is large, the sea's displacement fades in over two widths while the river's
  flow fades out (#110); at a lake the slope goes to zero over five to ten widths, the channel
  widens, a fan is painted on the lake floor, no bank step (#114).
- **Materials from the hydraulics, inside the bankfull width:** gravel on riffles and over 1 %,
  sand on bars and in pools, mud in backwaters and deltas, bedrock in the mountains; a wet
  band between the low-flow surface and the bank top; the riverbed layer never wider than
  `W`. **A riparian strip** as a placement rule from two fields, the distance to the channel
  and the height over the water: reeds within a width, shrubs to two or three, trees kept off
  the channel and the bars.
- **Scale: fewer, larger rivers.** The carved-channel threshold from 0.5 to 3–5 km² (about a
  dozen rivers instead of 43; the rest brooks: a wet gully in the layer map, a riparian strip,
  a narrow ribbon near the camera), and the uplift shaped so that two to four basins of
  20–50 km² exist (a 40 km² basin gives a 10 m channel by the curve, 20 m with `k = 2`).
- **The far water is part of the terrain:** beyond one or two kilometres a water layer in the
  terrain material over the water mask (flat normals, the coarse flow map), the ribbon fading
  in as the tiles refine; transitions as material blends over an overlap, no end caps;
  vertical motion on rapids (standing waves, plunge pools) as displacement, not only normals.
- **No simulation now.** Nothing in the owner's list needs a solver. Later, a shallow-water
  pass baked offline per river tile into the flow textures the ribbons read; in Phase 3 a
  runtime window of 512²–1024² cells at 0.5–1 m (0.3–0.8 ms on the compute queue, about
  25 MB) for boats and characters, visual-only under D-016.

*(research: rivers.md §1–§6 and its recommendation; water.md §3; terrain-genesis.md §1–§2;
D-016, D-038, D-040; issues #109, #110, #112, #113, #114)* Proposed 2026-10-01; accepted by the
owner the same day as proposed ("Go with recommendations for D-040 and D-041"; "the goal is
beautiful and playable, not overly realistic", and many more biomes to come). It is the
engines' way (a feature carves and paints the ground around it, the terrain conforms) applied
to the generated island; each part is an issue of its own. The width's `k` is 3, not 2: with
the coastal plain, `k = 2` narrowed the largest rivers (34 → 27 m), and the owner picked 3
from the comparison (`reports/2026-10-01-112/river-size.png`; "use k = 3 and keep the plain as
default", 2026-10-01). The floors (#116, `forge_procgen::carve_valleys`) stop where the ground
stands 6 m over them, so a deep V keeps its walls with room for its water, and the walls rise
no steeper than a little more than the ground beyond them, in place of a shape per setting.
The far water in the terrain's material was not built (#122, for the owner to overrule): the
ribbons already reach every distance, resting on the ground a pixel wide past a footprint so no
coarser level covers them (#113), for about 0.3 ms of the whole water surface from 2.5 km, mostly
the sea's. In the far views they mirror the hills behind them from low and the sun's glitter from
high, as the sea does. The type A reaches' steps and pools are #122: pools between steps, a
boulder or two on most lips, the falls white (`forge_procgen::StepParams`). The scale is #123:
the hills' uplift lowered along three trunk valleys (`IslandParams::basins`), the plain by 70 %
of it, gathers seed 7 into basins of 23, 18 and 12 km² at the sea (the dome's largest were 11,
11 and 9), short of the 20–50 asked on a 16 km island whose land is about 80 km². The lakes
the trunks drained come back as a bowl on each trunk (D-040's lakes placed on purpose). The
lower courses keep a grade of 0.3 % to the sea, the erosion having laid the trunks flat at its
level, and under 3 km² of catchment the rivers ease to nature's size, brooks (the
carved-channel threshold kept at 0.5 km²). The lake entry is #120's deltas
(`forge_procgen::DeltaParams`): over 8 m and five widths before the lake's edge the water eases
flat to the lake's level and the channel widens to twice its width, and in front of the mouth a
fan raises the lake's floor to a top 0.3–1.1 m under the water, painted with silty sand, whose
front drops off into the lake. The outlet's sill is #120's too: past an outlet, the shallow arm a
flat valley floor at the lake's level floods into rises 0.2 m over the level, the river's channel
cut through it, and the lake's water is trimmed off it (`forge_procgen::trim_outlets`). The
mouths' bars are #127 (`forge_procgen::BarParams`): a river 20 m wide or more at the sea has one
bar of sand in its widened mouth, 40 m or more two, teardrops whose crests stand 0.3 m over the
water, the river widened by their breadth, so its water runs round them to the sea in two or
three channels. They stand inside the mouth's reach only; distributaries leaving the river apart
were not built.

## D-042 — The island's geology: a granite core, a limestone coast ✅ (2026-10-02)

The owner's pick of 2026-10-02, when the rock types came up ("it has to make sense for the
island"). The island's rock had no geology: one dark grey on every slope over 0.45, which the
notes of #96 called volcanic. Then the black sand of #128 read the hardness field as basalt, and
the owner judged that it makes no sense on this island. The options were all granite
(Seychelles), all limestone (Mallorca, Dalmatia), a granite core under a limestone coast, or a
plain grey. The owner took the core and the coast:
- **The hills are granite:** grey to pink, coarse-grained, weathering to smooth slabs and domes
  and to rounded boulders (the Seychelles, Corsica).
- **The coast is limestone:** the low ground round the island, the coastal plain and the sea
  cliffs, is pale limestone, the old reefs raised with the island. Where the rain etches its
  bare ground into pavements, karst.
- **The beaches stay pale sand,** quartz from the granite and coral from the reefs. Shingle lies
  on the headlands (#128), and no black sand.

Built as stage 6's rule (D-041's materials as rules): the limestone below a height that wanders
round the island, the granite above it, each with its own texture, and karst on the limestone's
bare, gentler and drier ground. The boulders follow the rocks since #130: granite's rounded
corestones, tors and slabs, limestone's blocks and flags, where rocks gather. The grus (the
granite's coarse sandy soil) lies on its gentle ground round the outcrops since #135.

## D-043 — Changes are checked in tiers: an impact map, sentinels, accepted sets ✅ (2026-10-02)

The owner's pick of 2026-10-02, on #131: "Ok with your plan, the 3DMark model would be way less
often anyway. Go with 1, 2, 3 way of testing." The whole batch on every change had grown to
15–20 minutes of captures, 35–50 with validation and timings, and most changes touch the island
alone: the rest of the batch read 0 px every time. #133 was checked in tiers by hand first, in
about 4 minutes of captures. Built in #134 (`tools/verify.sh`, `docs/PROCESS.md`, "Checking a
change in tiers"):
- **Tier 0, every change:** the gate (fmt, clippy, the tests beside the captures, the credits),
  ten sentinel captures on the mesh path (meshlets, the ballad without TAA and its HDR output,
  the city, the gallery: about a minute), and the sets the changed paths select in
  `tools/impact.toml`. `--recook` when the cooking code changed. The fallback and validation
  only when GPU code or shaders changed, and the timings only when a change claims speed or
  adds or moves a pass. A change to the docs alone runs the gate only.
- **Tier 1, a change to shared rendering:** `forge-gpu`, `forge-app`, the render graph, a
  shared pass or shader, `Cargo.lock`, the toolchain, and any path the map does not know (fail
  safe). The full batch on both paths, validation and timings, as before.
- **Tier 2, a milestone** (every ~5 commits, before a showcase, when a system closes): Tier 1,
  plus the batch with `FORGE_ASYNC=0`, `tools/origins.sh` and the real-time tour.
- **Accepted sets** (`captures/accepted/<sha>/`, with a manifest of hashes) replace the
  "before" batch: a run compares with the last accepted commit's images. An image a change is
  meant to alter is accepted when the report names it (`--expect`).
- **#71's flake** is told by its signature (at most 500 px, ꟻLIP mean at most 0.0015, largest
  at most 0.15) and no longer fails a run. It also shows on the HDR output's TAA frame 600, a
  sentinel (two runs in six on 2026-10-02): its PQ codes count as the flake only when its SDR
  preview does. Since 2026-10-03, one PQ code on at most 100 px of either HDR capture (frame
  240 or 600) counts too, with the preview the same (two Tier 2 runs: 20 and 19 px).

Measured on 2026-10-02: a docs-only change runs the gate in 16 s. #133 against its parent runs
Tier 0 (the sentinels, the island and the city, recooked) in 233 s, the gate included: the
sentinels and the city read 0 px, the island its expected changes. A first Tier 2 at `ec4e626`
took 1 592 s, the timings included: the full batch in 390 s, the same with `FORGE_ASYNC=0` in
333 s (0 px against it but for one recognised flake), `origins.sh`, clean validation, and the
timings of the build against itself, within 0.04 ms per view.

Not chosen: the full batch every time (too slow), and the sentinels alone (an island change
must see the island). A 3DMark-style benchmark in one process ("forge-mark") is a later step,
run far less often.

## D-044 — A particle liquid for the lab; the heightfield and particles for the world ✅ (2026-10-03)

The owner's request of 2026-10-03 (#155, #156): glass tanks in the physics lab with the water's
volume drawn, a dam break and a gate with a round hole, the camera crossing the surface, and a
liquid that behaves round obstacles; "I think we will have to use fluid simulation using
particles for that." Research: [research/particle-fluids.md](research/particle-fluids.md)
(67 sources, read with WebFetch and WebSearch; claims from a search snippet alone marked †).

**The owner's answers of 2026-10-03** (to the seven questions below):
1. "Let's try with hybrid for now": the particle–grid hybrid.
2. "I would stay visual lab for now": the liquid stays in the lab and moves nothing outside it.
3. "Let's try bigger or finer (1 cm, ~640 000), if it's too much after some testing we will
   reduce a bit": the first tank runs at 1 cm, and the size comes down only if measured too
   slow.
4. "Hard too tell, we wil need to do some testing to find the right compromise": the budget is
   measured, not set. Every pass is in the F1 overlay and `docs/PROFILE.md`, and the cell size
   and substeps are run-time options, so it can be found by trying.
5. "Water should be physically pale (pure water barely tints a metre) and the hole's jet should
   foam (let's me see). Also some future game or demo may need more stylised fluids": pure
   water's absorption by default, the jet's foam drawn, and the colour, absorption and
   scattering kept as a material's parameters, not constants, so a stylised liquid is a
   setting.
6. "Frozen until the lab's liquid exists": the column model's flood is not tuned further.
7. "You could use the RDNA 2 iGPU but it's far less powerful and many users may not have it or
   an equivalent. You could use it in labs if you need additional computation power": the iGPU
   stands in as the AMD check. The liquid never requires a second GPU, and the lab may use one
   only as an option.

**Taken:**
1. **The lab's liquid:** a GPU particle–grid hybrid.
   - Particles carry the water with APIC transfers (MLS-MPM's quadratic B-splines).
   - A grid's pressure projection keeps it incompressible, and a correction on the particles'
     density holds its volume and level (Kugelstadt et al. 2019).
   - It runs as Vulkan compute in Slang, with no CUDA, no float atomics and no wave-size
     assumption.
   - Its transfers and sums use 32-bit fixed-point atomics, and any ordering of the particles uses
     a stable radix sort, so it is bit-deterministic on one GPU.
   - EA SEED's Position-Based MPM (BSD-3) stays the alternative pressure model, behind the same
     data, decided by an A/B at the second milestone.
2. **Its drawing:** a ray-march of the solver's density grid.
   - The glass walls and floor are analytic planes.
   - Snell's refraction, Beer–Lambert absorption (pure water's, scaled), single scattering and
     total internal reflection.
   - A near-plane underwater mask with a meniscus line for the camera crossing the surface.
   - Motion vectors from the grid's velocity for TAA.
   - Spray, foam and bubbles as diffuse particles (Ihmsen et al. 2012), shared with the island.
3. **Jolt:** bodies are written into the grid as moving solids. The GPU hands back each body's
   submerged volume, centre of buoyancy and the water's velocity, a frame late, for Jolt's
   buoyancy impulse. Full momentum exchange is the second milestone.
4. **Determinism:** the GPU liquid is never network or gameplay state outside the single-player
   lab. In the lab it replays to the same digests on one GPU and is checked by measures
   (volume, level, outflow) on others.
5. **The island:** the heightfield stays authoritative (the CPU column model, the GPU's shallow
   water near the player). Ballistic GPU particles are spawned where it fails (Chentanez &
   Müller 2010) and returned to it when they land.
6. **Code to learn from or port, with credit:** MIT, BSD-3 and Apache-2.0 only (WebGPU-Ocean,
   pbmpm, Blub, FidelityFX Parallel Sort, GPUPrefixSums). Not GPUMPM (GPLv3). PhysX and FleX as
   references only (CUDA).

**A first milestone, "Glass tank 1" (estimates, to be measured):**
- One tank, 1.0 × 0.5 × 0.6 m inside, with a 0.4 m block of water behind a gate.
- A 1 cm grid (the owner's answer 3; 1.5 cm was proposed), about 640 000 particles, 4 substeps a
  frame.
- Proposed at 1.5 cm: at most 2.5 ms of simulation and 1.5 ms of drawing at 1440p on the RTX
  5070 Ti. At 1 cm the particles are 3.4 times as many: measured, then the owner picks (answer 4).
- Checks:
  - the volume within 1 %;
  - the final level within 2 mm of V / A, the check the column model fails;
  - the front against Ritter's speed;
  - the hole's jet near √(2gh) with a discharge coefficient near 0.6;
  - three runs to the same digests.

**Built, the first step (#156, 2026-10-03):** `physics-lab --lab tank` and `--lab tank-bench`
(`docs/demos/physics-lab.md`).
- **The tank:** larger than proposed, at the owner's ask: 1.6 × 0.6 × 0.6 m with the same 0.4 m
  of water behind the gate. It runs at 1.25 cm (590 000 particles: answer 3's budget spent on
  "bigger"); `--liquid-cell 0.01` gives 1.15 million.
- **The bench:** the same tank to tune by, the owner's ask: no glass, a floor of squares, a plain
  background, tinted water.
- **Departures from the proposal:**
  - trilinear weights, not MLS-MPM's quadratic B-splines (8 faces a component, not 27, at this
    many particles);
  - the volume held by moving the particles down their crowding's gradient, not by a second
    projection or a divergence target (as a target it fed the velocity and shook still water
    apart);
  - the grid's sums as 64-bit atomics, weight and momentum packed.
- **Measured:**
  - still water at rest to the bit;
  - settled 0.8 mm under the level its volume gives (D-044's check: 2 mm);
  - the front at three quarters of Ritter's speed;
  - three runs to the same digests;
  - 4.5 ms a frame of simulation (3.0 sorted, below) and 0.16 ms of drawing at 1600 × 900,
    against 2.5 + 1.5 proposed for a third of the particles: the owner's answer 4 is to find the
    compromise by trying.
- **Then (#156):** `--lab tank-hole`, a jet through a round hole in the fixed gate (a discharge
  coefficient of 0.75 at 6.4 cells across, against a sharp-edged hole's 0.6); white water carried
  by the particles, short-lived in fresh water (the owner: "foam on clear, non salt water does
  not make too much sense"), its life a property of the liquid; the speed view for tuning.
- **Under the water:** rays start on the near plane, so the camera can cross the surface.
- **The particles sorted** by cell once a frame: 3.0 ms of simulation, from 4.5.
- **Where a bent ray lands:**
  - **How it is found now:** the ray is traced against the scene's ray-tracing structures. It
    takes the screen's colour where the camera sees that point; where it does not, the point is
    shaded plainly, as a mirror ray's hit is.
  - **What it replaced:** a search over the screen against the depth. It gave the sawtooth edges
    and speckled panes of the owner's report.
  - **Its cost:** nothing measurable (`liquid/draw` 0.20 ms).
  - **The underwater hatching:** that was rounding, and is fixed.
- **Blocks in the water** (`tank-blocks`): fixed boxes in the solver's cells and particles, the
  density field smoothed round them; the water goes round and over them and settles 0.6 mm short,
  as in the plain tank.
- **Not yet:** bodies that move in the water, caustics. A multigrid pressure was tried and stays opt-in
  (`--liquid-cycles`): at the settings that keep still water still, it saves a fifth of the
  sweeps' time at best, and leaves its worst cell 20 times as far off (`demos/physics-lab.md`).

**Built, item 5's first step (#162, 2026-10-04):** the GPU's shallow-water layer over the lab's
flood (`forge_render::ShallowLayer`, `docs/demos/physics-lab.md`). It is the column model's scheme
at four times the resolution (6.25 cm over the whole basin), pulled a quarter of the way to the
columns each frame, never read back; +0.30 ms at 1600 × 900. The ballistic particles: the
splashes' drops (#107), sprayed where a column's water runs fast over the dry floor or into a wall
or a block, found from the columns; they take no water from them (a few litres against the
basin's 468 m³). Left: a window round the player for the island.

**The questions put to the owner** (answered above):
1. Does a particle–grid hybrid count as the "particle simulation" asked for, or must it be
   grid-free (PBF or SPH: more compression, more tuning)?
2. Should the lab's liquid stay visual and lab-only, or move bodies in ways that matter beyond
   the lab?
3. Is the tank's size and detail right (1.5 cm, ~190 000 particles), or bigger or finer (1 cm,
   ~640 000)?
4. How many of the 16.7 ms may the liquid take next to the ray-traced shadows, the GI and TAA?
5. Should the water be physically pale (pure water barely tints a metre) or stylised bluer and
   cloudier, and should the hole's jet foam?
6. Should the column model's flood be fixed further in the meantime (its look round obstacles
   was #153), or frozen until the lab's liquid exists?
7. Is the dev machine's RDNA 2 iGPU acceptable as the AMD check until an RX 9070 XT is
   available?

---

## D-045 — An image that stays sharp in motion, and deeper blacks ✅ (2026-10-03)

The owner's report of 2026-10-03 (#159): "I feel like the image is always a bit blurry of fuzzy,
never clear and neat as it should". Then, with a summary of Digital Foundry's piece on TAA:
"what about other techniques?" (DLAA, SSAA, DLSS or FSR at native resolution, SMAA). Measured in
the physics lab's sharpness room (`--lab room`, `tools/sharpness`: slanted edges, MTF50 in
cycles a pixel, an ideal pixel's 0.60; [demos/physics-lab.md](demos/physics-lab.md)).

**Where the softness is:**

| | Still | Panning 0.5 m/s | Panning 2 m/s | Cost at 1600 × 900 |
|---|---|---|---|---|
| TAA (today's) | 0.54 | 0.30–0.38 | 0.30–0.33 | 0.055 ms |
| DLAA | 0.60–0.61 | 0.41–0.44 | 0.43–0.48 | 0.48 ms |
| TAA, Lanczos-3 history | 0.54 | | 0.35–0.375 | 36 loads a pixel |
| No anti-aliasing | over 1, aliased | over 1, aliased | over 1, aliased | |

- **Still, TAA is sharp.** It is within a tenth of an ideal pixel.
- **Moving, it is not.** The edges across the motion lose half their contrast at a quarter cycle
  a pixel. TAA resamples its history each frame where the motion is a fraction of a pixel (at
  whole pixels it stays sharp), over the ten or so frames its blend of 0.1 keeps.
  - The history's filter changes little.
  - Its weight decides: at a blend of 0.6 the moving edges are as sharp as still ones, but the
    aliasing and the noise TAA averages away come back. The sun disc's soft shadows and the
    probes' noise are averaged over its 8 jitter phases.
- **A sharpening pass** (FidelityFX RCAS, previewed on the captures) gives back the contrast at
  a quarter cycle: 0.85 to 0.99 still at 1 stop, 0.65 to 0.85 panning at half a stop. It does not
  give back the finest detail.
- **The flat look is the tone curve.** AgX, the default, shows the black squares at sRGB
  0.20–0.23, a milky grey; ACES and neutral show them at 0.07–0.09. Bloom at 4 % lifts them by a
  further fifth. A sunlit white wall stays short of white under all three at EV 15.

**The options:**
1. **DLAA on NVIDIA's RTX cards**, TAA elsewhere. It is in the engine already (the asteroids'
   `--upscaler dlaa`, the lab's `--dlaa`). It is the sharpest measured, still and moving, and it
   is temporal, so it keeps TAA's calm: no shimmer. It is NVIDIA-only and costs 0.43 ms more, and
   Streamline's DLLs ship with the build.
2. **A better TAA for every GPU:**
   - an RCAS pass after the resolve (about 0.05 ms);
   - a Lanczos-3 history (+15 % moving);
   - later, a history at twice the resolution, which quarters the resampling's loss, as Unreal's
     TSR offers (`r.TSR.History.ScreenPercentage`). That means four times the history's memory and a
     larger change.
3. **FSR 3 at native resolution** (AMD FidelityFX, MIT, both vendors): a tuned TAA with its own
   sharpening, not yet measured here. AMD-specific work waits for an RX 9070 XT (#67), but this
   also runs on NVIDIA.
4. **SSAA**, the scene drawn at 2 × 2 the pixels: the reference image, still and moving. It costs
   about four times the frame (the island and the city would leave 60 fps), so it suits screenshots
   and cinematics, not play.
5. **SMAA** (1x or T2x): sharp and without ghosts. It does not settle sub-pixel shimmer (the
   asteroids' first complaint) nor the noise of the effects TAA averages today: the soft shadows
   and GTAO.
6. **The tone:** neutral or ACES by default in place of AgX, or AgX with more contrast (its
   "punchy" look); bloom at 2 %; the exposure half a stop to a stop brighter so white reads
   white.

**Proposed:** 1 and 2 together.
- DLAA by default where the GPU has it: the owner's 5070 Ti.
- TAA with an RCAS pass and a Lanczos-3 history everywhere else, each measured in the room
  before and after.
- SSAA as an option for captures.
- The tone curve chosen by the owner's eye from the room's captures under each (G cycles them).

**Questions for the owner** (answered below):
1. DLAA by default on RTX cards, with TAA where there is none?
2. A sharpening pass after TAA, and how strong: half a stop or a whole one (shown on the same
   captures)?
3. Which tone: AgX as it is, AgX with more contrast, neutral or ACES?
4. SSAA for screenshots and cinematics?

**The owner's answers of 2026-10-03:** "I will follow your recommendation, 'Proposed: 1 and 2
together'."
1. "DLAA by default on RTX, and TAA where none."
2. "Use the best compromise, performance in this case might be weighting a bit more": the
   sharpening's strength is chosen by measuring, leaning to the cheaper side.
3. "Keep AgX but it would be nice to have the others as options in engine (maybe in a dev
   console?)": AgX stays the default, and the other curves become a run-time setting.
4. "Ok for SSAA for screenshots."

**Taken:**
- **Where NVIDIA's DLAA runs (RTX cards):** it is the default, through Streamline, loaded only on
  NVIDIA (the cross-vendor rule).
- **Everywhere else:** TAA with an RCAS pass after the resolve and a Lanczos-3 history, each
  measured in the room before and after. RCAS's strength is the lower of the strengths that give
  back most of the moving edges' contrast (the cost counts).
- **SSAA for screenshots** (2 × 2), not for play.
- **The tone curve:** AgX by default; neutral, ACES and AgX with more contrast are selectable at
  run time, for now by key and flag, later in the settings and the dev console (#160).
- **Still open:**
  - bloom's strength;
  - the exposure's half to whole stop;
  - a history at twice the resolution;
  - FSR 3 at native resolution, to measure when the AMD work starts (#67).

**Done so far:**
- **RCAS after TAA**, on by default at half a stop (`--rcas`, `--no-rcas`, T cycles). It gives
  back about 60 % of the contrast moving edges lose (MTF at 0.25 c/px, panning at 2 m/s: 0.62–0.67
  → 0.83–0.87) and overshoots still edges by 9 %. At 1 stop it gives back a third. It costs
  +0.06 ms at 1600 × 900. Frame to frame, the image changes a little more (the city: ꟻLIP mean
  0.0078 → 0.0085).
- **AgX punchy** among the curves (`--tonemap agx-punchy`, G): black squares at sRGB 0.055–0.08,
  the white wall at 0.55 (AgX: 0.20–0.23 and 0.67). AgX stays the default.
- **DLAA by default where it runs** (the `dlss` feature on by default; an RTX GPU and the
  Streamline SDK), in interactive runs. Scripted runs keep to TAA unless `--dlaa`: DLAA's images
  differ by up to 3 codes from run to run, and the captures' checks want them to the bit. Its
  display pass mixes in bloom now. T cycles DLAA, TAA sharpened, TAA plain and off.
- **The Lanczos-3 history** by default (`--taa-catmull-rom` for the old filter): panning at
  2 m/s, sharpened TAA's MTF50 goes 0.37–0.40 → 0.42–0.44 (DLAA: 0.43–0.48), for 0.03 ms.
- **SSAA 2 × 2 for screenshots** (`--ssaa`, in the shell for every demo): MTF50 0.60–0.66
  panning at 2 m/s, an ideal pixel's; the city's frame 2.4 → 5.1 ms.
- That completes what D-045 took. The open points stay open.

## D-046 — A night sky over the ground ✅ (proposed and accepted 2026-10-04)

Phase 4 lists a night sky beside the clouds (`docs/ROADMAP.md`). Today the sun sets into the
atmosphere's twilight and the sky goes dark. The ballad's stars are art-directed for space,
and the automatic exposure either leaves the night black or lifts its noise.
Research: [research/night-sky.md](research/night-sky.md) (Jensen et al. 2001's physical night
sky, Kirk & O'Brien 2011's low-light tone mapping, the Moon's photometry, measured light levels).

**Proposed:**
1. **The sky at night:** the atmosphere's tables lit by the Moon as by the sun (a second source
   through the same transmittance and scattering), over a starfield and a faint Milky Way.
   Airglow sets the moonless sky's floor.
2. **The Moon:** a disc shaded by phase (the sun's and the Moon's directions) with a
   Lommel–Seeliger law, and a second directional light at its illuminance: up to about 0.3 lux
   full, its shadows traced as the sun's (#45), lighting the probes (#53).
3. **Seeing it:** the automatic exposure's range reaches night, but stops a few stops short of
   the eye's full adaptation, so the night reads as night. A Purkinje shift in the tone curve's
   pass blends towards a desaturated blue as the scene's luminance falls.
4. **`--day` runs on through the night,** for the demos and the captures.

**Questions for the owner:**
1. **Stars:** the real sky from the Yale Bright Star Catalogue (9 110 stars; a US government
   work; a download of a few hundred kilobytes, so your go first), or the procedural stars
   `Starfield` already draws?
2. **The Moon's surface:** a plain shaded disc first, or with an albedo map of the real Moon
   (public NASA imagery, another download)?
3. **How dark:** night as films show it (blue, readable, the exposure capped about 4 stops
   under the day's), or darker and closer to the eye's (very little colour, much of the scene
   near black under a new Moon)?
4. **The Moon's light:** traced shadows like the sun's (the cost is the sun's, which is down),
   or a soft unshadowed fill?
5. **Light pollution and night lights** (the town's windows, street lamps) — later, with the
   emitters of the `dusk-town` demo (Phase 4, step 4), or now with the sky?

**A first step once answered:** the Moon as a light and a disc, the night through the existing
atmosphere, the exposure's range and the blue shift, on the island with `--day` running to
midnight; the stars as answered.

**The owner's answers (2026-10-04),** the proposal accepted as written:
1. **Stars:** both. The procedural stars stay the default; the Yale Bright Star Catalogue is
   downloaded too, to see the real sky in the engine.
2. **The Moon:** a plain shaded disc by default, and the real Moon's albedo map (NASA imagery,
   downloaded) to compare. One switch, a flag and a key, turns on the real sky and the real
   Moon together.
3. **How dark:** night as films show it: blue, readable, the exposure capped about 4 stops
   under the day's.
4. **The Moon's light:** the recommendation, traced shadows as the sun's, with the soft
   unshadowed fill kept behind a flag so both can be seen side by side.
5. **Light pollution and night lights:** later, with the emitters of the `dusk-town` demo
   (Phase 4, step 4).

**Built, the first step (#164, 2026-10-04):** `forge_render::night`, the sky's passes and the
display pass; `docs/demos/island.md`, "The night".
- **The day's cycle:** `--day` runs on through a night as long as the day; `--time-of-day`
  takes 0–2 (1.5 midnight). The Moon's age is `--moon-age`, 0.4 by default (a waxing gibbous).
- **The key light:** the sun until 3° under the horizon, then the Moon. The sky's tables are
  per unit of a reference illuminance that follows the scene's light, and the sun's twilight
  stays as a second light until 20° under the horizon.
- **The sky-view table runs round the whole circle of azimuth (384 × 108),** not the half the
  sun's symmetry allowed: with two lights the sky is not symmetric. Every ground-sky image
  moved slightly.
- **How dark: 2 stops under the eye's adaptation, not 4.** At 4 the moonlit frames were near
  black and the blue did not read. The meter takes the scene's brighter half (the sky and the
  clouds), so at 2 the moonlit land shows about 4 stops under its day's brightness, which was
  the answer's intent. `--night-stops` sets it; the doc shows 4, 3 and 2 side by side.
- **The Purkinje shift in two halves:** the colour fades towards the rods' luminance before the
  tone curve, and the display's colour is tinted blue after it. A blue given before the curve
  was washed out by ACES 2.0's chroma compression in the darks. The HDR path takes the fade
  alone for now (#94).
- **The exposure at a night's first frame:** metered at the day's EV, only the Moon's disc
  stood above the histogram, and the exposure snapped to it. With a night, a frame where under
  5 % of the pixels are metered now looks 12 stops brighter.
- **Stars:** procedural, 8 912 of them to magnitude 6.5 with the sky's counts per magnitude,
  twice as dense on the galactic plane, drawn about a pixel wide and 16 times brighter than
  physical (`--star-gain`), as an eye sees them rather than a camera.
- **The second step, built the same day:** `--real-sky` and **.** draw the Yale catalogue's
  9 096 stars (NASA HEASARC's copy) and the Moon with NASA's CGI Moon Kit albedo map, both
  downloaded with the owner's go and kept as small derived files in `assets/sky`. A telephoto
  (`--fov`) shows the maria: past 8 pixels across, the disc is scaled towards the scene's
  adaptation (the eye's local adaptation), since the night's exposure burns it white.

## D-047 — Texture coordinates in the cluster pages, and textures from glTF ✅ (proposed and accepted 2026-10-04)

#166. A cluster vertex (`PagedVertex`, 16 bytes) holds a position and a normal, but no texture
coordinate. Every surface is textured by projection (triplanar), and the glTF importer reads
no `TEXCOORD_0` and no images. So an imported model shows only its material's flat colours,
and a skinned body cannot carry a painted texture. #166's first step projects the creatures'
textures from their bind pose so they stay on the body. That works for wood or fur, not for
a face, a label or a baked normal map. Unreal and Unity store UVs with the vertex. Nanite
stores them per cluster, quantised to each cluster's range.

**Proposed:**
1. **An optional UV stream in the cluster's payload.** After the vertices, a mesh with UVs
   adds:
   - its range (16 bytes: the cluster's UV minimum and extent as `float2`s);
   - per vertex, two 16-bit unorms within that range (4 bytes).

   Then come the triangles as now. A mesh without UVs is unchanged and pays nothing: the
   island, the city and the procedural props keep their 16-byte vertices. A textured
   vertex costs 20 bytes, plus 16 per cluster. A flag in `Mesh` (its `pad`) says whether
   the stream is there.

   The range per cluster rather than per mesh keeps the precision when UVs tile far past
   0–1 (a wall repeated 100 times). 16 bits over one cluster's range is well under a texel.
2. **No tangents stored.** The pixel's tangent frame comes from the derivatives of its
   position and UV, which the resolve already computes (`dpdx`, `dpdy`).

   Caveat: a normal map baked by a tool against MikkTSpace tangents can differ slightly
   from that frame, most visibly on low-polygon bakes. If it shows, a stored tangent would
   add 4 bytes per vertex (an angle and a sign).
3. **The cook keeps UV seams.**
   - glTF already splits vertices at seams, and the clusters copy their vertices.
   - The simplifier gets the UVs as attributes beside the normals.
   - A seam's vertices are locked as section borders are, so coarse levels don't smear a
     texture across a seam.
4. **The importer:**
   - `TEXCOORD_0`;
   - from the `.glb`'s embedded images: base colour, normal, metallic-roughness and
     occlusion, with emissive optional;
   - decoded at load to RGBA8 with mips made as `textures.rs` makes them (albedo averaged
     in linear light), into the bindless set.

   Compression (BC7 and BC5) comes later, measured.
5. **Shading:** a material row that samples by UV, beside the triplanar rows; the material
   table picks one per section. Skinned meshes with UVs need no bind-pose projection:
   their UVs travel with the vertices. The projection stays for triplanar materials on
   skinned meshes.
6. **Measured:** the island's page count and bandwidth must not move (no UVs there).
   Textured meshes report their page bytes against the same mesh without UVs.

**Not chosen:**
- **A wider vertex for every mesh** (24 bytes): 50 % more page bytes on the island for no
  gain.
- **Half floats:** a 10-bit mantissa steps half a texel of a 4K texture near 1.0.
- **A UV buffer outside the pages:** pages would no longer stand alone (D-018), and streaming
  would track two things.

**Questions for the owner:**
1. **The format:** the optional per-cluster stream above (recommended), or one of the
   rejected options?
2. **JPEG:** many `.glb` files embed JPEG images. Turning on the `image` crate's `jpeg`
   feature fetches one more crate (`zune-jpeg`, MIT/Apache/Zlib). Your go to add it, or PNG
   only for now?
3. **Texture compression:** RGBA8 first and BC7/BC5 later (recommended), or compressed from
   the start? The latter needs an encoder crate, another download, and a slower cook.
4. **The first textured model:** our creatures UV-unwrapped in
   `assets/blender/skinned_creatures.py` with our own painted or procedural textures
   (recommended, nothing downloaded). Or a free reference model, chosen under the separate
   proposal about third-party models?

**A first step once answered:** the stream in the cook and the pages, `TEXCOORD_0` and PNG
images in the importer, the UV material row, and the creatures unwrapped and textured, with a
capture and the page bytes measured.

**The owner's answers (2026-10-04),** the proposal accepted as written, with one addition:
the material properties the artist gave are kept wherever the file has them.
- That means:
  - each texture's sampler (repeat, clamp or mirror on each axis);
  - its transform (`KHR_texture_transform`);
  - the factors that scale each texture (base colour, metalness and roughness, the normal
    map's scale, the occlusion's strength, the emissive colour and strength).
- What Forge cannot draw yet is logged when the model loads, not dropped silently: alpha
  cut-outs and blending, a second UV set, and nearest filtering.
1. **The format:** the optional per-cluster stream.
2. **JPEG:** yes. PNG is preferred: our own models embed PNG, and JPEG is read when a file
   has only that.
3. **Compression:** RGBA8 first, block compression later and measured. The options:

   | Approach | For | Against |
   |---|---|---|
   | RGBA8, decoded at load (first) | No new dependency beyond the decoders. Exactly as authored. Simple. | 4 bytes per texel. A 2048² map with its mips is 22 MB, and a material of four maps is about 90 MB. The most bandwidth. |
   | BC7 for colour, BC5 for normals, at cook time, cached | A quarter of the memory and bandwidth. The PC standard. BC7 is near lossless. | An encoder crate. The cook takes minutes for large sets. Cache files to keep. |
   | KTX2 with Basis Universal, transcoded at load | Smallest on disk and to download. One file for every GPU. | Below BC7's quality. A C++ transcoder. Transcoding time at load. |
   | The model's own compressed textures (`KHR_texture_basisu`, DDS) | Nothing to encode. | Few free models ship them. Needs a KTX2 reader. |
4. **The first textured model:** our creatures. Advanced third-party models follow soon,
   under their own proposal, because ours are simple.

**Built, the first step (#166, 2026-10-04):**
- **The stream** (`forge_geom::page`): after a cluster's triangles, not right after its
  vertices as proposed. The triangles keep their offset, so the skin pass, the index fallback
  and the software rasteriser are unchanged, and only the resolve finds the stream
  (`uv_stream` in `meshlet.slang`). A flag in `Mesh` says it is there, and the mesh cache
  stores it (format 3: every cached mesh cooks again once).
  - **Precision:** within half a step of 1/65535 of the cluster's UV extent (the test tiles
    UVs well past 0–1).
  - **Cost on the creatures:** 21 % more payload bytes (181 KB against 149 KB for the
    mannequin), two pages each as before. A static cook of the same mesh grows 22 %, from 4
    pages to 5. Meshes without UVs are unchanged.
  - **UVs do not steer the simplifier:** a cook with UVs makes the same clusters. Seams hold
    because glTF splits their vertices. Weighing the UVs in the simplifier's error waits for
    models with many LOD levels.
- **The importer** (`forge_geom::model`) reads:
  - `TEXCOORD_0`, the images in the `.glb`, and each texture's wrapping and
    `KHR_texture_transform`;
  - every factor the material gives.

  The normal and occlusion maps take the base colour's transform, because the `gltf` crate
  gives theirs only through raw extensions. What Forge cannot draw is listed in
  `Model::unsupported` and logged once per model: alpha cut-outs and blending (drawn
  opaque), a second UV set, nearest filtering.
- **The textures** (`forge_render::textures::decode_image`): PNG or JPEG decoded to RGBA8,
  resampled to powers of two in linear light. Mips are made by use: colour averaged in
  linear light, normals renormalised, data as stored. Each image is decoded once per use
  (`ModelTextures`). Nine anisotropic samplers cover glTF's wrapping on each axis
  (`SamplerKind::AnisotropicWrap`).
- **The UV row** (`MATERIAL_FLAG_UV`, `shade_uv_mapped`):
  - glTF's metallic-roughness model on Forge's highlight. A metal keeps no diffuse colour, its
    reflectance at normal incidence is its base colour's brightest channel, and its highlight
    weighs `METAL_SPECULAR` (0.35) more. The highlight and the reflection are not tinted by a
    metal's colour yet.
  - Occlusion darkens the sky's and the probes' light only.
  - The tangent frame is Schüler's cotangent frame of the position's and the transformed UVs'
    derivatives; glTF's +y points towards −v. On a skinned mesh it is taken in the bind pose
    and turned with the body.
  - `GpuMaterial` grows from 112 to 176 bytes.
- **The creatures:** unwrapped and baked in Blender (`docs/demos/physics-lab.md`, "Their
  textures"). The procedural wood and fur of the first step are gone.

**Built, cut-outs and double-sided rows (#171, 2026-10-04):** the owner asked for Sponza
fixed fully, and its foliage and chains are glTF `MASK`.
- **Rows:** a row with `alphaCutoff` (`MATERIAL_FLAG_MASKED`) or `doubleSided`
  (`MATERIAL_FLAG_DOUBLE_SIDED`) keeps both in `GpuMaterial`'s padding, so the size is
  unchanged.
- **Raster:** their clusters take a hardware raster of their own: no face culled, the
  fragments tested (`docs/ARCHITECTURE.md`).
- **Rays:** the rays test the same texels at the ray-traced copy's UVs.
- **Our own models:** Blender exports every material double-sided, and our closed models are
  now exported, and their files set, one-sided.
- **Still drawn opaque, and logged:** blending (`BLEND`).
- **Mip coverage:** a cut-out's mips average its alpha, so it thins with distance.
  Coverage-preserving mips wait until a model shows it.

## D-048 — Reference models from outside: the Khronos glTF sample assets ✅ (proposed and accepted 2026-10-04)

Our own models are simple: the lab's vehicles and creatures are built from a few shapes in
Blender. The owner asked for detailed models made by others (2026-10-04), for two reasons:
renderings to compare with other engines', and loads to measure. Khronos publishes the glTF
sample assets (<https://github.khronos.org/glTF-Assets/>). Each comes with a screenshot from
its reference viewer, and most are CC0 or CC-BY 4.0. Licences and sizes below were read from
the repository on 2026-10-04 and are checked again at download.

**Proposed:**
1. **Not in git.** `assets/external.toml` lists each model:
   - name, URL at a pinned commit, licence, authors, size, SHA-256.

   `tools/fetch-assets.sh` (bash and curl) downloads them into `assets/external/`, which git
   ignores, and checks each hash. Each CC-BY model gets its `CREDITS.md` line in the commit
   that lists it. Captures that need a missing model are skipped and say so; CI downloads
   nothing.
2. **A models scene** (`physics-lab --lab models`): each model on a plinth under the sky, a
   capture per model from the angle of its Khronos screenshot, and the two shown side by side
   in its doc page.
3. **The importer catches up** where the set needs it:
   - `.gltf` files with their buffers and images beside them (FlightHelmet, SciFiHelmet and
     TextureTransformTest exist only in that form);
   - alpha cut-outs (`MASK`), once a model needs them.

**The first set** (CC0 or CC-BY 4.0, about 160 MB):

| Model | Licence | Size | What it checks |
|---|---|---|---|
| TextureCoordinateTest | CC0 | 14 KB | UV orientation |
| TextureTransformTest | CC0 | 31 KB (`.gltf`) | `KHR_texture_transform`, its rotation's sign included |
| TextureSettingsTest | CC-BY 4.0 | 43 KB | wrapping modes, double-sided faces |
| NormalTangentTest | CC0 or CC-BY 4.0 (the lists disagree) | 1.8 MB | our derivative tangent frame |
| NormalTangentMirrorTest | CC-BY 4.0 | 1.6 MB | mirrored UVs |
| MetalRoughSpheres | CC-BY 4.0 | 11 MB | metalness and roughness, textured |
| Fox | CC0 or CC-BY 4.0 | 163 KB | a skinned animal with three clips |
| CesiumMan | CC-BY 4.0 | 438 KB | a textured skinned figure |
| WaterBottle | CC0 | 9.0 MB | a PBR object, metal and plastic |
| Lantern | CC0 | 9.6 MB | wood and metal |
| BoomBox | CC0 | 11 MB | emission |
| Corset | CC0 | 13 MB | fabric |
| AntiqueCamera | CC0 | 18 MB | many parts |
| SciFiHelmet | CC0 | 30 MB (`.gltf`) | dense hard-surface detail |
| FlightHelmet | CC0 | 53 MB (`.gltf`, 14 PNGs) | many materials, leather, wood, glass |

**Reference only, never shipped** (the owner, 2026-10-04: "include them, just make sure we
don't use them in the final engine game"). These models' licences restrict use, so they serve
for tests and comparisons only:
- **Khronos's Sponza** (about 20 MB, `.gltf` with JPEG textures): under the CryEngine Limited
  License, not Creative Commons.
- **DamagedHelmet** (3.8 MB): lists CC-BY-NC 4.0 beside CC-BY 4.0.
- **BrainStem** (Poser EULA): a skinned figure with many joints.
- **Duck** (SCEA Shared Source).

How they are kept out of a release:
- **The manifest marks them** `use = "reference-only"`; every other model is
  `use = "shippable"`, under CC0 or CC-BY 4.0.
- **Nothing that ships reads `assets/external/`.** Only the lab's models scene and the tests
  load from there.
- **A check enforces it:** `credits --check`, run by CI, fails if any path outside the lab,
  tools and tests names `assets/external/` or a reference-only model.
- **The release packaging** (when it exists) copies nothing from `assets/external/`.

**Later:**
- **A Beautiful Game** (CC-BY 4.0, 43 MB): its glass needs transmission.

**Later, under their own approval:** heavy scenes to measure performance:
- Intel's new Sponza and the Amazon Lumberyard Bistro (CC-BY 4.0 as published; hundreds of MB
  to several GB; Bistro is FBX, converted in Blender);
- a few rigged and animated characters (Quaternius, CC0) beside the creatures.

**Questions for the owner:**
1. **The set:** all fifteen shippable models and the four reference-only ones (about 185 MB),
   or a smaller start? A smaller start would be the test models, Fox, CesiumMan, WaterBottle,
   FlightHelmet and Sponza (about 100 MB).
2. **The heavy scenes** (Intel's new Sponza, Bistro): a proposal of their own once the first
   set renders?

**The owner's answers (2026-10-04),** the proposal accepted:
- **The restricted models are included,** as reference only and never in the final engine or
  a game (above).
- **External models are for the labs only:** the physics lab's models scene and the tests
  load them; no other demo and nothing that ships.
- **The set:** the smaller start first. All of it, about 185 MB, may be downloaded.
- **The heavy scenes** (Intel's Sponza, Bistro): a proposal of their own once the first set
  renders.
- **The restricted models stay out of git,** as every external model does. The fetch script
  leaves them out unless asked (`--reference-only`), and the lab's docs say how to download
  them for anyone who wants to test with them.

**A first step once answered:** the manifest and the fetch script, the `.gltf` reader, the
models scene, and the test models' captures beside their Khronos screenshots.

**Built, the first step (#170, 2026-10-04):** `docs/demos/physics-lab.md`, "`models`".
- **Fetching:** `assets/external.tsv` and `tools/fetch-assets.sh`.
  - Every file is pinned to the Khronos repository's commit `edc7c9e`, with its size and
    SHA-256, and each model's `LICENSE.md` comes with it.
  - The ten open models are 72 MB; Sponza, reference only, is 53 MB.
- **The guard:** `credits --check` (CI) fails if any source outside the labs, the tools and
  the tests names `assets/external` or a reference model.
- **The importer:** reads `.gltf` files with their buffers and images beside them
  (`forge_geom::model::load_gltf`).
- **The scene:** `physics-lab --lab models`, each model alone with `--model`. Skinned models
  stand in their rest pose. Its captures enter the batch where the models are there.
- **Found and fixed:**
  - NaN at roughness 1: the highlight exponent of 0 and `pow(0, 0)`.
  - `KHR_texture_transform`'s rotation had the wrong sign. The README's matrix as read here was
    wrong; TextureTransformTest settled it.
- **Found and left:**
  - Double-sided materials are drawn one-sided, and alpha cut-outs opaque (both drawn since
    #171).
  - The probes go dark inside Sponza (#171: the lab's fixed exposure, not the probes).
  - The ten models' textures take 534 MiB uncompressed.

## D-049 — Denoising the sun's soft shadows ✅ (proposed and decided 2026-10-04; built in #172 and #173)

**Decided (the owner, 2026-10-04):**
1. **NVIDIA's NRD, its SIGMA denoiser, used as NRD itself, not rewritten.** Our own filter sized
   from the occluder's distance stays unwritten: it would sit close to US 10,740,954 B2.
   - **Its licence** (NVIDIA RTX SDKs License): free and royalty-free.
     - An application may ship it in object code, under terms at least as protective as
       NVIDIA's.
     - It may never be put under an open-source licence.
     - It grants no patent right in so many words. As NVIDIA's own implementation, used as it
       licenses it, it stays clear of rewriting the patented technique. This is not a legal
       reading.
   - **Like the Streamline SDK:** never in git. `tools/fetch-assets.sh` or a sibling fetches it
     into an ignored folder at a pinned release, and Forge loads it when present.
   - **Without it** (CI, a fresh clone): the shadows stay as they are today.
   - **The fallback:** if NRD cannot run on AMD's RDNA 4 or within Forge's rules (its HLSL
     outside `slangc`, its dispatches as graph passes, the FFI's `unsafe` kept in `forge-gpu`),
     the next choice is a port of AMD's FidelityFX shadow denoiser (MIT), whose filter width
     follows the variation, not the distance.
2. **The softer look, for art's sake.** It costs the same: the blur's taps do not grow with the
   penumbra. The sun's apparent size for the shadows becomes a setting, with a softer default
   than the real 0.27°, to judge on Sponza.
3. **NRD may be downloaded:** to build with, and to compare against a 256-ray reference.
4. **AMD's FidelityFX shadow denoiser as the built-in fallback** (the owner, 2026-10-04, #173):
   where NRD is absent (a fresh clone, CI, a build that does not ship it), a Slang port of the
   MIT denoiser takes its place, behind the same trace pass and the same hook in the resolve.
   NRD stays the first choice when present. Built below.

**Built (#172, 2026-10-04):**
- **The library.** `tools/fetch-nrd.sh` clones NRD v4.17.3 at its pinned commit and builds
  `NRD.dll` into the git-ignored `nrd-sdk/bin` (Visual Studio's C++ tools, the Vulkan SDK's
  DXC, SPIR-V only). `forge_gpu::nrd` loads it at run time through a hand-written FFI whose
  structures were checked against MSVC's layout, behind the `nrd` cargo feature (on in the
  demos). `FORGE_NRD_DIR` points elsewhere; an empty folder runs without it.
  - **Since #210 (2026-10-09): NRD 4.18.0, `master` at d3df343**, not yet a tagged release.
    4.17.3's `SIGMA_ClassifyTiles` raced on its groupshared tile counters (`GroupMemoryBarrier`
    without a group sync, 7 206 reports from GPU-assisted validation over 1 200 frames of the
    yard); `master` synchronises them. The settings gained `motionVectorBias`,
    `outputRectOrigin` and SIGMA's `checkerboardMode` (left off); quad intrinsics are built off
    (they need `VK_KHR_compute_shader_derivatives`). `fetch-nrd.sh` patches one macro of
    `master`'s that DXC rejects without material IDs (RELAX, unused).
- **The passes.** `shadow/trace` (`sun_shadow_trace_main`, `shaders/meshlet.slang`) traces one
  ray a pixel to a point of the sun's disc, keeps the closest hit and writes NRD's inputs: the
  penumbra's radius, the normal and roughness, the view depth. Each of NRD's dispatches is a
  graph pass (`shadow/SIGMA …`), its images bound with push descriptors. The resolve multiplies
  in the denoised visibility instead of its own ray; without NRD it keeps the ray, pixel for
  pixel as before.
- **NRD's kernels assume D3D12.** Several (its clears, SIGMA's history copy, the tile smoothing)
  run their last workgroup past the image's edge unchecked: D3D12 drops those writes, Vulkan
  leaves them undefined. On the 5070 Ti the clears' stray writes landed in the shadow ray's
  inputs, and `FORGE_ASYNC=0` differed from the async frame by 80 000 pixels. NRD's pipelines
  alone are created robust (`VK_EXT_pipeline_robustness`, robust image access), and its clears
  became `vkCmdClearColorImage`. Forge's own pipelines keep the device's default.
- **The look.** The sun's disc for the denoised shadows is 1° in radius
  (`DENOISED_SUN_RADIUS`), against the real 0.27°: `--sun-size DEG` changes it,
  `--no-shadow-denoiser` and F6 turn SIGMA off, `--shadow-reference` traces 256 rays a pixel.
- **On Sponza** (the #172 view at noon, 1600 × 900, frames 600–615):
  - The curtains' penumbra, the per-pixel range over TAA's 16-frame cycle: 5.0 codes (99th
    percentile 78) with the one ray and TAA, 1.5 (14) with SIGMA, 1.2 (8) for the 256-ray
    reference. On the floor SIGMA's 0.68 equals the reference's.
  - Frames 600 and 616 differ in 0.019 % of their pixels.
  - In motion (the owner, walking forward or along the path) the curtains' edge still
    shimmers. The 256-ray reference does not, but it is far too slow to play. Measured in
    motion against AMD's denoiser in #173.
- **Cost at 1440p** on the 5070 Ti (`docs/PROFILE.md`, the physics lab):
  - Sponza: `shadow/trace` 0.44 ms, SIGMA's passes 0.44 ms; the frame 3.48 → 3.97 ms.
  - The city's south view: 4.03 → 4.72 ms. The island: 3.69 → 3.95 ms.
  - The proposal's estimate was 0.35 to 0.6 ms; Sponza and the city cost more.

**Built (#173, 2026-10-04): AMD's FidelityFX shadow denoiser, the fallback.**
- **The port.** `shaders/ffx_shadows.slang` is AMD's `ffx-shadows-dnsr` (MIT, commit d7dfecb)
  in Slang, driven by `forge_render::ffx_shadows`:
  - `shadow/FFX pack`: the rays' results, a bit a pixel, a word per 8 × 4 tile;
  - `shadow/FFX classify`: tiles whose surroundings are all lit or all shadowed are skipped;
    elsewhere the history is reprojected, clamped to a 17 × 17 neighbourhood, and blended;
  - `shadow/FFX filter 1`, `2`, `3`: edge-avoiding à-trous passes, steps 1, 2 and 4.
- **Forge's rules.** No wave or quad operation (a groupshared flag, plain reads of the 2 × 2
  quad), every image access kept inside the image (NRD's lesson above), 32-bit groupshared
  values, and the output in SIGMA's form, so the resolve reads either alike.
- **One interface.** `SunShadowDenoiser` holds SIGMA or FFX behind the same trace pass and the
  same hook in the resolve. FFX needs no distance, so its trace stops at the first hit.
  `--shadow-denoiser sigma|ffx`; F6 steps through SIGMA, FFX and none. Without NRD, FFX is the
  default.
- **On Sponza** (the first curtain view at noon, 1 m probes since #175), the curtains' penumbra
  over TAA's cycle, mean and 99th percentile of the per-pixel range:
  - reference 1.3 / 8, SIGMA 1.4 / 9, FFX 1.8 / 20;
  - moving forward (`--dolly 0.3`), pixels a frame whose change differs from the reference's:
    SIGMA 7 700, FFX 10 800.
  - The owner: SIGMA looks right; FFX still shimmers, none is far worse.
- **FFX's limit.** Its filters reach about 8 pixels; the 1° sun's penumbrae from distant
  occluders are wider. There it leaves blocky steps along the edge and shimmers in motion.
  This holds wherever such penumbrae fall, not only in Sponza. To improve later.
- **Cost at 1440p** (zones; FFX passes / its first-hit trace): Sponza 0.39 / 0.30 ms, the city
  0.40 / 0.21, the island 0.18 / 0.14 (`docs/PROFILE.md`).

The proposal as written follows. Its trace pass, guide images and checks still hold with NRD
in place of the in-house passes.

On Sponza at noon (#172), the shadow of a ledge across the courtyard falls on the curtains with
a hard, stepped edge that crawls in motion.
- **The cause.** One ray a pixel and frame aims at one of 8 points of the sun's disc, and TAA is
  left to average them (#54).
- **Why it shows here.** The exposure is metered for the arcade, 8 stops under the sun.
  - One point in eight is already white.
  - TAA averages Karis-compressed colour, not visibility. Its result is biased dark and ripples
    with the 8-point cycle at the penumbra's lit rim.

The research is `docs/research/shadow-denoising.md`. The owner chose a denoiser over more rays
(2026-10-04).

**Proposed:**
1. **A sun-shadow denoiser of Forge's own, in Slang, with the structure of NVIDIA's SIGMA,**
   written from the published techniques: SVGF, Boksansky et al. (Ray Tracing Gems ch. 13),
   Heitz et al.'s ratio estimator, PCSS's penumbra width, and NRD's documented interface. It is
   not translated from NRD's code, whose licence (NVIDIA RTX SDKs License, not MIT) forbids
   that.
2. **`shadow/trace`:** a compute pass before the resolve.
   - It traces the closest hit, from more disc points: about 128, stratified over a 4×4 pixel
     block and 8 frames, still repeating with the jitter.
   - It writes the visibility and the penumbra's radius (hit distance × tan θ) in 16-bit floats,
     never 8-bit.
3. **`shadow/classify`:** 16×16 tiles skip what is fully lit, fully shadowed or hard. The ballad's
   hard shadows keep their pixels.
4. **`shadow/blur`, then `shadow/post-blur`:**
   - The penumbra is estimated from the 5×5 neighbours' distances; lit pixels borrow them.
   - The radius in pixels is 1 to 32.
   - 12 to 16 rotated taps, weighted by plane and normal.
5. **`shadow/temporal`:** up to 16 frames of history, clamped to the blurred neighbourhood's mean
   ± 1 to 1.5σ.
6. **The resolve's sun term is unchanged.** It multiplies in the denoised visibility instead of
   its own ray.
7. **Cost:** about 0.35 to 0.6 ms at 1440p on the 5070 Ti, to measure (SIGMA: 0.40 ms on an RTX
   4080).
8. **Cross-vendor and deterministic:** no wave intrinsics, about 2 KB of groupshared memory,
   compute passes only, the mesh path and the fallback 0 px apart.

**For the owner to decide:**
1. **The filter's width.**
   - **From the blocker's distance:** what was sketched, and the better result.
   - **The patent:** NVIDIA's US 10,740,954 B2 (active, to 2039) claims a shadow filter whose
     footprint comes from the occluder's distance. This is not a legal reading.
   - **The alternative:** a width from the variation over frames, as AMD's MIT-licensed FidelityFX
     denoiser has it. It needs no distance and is weaker on wide penumbrae (about 15 px at most).
2. **The look.** At 8 stops under the sun, a correct penumbra is narrow: only its darkest eighth
   is not white, about 3 of Sponza's 16 pixels. Is that the look wanted, or a softer one (a larger
   disc for art's sake)?
3. **A yardstick.** May NRD be downloaded to compare with, never shipped?

**How it would be checked:**
- A 256-ray reference.
- On the Sponza view, the per-pixel range over TAA's 16-frame cycle under 2 codes.
- The slow change between frames 600 and 616 back to the hard shadows' 0.035 %.
- ꟻLIP against the reference and the other scenes.
- The motion check with `imgdiff --then`.

## D-050 — Steadying the glass's mirror reflections ✅ (proposed and decided 2026-10-04)

The owner, 2026-10-04: "in the city, towers with reflection were shimmering" (#176).

**Decided (the owner, 2026-10-04: "I will follow your recommendations"):**
1. **(a) A sharp mirror, as today,** steadied by a history of Forge's own for the mirror rays
   (proposed item 1 below). No third-party code.

2. **The water after the glass** (asked again the same day, with "I'd do them after": "yes").
   The sea's and the rivers' mirror rays get the same history in a later change.
3. **The water left as it is** (the owner, 2026-10-07, "yes, record it", once measured): its
   mirror rays add no measurable shimmer. On the island's lake shot with the waves held still
   (`--sea-time 10`, frames 200–214, the lake's pixels moving by 8 codes or more): 1.22 % with
   them and 1.12 % without under TAA, 0.001 % and 0 % under DLAA. On the mouth's glare at dawn
   the sea's are no different (#174). Item 2 is dropped; a history for the water would come
   back only with a view where its reflections shimmer.

**Measured** (the city's default view, 1600 × 900; the share of the glass's pixels that move by
8 codes or more):
- **Held still,** over TAA's cycle (frames 200–214): 9.1 % and 11.2 % on the left and right
  towers' glass. Without the mirror rays (`--no-ray-reflections`): 2.4 % and 4.5 %. The frame as a
  whole: 3.0 %.
- **Dollying in at 3 m/s,** each frame step compared with the same run supersampled 2 × 2
  (`imgdiff --then`): 3.2 % and 2.7 % of the glass changes unlike the supersampled run; 1.1 % and
  0.7 % without the mirror rays.
- **Not the cause:** the sun-shadow denoiser (identical without it), the probes and the bloom.

**The cause.** Every row at Blinn-Phong power 60 and above traces one exact mirror ray a pixel
(#50, #52). The towers' glass is power 300 (GGX roughness 0.285, α ≈ 0.08), yet its reflection
keeps every window bay of the towers across the street: a few pixels each, met by a ray that
TAA's jitter moves every frame. TAA cannot settle it:
- **Still,** its clip (the 3 × 3 neighbourhood's mean ± 1.25 σ) keeps most of the jitter's
  variation where the reflected grid makes the variance high.
- **In motion,** it follows the glass's motion, not the reflected image's, so the history falls
  outside the clip and the raw frame shows.

**What the libraries offer** (their sources read: NRD v4.17.3 in `nrd-sdk/src`, AMD's
FidelityFX-Denoiser at d7dfecb):
- **The reprojection a mirror needs is published and the same in both:** a *virtual* point on the
  pixel's view ray, as far from the camera as the surface plus the ray's length to its hit, moved
  with the camera (NRD's README, "Primary Surface Replacement"; AMD's
  `GetHitPositionReprojection`). On a flat pane it is exact. Not searched for patents.
- **AMD's reflection denoiser (MIT) skips mirrors:** it denoises only rows between its glossy and
  mirror thresholds (`prefilter.h`: `IsGlossyReflection && !IsMirrorReflection`). It treats a
  mirror as noise-free, and here the mirror's trouble is aliasing, not noise.
- **NRD's REBLUR_SPECULAR handles mirrors** through that virtual point. NVIDIA's own figure for
  REBLUR's diffuse and specular together is 2.50 ms at 1440p on an RTX 4080; specular alone,
  perhaps half. Only where NRD is installed, so it would need a fallback anyway.

**Proposed:**
1. **A history for the mirror rays alone, of Forge's own** (`shading/reflection history`, after
   `shading/reflections`):
   - `reflections_main` writes what it adds (the hit's light minus the sky it replaces, weighted)
     and the ray's length to two images, instead of adding into the colour.
   - The new pass reprojects the previous frame's reflection to the virtual point, rejects the
     history where the glass's own depth or normal differ (a disocclusion), clips it to the
     current reflection's 3 × 3 neighbourhood, blends about a tenth of the new frame in, and adds
     the result into the colour. TAA then meets a reflection that no longer moves with its jitter.
   - Two rgba16f images and a ray-length image, a compute pass over the reflection tiles: about
     0.05–0.15 ms at 1600 × 900, to measure.
   - It keeps the mirror as sharp as it is.
2. **No third-party code.** FidelityFX would not touch a mirror; NRD's specular, by NVIDIA's
   figure, costs about ten times the history's estimate, and needs a fallback anyway.

**For the owner to decide:**
1. **The look of the glass.**
   - **(a) A sharp mirror, as today**, steadied by the history (proposed). Finest in the far
     reflection; a mirror's detail can still crawl a little where the history is rejected (the
     glass's edges, fast turns).
   - **(b) Glossy, as the row says:** each pixel's ray aims inside the GGX lobe of roughness
     0.285, and a denoiser averages them; AMD's reflection denoiser (MIT) then fits as it was
     made. The window grids blur away at a distance, and with them the aliasing; near
     reflections soften too. A larger port (its four headers, about 850 lines); its cost is
     unmeasured, likely several times the history's.
   - **(c) Both:** a smoother row for a sharp mirror where wanted (power 2000 and above), the
     lobe below it.
2. **The water.** The sea's and the rivers' mirror rays (`water/reflections`, #105) have the same
   pattern on their own pass. Include them now or after the glass?

**How it would be checked:**
- The still view: the glass at 8 codes or more back to about the `--no-ray-reflections` numbers
  (2.4 % and 4.5 %).
- The dolly against the supersampled run: the glass's 3.2 % and 2.7 % down to about 1 %.
- The mesh path and the fallback 0 px apart, `FORGE_ASYNC=0` the same, and the cost in
  `docs/PROFILE.md`.

**Built (#176, 2026-10-07):**
- **Where.** `steadied_reflection` and `carry_reflection` in `shaders/meshlet.slang`, inside
  `reflections_main`: no pass of its own beside the rays. `forge_render::ReflectionHistory` holds
  the two rgba16f images (what the rays add before the exposure, and the view depth of the
  virtual point), written and read in turn, and declares `shading/reflection history`, which
  clears the one about to be written. `--no-reflection-history` turns it off.
- **Departure from item 1: no clip to the neighbourhood.** The history is kept or dropped by the
  virtual point's depth instead: each of the four texels around the reprojected point counts
  while its own virtual point would land within about a pixel of it. A clip to the current 3 × 3
  would have kept the jitter's variation it is there to remove; the glass's window grids make
  that neighbourhood's variance large. What the depth test cannot see, a reflected object whose
  light changes in place, lags about ten frames. Movers seen in the glass, and glass on movers,
  keep each frame's ray.
- **Blend:** a tenth. A twentieth or a thirtieth measured the same.
- **Measured** (`docs/demos/city-blocks.md`, "The city in the glass"): held still, the glass at
  8 codes or more 9.1 % / 11.2 % → 3.2 % / 6.2 % under TAA; under DLAA 0 % either way. Dollying,
  against a supersampled run: TAA 3.2 % / 2.7 % → 1.9 % / 1.4 %, DLAA 0.95 % / 0.31 % → 0.45 % /
  0.13 %. Interactive runs use DLAA, scripted ones TAA; the shimmer the owner reported was seen
  live.
- **Cost:** 0.023 ms beside the rays and 0.003 ms of clear at 1600 × 900 (`docs/PROFILE.md`).

## D-051 — Jelly: a body seen through by rays ✅ (2026-10-08, the owner's ask on #180)

The owner asked for the slime "more jelly-like, translucent with a face", and then for the
tropical island's slime itself, with its eyes: a squat drop of mint jelly with a darker nucleus inside and two tall glossy eyes,
drawn in its forward pass by bending the scene behind it by the surface's normal
(`tropical-island/crates/ti-engine/src/shaders/slime.wgsl`). Forge had no see-through material:
ice lets the sun through its thickness (D-033), the water refracts a copy of the scene in a
forward pass (D-038).

- **A fourth shading class, `Jelly`** (`ShadingClass::Jelly`, `shading/jelly`, `jelly_through`
  and `shade_jelly` in `meshlet.slang`). The jelly stays in the visibility buffer like any
  surface; its class traces what lies behind it. At a pixel the view ray bends into the body
  (index 1.12) and crosses it: every crossing of the pixel's own instance, none committed, in
  both structures (the slime is a mover), the nearest past the start being where it leaves, as
  the ice finds its crossings. From there it goes on straight and meets what lies behind,
  shaded plainly as a mirror ray's hit. Over the path the jelly absorbs (`color_a`: what 0.3 m
  of it lets through) and its cloud scatters (`bubbles`, as the ice's), in its own colour
  (`color_b`) lit as a surface. The sun glows through it by the ice's rule. A crossing of
  another row of its own (the nucleus, an eye from behind) shows that row.
- **Its rays skip itself where it would wrongly hide light.** The hits behind it reach the sun
  through it (it lets most light by), and its mirror ray starts at its surface and skips its
  body. The mirror rays' pass starts theirs 1.5 m out for the city's cuts; for a small body
  lying on the ground that start is under the ground.
- **One bend, at the front.** Bent again at the far side, the ray followed that side's wobbles
  and the cut's facets (it has no smooth normals) and broke the horizon behind into hard-edged
  pieces. At gelatine's index (1.35) a ball is a lens showing what lies behind upside down, a
  glass marble. The island's slimes bend it gently by their front alone, and so does this.
- **Without ray queries or a sky** the jelly shows its cloud's colour, opaque.
- **Its shadow is tinted light, not a solid body's** (the owner: "make the shadow lighter, like
  translucent jelly"). In a scene with jelly rows (`FLAG_JELLY`) every shadow ray sees through
  jelly: the resolve's, SIGMA's and FFX's traces, and the hits' (`shadow_candidate_kept`). What
  else of the body stands in the way still blocks: the nucleus casts a dark core. The sun's light, now a colour, is tinted by one more ray to a point of the
  sun's disc (`jelly_transmittance`): Beer–Lambert, as the eye's ray through the jelly, over the
  length the ray runs inside jelly bodies. That length is the sum of the crossings, none
  committed, each distance counted negative into a body and positive out of it. The point of
  the disc turns with TAA's cycle, which softens the tint's edge into a penumbra, as the
  shadows were before SIGMA (#54).

*Not chosen:* a forward pass after the resolve reading a copy of the scene, as the water and
the island's slimes do. It needs the body drawn outside the visibility buffer (a forward draw of
skinned clusters, a new path), and it sees only what the screen shows. For the shadow, SIGMA's
translucency variant (`SIGMA_SHADOW_TRANSLUCENCY`) denoises a tint with the visibility. It needs
another input image and an RGBA output through `forge_gpu::nrd`, and FFX and the undenoised
path would still need their own tint; the tint is smooth across the body, so one ray and TAA
are enough.

**Left for later:** the hits seen through it are plain (their rows' colours, the textures'
averages), and a hit seen through a jelly is lit through other jellies untinted (#182); no caustic
focuses the light under the body (#181).

*Measured* (`--lab creatures`, RTX 5070 Ti, 1600 × 900, 600 frames): `shading/jelly`
0.043 ms in the lab's view, 0.066 ms with the slime filling a sixth of the screen. The tinted
shadow, four slimes: the frame 1.59 → 1.66 ms, `shading/standard` 0.194 → 0.234 ms.

## D-052 — Joints that twist without thinning ✅ (proposed and accepted 2026-10-09, #169's item 2)

**The problem.** The skin pass (`shaders/skin.slang`, #165) blends a vertex's four joints'
matrices linearly. Where a limb turns about its own axis, the blended matrix shrinks, and the
limb thins to a twisted neck: the "candy wrapper". The creatures' rigs
(`assets/blender/skinned_creatures.py`) have eleven bones each and no helper bones. Their
powered ragdolls twist their forearms and shins within the constraints' twist limits, most in a
fall.

**The options:**
1. **Dual quaternion skinning (DQS), an option per mesh.**
   - How: the skin pass blends the joints as unit dual quaternions instead of matrices (Kavan
     et al., 2007). The skinned mesh's record flags it. The joints' rings carry the dual
     quaternions written on the CPU, so the shader blends four and normalises. A quaternion
     against the first joint's is flipped to the same hemisphere.
   - For: Forge's joints are rigid (ragdoll bodies, no scale or shear), so the plain form is
     enough. No rig changes, and it works for any glTF rig. The output (positions and normals)
     is what the BLAS refit and the motion vectors read now, so nothing after it changes.
   - Against: DQS swells a joint that bends (the elbow, the knee, the shoulder raised). Unity
     and Unreal ship linear blending only. DQS comes to them through third-party tools, which
     blend it with linear skinning by a mask per vertex. Disney's production DQS (*Frozen*)
     needed the same blend.
2. **Twist bones in Forge's own rigs.**
   - How: one helper bone per long limb segment (upper arm, forearm, thigh, shin), weighted
     along the segment by Blender's bone heat. At run time it takes half of the segment's
     twist: the twist part of a swing-twist split of the child's rotation against the parent's.
   - For: the industry's usual way, which stays linear and works in any engine. One twist bone
     a limb is usually enough for a game character seen in third person.
   - Against: rig work and a runtime step for each creature, more joints a frame, and every
     imported rig needs its own.
3. **Corrective shapes:** blend shapes authored per pose. They wait on morph targets
   (#169's item 3), and each pose takes authoring.

**Proposed:** 1 first, as an option per mesh, measured against linear blending on the mannequin:
- its forearm turned 90°: the forearm's thickness at its middle, as a share of the rest pose's;
- its elbow bent 90°: how far the elbow's outer surface swells past the rest pose's.

It costs one path in the skin pass and no rig work. If the swelling at the bends shows, add 2
to the limbs of Forge's own creatures. Leave 3 until morph targets exist.

**Decided (the owner, 2026-10-09):** "go with dual quaternion skinning, on request only".
Linear blending stays the default, and a mesh asks for dual quaternions.

**Built (#169):**
- `forge_render::SkinBlend` is the choice per mesh, set with
  `MeshletSceneBuilder::set_skin_blend`, and the skin pass's clusters carry it as a flag.
- `skin.slang`'s `dual_blend` turns each joint's three rows into a unit dual quaternion
  (Shepperd's method for the rotation). It takes the shorter way round from the first joint's
  rotation, normalises the blend, and blends a uniform scale apart. The frame before's
  positions, which the motion vectors read, use the same blend.
- `forge_render::skin_vertex` is the pass's twin on the CPU.
- `physics-lab --dual-quaternion` skins the creatures and the gulls by dual quaternions. The
  slimes' joints are their points' translations, where the two blends agree, so they stay
  linear.
- `--arm-pose twist|bend` turns the mannequins' left forearm, for comparing the two.

**Measured:**
- The mannequin's arm, through the CPU twin (`dual_quaternions_keep_a_twisted_arm_s_thickness`).
  Per slice of the arm, the vertices' mean distance from the bones as a share of the rest
  pose's, the least and the most:

  | Pose | Linear | Dual quaternions |
  |---|---|---|
  | forearm turned 90° | 0.753–1.000 | 1.000–1.001 |
  | elbow bent 90° | 0.825–1.000 | 0.955–1.006 |
  | shoulder turned 0.8 rad (the ragdoll's limit) | 0.964–1.000 | 1.000–1.000 |

  - Every vertex stays inside the culling sphere the clusters share, under either blend.
  - The bent elbow's swelling stays under 1 %.
- **The cost:** `skin/vertices` is 0.008 ms either way in `--lab creatures`, and the frame 1.72
  ms (RTX 5070 Ti, 1600 × 900, 600 frames, two runs each).

**Option 3 since (#169's item 3):** morph targets exist. `physics-lab --elbow-correctives` gives
the mannequins a corrective per elbow, weighted by its bend, made from the dual quaternions'
bend. With it, the linear blend keeps 0.955 of the bent elbow (0.825 without), as the dual
quaternions do. Both stay on request.

*Sources:* Kavan, Collins, Žára and O'Sullivan, "Skinning with Dual Quaternions" (I3D 2007,
<https://users.cs.utah.edu/~ladislav/dq/>); Disney Animation, "Enhanced Dual Quaternion
Skinning for Production Use"
(<https://disneyanimation.com/publications/enhanced-dual-quaternion-skinning-for-production-use>);
riggers' practice on twist bones and DQS (<https://polycount.com/discussion/comment/1354571>,
<https://www.tech-artists.org/t/dual-quaternion-skinning/2112>).

---

## D-053 — Generated content as data: world files, an engine crate that makes them, one derived-data cache, packages later ✅ (proposed and accepted 2026-10-09, the owner's question on #208)

**The owner's question:** "maybe it's the occasion to make the engine a bit more data driven?
why is island in city-blocks for example? [...] Maybe packages in a dedicated directory like
game engines do? but not zipped for now?"

**Where things are today:**
- **The island's making** lives in `demos/city-blocks/src/lib.rs` (7 568 lines), beside the
  city, the ballad's scene glue and the labs. It grew there: the island started as
  `city-blocks --island 7`. The `island` and `physics-lab` binaries are thin `main`s over the
  `city-blocks` library (`city_blocks::main_lab`).
- **Its settings** are command-line flags and constants in that file (`RIBBON_PARAMS`,
  `SILLS`, `SAND_BELOW`, `DETAIL_FADE`, the rules' defaults). The cache keys built from them
  miss some: the cooked tiles' key leaves out `--no-sills`.
- **What it generates** is memoized per process, except the eroded heights and the cooked
  tiles in `mesh-cache/`, keyed without the code that made them (`--recook` by hand). The
  SPIR-V is in `shader-cache/` (#209).
- **The engine crates** that exist for it: `forge-procgen` (the algorithms: erosion, rivers,
  channels, layers…), `forge-geom` (meshes, cooking), `forge-world` (frames, cells, streaming).
  None holds "an island": the glue that runs the algorithms in order is the demo's.

**How engines split it:** three kinds of data, kept apart.
- **Sources,** authored or described: Unreal's `Content/`, Unity's `Assets/`, Godot's
  `res://`.
- **Derived data,** made from the sources on each machine, never versioned, always
  regenerable: Unreal's Derived Data Cache, Unity's `Library/`, Godot's `.godot/imported/`.
  Each entry is keyed by its inputs and by the version of the code that makes it.
- **Packages,** what a game ships: Unreal's `.pak` and IoStore containers, Unity's asset
  bundles, Godot's `.pck`. A cook step fills them from the sources through the derived data;
  in development the editor reads loose files.

**Proposed:**
1. **Sources: `assets/` as now, plus world descriptions.** `assets/worlds/island.toml` would
   hold the island's parameters: seed, size, spacing, erosion, rivers, valleys, layers and
   their rules, the flags' defaults. A flag overrides one field for an experiment and says so
   in the log. TOML needs the `toml` crate, a new dependency to download (the owner's OK
   first); JSON works with what Forge already has but is harder to edit by hand.
2. **Derived data: one cache, `cache/`.** #208's get-or-make cache, keyed by the inputs, the
   digests of the products it is made from, a digest of its source files and its format.
   `cache/shaders/`, `cache/meshes/` and `cache/world/` would replace `shader-cache/` and
   `mesh-cache/`. Never in git, never shipped.
3. **The makers in an engine crate.** A new `forge-terrain`: a world description in, the
   products out (heights, water, layers, drawn ground, stones, ground tiles). CPU only,
   deterministic, over `forge-procgen`, `forge-geom` and the cache. The upload to the GPU and
   the materials stay in `forge-render` and the demo. `city-blocks` keeps the city; the labs
   move to `physics-lab`'s own crate; the `island` demo takes a world file.
4. **Packages, later, unzipped.** `packages/<name>/`: a `manifest.toml` (each entry's id,
   type, file, hash, format, dependencies) and the cooked files beside it. A cook tool makes
   one from a world description by running the makers through the cache. The runtime opens
   a package by its manifest. Zipping, or one container file, is the same manifest with a
   different file system under it, to decide at a release. A package earns its place when
   something must load a world without making it: a release, or the server of Phase 5. Until
   then the cache does the job.

**The order, an issue each:**
1. #208 builds the cache as an engine module, keyed as above. Its root stays `mesh-cache/`
   until this is decided.
2. The island's description in a file; the flags become overrides.
3. `forge-terrain`, with the island's makers moved out of `city-blocks`; the labs out too.
4. One `cache/` root for the shaders, meshes and world products.
5. Packages and the cook tool, when a release or the server needs them.

**For:**
- A world is changed by editing a file.
- Every setting reaches the cache keys.
- The engine owns how a world is made, and the demos only show it.
- The steps are small, and none blocks the current work.

**Against:**
- Moving the island's glue touches most of `city-blocks/src/lib.rs`: a large diff with no
  visible change, checked by the island's captures staying identical.
- A world file is one more format to keep compatible.

**Decided (the owner, 2026-10-09):** "go with D-053, use TOML, file the issues". The steps:
#208 (the cache), #211 (the world file), #212 (`forge-terrain`; the labs out of
`city-blocks` split off to #216, the owner, 2026-10-09: they need a scene interface in the
shared app first), #213 (one `cache/` root), #214 (packages, milestone "Later").

**Done (2026-10-09):** #208, #211, #212, #213 and #216. The labs run in the shared app through
its scene interface (`city_blocks::scenario`, #216); the shared app itself stays in
`city-blocks`, where it was, beside the city and the island: moving it to a crate of its own (or
into `forge-app`) is a decision for when another demo than the lab needs it.

**Larger worlds (the owner, 2026-10-09):**
- **Generation** may keep whole products, made complete through the cache: a world, a
  planet, a place or an entity. It produces the packages and assets a game ships.
- **Those packages** are cut along `forge-world`'s cells and streamed. A package's entries
  are chunks a viewer can load alone, not whole worlds.
- **The engine** can still generate on the fly where a game needs it: the same makers, run
  for a chunk at run time instead of loading it.

ROADMAP's Phase 9 holds the rest for later.

## D-054 — The caches packed with LZ4 ✅ (the owner's choice on #215, 2026-10-09)

**The owner's question:** "What do games use for efficient compressions?" The answer was posted
on #215:
- Games compress in two layers: a codec that knows the data, then a general compressor.
- Oodle is the industry's (closed), zstd the open one closest to it. LZ4 is the fastest to
  decompress.
- Floats, meshes and textures get their own codecs in front of the compressor.

**Decided (the owner):** "since it's more important to have shorter loading times and better
runtime performances I would go for lz4_flex instead, reading is more important than writing
(except for saving the game state but we are not there yet)". So the compressor is
`lz4_flex` (pure Rust, MIT), over zstd, which was already on the machine.

**Built (#215's step 1):** `forge_core::pack`.
- **The world's products** (`forge_core::derived`, format 2):
  - The payload is packed in frames of 1 MiB, unpacked in parallel. The checksum covers the
    packed bytes.
  - `f32` grids (`Stored::put_grid`: the heights, the drawn ground, the detail cells) are
    coded losslessly first:
    - each sample's bits are ordered as the numbers;
    - each sample is predicted from its left, upper and upper-left neighbours (`a + b − c`);
    - the zigzagged differences go on four byte planes, in bands that decode on their own.
- **The cooked meshes** (`forge_geom::cache`, format 4): each 128 KiB page is packed on its
  own, as LZ4, as LZ4 over its 16-byte records' planes, or as is, whichever is smallest. A
  table gives each page's length and codec, so the streamer still reads one page alone.
- **Lossless:** every product and page comes back to the bit, and every capture is unchanged.

| On disk | Before | Packed |
|---|---|---|
| The island's products (heights, water, layers, drawn ground, stones) | 455 MB | 113 MB (25 %) |
| — the drawn ground (8193² heights and the detail cells) | 314 MB | 100 MB |
| — the water | 103 MB | 6.3 MB |
| The island's 2 m tiles (64) | 4.61 GB | 2.65 GB (57 %) |
| The island's 8 m tiles (64) | 583 MB | 358 MB (61 %) |
| The other props (138: the city, the rocks, the lab's) | 1.24 GB | 723 MB (59 %) |
| The glTF models (45) | 44 MB | 30 MB (68 %) |

| Island start (`island --frames 5`, medians of 5, alternated with the build before) | Before | Packed |
|---|---|---|
| Warm: prepared | 597 ms | 599 ms |
| Warm: total | 2 577 ms | 2 522 ms |
| Warm: the drawn ground loaded | 115 ms | 95 ms |
| Warm: the rays' cuts (`blas_ms`) | 321 ms | 277 ms |
| Cold: the products read from the NVMe drive, no OS cache | 118–160 ms | 31–36 ms |

- **The warm start stays as fast.** Three changes keep it there, where the first version
  was 80 ms slower:
  - The drawn ground's heights move into an `Arc<Vec<f32>>` instead of being copied (268 MB).
  - Small batches of pages unpack on the caller's thread: a thread costs more to start than a
    page to unpack.
  - The rays' cuts read their meshes' pages side by side.
- **The products step** (prepared) is within its noise. The sand window's cook beside it
  (`SandWindow::cooked_mesh`, not cached) shares the CPU with the decoding.
- **A cold start** reads a quarter of the products' bytes. A drive slower than this NVMe
  (3–4 GB/s) gains more.

**Left (#215's next steps):**
- No 2 m mesh of the whole island: D-055.
- Content-addressed chunks with the packages (#214).
- A tighter compressor for shipped packages, if a download's size ever matters more than its
  load. The frames and pages carry their codec, so another can join without a new format.

## D-055 — The island's fine ground without a 2 m mesh of the whole island ✅ (proposed and decided 2026-10-09, #215's step 2)

**Today:**
- The ground is drawn from its 2 m field (8193² samples, #106). It's cooked into 64 tiles of
  cluster DAGs (`island@x-z-2m`).
- Every tile is cooked whole and kept: 4.6 GB of pages raw, 2.65 GB packed since #215 (D-054).
  That is more than half the cache, for a 16 km island.
- The 8 m tiles (`island@x-z`) are kept too. Only the start view's pages, and the pages the
  camera then needs, are read: the streamer already reads little of it.
- The disk cost grows with the area: a world four times the island's would need over 10 GB of 2 m tiles.

**How engines draw large ground:**
- **From the height field, on the GPU.** This is most engines' terrain.
  - Geometry clipmaps (Losasso and Hoppe, SIGGRAPH 2004) and CDLOD (Strugar, 2009): nested
    grids around the camera, displaced by the heights in the shader.
  - Unreal's landscape and Far Cry 5's terrain (GDC 2018) work the same way: a quadtree of
    patches over a height map.
  - Disk holds only the heights (16-bit in Unreal), and the mesh is never stored.
- **Cooked meshes of the ground, streamed near the camera.** Unreal's Nanite landscape
  (5.3 and later) makes Nanite meshes from the landscape and keeps them in the derived-data
  cache. Far tiles keep their coarse levels.

**Options for Forge:**
1. **Cook the 2 m tiles on demand, near the camera.**
   - The 8 m tiles stay everywhere. A 2 m tile is cooked on the job system when the camera
     comes within a ring of tiles (3×3 or 5×5), into a cache capped by recent use.
   - It is the same path as today: meshlets, rays, shadows and the start view's cut.
   - A tile is about 4 s of cooking work, a third of a second on all cores, and 2 km wide: walking takes
     minutes to cross one, and a flight at 300 m/s seven seconds.
   - Against: a tile cooking as the camera arrives costs CPU at run time. Until it is ready,
     the 8 m tile is drawn, a visible step that must blend (the owner sees LOD pops).
2. **Draw the near ground from the height field** (a clipmap or CDLOD).
   - Disk holds only the heights: 71 MB packed for the 2 m field.
   - Against: a second geometry path beside the meshlets. The culling, the visibility buffer,
     the rays' BLAS, the shadows and the probes would each need it too. The river channels'
     cells drawn at a metre (#105) don't fit a regular grid. Weeks of work, and two ways to
     draw the ground.
3. **Keep the cooked tiles, but store only their coarse levels for far tiles.**
   - Against: any tile is near the camera at some point, so this comes back to cooking on
     demand (option 1) with a smaller first cook.
4. **Store the clusters' vertices smaller** (D-025's "compressed vertices", left for later).
   - Positions quantised to a grid the whole mesh shares (as Nanite does, so neighbouring
     clusters stay crack-free) fit in 6 bytes relative to their cluster instead of 12; a ground
     tile's lie on a regular grid in x and z, which needs fewer still.
   - Pages would hold about twice the triangles: the disk, the streamed bandwidth and the GPU
     pool would all halve, for every mesh, not only the ground.
   - Against: the shaders decode the vertices (a few ALU), and the cooking and the page
     format change. Positions move by under a millimetre, so the captures change a little and
     are passed by their ꟻLIP numbers.

**Proposed:** option 4 first, then option 1 when a world larger than the island needs it.
- Option 4 halves everything, every mesh, on the one path, and helps the frame as well as the
  disk.
- Option 1 bounds the disk for any size of world, but trades it for run-time cooking and a
  blend to make invisible. It is worth it with the packages (#214) and larger worlds (D-053's
  "Larger worlds").
- Option 2 is not proposed: a second terrain path costs more than the disk it saves while one
  path suffices.

**For:**
- One way to draw the ground.
- The gain reaches every mesh, the city's included.
- No run-time cooking yet.

**Against:**
- The 2 m tiles stay cooked whole until option 1: about 1.3 GB after option 4 and
  #215's packing.

**The 1 m experiment (2026-10-09, before the decision).** `island --island-drawn 1`, nothing
committed:
- **The cost:**
  - the tiles take 9.6 GB packed, against 2.65 GB at 2 m;
  - a cold start takes 195 s, with 1 241 s of tile cooking work;
  - a warm start takes 5.4 s against 2.5 s, with 1.0 s for the rays' cuts;
  - the GPU frame costs 0.01–0.41 ms more on the same views;
  - the peak working set is 11.2 GB.
- **The gain:** small at walking height. The ground's look there comes from the layer map and
  the textures more than from the geometry.
- **Two things 1 m would need first:**
  - The rivers' channels and the lakes' shores are carved only into cells drawn finer than the
    grid. At 1 m there are none (`refined_cells=0`), so the near river lies under dry ground and
    a lake's shore is a cliff.
  - The ground's ray budget (`TERRAIN_BUDGET`) is fixed, so the rays' cut stands up to 107 m
    off the drawn ground (0.67 m at 2 m), and the shadows in the valleys change.

**Decided (the owner, 2026-10-09):** "keep 2 m, go with D-055 option 4". The ground stays drawn
at 2 m. The clusters' vertices are stored smaller, for every mesh: positions quantised to a grid
the mesh shares, so neighbouring clusters stay crack-free.

**Built (#218, 2026-10-09):** `forge_geom::page`'s packed payloads.
- **The grid:** every mesh's positions are snapped to a power-of-two step before its DAG is
  built: about 2^-17 of its reach, at most 1 mm (2^-10 m) and at least 2^-16 m. The DAG's
  bounds and errors are then those of the decoded positions, to the bit.
- **The payload:** each cluster holds a 16-byte header (its origin on the grid, the widths, the
  exponent), a 32-bit normal per vertex, and the offsets bit-packed. Bit 24 of `section` marks
  it. Skinned meshes keep 16-byte records, since the skin pass writes them every frame.
- **Exact everywhere:** the mesh shader, the fallback, the software rasteriser, the resolve and
  the rays' cuts on the CPU all decode the same integers times the same power of two. Mesh
  against fallback and the A/B harness stay at 0 px.

| Pages | Before | Packed |
|---|---|---|
| The island's 2 m tiles | 32 370 | 21 501 (66 %) |
| The island's 8 m tiles | 4 100 | 2 820 (69 %) |
| The other props | 8 685 | 5 502 (63 %) |
| The glTF models | 315 | 239 (76 %) |

- **The GPU:** the island's scene takes 2.74 GB of pages instead of 4.12 GB, and its start view
  33 MiB instead of 44. The frame costs 0.01–0.03 ms more (`docs/PROFILE.md`).
- **Disk:** the props drop 13 % (723 → 629 MB), but the 2 m tiles don't move (2.65 → 2.69 GB).
  #215's LZ4 had already taken the room out of the raw floats, so this proposal's "halve the
  disk" held only for the GPU. What remains on disk is the normals (32 bits) and the triangles
  (3 bytes each), which a later codec could take on.
- **The captures move** with the cooks: the positions by under half a millimetre, and the
  clusters' boundaries with them. The meshlets demo colours each cluster, so its views move by
  a ꟻLIP mean of 0.12–0.19; the others by 0.0002–0.035.

## D-056 — The planet: a cube sphere of cluster-DAG tiles, flown from orbit down to the island ✅ (proposed and decided 2026-10-09)

**The owner's pick (2026-10-09):** the island demo's second step, the planet variant, ahead of
the network and sound systems.

**Where things are:**
- **`forge-world`:** D-037's cube sphere, its `u64` cell ids, the clipmap's streaming plan and the
  residency are all unit-tested, but nothing draws them yet.
- **D-014:** names a CDLOD far field for planets. It was never built.
- **The island:** a flat 16 km square.
  - Its genesis runs at 8 m (2049²). Its ground is drawn at 2 m, in 2 km tiles of cluster DAGs
    made through the cache (#208) and packed (#215, #218).
  - Everything on it is flat, its sea at y = 0: the FFT sea's clipmap, the rivers, the lakes, the
    shore, the splashes, the physics' height fields and the walker.
- **The genesis:** drainage, flooding and erosion work on a flat grid (`Field2`, D8 neighbours).
- **The sky:** Hillaire's atmosphere at Earth's radius (6 360 km), with the planet-view table for
  the view from outside (#26). The ballad's planet is that disc seen from space. It has no ground.
- **The research** (`docs/research/planet-terrain.md`, 2026-09-26) recommends:
  - one cluster-DAG tile per cell of the clipmap, borders locked, skirts at level changes;
  - the six level-0 tiles always resident;
  - a coarse genesis on the six faces, amplified per tile as the tiles stream in;
  - one occlusion point per tile for the horizon;
  - a finer tile swapped in only once its parent's clusters there are under a pixel of error,
    each swap's ꟻLIP logged;
  - the island placed on a cell as an override of the coarse field.

**Proposed:** the research's recommendation, with these changes.
1. **D-014 amended:** the planet's ground is cluster-DAG tiles on the existing path (meshlets,
   rays, shadows, probes). No CDLOD, as D-055 already chose for the island's ground.
2. **Through what exists since:** each tile is made by `forge-terrain` through the derived cache
   (#208), keyed by the seed, the cell id and the code. It is packed (#215, #218): about a
   megabyte of pages a tile, where the research counted 3.4. The planet is described in
   `assets/worlds/planet.toml` (D-053).
3. **The ground before the geology:** the first build shapes the planet from seeded noise
   (continents, ranges, sea floor). The genesis across the six faces comes after, as its own
   step. The risks are in the drawing (pops, cracks, precision, streaming, the horizon), and the
   genesis on the faces is CPU work that can't show anything until the tiles draw.
4. **A `planet` demo of its own** (`cargo run -p planet`), sharing `city-blocks`' renderer as
   `island` does. It has a scripted descent with golden shots at 400 km, 10 km and the coast,
   `--radius`, and the F1 overlay's tile counters.

**The steps, an issue each:**
1. **The ground from noise, drawn and streamed:**
   - tiles of 257² samples with a halo, made on the job system;
   - roots at start, the clipmap's wants each frame, prefetch along the velocity;
   - the horizon test in the instance cull, and the near plane from the height over the ground;
   - the swap rule, with each swap's ꟻLIP logged;
   - the sea as a sphere at sea level, shaded as the far sea;
   - the atmosphere at the planet's radius.
2. **The island on the planet** (question 3).
3. **The sea's waves on the sphere:** the FFT clipmap around the camera, bent onto the sphere,
   so a low flight over open sea has waves.
4. **The genesis across the faces** (question 2): drainage, flooding and erosion with a neighbour
   rule across face edges. The tiles then amplify its coarse field.
5. **The descent's checks:** the ꟻLIP of every swap under D-017's thresholds, timings at the
   three altitudes, and the cells' digests at one and six workers.

**Question 1: the radius** (`--radius` changes it for a run either way).

| Radius | Face width | The island's flat error at its edge (8 km, corner 11 km) | Horizon from 2 m | Notes |
|---|---|---|---|---|
| **6 371 km (Earth)** | 10 000 km | 5 m (10 m) | 5.0 km | The sky is already Earth's. Two more levels of tiles. The flat island fits best (question 3). |
| 1 500 km (the research's example) | 2 360 km | 21 m (43 m) | 2.4 km | The curve shows from a plane. The coarse genesis is 1.2 km at 2049² a face. |
| 300–600 km (KSP's Kerbin is 600) | 470–940 km | 53–107 m | 1.1–1.5 km | Quick to fly round. The curve shows at the beach. The island would have to be bent onto the sphere. |

**Recommended: Earth's radius.** The sky and the sun's light are already tuned for it, and
the island's flatness is then within 5 m at its edge.

**Question 2: the rest of the planet.**
- **(a) Noise first, the genesis on the faces after (recommended):** the planet is visible after
  step 1. Mountain ranges and rivers across it come with step 4.
- **(b) The genesis first, as the research ordered:** continents with drainage from the start,
  but nothing to see until the tiles draw.
- **(c) Noise only:** no rivers or erosion outside the island, as the shipped planet games do.
  Cheapest, and the land away from the island looks generic.

**Question 3: the island on the sphere.**
- **(a) Flat in its own frame (recommended for now):** the island stays a flat 16 km square in a
  tangent frame on its cell, and every system on it works unchanged.
  - The planet's tiles leave a hole for it and bend to meet its rim over a 1 km band. Its sea
    joins the sphere's sea over the same band.
  - On Earth's radius the error is 5 m at the rim, below the ground's own detail from where its
    rim is seen.
- **(b) Bent onto the sphere:** the island is made in the planet's coordinates, its tiles too.
  - Right at any radius, and needed for a small planet or for detailed places side by side.
  - Every island system must then work on a curved surface: the sea, the rivers, the lakes, the
    shore, the splashes, the physics' height fields and the walker.

**Left out:**
- **Bending the flat island in the vertex shader** (games' "world curvature" trick): the rays'
  acceleration structures would keep the flat copy, so the shadows and reflections would not
  match the raster.
- **CBT or CDLOD:** a second ground path (D-055's option 2).
- **Caves and overhangs** (D-014's volumetric near field) wait for a place that needs them.

**For:**
- One way to draw ground, from orbit to the island's 2 m.
- Each step shows something.
- The island keeps everything it has.

**Against:**
- A flat island is right only on a large planet.
- Tiles made at run time cost CPU while flying. Their cache bounds it after the first visit.
- Step 1 alone touches the instance cull, the streaming, the sky's radius and the water's far
  shading.

**Decided (the owner, 2026-10-09):**
- **Question 1:** Earth's radius, 6 371 km.
- **Question 2:** noise first, the genesis on the faces later.
- **Question 3:** "either one or we can do it in a separate demo", then "I just prefer the
  planet (or maybe planets and suns in the future) to have their own demo". So the `planet`
  demo starts without the island, and later planets and suns go in it too. Any of the three
  ways is allowed, the vertex-shader curve included. Step 2 picks one by what it measures; the
  curve would need the rays' copy bent too.

The steps are issues: #220 (step 1). Steps 2 to 5 are filed when step 1 draws.

**Worlds at every scale (the owner, 2026-10-09, after asking how No Man's Sky, Elite
Dangerous, Star Citizen and X4 make their places):**
- **This demo:** one planet, maybe a moon, built the way Star Citizen builds its planets: rules
  and coarse maps for the whole body, detail and scattering at run time, made places set in. It
  is the reference frame for what follows.
- **A galaxy:** No Man's Sky's and Elite Dangerous's way, everything from a seed at run time,
  likely after the voxel work (D-014's near field).
- **Large detailed maps without a galaxy:** The Witcher 4's and Battlefield 6's kind, in
  styles from Zelda's to realistic, and voxel worlds like Enshrouded's. These are later targets.
- **The research:** `docs/research/worlds-at-every-scale.md`, written before the build goes on.
  It covers those games' latest techniques and the papers and repositories near them.

**Real maps for the coarse ground (the owner, 2026-10-09):** "if it helps you could download the
earth and moon data for this demo". With the owner's yes to the three files, the Earth's ground
comes from NOAA's ETOPO 2022 (60 arc-seconds) and the Moon's from NASA's CGI Moon Kit, both
public. Noise adds only the detail under their resolution. This is Star Citizen's way, coarse
maps with detail at run time. Noise alone remains (`map = false`), as decided, for planets with
no map.

**Built, step 1's first part (#220, 2026-10-10):** `cargo run -p planet`,
`docs/demos/planet.md`.
- **The tiles:** a fixed cut around the target, made and cooked through the cache. 492 tiles on
  the Earth, from level 1 to 14 (2.4 m samples).
- **The descent:** held at three golden shots, over the Mediterranean, the Côte d'Azur and Èze.
  The Moon is the same, at Tycho.
- **Frame cost:** 0.81–0.95 ms on the 5070 Ti at 1600 × 900.
- **What the renderer gained:**
  - instances at `f64` positions (`add_instance_at`);
  - the frame's planet for the layered ground (`set_planet`), with a body's colour map;
  - meshes the rays take as ground whatever their size (`set_ray_terrain`);
  - the sky's far pixels marched on their own (`march_beyond`);
  - a packing grid that coarsens past 2^20 m.
- **Left for step 1:** tiles streamed while flying, which needs a scene that can take and free
  meshes and instances at run time and a much cheaper cook of a tile. Then the swap rule and the
  horizon test.

**After the owner's review (2026-10-10):** "atmospheres rendering needs a bit of work to look more
real, and rendered terrain geometry as well (from orbit)", with a photograph of Corsica from
space, then "the sea / terrain borders are too marked", "why does the moon not looking
spherical?", and a real sky for space. With the owner's yes to two more downloads (the Blue
Marble, the Deep Star Maps):
- **From afar the maps stand for the ground:** the Blue Marble's colour, and the sea from a mask
  made from the elevation, instead of the coarse tiles' triangles. Water is shaded flat, and the
  layers' highlights blend in power as in strength, so the coasts lost their bright lines.
- **The elevation is read with Catmull-Rom.** Bilinear left flat facets a texel wide, such as a
  square pyramid for Tycho's central peak and bands of light on the Alps.
- **The ground's sky light** comes from tables at the ground under the camera when it is high.
- **The real sky** (`forge_render::SkyBox`) is turned by each body's pole and meridian.
- **Exposure:** sunny 16 by default. A metered exposure over a frame of black sky burned the
  Moon white.
- **The Moon's descent** starts 4 000 km out, from where it is seen whole. The offset from the
  target is capped at a tenth of the radius: on a small body the curve was seen from the side and
  stretched by the lens.
- **A tour** (`--tour`, 2026-10-10): the Earth's places from orbit to Èze and the Moon over the
  sea; the Moon from 4 000 km to Tycho and the Earth over it. Each body in the other's sky is a
  disc of its colour map, lit by the same sun (`forge_render::SkyBody`).
- **The tiles follow the camera** (2026-10-10, `docs/demos/planet.md`, "The tiles as the camera
  flies"): the cut is made again around where the camera will be and where the run heads, and a
  worker builds its whole scene, which the demo swaps in at a frame's start. The engine gained
  scenes over one shared texture set, one-shot copies on the transfer queue and structure builds
  on the compute queue, BLAS uploads in two copies, and per-instance transients that keep the
  graph's layout across scenes of a few tiles more or less. Staged uploads go through one
  buffer of at most 16 MiB: whole-buffer staging allocated host memory for each upload and stalled
  the frames by up to 50 ms a swap, now 7.7 ms on average.
- **The swaps checked** (`planet --check-swaps`, `tools/swap-check.sh`): each swap held still and
  its frames before and after compared with ꟻLIP. On the tour, every mean is at most 0.0027.
  Three swaps of 14 peak at 0.15–0.24 on a patch: the snow's shading near the camera over Mont
  Blanc as a cell splits.
  - **The patches are the normals' doing, not the heights'.** Where a cell splits, its
    children's heights differ by 0.13 px, so the research's swap criterion on the height's error
    holds, but their slopes differ by about 8°. The DAG also keeps each vertex's own normal as
    it simplifies, so a fine tile's coarse clusters sample its finest slopes.
  - **Wider rings** (2.3) moved the patches to the horizon at twice the tiles.
- **The horizon test** of step 1 is left out: the planet's whole instance cull takes 0.014 ms
  a frame, and the depth pyramid culls the far side.
- **A normal map per tile** (2026-10-10): 256² texels from the next level's height, mip-mapped,
  named by the tile's instance (the record's spare word) and read by the layered ground through
  the tile's UVs.
  - Of 11 swaps on the tour, 9 now pass both thresholds. The two that peak are the coarse
    geometry showing through the ambient occlusion and the shadows.
  - From orbit the Alps gained their ridges, and the snow lies on them instead of in blobs.
  - Its alpha carries the tile's coast, the height before the sea flattens it. Nearer than the
    sea mask reaches, the sea starts at that contour, read per pixel, rather than at the
    triangles' height: the owner's "very geometric non natural shapes" are gone from 10 km up
    and at Èze.
  - It costs 341 KB a tile, and cooking goes from 28 tiles a second to 19.
- **The tour's region at 90 m** (the owner's go, 2026-10-10: "download GLO-90 for the tour's
  region"): 25 tiles of the Copernicus DEM GLO-90 over 41°–47° N and 5°–10° E.
  - They're mosaicked into one grid. The world file names it as a region read over ETOPO
    (`[[map.regions]]`), faded in over a quarter of a degree, with its own noise from 180 m.
  - Its sea takes ETOPO's depths.
  - The Alps from the Mont Blanc stop gained their ridges, valleys and cliffs, and Èze its hills
    and Cap Ferrat.
  - Credit: "produced using Copernicus WorldDEM-90 © DLR e.V. 2010-2014 and © Airbus Defence and
    Space GmbH 2014-2018 provided under COPERNICUS by the European Union and ESA; all rights
    reserved" (`CREDITS.md`).
- **The scene takes and frees tiles in place** (2026-10-10, the owner's "go on with the in-place
  scene"; `docs/research/planet-terrain.md`'s design, built): a streamed scene reserves its
  tables at a capacity (`MeshletSceneBuilder::reserve_dynamic`), and the demo's worker edits it
  (`forge_render::SceneEditor`): a new tile's records and rays' cut into free ranges no frame
  reads, its structure and the next top-level one built beside the frames, its pages read; the
  frame takes the edit in at its start (`MeshletScene::apply`) and copies the rest in a
  `scene/edit` pass. A removed tile's slot is vacant, and what it held is freed three frames
  later.
  - An edit of up to 10 tiles is ready 11 ms after it is asked for, where a whole scene took
    0.39 s; the longest frame between changes is 2.4 ms on average, against 9.4.
  - The ground shot reached by an edit draws what the scene built whole draws, 0 pixels apart
    (`planet --edited`, a pair of the capture batch).
  - The rays cut each tile on its own (4 000 triangles) rather than each level's tiles as one
    surface, so a tile's cut does not depend on its neighbours. The planet's shadows moved a
    little: the planet's captures were accepted again.
  - `--resident` and a cut an edit has no room for still build a whole scene.
- **The swap rule** (2026-10-10, the owner's "go on with the swap rule"; `docs/demos/planet.md`,
  "The swap rule"): the cut splits a cell where a split would show, not within `rings` of the
  camera. A cell's error is the largest height its children add (sampled 9 × 9) and its
  triangles' sag; it splits where its samples would stand over 2.5 px apart on ground whose
  children add slopes of 1° or more, or where their height would show over a pixel. The worker
  makes the cut every 50 ms and keeps the errors it worked out.
  - Around the camera, rough ground keeps `rings`' tiles (Mont Blanc 390 against 375); the sea
    and the plains take fewer (Corsica from orbit 114 against 159). The held shots differ from
    `rings` by a ꟻLIP mean of 0.0003–0.0033 on the Earth, 0.008 on the Moon's floor.
  - It does not remove the pops: 4 of 14 changes on the tour still peak at 0.17–0.79, the share
    `rings` had, and 10 of 35 without ambient occlusion and shadows. A split brings its
    children's normal maps' finer band, magnified 2.5 times in the parent's; splitting at a
    pixel would take three times the tiles.
- **The first frame of a change** (`--check-swaps` saves it, a jitter period after the one
  before): of 21 changes on the tour none jumps over 0.13, where 4 peak at 0.15–0.21 settled. TAA
  takes a split in over several frames, so it is a short blend already; a longer one (the parent
  dithered into its children) is left until a view shows it is needed.
- **The haze** (2026-10-10, the owner's "a light fog" even near the ground): the world file's
  `[view] haze`, the air between the camera and the ground that many times as dense (its
  transmittance raised to it, its light scaled to match); the Earth at 0.5. The air is
  Hillaire's Earth, yet from 6 km over the Alps it laid as much blue over the grass as the sky's
  own. At Èze the fog is mostly AgX's greyish sky (D-045's curve, the owner's to change).
- **Faster new tiles** (2026-10-10, `docs/demos/planet.md`, "A new tile"): a fine tile waits
  0.14 s rather than 0.46 (0.27 s of one core; its heights spread over the threads); the caches'
  stores no longer scan their directories; the normal map's unread samples left out and the
  noise's seeds kept (the same bits); the DAG's first level cut into the grid's 7 × 7 cells (the
  same errors and GPU frame, other clusters). A DAG template shared by every tile cooked in 23 ms
  but erred 7–10 times as much: set aside.
- **Next:** a tile in milliseconds still (its DAG 0.11 s on one thread, its normal map 0.12 s of
  one core).

**Proposed from the research 🟡** (`docs/research/worlds-at-every-scale.md`, 2026-10-10; for the
owner's yes, nothing built on it):
- **Every scale shares one structure:**
  - coarse fields that are cheap to keep;
  - a deterministic producer that amplifies them per cell;
  - rules that read them for materials and placement;
  - places stamped in;
  - edits kept as a log.
  Only the coarse fields, the places, the rules and the parts ship. Tiles and placements are
  derived data in a capped cache.
- **D-056 gains a step 1b, the planet atlas,** before step 4:
  - per cube face, 1025² or 2049² samples of height, temperature, humidity, rock, soil depth and a
    biome;
  - painted overrides in the world file, as Star Citizen's brushes;
  - biomes as rules (`[[biome]]`: layers, rock and flora sets, densities by slope, soil and
    humidity, scatter cell sizes);
  - places (`[[place]]`: a footprint, a height rule, its own content). The island becomes the
    first place.
  The Earth's and the Moon's real maps are already the atlas's height.
- **D-014:** the volumetric near field and the SDF bricks are per cell and only where caves,
  overhangs or edits need them. An edit is a small CSG operation appended to the log of the cells
  it touches, replayed deterministically and compacted into bricks.
- **D-016 gains a rule for the GPU:** a GPU generator becomes authoritative only as integer noise
  or as float code with round-to-nearest, a fixed denormal mode, no contraction and no division
  or transcendental, checked by digests on two vendors.
- **For a galaxy:** a 64-bit body id in Elite's shape first (sector, layer, system, body), with
  each body's record made top-down. Voxels only if the game digs.
